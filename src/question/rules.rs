//! The `question` rule family evaluated by `tophat check`.

use std::collections::{BTreeMap, HashMap};

use super::{Citation, HeadingIssue, PROJECT_FILE, QuestionStatus, QuestionTree, parse_project};
use crate::check::{Diagnostic, Project, Severity};
use crate::study::{
    Epistemic, Mode, STUDIES_DIR, STUDY_FILE, SUMMARY_FILE, Status, Study, TagProblem,
    classify_tag, is_valid_question_id, is_valid_study_id, parse_study_text,
};

/// A study as seen from the question tree.
pub(super) enum StudyEntry {
    Parsed(Box<Study>),
    Unparseable,
}

/// Loads every study directory that contains a `STUDY.md`, keyed by study ID.
pub(super) fn load_studies(project: &Project) -> BTreeMap<String, StudyEntry> {
    let mut studies = BTreeMap::new();
    for id in project.list_dir(STUDIES_DIR).unwrap_or_default() {
        let file = format!("{STUDIES_DIR}/{id}/{STUDY_FILE}");
        if !project.is_file(&file) {
            continue;
        }
        let entry = match project
            .read_to_string(&file)
            .ok()
            .and_then(|text| parse_study_text(&id, &file, &text).ok())
        {
            Some(study) => StudyEntry::Parsed(Box::new(study)),
            None => StudyEntry::Unparseable,
        };
        studies.insert(id, entry);
    }
    studies
}

/// Reads and parses `PROJECT.md`. A missing file is an empty tree; unreadable text is an error.
pub(super) fn load_tree(project: &Project) -> Result<QuestionTree, String> {
    if !project.is_file(PROJECT_FILE) {
        return Ok(QuestionTree::default());
    }
    let bytes = project.read(PROJECT_FILE).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| "the file is not valid UTF-8".to_string())?;
    Ok(parse_project(&text))
}

fn diag(
    rule: &'static str,
    severity: Severity,
    path: &str,
    line: Option<u32>,
    message: String,
    hint: &str,
) -> Diagnostic {
    Diagnostic {
        rule,
        severity,
        path: path.to_string(),
        line,
        message,
        hint: Some(hint.to_string()),
    }
}

fn error(rule: &'static str, line: u32, message: String, hint: &str) -> Diagnostic {
    diag(
        rule,
        Severity::Error,
        PROJECT_FILE,
        Some(line),
        message,
        hint,
    )
}

// rq-cc797bfb rq-ceffcdab
/// The `question` rule family.
pub fn question_rules(project: &Project) -> Vec<Diagnostic> {
    let tree = match load_tree(project) {
        Ok(tree) => tree,
        Err(message) => {
            return vec![diag(
                "question.parse-error",
                Severity::Error,
                PROJECT_FILE,
                None,
                format!("{PROJECT_FILE} cannot be read: {message}"),
                "Save PROJECT.md as UTF-8 text",
            )];
        }
    };
    let studies = load_studies(project);
    let mut out = Vec::new();
    check_structure(&tree, &mut out);
    check_bodies(&tree, &mut out);
    check_origins(&tree, &studies, &mut out);
    check_evidence(&tree, &studies, project, &mut out);
    check_study_links(&tree, &studies, &mut out);
    out
}

// rq-8148f10b
fn check_structure(tree: &QuestionTree, out: &mut Vec<Diagnostic>) {
    for issue in &tree.issues {
        out.push(match issue {
            HeadingIssue::Unstamped { line } => error(
                "question.unstamped",
                *line,
                "Question heading has no ID".into(),
                "Run `tophat question stamp` to assign IDs",
            ),
            HeadingIssue::MalformedId { line, token } => error(
                "question.id-invalid",
                *line,
                format!("`{token}` is not a valid question ID"),
                "Use `q-` followed by 8 lowercase hex characters, or remove it and re-stamp",
            ),
            HeadingIssue::TextEmpty { line } => error(
                "question.text-empty",
                *line,
                "Question heading has no question text".into(),
                "Write the question after the ` · ` separator",
            ),
            HeadingIssue::LevelSkip { line } => error(
                "question.level-skip",
                *line,
                "Question heading skips a level".into(),
                "Start with `##` and go at most one level deeper than the previous question",
            ),
        });
    }
    let mut first_seen: HashMap<&str, u32> = HashMap::new();
    for q in &tree.questions {
        if let Some(id) = q.id.as_deref() {
            if let Some(first) = first_seen.get(id) {
                out.push(error(
                    "question.duplicate-id",
                    q.heading_line,
                    format!("Question ID `{id}` is already used on line {first}"),
                    "Give this question a different ID (remove it and re-stamp)",
                ));
            } else {
                first_seen.insert(id, q.heading_line);
            }
        }
    }
}

