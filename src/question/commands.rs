//! The `tophat question` commands: `stamp` and `tree`.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::rules::{StudyEntry, load_studies, load_tree};
use super::{HeadingId, PROJECT_FILE, QuestionError, QuestionTree, heading_id, parse_project};
use crate::check::Project;
use crate::fsutil::write_atomic;
use crate::markdown::atx_heading;
use crate::study::STUDIES_DIR;

/// What `stamp` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StampOutcome {
    /// `(id, text)` of every question that received an ID.
    pub assigned: Vec<(String, String)>,
    /// Malformed or duplicated IDs left unchanged.
    pub unresolved: Vec<String>,
}

/// A random question ID. Not cryptographic; callers check uniqueness.
fn random_id(attempt: u64) -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(attempt);
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    format!("q-{:08x}", hasher.finish() as u32)
}

/// Splits off a line's ending (`\n`, `\r\n`, or none).
fn split_eol(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}

/// Malformed IDs and IDs used more than once, in first-occurrence order.
fn unresolved_ids(text: &str, tree: &QuestionTree) -> Vec<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for q in &tree.questions {
        if let Some(id) = q.id.as_deref() {
            *counts.entry(id).or_default() += 1;
        }
    }
    let mut out: Vec<String> = Vec::new();
    for q in &tree.questions {
        let (line, _) = split_eol(lines[q.heading_line as usize - 1]);
        let token = match atx_heading(line).map(|(_, t)| heading_id(t)) {
            Some(HeadingId::Malformed { token, .. }) => Some(token),
            Some(HeadingId::Stamped { id, .. }) if counts[id] > 1 => Some(id),
            _ => None,
        };
        if let Some(token) = token
            && !out.iter().any(|t| t == token)
        {
            out.push(token.to_string());
        }
    }
    out
}

/// Performs `tophat question stamp` and reports what it did, including IDs it left unresolved.
pub fn stamp_outcome(root: &Path) -> Result<StampOutcome, QuestionError> {
    let path = root.join(PROJECT_FILE);
    if !path.is_file() {
        return Err(QuestionError::ProjectFileMissing);
    }
    let text = fs::read_to_string(&path)?;
    let tree = parse_project(&text);
    let mut outcome = StampOutcome {
        assigned: Vec::new(),
        unresolved: unresolved_ids(&text, &tree),
    };

    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    let default_eol = if lines.first().is_some_and(|l| l.ends_with("\r\n")) {
        "\r\n"
    } else {
        "\n"
    };
    let mut used: HashSet<String> = tree.questions.iter().filter_map(|q| q.id.clone()).collect();
    let mut status_after: Vec<usize> = Vec::new();
    let mut attempt = 0u64;

    for q in &tree.questions {
        let index = q.heading_line as usize - 1;
        let (content, eol) = split_eol(&lines[index]);
        let Some((level, heading_text)) = atx_heading(content) else {
            continue;
        };
        let stamped = match heading_id(heading_text) {
            HeadingId::Malformed { .. } => false,
            HeadingId::Stamped { .. } => true,
            HeadingId::Unstamped => {
                let id = loop {
                    attempt += 1;
                    let candidate = random_id(attempt);
                    if used.insert(candidate.clone()) {
                        break candidate;
                    }
                };
                let indent = content.len() - content.trim_start_matches(' ').len();
                let rewritten = format!(
                    "{}{} {id} {} {heading_text}{eol}",
                    &content[..indent],
                    "#".repeat(level as usize),
                    super::SEPARATOR
                );
                outcome.assigned.push((id, heading_text.to_string()));
                lines[index] = rewritten;
                true
            }
        };
        if stamped && q.status.is_none() {
            status_after.push(index);
        }
    }

    for &index in status_after.iter().rev() {
        if !lines[index].ends_with('\n') {
            lines[index].push_str(default_eol);
        }
        lines.insert(index + 1, format!("status: open{default_eol}"));
    }

    if !outcome.assigned.is_empty() || !status_after.is_empty() {
        write_atomic(&path, &lines.concat())?;
    }
    Ok(outcome)
}

