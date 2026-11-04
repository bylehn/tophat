//! Parsing of `STUDY.md` into a [`Study`].

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use yaml_rust2::{Yaml, YamlLoader};

use super::{STUDY_FILE, Status, StudyError};
use crate::markdown::{
    FenceState, FenceTracker, NumberedLine, Section, meaningful_lines, numbered_lines, sections,
    split_front_matter,
};

/// A front-matter value with the 1-based line of its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field<T> {
    pub value: T,
    pub line: u32,
}

/// The `preregistered` block. Sub-fields are `None` when absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preregistration {
    pub date: Option<String>,
    pub frozen_hash: Option<String>,
    /// `sha256:<hex>` of `EXPLORATION.md` at pre-registration (see `rqm/study/exploration.md`).
    pub exploration_hash: Option<String>,
    /// `true` or `false`: whether the exploration saw the deciding data.
    pub deciding_data_seen: Option<String>,
    /// Unknown sub-field names.
    pub unknown: Vec<String>,
}

// rq-a3b4939d
/// The front-matter fields of `STUDY.md`. Each is `None` when the key is absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrontMatter {
    pub study: Option<Field<String>>,
    pub title: Option<Field<String>>,
    pub question: Option<Field<String>>,
    pub status: Option<Field<String>>,
    pub visibility: Option<Field<String>>,
    pub variants: Option<Field<Vec<String>>>,
    pub preregistered: Option<Field<Preregistration>>,
    pub superseded_by: Option<Field<String>>,
    pub supersedes: Option<Field<Vec<String>>>,
    /// Unknown top-level keys.
    pub unknown: Vec<Field<String>>,
}

/// A Gherkin scenario in the `## Scenarios` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    /// The scenario number from its `@S<n>` tag, when it has exactly one valid tag.
    pub number: Option<u32>,
    pub title: String,
    /// 1-based line of the `Scenario:` keyword, or of the checklist item.
    pub line: u32,
}

/// A problem with a scenario's tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagIssue {
    /// The scenario is not preceded by exactly one valid `@S<n>` tag.
    Untagged { line: u32 },
    /// A tag of the form `@S<digits>` that is not valid (zero or leading zeros).
    Invalid { line: u32, tag: String },
}

// rq-fd31255d
/// A well-formed amendment line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amendment {
    /// The date text (`YYYY-MM-DD` shape; calendar validity is checked by the rules).
    pub date: String,
    pub results_seen: bool,
    pub reason: String,
    pub line: u32,
}

// rq-8071e60f
/// A parsed `STUDY.md`.
#[derive(Debug, Clone)]
pub struct Study {
    /// The study ID: the name of the study directory.
    pub id: String,
    pub front: FrontMatter,
    /// All `##` sections of the body, in file order.
    pub sections: Vec<Section>,
    /// Scenarios found in the first `## Scenarios` section.
    pub scenarios: Vec<Scenario>,
    pub tag_issues: Vec<TagIssue>,
    /// Well-formed amendments in the first `## Amendments` section.
    pub amendments: Vec<Amendment>,
    /// Lines of the first `## Amendments` section that are not well-formed amendments.
    pub malformed_amendments: Vec<u32>,
}

impl Study {
    /// The parsed status, if the field is present and valid.
    pub fn status(&self) -> Option<Status> {
        self.front
            .status
            .as_ref()
            .and_then(|f| Status::parse(&f.value))
    }

    /// All sections named `name`, in file order.
    pub fn sections_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections.iter().filter(move |s| s.name == name)
    }

    /// The first section named `name`.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Whether the first section named `name` exists and is empty, or does not exist.
    pub fn section_is_empty(&self, name: &str) -> bool {
        self.section(name)
            .is_none_or(|s| crate::markdown::is_empty(&s.content))
    }

    /// The meaningful lines of the first `## Result` section.
    pub fn result_lines(&self) -> Vec<(u32, String)> {
        self.section("Result")
            .map(|s| meaningful_lines(&s.content))
            .unwrap_or_default()
    }

    // rq-3a21c28f
    /// Whether the `preregistered` block says the exploration saw the deciding data.
    pub fn deciding_data_seen(&self) -> bool {
        self.front
            .preregistered
            .as_ref()
            .and_then(|p| p.value.deciding_data_seen.as_deref())
            == Some("true")
    }

    /// The variant names, or an empty list if the field is absent.
    pub fn variants(&self) -> &[String] {
        self.front
            .variants
            .as_ref()
            .map_or(&[], |f| f.value.as_slice())
    }
}