// rq-76dbbbfa
fn check_bodies(tree: &QuestionTree, out: &mut Vec<Diagnostic>) {
    for q in &tree.questions {
        let Some((value, status_line)) = &q.status else {
            out.push(error(
                "question.status-missing",
                q.heading_line,
                "Question has no status line".into(),
                "Put `status: open`, `answered` or `dropped` on the first line after the heading",
            ));
            continue;
        };
        match QuestionStatus::parse(value) {
            None => out.push(error(
                "question.status-invalid",
                *status_line,
                format!("`status: {value}` is not a valid question status"),
                "Use open, answered or dropped",
            )),
            Some(QuestionStatus::Answered) => {
                if q.answer.as_ref().is_none_or(|(a, _)| a.is_empty()) {
                    out.push(error(
                        "question.answer-missing",
                        q.answer.as_ref().map_or(q.heading_line, |(_, l)| *l),
                        "Answered question has no `**Answer:**` line with text".into(),
                        "Add `**Answer:** <the answer>`",
                    ));
                }
                if q.evidence.as_ref().is_none_or(|(c, _)| c.is_empty()) {
                    out.push(error(
                        "question.evidence-missing",
                        q.evidence.as_ref().map_or(q.heading_line, |(_, l)| *l),
                        "Answered question cites no evidence".into(),
                        "Add `Evidence: <study IDs and/or sub-question IDs>`",
                    ));
                }
            }
            Some(QuestionStatus::Dropped) => {
                if q.reason.as_ref().is_none_or(|(r, _)| r.is_empty()) {
                    out.push(error(
                        "question.reason-missing",
                        q.reason.as_ref().map_or(q.heading_line, |(_, l)| *l),
                        "Dropped question has no `**Reason:**` line with text".into(),
                        "Add `**Reason:** <why the question was dropped>`",
                    ));
                }
            }
            Some(QuestionStatus::Open | QuestionStatus::Emerged) => {}
        }
    }
}

// rq-7e178b97
fn check_origins(
    tree: &QuestionTree,
    studies: &BTreeMap<String, StudyEntry>,
    out: &mut Vec<Diagnostic>,
) {
    for q in &tree.questions {
        match &q.origin {
            None if q.parsed_status() == Some(QuestionStatus::Emerged) => out.push(error(
                "question.origin-missing",
                q.heading_line,
                "Emerged question has no `Emerged from:` line".into(),
                "Add `Emerged from: <study-id>`, or record it with `tophat question emerge`",
            )),
            Some((value, line)) if !is_valid_study_id(value) || !studies.contains_key(value) => {
                out.push(error(
                    "question.origin-invalid",
                    *line,
                    format!("`Emerged from: {value}` names no existing study"),
                    "Name an existing study's directory",
                ));
            }
            _ => {}
        }
    }
}

// rq-ef5ab539
fn check_evidence(
    tree: &QuestionTree,
    studies: &BTreeMap<String, StudyEntry>,
    project: &Project,
    out: &mut Vec<Diagnostic>,
) {
    for (index, q) in tree.questions.iter().enumerate() {
        if q.parsed_status() != Some(QuestionStatus::Answered) {
            continue;
        }
        let Some((citations, line)) = &q.evidence else {
            continue;
        };
        let line = *line;
        for citation in citations {
            let target = &citation.target;
            if is_valid_question_id(target) {
                if citation.label.is_some() {
                    out.push(error(
                        "question.evidence-invalid",
                        line,
                        format!("Question citation `{target}` cannot carry a label"),
                        "Question citations are bare question IDs",
                    ));
                    continue;
                }
                check_question_citation(tree, index, target, line, out);
            } else if is_valid_study_id(target) {
                if let Some(label) = &citation.label
                    && !matches!(label.as_str(), "exploratory" | "post hoc")
                {
                    out.push(error(
                        "question.evidence-invalid",
                        line,
                        format!("`{label}` is not a citation label"),
                        "Labels are `exploratory` or `post hoc`; an unlabelled citation cites pre-specified results",
                    ));
                    continue;
                }
                check_study_citation(tree, index, citation, line, studies, project, out);
            } else {
                out.push(error(
                    "question.evidence-invalid",
                    line,
                    format!("`{target}` is neither a question ID nor a study ID"),
                    "Cite study IDs or sub-question IDs, separated by commas",
                ));
            }
        }
    }
}

fn check_question_citation(
    tree: &QuestionTree,
    citing: usize,
    citation: &str,
    line: u32,
    out: &mut Vec<Diagnostic>,
) {
    let Some(cited) = tree.index_of(citation) else {
        out.push(error(
            "question.evidence-invalid",
            line,
            format!("Cited question `{citation}` does not exist"),
            "Cite an existing sub-question",
        ));
        return;
    };
    if !tree.is_descendant(cited, citing) {
        out.push(error(
            "question.evidence-invalid",
            line,
            format!("Cited question `{citation}` is not a sub-question of this question"),
            "Only sub-questions can serve as evidence",
        ));
        return;
    }
    if tree.questions[cited].parsed_status() != Some(QuestionStatus::Answered) {
        out.push(error(
            "question.evidence-unanswered",
            line,
            format!("Cited question `{citation}` is not answered"),
            "Answer the sub-question first, or cite other evidence",
        ));
    }
}