// rq-ef3d43f8 rq-1513da11
/// Performs `tophat question stamp`, returning the `(id, text)` pairs it assigned. Changes are
/// written even when malformed or duplicated IDs remain; those are returned as
/// [`QuestionError::Unresolved`].
pub fn stamp(root: &Path) -> Result<Vec<(String, String)>, QuestionError> {
    let outcome = stamp_outcome(root)?;
    if outcome.unresolved.is_empty() {
        Ok(outcome.assigned)
    } else {
        Err(QuestionError::Unresolved(outcome.unresolved))
    }
}

/// Studies attached to each question ID: `(study id, status)`, sorted by study ID.
type AttachedStudies = BTreeMap<String, Vec<(String, String)>>;

/// The tree with the studies attached to each question.
fn tree_and_studies(root: &Path) -> Result<(QuestionTree, AttachedStudies), QuestionError> {
    let project = Project::new(root);
    let tree = load_tree(&project).map_err(QuestionError::Io)?;
    let mut attached = AttachedStudies::new();
    for (id, entry) in load_studies(&project) {
        let StudyEntry::Parsed(study) = entry else {
            continue;
        };
        if let Some(question) = &study.front.question {
            let status = study.status().map_or("?", |s| s.as_str()).to_string();
            attached
                .entry(question.value.clone())
                .or_default()
                .push((id, status));
        }
    }
    Ok((tree, attached))
}

// rq-5d07156c
/// Renders `tophat question tree` as indented text.
pub fn render_tree(root: &Path) -> Result<String, QuestionError> {
    let (tree, attached) = tree_and_studies(root)?;
    let statuses = study_statuses(root);
    let mut out = String::new();
    for q in &tree.questions {
        let indent = "  ".repeat(q.level.saturating_sub(2) as usize);
        let id = q.id.as_deref().unwrap_or("(no id)");
        let status = q.parsed_status().map_or("?", |s| s.as_str());
        out.push_str(&format!("{indent}{id} [{status}] {}\n", q.text));
        for (study, study_status) in
            q.id.as_ref()
                .and_then(|id| attached.get(id))
                .into_iter()
                .flatten()
        {
            out.push_str(&format!("{indent}  study {study} [{study_status}]\n"));
        }
        if let Some((origin, _)) = &q.origin {
            let origin_status = statuses.get(origin).map_or("?", String::as_str);
            out.push_str(&format!(
                "{indent}  emerged from {origin} [{origin_status}]\n"
            ));
        }
    }
    Ok(out)
}

/// The status of every parseable study, by ID.
fn study_statuses(root: &Path) -> BTreeMap<String, String> {
    let project = Project::new(root);
    load_studies(&project)
        .into_iter()
        .filter_map(|(id, entry)| match entry {
            StudyEntry::Parsed(study) => {
                Some((id, study.status().map_or("?", |s| s.as_str()).to_string()))
            }
            StudyEntry::Unparseable => Some((id, "?".to_string())),
        })
        .collect()
}