// --- Front-matter helpers shared with SUMMARY.md ----------------------------------------------

/// Loaded front matter: the top-level mapping plus the source line and raw value text per key.
pub(super) struct LoadedFrontMatter {
    pub entries: Vec<(String, Yaml)>,
    lines: BTreeMap<String, (u32, String)>,
}

impl LoadedFrontMatter {
    /// The 1-based line of a top-level key, falling back to `fallback` when it cannot be located.
    pub fn line_of(&self, key: &str, fallback: u32) -> u32 {
        self.lines.get(key).map_or(fallback, |(line, _)| *line)
    }

    /// The raw source text of a top-level key's value, without quotes or a trailing comment.
    fn raw_value(&self, key: &str) -> Option<&str> {
        self.lines.get(key).map(|(_, raw)| raw.as_str())
    }

    /// A scalar value as text. Numbers and booleans use their source spelling, so `007` stays
    /// `007`. `Null` becomes the empty string. Collections are a type error.
    pub fn scalar(&self, key: &str, value: &Yaml) -> Result<String, String> {
        match value {
            Yaml::String(s) => Ok(s.clone()),
            Yaml::Null => Ok(String::new()),
            Yaml::Integer(_) | Yaml::Real(_) | Yaml::Boolean(_) => Ok(self
                .raw_value(key)
                .map(str::to_string)
                .unwrap_or_else(|| scalar_to_string(value))),
            _ => Err(format!("field `{key}` must be a single value")),
        }
    }
}