/// The distinct tags a study's result lines carry, or `None` when a line has no valid tag group.
fn study_result_tags(study: &Study) -> Option<Vec<Epistemic>> {
    let lines = study.result_lines();
    if lines.is_empty() {
        return None;
    }
    let mut tags = Vec::new();
    for (_, raw) in lines {
        match classify_tag(&raw) {
            Ok((_, tag)) => {
                if !tags.contains(&tag.epistemic) {
                    tags.push(tag.epistemic);
                }
            }
            Err(TagProblem::Missing | TagProblem::Invalid | TagProblem::InvalidSplit(_)) => {
                return None;
            }
        }
    }
    Some(tags)
}

fn check_study_citation(
    tree: &QuestionTree,
    citing: usize,
    citation: &Citation,
    line: u32,
    studies: &BTreeMap<String, StudyEntry>,
    project: &Project,
    out: &mut Vec<Diagnostic>,
) {
    let citation_id = &citation.target;
    let study = match studies.get(citation_id) {
        None => {
            out.push(error(
                "question.evidence-invalid",
                line,
                format!("Cited study `{citation_id}` does not exist"),
                "Cite an existing study by its directory name",
            ));
            return;
        }
        Some(StudyEntry::Unparseable) => return,
        Some(StudyEntry::Parsed(study)) => study,
    };
    match study.status() {
        Some(Status::Reported) => {}
        Some(status @ (Status::Draft | Status::Preregistered)) => {
            out.push(error(
                "question.evidence-unreported",
                line,
                format!("Cited study `{citation_id}` is {status}, not reported"),
                "Cite only reported studies",
            ));
            return;
        }
        Some(Status::Superseded) => {
            let hint = match &study.front.superseded_by {
                Some(by) => format!("Cite the replacing study `{}` instead", by.value),
                None => "Cite the study that replaced it instead".to_string(),
            };
            out.push(error(
                "question.evidence-superseded",
                line,
                format!("Cited study `{citation_id}` has been superseded"),
                &hint,
            ));
            return;
        }
        None => return,
    }
    let summary = format!("{STUDIES_DIR}/{citation_id}/{SUMMARY_FILE}");
    if study.mode() == Mode::Retrospective && !project.is_file(&summary) {
        out.push(diag(
            "question.evidence-retrospective",
            Severity::Legacy,
            PROJECT_FILE,
            Some(line),
            format!(
                "Cited study `{citation_id}` is retrospective and its results are hand-written and unverified"
            ),
            "Reproduce it with a pipeline, then cite its verified summary",
        ));
    } else if let Some(tags) = study_result_tags(study) {
        let required = match citation.label.as_deref() {
            None => Epistemic::PreSpecified,
            Some("exploratory") => Epistemic::Exploratory,
            Some("post hoc") => Epistemic::PostHoc,
            Some(_) => unreachable!("labels checked before"),
        };
        if !tags.contains(&required) {
            let present: Vec<&str> = tags.iter().map(|t| t.as_str()).collect();
            out.push(error(
                "question.evidence-tag-mismatch",
                line,
                format!(
                    "Cited study `{citation_id}` has no result line tagged `{}`",
                    required.as_str()
                ),
                &format!("The study's result lines carry: {}", present.join(", ")),
            ));
        }
    }
    let study_question = study.front.question.as_ref().map(|f| f.value.as_str());
    let related = study_question
        .and_then(|id| tree.index_of(id))
        .is_some_and(|i| i == citing || tree.is_descendant(i, citing));
    if !related {
        out.push(diag(
            "question.evidence-unrelated",
            Severity::Warning,
            PROJECT_FILE,
            Some(line),
            format!(
                "Cited study `{citation_id}` answers `{}`, which is not this question or below it",
                study_question.unwrap_or("?")
            ),
            "Check that the right study is cited",
        ));
    }
}

// rq-a147f8cf
fn check_study_links(
    tree: &QuestionTree,
    studies: &BTreeMap<String, StudyEntry>,
    out: &mut Vec<Diagnostic>,
) {
    for (id, entry) in studies {
        let StudyEntry::Parsed(study) = entry else {
            continue;
        };
        let Some(field) = &study.front.question else {
            continue;
        };
        if is_valid_question_id(&field.value) && tree.index_of(&field.value).is_none() {
            out.push(diag(
                "question.unknown-reference",
                Severity::Error,
                &format!("{STUDIES_DIR}/{id}/{STUDY_FILE}"),
                Some(field.line),
                format!(
                    "Question `{}` does not exist in {PROJECT_FILE}",
                    field.value
                ),
                "Add the question to PROJECT.md, or fix the study's `question` field",
            ));
        }
    }
}
