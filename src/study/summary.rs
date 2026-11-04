//! Parsing of the generated `SUMMARY.md`.

use std::fs;
use std::path::Path;

use yaml_rust2::Yaml;

use super::document::load_front_matter;
use super::{StudyError, matches_id_pattern};
use crate::markdown::{meaningful_lines, numbered_lines, sections, split_front_matter};

/// One result line: `- <variant>/<stage>: <text>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultLine {
    pub variant: String,
    pub stage: String,
    pub text: String,
    /// The whole line as written (trailing whitespace stripped); what gets copied into STUDY.md.
    pub raw: String,
    /// 1-based line in SUMMARY.md.
    pub line: u32,
}

// rq-6764653f rq-09eda840
/// A parsed `SUMMARY.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub study: String,
    pub commit: String,
    pub dirty: bool,
    /// 1-based line of the `dirty` field.
    pub dirty_line: u32,
    pub generated: String,
    pub tophat_version: String,
    /// `<variant>/<stage>` → the stage key each result line was read from, when recorded.
    pub keys: Option<std::collections::BTreeMap<String, String>>,
    /// Whether the study's plan was informed by its deciding data; `false` when absent.
    pub deciding_data_seen: bool,
    pub results: Vec<ResultLine>,
}

/// Parses a result line, or returns why it is malformed.
fn parse_result_line(line: u32, raw: &str) -> Result<ResultLine, String> {
    let malformed = || format!("line {line}: expected `- <variant>/<stage>: <text>`");
    let rest = raw.strip_prefix("- ").ok_or_else(malformed)?;
    let (key, text) = rest.split_once(": ").ok_or_else(malformed)?;
    let (variant, stage) = key.split_once('/').ok_or_else(malformed)?;
    if !matches_id_pattern(variant) || !matches_id_pattern(stage) || text.trim().is_empty() {
        return Err(malformed());
    }
    Ok(ResultLine {
        variant: variant.to_string(),
        stage: stage.to_string(),
        text: text.trim().to_string(),
        raw: raw.to_string(),
        line,
    })
}

/// Parses the text of a `SUMMARY.md`. Errors are human-readable descriptions.
pub(super) fn parse_summary_text(text: &str) -> Result<Summary, String> {
    let lines = numbered_lines(text);
    let split = split_front_matter(&lines)?;
    let loaded = load_front_matter(&split.front, 1).map_err(|(line, message)| match line {
        Some(line) => format!("line {line}: {message}"),
        None => message,
    })?;

    let mut study = None;
    let mut commit = None;
    let mut dirty = None;
    let mut generated = None;
    let mut tophat_version = None;
    let mut keys = None;
    let mut deciding_data_seen = false;
    for (key, value) in &loaded.entries {
        match key.as_str() {
            "study" => study = Some(loaded.scalar(key, value)?),
            "commit" => commit = Some(loaded.scalar(key, value)?),
            "generated" => generated = Some(loaded.scalar(key, value)?),
            "tophat_version" => tophat_version = Some(loaded.scalar(key, value)?),
            "deciding_data_seen" => match value {
                Yaml::Boolean(b) => deciding_data_seen = *b,
                _ => return Err("field `deciding_data_seen` must be `true` or `false`".into()),
            },
            "dirty" => match value {
                Yaml::Boolean(b) => dirty = Some(*b),
                _ => return Err("field `dirty` must be `true` or `false`".into()),
            },
            "keys" => match value {
                Yaml::Hash(hash) => {
                    let mut map = std::collections::BTreeMap::new();
                    for (k, v) in hash {
                        match (k, v) {
                            (Yaml::String(k), Yaml::String(v)) => {
                                map.insert(k.clone(), v.clone());
                            }
                            _ => {
                                return Err(
                                    "field `keys` must map `<variant>/<stage>` to keys".into()
                                );
                            }
                        }
                    }
                    keys = Some(map);
                }
                _ => return Err("field `keys` must be a mapping".into()),
            },
            _ => {}
        }
    }
    let missing = |name: &str| format!("field `{name}` is missing");
    let study = study.ok_or_else(|| missing("study"))?;
    let commit = commit.ok_or_else(|| missing("commit"))?;
    let dirty = dirty.ok_or_else(|| missing("dirty"))?;
    let generated = generated.ok_or_else(|| missing("generated"))?;
    let tophat_version = tophat_version.ok_or_else(|| missing("tophat_version"))?;

    if commit.len() != 40
        || !commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("field `commit` must be 40 lowercase hex characters".into());
    }
    if chrono::DateTime::parse_from_rfc3339(&generated).is_err() {
        return Err("field `generated` must be an RFC 3339 timestamp".into());
    }
    if tophat_version.trim().is_empty() {
        return Err("field `tophat_version` is empty".into());
    }

    let body_sections = sections(&split.body);
    let results_section = body_sections
        .iter()
        .find(|s| s.name == "Results")
        .ok_or("there is no `## Results` section")?;
    let results = meaningful_lines(&results_section.content)
        .into_iter()
        .map(|(no, raw)| parse_result_line(no, &raw))
        .collect::<Result<Vec<_>, _>>()?;
    if results.is_empty() {
        return Err("the `## Results` section has no result lines".into());
    }

    Ok(Summary {
        study,
        commit,
        dirty,
        dirty_line: loaded.line_of("dirty", 1),
        generated,
        tophat_version,
        keys,
        deciding_data_seen,
        results,
    })
}