/// Converts a scalar YAML value to text.
pub(super) fn scalar_to_string(value: &Yaml) -> String {
    match value {
        Yaml::String(s) | Yaml::Real(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

/// A list of scalars. `Null` is an empty list; anything else that is not a list is a type error.
pub(super) fn string_list(key: &str, value: &Yaml) -> Result<Vec<String>, String> {
    match value {
        Yaml::Null => Ok(Vec::new()),
        Yaml::Array(items) => items
            .iter()
            .map(|item| match item {
                Yaml::String(_) | Yaml::Integer(_) | Yaml::Real(_) | Yaml::Boolean(_) => {
                    Ok(scalar_to_string(item))
                }
                _ => Err(format!("items of `{key}` must be single values")),
            })
            .collect(),
        _ => Err(format!("field `{key}` must be a list")),
    }
}

/// Strips matching single or double quotes and a trailing ` #` comment from a raw scalar.
fn clean_raw_value(raw: &str) -> String {
    let raw = raw.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = raw.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) {
            return inner.to_string();
        }
    }
    raw.split(" #").next().unwrap_or(raw).trim().to_string()
}

/// Parses front-matter lines as a YAML mapping. Errors carry a 1-based file line when known.
pub(super) fn load_front_matter(
    front: &[NumberedLine<'_>],
    open_line: u32,
) -> Result<LoadedFrontMatter, (Option<u32>, String)> {
    let yaml_text: String = front.iter().map(|(_, l)| format!("{l}\n")).collect();
    let docs = YamlLoader::load_from_str(&yaml_text).map_err(|e| {
        let line = open_line + e.marker().line() as u32;
        (Some(line), format!("invalid YAML: {}", e.info()))
    })?;
    let entries = match docs.into_iter().next() {
        None | Some(Yaml::Null) => Vec::new(),
        Some(Yaml::Hash(hash)) => hash
            .into_iter()
            .map(|(k, v)| match k {
                Yaml::String(k) => Ok((k, v)),
                other => Err((
                    Some(open_line + 1),
                    format!("front-matter keys must be text, found {other:?}"),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err((
                Some(open_line + 1),
                "the front matter must be a mapping of fields".into(),
            ));
        }
    };
    let mut lines = BTreeMap::new();
    for &(no, line) in front {
        if line.starts_with([' ', '\t', '#', '-']) {
            continue;
        }
        if let Some((key, rest)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty() {
                lines
                    .entry(key.to_string())
                    .or_insert((no, clean_raw_value(rest)));
            }
        }
    }
    Ok(LoadedFrontMatter { entries, lines })
}

// --- STUDY.md ---------------------------------------------------------------------------------

fn front_matter_error(path: &str, line: Option<u32>, message: impl Into<String>) -> StudyError {
    StudyError::FrontMatter {
        path: path.to_string(),
        line,
        message: message.into(),
    }
}

/// Parses the `preregistered` mapping.
fn parse_preregistration(value: &Yaml) -> Result<Preregistration, String> {
    let mut out = Preregistration::default();
    let hash = match value {
        Yaml::Null => return Ok(out),
        Yaml::Hash(hash) => hash,
        _ => {
            return Err(
                "field `preregistered` must be a mapping with `date` and `frozen_hash`".into(),
            );
        }
    };
    for (k, v) in hash {
        let key = match k {
            Yaml::String(k) => k.as_str(),
            _ => return Err("keys of `preregistered` must be text".into()),
        };
        let text = match v {
            Yaml::String(_) | Yaml::Integer(_) | Yaml::Real(_) | Yaml::Boolean(_) => {
                Some(scalar_to_string(v))
            }
            Yaml::Null => None,
            _ => return Err(format!("`preregistered.{key}` must be a single value")),
        };
        match key {
            "date" => out.date = text,
            "frozen_hash" => out.frozen_hash = text,
            "exploration_hash" => out.exploration_hash = text,
            "deciding_data_seen" => out.deciding_data_seen = text,
            other => out.unknown.push(other.to_string()),
        }
    }
    Ok(out)
}

fn parse_front(
    loaded: &LoadedFrontMatter,
    path: &str,
    open_line: u32,
) -> Result<FrontMatter, StudyError> {
    let mut front = FrontMatter::default();
    let fallback = open_line + 1;
    for (key, value) in &loaded.entries {
        let line = loaded.line_of(key, fallback);
        let err = |message: String| front_matter_error(path, Some(line), message);
        let scalar = || {
            loaded
                .scalar(key, value)
                .map(|value| Field { value, line })
                .map_err(err)
        };
        match key.as_str() {
            "study" => front.study = Some(scalar()?),
            "title" => front.title = Some(scalar()?),
            "question" => front.question = Some(scalar()?),
            "status" => front.status = Some(scalar()?),
            "visibility" => front.visibility = Some(scalar()?),
            "superseded_by" => front.superseded_by = Some(scalar()?),
            "variants" => {
                front.variants = Some(Field {
                    value: string_list(key, value).map_err(err)?,
                    line,
                })
            }
            "supersedes" => {
                front.supersedes = Some(Field {
                    value: string_list(key, value).map_err(err)?,
                    line,
                })
            }
            "preregistered" => {
                front.preregistered = Some(Field {
                    value: parse_preregistration(value).map_err(err)?,
                    line,
                })
            }
            _ => front.unknown.push(Field {
                value: key.clone(),
                line,
            }),
        }
    }
    Ok(front)
}

/// Whether `tag` is of the form `@S<digits>`.
fn is_s_digits_tag(tag: &str) -> bool {
    tag.strip_prefix("@S")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// The scenario number of a valid `@S<n>` tag: a positive integer without leading zeros.
fn scenario_tag_number(tag: &str) -> Option<u32> {
    let digits = tag.strip_prefix("@S")?;
    if digits.starts_with('0') || !is_s_digits_tag(tag) {
        return None;
    }
    digits.parse().ok()
}

/// A checklist scenario `- S<digits>: <expectation>` as `(@S<digits>, expectation)`. Only
/// top-level list items count, and the expectation must not be empty.
fn checklist_item(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("- S")?;
    let (digits, title) = rest.split_once(':')?;
    let title = title.trim();
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && !title.is_empty())
        .then(|| (format!("@S{digits}"), title.to_string()))
}

// rq-04595081
/// Finds the scenarios in the `## Scenarios` section, in the checklist and the Gherkin form, and
/// checks their tags.
fn parse_scenarios(section: &Section) -> (Vec<Scenario>, Vec<TagIssue>) {
    let mut scenarios = Vec::new();
    let mut issues = Vec::new();
    let mut fences = FenceTracker::default();
    let mut in_gherkin = false;
    let mut previous: Option<&str> = None;
    for (no, line) in &section.content {
        match fences.next(line) {
            FenceState::Opens(info) => {
                in_gherkin = info.split_whitespace().next() == Some("gherkin");
                previous = None;
                continue;
            }
            FenceState::Closes => {
                in_gherkin = false;
                continue;
            }
            FenceState::Outside => {
                in_gherkin = false;
                // The checklist form: a top-level list item `- S<n>: <expectation>`.
                if let Some((tag, title)) = checklist_item(line) {
                    match scenario_tag_number(&tag) {
                        Some(number) => scenarios.push(Scenario {
                            number: Some(number),
                            title,
                            line: *no,
                        }),
                        None => {
                            issues.push(TagIssue::Invalid { line: *no, tag });
                            scenarios.push(Scenario {
                                number: None,
                                title,
                                line: *no,
                            });
                        }
                    }
                }
                continue;
            }
            FenceState::Inside if !in_gherkin => continue,
            FenceState::Inside => {}
        }
        let trimmed = line.trim();
        let title = trimmed
            .strip_prefix("Scenario Outline:")
            .or_else(|| trimmed.strip_prefix("Scenario:"));
        if let Some(title) = title {
            let tags: Vec<&str> = previous
                .map(str::trim)
                .filter(|p| !p.is_empty() && p.split_whitespace().all(|t| t.starts_with('@')))
                .map(|p| p.split_whitespace().collect())
                .unwrap_or_default();
            let invalid: Vec<&str> = tags
                .iter()
                .copied()
                .filter(|t| is_s_digits_tag(t) && scenario_tag_number(t).is_none())
                .collect();
            let numbers: Vec<u32> = tags.iter().filter_map(|t| scenario_tag_number(t)).collect();
            for tag in &invalid {
                issues.push(TagIssue::Invalid {
                    line: *no,
                    tag: tag.to_string(),
                });
            }
            if invalid.is_empty() && numbers.len() != 1 {
                issues.push(TagIssue::Untagged { line: *no });
            }
            scenarios.push(Scenario {
                number: (invalid.is_empty() && numbers.len() == 1).then(|| numbers[0]),
                title: title.trim().to_string(),
                line: *no,
            });
        }
        previous = Some(line);
    }
    (scenarios, issues)
}

/// Parses an amendment line: `- YYYY-MM-DD — results seen: yes|no — <reason>`.
fn parse_amendment(line: u32, text: &str) -> Option<Amendment> {
    let rest = text.strip_prefix("- ")?;
    let (date, rest) = rest.split_once(" — results seen: ")?;
    let shape_ok = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    if !shape_ok {
        return None;
    }
    let (seen, reason) = rest.split_once(" — ")?;
    let results_seen = match seen {
        "yes" => true,
        "no" => false,
        _ => return None,
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return None;
    }
    Some(Amendment {
        date: date.to_string(),
        results_seen,
        reason: reason.to_string(),
        line,
    })
}

// rq-b01ceeeb
/// Parses `STUDY.md` in a study directory. The study ID is the directory name.
pub fn parse_study(dir: &Path) -> Result<Study, StudyError> {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = dir.join(STUDY_FILE);
    let text = fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StudyError::StudyNotFound(id.clone())
        } else {
            StudyError::Io(format!("{}: {e}", path.display()))
        }
    })?;
    parse_study_text(&id, &path.display().to_string(), &text)
}

/// Parses the text of a `STUDY.md` for the study `id`. `path` is used in error messages.
pub fn parse_study_text(id: &str, path: &str, text: &str) -> Result<Study, StudyError> {
    let lines = numbered_lines(text);
    let split = split_front_matter(&lines).map_err(|m| front_matter_error(path, Some(1), m))?;
    let loaded = load_front_matter(&split.front, 1)
        .map_err(|(line, message)| front_matter_error(path, line, message))?;
    let front = parse_front(&loaded, path, 1)?;
    let sections = sections(&split.body);

    let (scenarios, tag_issues) = sections
        .iter()
        .find(|s| s.name == "Scenarios")
        .map(parse_scenarios)
        .unwrap_or_default();

    let mut amendments = Vec::new();
    let mut malformed_amendments = Vec::new();
    if let Some(section) = sections.iter().find(|s| s.name == "Amendments") {
        for (no, text) in meaningful_lines(&section.content) {
            match parse_amendment(no, &text) {
                Some(a) => amendments.push(a),
                None => malformed_amendments.push(no),
            }
        }
    }

    Ok(Study {
        id: id.to_string(),
        front,
        sections,
        scenarios,
        tag_issues,
        amendments,
        malformed_amendments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "---\nstudy: abc\ntitle: T\nquestion: q-1a2b3c4d\nstatus: draft\n\
                           visibility: public\nvariants: [base]\n---\n";

    #[test] // rq-a3b4939d
    fn parses_front_matter_fields_and_lines() {
        let study = parse_study_text("abc", "STUDY.md", MINIMAL).unwrap();
        assert_eq!(study.front.study.as_ref().unwrap().value, "abc");
        assert_eq!(study.front.status.as_ref().unwrap().line, 5);
        assert_eq!(study.variants(), ["base".to_string()]);
        assert_eq!(study.status(), Some(Status::Draft));
    }

    #[test] // rq-a3b4939d
    fn numeric_scalars_keep_their_source_spelling() {
        let text = MINIMAL.replace("study: abc", "study: 007");
        let study = parse_study_text("007", "STUDY.md", &text).unwrap();
        assert_eq!(study.front.study.unwrap().value, "007");
    }

    #[test] // rq-a3b4939d
    fn wrong_types_are_front_matter_errors() {
        let text = MINIMAL.replace("variants: [base]", "variants: base");
        assert!(matches!(
            parse_study_text("abc", "STUDY.md", &text),
            Err(StudyError::FrontMatter { line: Some(7), .. })
        ));
        let text = MINIMAL.replace("title: T", "title: [a, b]");
        assert!(parse_study_text("abc", "STUDY.md", &text).is_err());
    }

    #[test]
    fn invalid_yaml_reports_a_line() {
        let text = MINIMAL.replace("title: T", "title: [unclosed");
        match parse_study_text("abc", "STUDY.md", &text) {
            Err(StudyError::FrontMatter { line: Some(l), .. }) => assert!(l >= 3),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test] // rq-04595081
    fn scenario_tags() {
        let body = "## Scenarios\n```gherkin\nFeature: f\n  @S1\n  Scenario: one\n  @S01\n  \
                    Scenario: bad\n  @Slow @S3\n  Scenario: three\n  Scenario: untagged\n  \
                    @S4 @S5\n  Scenario Outline: two tags\n```\n";
        let study = parse_study_text("abc", "STUDY.md", &format!("{MINIMAL}{body}")).unwrap();
        let numbers: Vec<_> = study.scenarios.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![Some(1), None, Some(3), None, None]);
        assert_eq!(study.scenarios[0].title, "one");
        assert_eq!(
            study.tag_issues,
            vec![
                TagIssue::Invalid {
                    line: 15,
                    tag: "@S01".into()
                },
                TagIssue::Untagged { line: 18 },
                TagIssue::Untagged { line: 20 },
            ]
        );
    }

    #[test] // rq-fd31255d
    fn amendment_format() {
        assert!(parse_amendment(1, "- 2026-04-02 — results seen: no — Fixed units").is_some());
        assert!(parse_amendment(1, "- changed the filter").is_none());
        assert!(parse_amendment(1, "- 2026-4-2 — results seen: no — x").is_none());
        assert!(parse_amendment(1, "- 2026-04-02 — results seen: maybe — x").is_none());
        assert!(parse_amendment(1, "- 2026-04-02 — results seen: yes — ").is_none());
        assert!(parse_amendment(1, "- 2026-04-02 - results seen: yes - x").is_none());
    }
}