// rq-5d07156c
/// Renders `tophat question tree --json`.
pub fn render_tree_json(root: &Path) -> Result<String, QuestionError> {
    let (tree, attached) = tree_and_studies(root)?;
    let statuses = study_statuses(root);
    let questions: Vec<Value> = tree
        .questions
        .iter()
        .map(|q| {
            let studies: Vec<Value> =
                q.id.as_ref()
                    .and_then(|id| attached.get(id))
                    .into_iter()
                    .flatten()
                    .map(|(id, status)| json!({ "id": id, "status": status }))
                    .collect();
            let origin = q
                .origin
                .as_ref()
                .map(|(id, _)| json!({"id": id, "status": statuses.get(id).cloned()}));
            json!({
                "id": q.id,
                "text": q.text,
                "status": q.parsed_status().map(|s| s.as_str()),
                "level": q.level,
                "parent": q.parent.and_then(|p| tree.questions[p].id.clone()),
                "line": q.heading_line,
                "studies": studies,
                "origin": origin,
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&json!({ "questions": questions }))
        .expect("JSON values always serialize");
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-1513da11
    fn random_ids_are_well_formed() {
        for attempt in 0..100 {
            assert!(crate::study::is_valid_question_id(&random_id(attempt)));
        }
    }

    #[test]
    fn eol_splitting() {
        assert_eq!(split_eol("a\r\n"), ("a", "\r\n"));
        assert_eq!(split_eol("a\n"), ("a", "\n"));
        assert_eq!(split_eol("a"), ("a", ""));
    }
}

/// Splits `text` into lines keeping their endings.
fn text_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

// rq-a88520bf rq-7449b1fb
/// Performs `tophat question emerge`: records a question born from a study's results.
pub fn emerge(
    root: &Path,
    text: &str,
    from: &str,
    under: Option<&str>,
) -> Result<(String, String), QuestionError> {
    let path = root.join(PROJECT_FILE);
    if !path.is_file() {
        return Err(QuestionError::ProjectFileMissing);
    }
    let question_text = text.trim();
    if question_text.is_empty() {
        return Err(QuestionError::TextInvalid);
    }
    let project = Project::new(root);
    let study_file = format!("{STUDIES_DIR}/{from}/{}", study_file_name());
    let study = project
        .read_to_string(&study_file)
        .ok()
        .and_then(|t| crate::study::parse_study_text(from, &study_file, &t).ok())
        .ok_or_else(|| QuestionError::OriginStudyInvalid(from.to_string()))?;

    let source = fs::read_to_string(&path)?;
    let tree = parse_project(&source);
    let parent_id = under
        .map(str::to_string)
        .or_else(|| study.front.question.as_ref().map(|f| f.value.clone()))
        .ok_or_else(|| QuestionError::ParentInvalid("(none)".to_string()))?;
    let parent_index = tree
        .index_of(&parent_id)
        .ok_or_else(|| QuestionError::ParentInvalid(parent_id.clone()))?;
    let parent = &tree.questions[parent_index];
    if parent.level >= 6 {
        return Err(QuestionError::ParentTooDeep);
    }

    let mut used: HashSet<String> = tree.questions.iter().filter_map(|q| q.id.clone()).collect();
    let id = loop {
        let candidate = random_id(used.len() as u64 + 1);
        if used.insert(candidate.clone()) {
            break candidate;
        }
    };

    let mut lines = text_lines(&source);
    let eol = if lines.first().is_some_and(|l| l.ends_with("\r\n")) {
        "\r\n"
    } else {
        "\n"
    };
    // The insertion point: before the next heading at most the parent's level, or at the end.
    let mut insert_at = lines.len();
    let mut fences = crate::markdown::FenceTracker::default();
    for (index, line) in lines.iter().enumerate() {
        let bare = line.trim_end_matches(['\n', '\r']);
        let state = fences.next(bare);
        if state == crate::markdown::FenceState::Outside
            && let Some((level, _)) = crate::markdown::atx_heading(bare)
            && index + 1 > parent.heading_line as usize
            && level <= parent.level
        {
            insert_at = index;
            break;
        }
    }
    if insert_at > 0 && !lines[insert_at - 1].ends_with('\n') {
        lines[insert_at - 1].push_str(eol);
    }
    let level = parent.level + 1;
    let block = vec![
        format!(
            "{} {id} {} {question_text}{eol}",
            "#".repeat(level as usize),
            super::SEPARATOR
        ),
        format!("status: emerged{eol}"),
        format!("Emerged from: {from}{eol}"),
        eol.to_string(),
    ];
    lines.splice(insert_at..insert_at, block);
    write_atomic(&path, &lines.concat())?;
    Ok((id, question_text.to_string()))
}

const fn study_file_name() -> &'static str {
    crate::study::STUDY_FILE
}