// rq-8da81b81
/// Parses a `SUMMARY.md` file.
pub fn parse_summary(path: &Path) -> Result<Summary, StudyError> {
    let text = fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StudyError::Summary(format!("{} does not exist", path.display()))
        } else {
            StudyError::Io(format!("{}: {e}", path.display()))
        }
    })?;
    parse_summary_text(&text).map_err(StudyError::Summary)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A valid SUMMARY.md for `study` with the given dirty flag and result lines.
    pub fn summary_text(study: &str, dirty: bool, lines: &[&str]) -> String {
        let mut text = format!(
            "---\nstudy: {study}\ncommit: \"{}\"\ndirty: {dirty}\ngenerated: \
             \"2026-10-01T12:00:00Z\"\ntophat_version: \"0.1.0\"\n---\n\n## Results\n\n",
            "0123456789abcdef0123456789abcdef01234567"
        );
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    #[test] // rq-09eda840 rq-6764653f
    fn parses_a_valid_summary() {
        let s = parse_summary_text(&summary_text("abc", false, &["- base/score: n=1"])).unwrap();
        assert_eq!(s.study, "abc");
        assert!(!s.dirty);
        assert_eq!(s.dirty_line, 4);
        assert_eq!(s.results[0].variant, "base");
        assert_eq!(s.results[0].stage, "score");
        assert_eq!(s.results[0].raw, "- base/score: n=1");
        assert_eq!(s.results[0].line, 11);
    }

    #[test] // rq-09eda840
    fn rejects_malformed_summaries() {
        let ok = summary_text("abc", false, &["- base/score: n=1"]);
        for bad in [
            ok.replace("- base/score: n=1", "- base score n=1"),
            ok.replace("- base/score: n=1", ""),
            ok.replace("dirty: false", "dirty: \"no\""),
            ok.replace("\"2026-10-01T12:00:00Z\"", "yesterday"),
            ok.replace("0123456789abcdef0123456789abcdef01234567", "abc"),
            ok.replace("## Results", "## Other"),
            ok.replace("tophat_version: \"0.1.0\"\n", ""),
        ] {
            assert!(parse_summary_text(&bad).is_err(), "accepted: {bad}");
        }
    }

    #[test] // rq-09eda840
    fn numeric_commit_keeps_its_digits() {
        let digits = "0123456789012345678901234567890123456789";
        let text = summary_text("abc", false, &["- base/score: n=1"])
            .replace("\"0123456789abcdef0123456789abcdef01234567\"", digits);
        assert_eq!(parse_summary_text(&text).unwrap().commit, digits);
    }
}
