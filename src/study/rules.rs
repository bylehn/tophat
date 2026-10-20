//! The `study` rule family evaluated by `tophat check`.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;

use super::fingerprint::is_fingerprint_format;
use super::summary::parse_summary_text;
use super::{
    frozen_fingerprint, is_valid_question_id, is_valid_study_id, matches_id_pattern,
    parse_study_text, Status, Study, StudyError, TagIssue, FROZEN_SECTIONS, GOVERNED_SECTIONS,
    STUDIES_DIR, STUDY_FILE, SUMMARY_FILE,
};
use crate::check::{Diagnostic, Project, Severity};

/// What was found in a study directory.
pub(super) enum Entry {
    Parsed(Box<Study>),
    Unparseable(StudyError),
    NoFile,
}

/// Everything the rules need: the project, the current date, and every study (for
/// cross-references).
pub(super) struct Context<'a> {
    pub project: &'a Project,
    pub today: NaiveDate,
    pub studies: BTreeMap<String, Entry>,
}

impl<'a> Context<'a> {
    /// Reads and parses every directory under `studies/`.
    pub fn load(project: &'a Project, today: NaiveDate) -> Self {
        let mut studies = BTreeMap::new();
        let ids = project.list_dir(STUDIES_DIR).unwrap_or_default();
        for id in ids {
            let dir = format!("{STUDIES_DIR}/{id}");
            if !project.is_dir(&dir) {
                continue;
            }
            let file = format!("{dir}/{STUDY_FILE}");
            let entry = if !project.is_file(&file) {
                Entry::NoFile
            } else {
                match project.read_to_string(&file) {
                    Ok(text) => match parse_study_text(&id, &file, &text) {
                        Ok(study) => Entry::Parsed(Box::new(study)),
                        Err(e) => Entry::Unparseable(e),
                    },
                    Err(e) => Entry::Unparseable(StudyError::FrontMatter {
                        path: file.clone(),
                        line: None,
                        message: format!("cannot read the file: {e}"),
                    }),
                }
            };
            studies.insert(id, entry);
        }
        Context {
            project,
            today,
            studies,
        }
    }

    fn exists(&self, id: &str) -> bool {
        matches!(
            self.studies.get(id),
            Some(Entry::Parsed(_) | Entry::Unparseable(_))
        )
    }

    fn parsed(&self, id: &str) -> Option<&Study> {
        match self.studies.get(id) {
            Some(Entry::Parsed(study)) => Some(study),
            _ => None,
        }
    }
}

/// Collects diagnostics for one file.
struct Reporter<'a> {
    out: &'a mut Vec<Diagnostic>,
    path: String,
}

impl Reporter<'_> {
    fn push(
        &mut self,
        rule: &'static str,
        severity: Severity,
        line: Option<u32>,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.out.push(Diagnostic {
            rule,
            severity,
            path: self.path.clone(),
            line,
            message: message.into(),
            hint: Some(hint.into()),
        });
    }

    fn error(
        &mut self,
        rule: &'static str,
        line: Option<u32>,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.push(rule, Severity::Error, line, message, hint);
    }
}

// rq-f28e6ab8
/// The status whose rules apply: a superseded study is validated as the status it had before
/// supersession.
fn effective_status(study: &Study) -> Option<Status> {
    match study.status()? {
        Status::Superseded => Some(if study.front.preregistered.is_none() {
            Status::Retrospective
        } else if study.result_lines().is_empty() {
            Status::Preregistered
        } else {
            Status::Reported
        }),
        other => Some(other),
    }
}

/// Parses a `YYYY-MM-DD` date with exactly that shape.
fn parse_date(s: &str) -> Option<NaiveDate> {
    (s.len() == 10)
        .then(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .flatten()
}

// rq-ce7c6815 rq-2431d4ff
/// The `study` rule family, evaluated with today's local date.
pub fn study_rules(project: &Project) -> Vec<Diagnostic> {
    study_rules_at(project, chrono::Local::now().date_naive())
}

/// The `study` rule family, evaluated as of `today`.
pub fn study_rules_at(project: &Project, today: NaiveDate) -> Vec<Diagnostic> {
    let ctx = Context::load(project, today);
    let mut out = Vec::new();
    for (id, entry) in &ctx.studies {
        let dir = format!("{STUDIES_DIR}/{id}");
        let mut dir_reporter = Reporter {
            out: &mut out,
            path: dir.clone(),
        };
        if !is_valid_study_id(id) {
            dir_reporter.error(
                "study.id-invalid",
                None,
                format!("Study directory name `{id}` is not a valid study ID"),
                "Use 3 to 64 lowercase letters, digits and single hyphens",
            );
        }
        match entry {
            Entry::NoFile => dir_reporter.error(
                "study.missing-file",
                None,
                format!("Study directory `{dir}` has no {STUDY_FILE}"),
                "Create it with `tophat study new`, or remove the directory",
            ),
            Entry::Unparseable(e) => {
                let (line, message) = match e {
                    StudyError::FrontMatter { line, message, .. } => (*line, message.clone()),
                    other => (None, other.to_string()),
                };
                Reporter {
                    out: &mut out,
                    path: format!("{dir}/{STUDY_FILE}"),
                }
                .error(
                    "study.parse-error",
                    line,
                    format!("Front matter cannot be parsed: {message}"),
                    "Fix the YAML between the `---` lines",
                );
            }
            Entry::Parsed(study) => out.extend(check_study(&ctx, study)),
        }
    }
    out
}

/// Evaluates every `study` rule for one parsed study.
pub(super) fn check_study(ctx: &Context<'_>, study: &Study) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let path = format!("{STUDIES_DIR}/{}/{STUDY_FILE}", study.id);
    let mut r = Reporter {
        out: &mut out,
        path,
    };
    check_front_matter(&mut r, study);
    check_sections(&mut r, study);
    check_scenario_tags(&mut r, study);
    check_configs(ctx, &mut r, study);
    check_supersession(ctx, &mut r, study);
    if let Some(status) = effective_status(study) {
        check_lifecycle(ctx, &mut r, study, status);
        check_tests(ctx, &mut r, study, status);
    }
    out
}

// rq-a3b4939d
fn check_front_matter(r: &mut Reporter<'_>, study: &Study) {
    let f = &study.front;
    let required: [(&str, bool); 6] = [
        ("study", f.study.is_some()),
        ("title", f.title.is_some()),
        ("question", f.question.is_some()),
        ("status", f.status.is_some()),
        ("visibility", f.visibility.is_some()),
        ("variants", f.variants.is_some()),
    ];
    for (name, present) in required {
        if !present {
            r.error(
                "study.field-missing",
                None,
                format!("Required front-matter field `{name}` is missing"),
                format!("Add `{name}:` to the front matter"),
            );
        }
    }

    if let Some(field) = &f.study {
        if field.value != study.id {
            r.error(
                "study.id-mismatch",
                Some(field.line),
                format!(
                    "Front matter says `study: {}` but the directory is `{}`",
                    field.value, study.id
                ),
                "Make `study` equal the directory name",
            );
        }
    }
    if let Some(field) = &f.title {
        if field.value.trim().is_empty() || field.value.contains('\n') {
            r.error(
                "study.field-invalid",
                Some(field.line),
                "`title` must be non-empty and on one line",
                "Give the study a one-line title",
            );
        }
    }
    if let Some(field) = &f.question {
        if !is_valid_question_id(&field.value) {
            r.error(
                "study.field-invalid",
                Some(field.line),
                format!("`question: {}` is not a valid question ID", field.value),
                "Use `q-` followed by 8 lowercase hex characters",
            );
        }
    }
    if let Some(field) = &f.status {
        if Status::parse(&field.value).is_none() {
            r.error(
                "study.field-invalid",
                Some(field.line),
                format!("`status: {}` is not a valid status", field.value),
                "Use draft, preregistered, reported, superseded or retrospective",
            );
        }
    }
    if let Some(field) = &f.visibility {
        if field.value != "public" && field.value != "private" {
            r.error(
                "study.field-invalid",
                Some(field.line),
                format!("`visibility: {}` is not a valid visibility", field.value),
                "Use public or private",
            );
        }
    }
    if let Some(field) = &f.variants {
        let mut seen = BTreeSet::new();
        if field.value.is_empty() {
            r.error(
                "study.field-invalid",
                Some(field.line),
                "`variants` is empty",
                "List at least one variant, e.g. `base`",
            );
        }
        for v in &field.value {
            if !matches_id_pattern(v) {
                r.error(
                    "study.field-invalid",
                    Some(field.line),
                    format!("Variant name `{v}` is malformed"),
                    "Use lowercase letters, digits and single hyphens",
                );
            }
            if !seen.insert(v) {
                r.error(
                    "study.field-invalid",
                    Some(field.line),
                    format!("Variant `{v}` is listed more than once"),
                    "List each variant once",
                );
            }
        }
    }
    let superseded = study.status() == Some(Status::Superseded);
    match (&f.superseded_by, superseded) {
        (None, true) => r.error(
            "study.field-missing",
            f.status.as_ref().map(|s| s.line),
            "A superseded study needs `superseded_by`",
            "Use `tophat study supersede` to record the replacing study",
        ),
        (Some(field), false) => r.error(
            "study.field-invalid",
            Some(field.line),
            "`superseded_by` is only allowed when `status: superseded`",
            "Remove `superseded_by`, or supersede the study with `tophat study supersede`",
        ),
        _ => {}
    }
    if let Some(block) = &f.preregistered {
        if let Some(date) = &block.value.date {
            if parse_date(date).is_none() {
                r.error(
                    "study.field-invalid",
                    Some(block.line),
                    format!("`preregistered.date: {date}` is not a valid YYYY-MM-DD date"),
                    "Pre-registration fields are written by `tophat study preregister`",
                );
            }
        }
        if let Some(hash) = &block.value.frozen_hash {
            if !is_fingerprint_format(hash) {
                r.error(
                    "study.field-invalid",
                    Some(block.line),
                    "`preregistered.frozen_hash` is not a valid fingerprint",
                    "Pre-registration fields are written by `tophat study preregister`",
                );
            }
        }
        for key in &block.value.unknown {
            r.push(
                "study.field-unknown",
                Severity::Warning,
                Some(block.line),
                format!("Unknown field `preregistered.{key}`"),
                "Remove it, or check its spelling",
            );
        }
    }
    for field in &f.unknown {
        r.push(
            "study.field-unknown",
            Severity::Warning,
            Some(field.line),
            format!("Unknown front-matter field `{}`", field.value),
            "Remove it, or check its spelling against the STUDY.md format",
        );
    }
}

// rq-e9d4388a
fn check_sections(r: &mut Reporter<'_>, study: &Study) {
    for name in GOVERNED_SECTIONS {
        let occurrences: Vec<_> = study.sections_named(name).collect();
        match occurrences.as_slice() {
            [] => r.error(
                "study.section-missing",
                None,
                format!("Section `## {name}` is missing"),
                format!("Add a `## {name}` heading"),
            ),
            [_] => {}
            [_, rest @ ..] => {
                for s in rest {
                    r.error(
                        "study.section-duplicate",
                        Some(s.heading_line),
                        format!("Section `## {name}` appears more than once"),
                        "Merge the duplicate sections into one",
                    );
                }
            }
        }
    }
}

// rq-04595081
fn check_scenario_tags(r: &mut Reporter<'_>, study: &Study) {
    for issue in &study.tag_issues {
        match issue {
            TagIssue::Untagged { line } => r.error(
                "study.scenario-untagged",
                Some(*line),
                "Scenario is not preceded by exactly one `@S<n>` tag",
                "Put a tag such as `@S1` on the line before `Scenario:`",
            ),
            TagIssue::Invalid { line, tag } => r.error(
                "study.scenario-tag-invalid",
                Some(*line),
                format!("`{tag}` is not a valid scenario tag"),
                "Use `@S` followed by a positive number without leading zeros",
            ),
        }
    }
    let mut seen = BTreeSet::new();
    for scenario in &study.scenarios {
        if let Some(n) = scenario.number {
            if !seen.insert(n) {
                r.error(
                    "study.scenario-duplicate",
                    Some(scenario.line),
                    format!("Scenario number S{n} is used more than once"),
                    "Give each scenario its own number",
                );
            }
        }
    }
}

// rq-e5eda0dd
fn check_configs(ctx: &Context<'_>, r: &mut Reporter<'_>, study: &Study) {
    let config_dir = format!("{STUDIES_DIR}/{}/config", study.id);
    if !ctx.project.is_file(&format!("{config_dir}/base.yaml")) {
        r.error(
            "study.config-base-missing",
            None,
            "`config/base.yaml` is missing",
            "Create the base pipeline configuration",
        );
    }
    let variants_line = study.front.variants.as_ref().map(|f| f.line);
    for v in study.variants() {
        if v != "base" && !ctx.project.is_file(&format!("{config_dir}/{v}.yaml")) {
            r.error(
                "study.config-missing",
                variants_line,
                format!("Variant `{v}` has no `config/{v}.yaml`"),
                "Create the variant's override file, or remove the variant",
            );
        }
    }
    let files = ctx.project.list_dir(&config_dir).unwrap_or_default();
    for name in files {
        let Some(stem) = name.strip_suffix(".yaml") else {
            continue;
        };
        let file = format!("{config_dir}/{name}");
        if stem == "base" || !ctx.project.is_file(&file) {
            continue;
        }
        if !study.variants().iter().any(|v| v == stem) {
            r.out.push(Diagnostic {
                rule: "study.config-unlisted",
                severity: Severity::Error,
                path: file,
                line: None,
                message: format!("`config/{name}` does not correspond to a listed variant"),
                hint: Some(format!(
                    "Add `{stem}` to `variants` in STUDY.md, or remove the file"
                )),
            });
        }
    }
}

fn check_supersession(ctx: &Context<'_>, r: &mut Reporter<'_>, study: &Study) {
    if let Some(field) = &study.front.superseded_by {
        let target = &field.value;
        if !ctx.exists(target) {
            r.error(
                "study.supersede-target-missing",
                Some(field.line),
                format!("`superseded_by: {target}` names a study that does not exist"),
                "Name an existing study in `superseded_by`",
            );
        } else if let Some(other) = ctx.parsed(target) {
            let backlinked = other
                .front
                .supersedes
                .as_ref()
                .is_some_and(|f| f.value.contains(&study.id));
            if !backlinked {
                r.error(
                    "study.supersede-backlink-missing",
                    Some(field.line),
                    format!(
                        "Study `{target}` does not list `{}` under `supersedes`",
                        study.id
                    ),
                    format!("Add `{}` to `supersedes` in study `{target}`", study.id),
                );
            }
        }
    }
    if let Some(field) = &study.front.supersedes {
        for old in &field.value {
            let valid = match ctx.studies.get(old.as_str()) {
                Some(Entry::Parsed(o)) => o
                    .front
                    .superseded_by
                    .as_ref()
                    .is_some_and(|f| f.value == study.id),
                Some(Entry::Unparseable(_)) => true,
                _ => false,
            };
            if !valid {
                r.error(
                    "study.supersedes-invalid",
                    Some(field.line),
                    format!(
                        "`supersedes` lists `{old}`, which does not exist or is not superseded by `{}`",
                        study.id
                    ),
                    "Use `tophat study supersede` so both studies link to each other",
                );
            }
        }
    }
}

// rq-f28e6ab8
fn check_lifecycle(ctx: &Context<'_>, r: &mut Reporter<'_>, study: &Study, status: Status) {
    let block = study.front.preregistered.as_ref();
    let status_line = study.front.status.as_ref().map(|f| f.line);
    let needs_block = matches!(status, Status::Preregistered | Status::Reported);

    // Pre-registration block and fingerprint.
    match (needs_block, block) {
        (true, None) => r.error(
            "study.prereg-missing",
            status_line,
            "The study has no `preregistered` block",
            "Pre-register with `tophat study preregister` (interactive, by a human)",
        ),
        (true, Some(b)) if b.value.date.is_none() || b.value.frozen_hash.is_none() => r.error(
            "study.prereg-missing",
            Some(b.line),
            "The `preregistered` block needs both `date` and `frozen_hash`",
            "Restore the block written by `tophat study preregister`",
        ),
        (false, Some(b)) => r.error(
            "study.prereg-unexpected",
            Some(b.line),
            format!("A `{status}` study must not have a `preregistered` block"),
            "Remove the block; only `tophat study preregister` writes it",
        ),
        _ => {}
    }
    if needs_block {
        if let Some(hash) = block
            .and_then(|b| b.value.frozen_hash.as_ref())
            .filter(|h| is_fingerprint_format(h))
        {
            if *hash != frozen_fingerprint(study) {
                r.error(
                    "study.frozen-modified",
                    block.map(|b| b.line),
                    "Frozen sections changed after pre-registration",
                    "Revert the change and record it under Amendments, or start a new study",
                );
            }
        }
    }

    // Non-empty frozen sections and scenarios.
    let must_be_filled: &[&str] = match status {
        Status::Preregistered | Status::Reported => &FROZEN_SECTIONS[..3],
        Status::Retrospective => &FROZEN_SECTIONS[..2],
        _ => &[],
    };
    for name in must_be_filled {
        if let Some(section) = study.section(name) {
            if study.section_is_empty(name) {
                r.error(
                    "study.section-empty",
                    Some(section.heading_line),
                    format!("Section `## {name}` is empty"),
                    format!("Fill in `## {name}`"),
                );
            }
        }
    }
    if needs_block && study.scenarios.is_empty() {
        r.error(
            "study.no-scenarios",
            study.section("Scenarios").map(|s| s.heading_line),
            "The study has no scenarios",
            "Add at least one tagged Gherkin scenario under `## Scenarios`",
        );
    }

    // Amendments.
    let amendments_heading = study.section("Amendments").map(|s| s.heading_line);
    if block.is_none() {
        if !study.section_is_empty("Amendments") {
            r.error(
                "study.amendment-unexpected",
                amendments_heading,
                "Amendments are recorded only after pre-registration",
                "Edit the study directly while it is a draft",
            );
        }
    } else {
        let prereg_date = block
            .and_then(|b| b.value.date.as_deref())
            .and_then(parse_date);
        for line in &study.malformed_amendments {
            r.error(
                "study.amendment-format",
                Some(*line),
                "Amendment is not in the form `- YYYY-MM-DD — results seen: yes|no — <reason>`",
                "Rewrite the line in the amendment format (with em dashes)",
            );
        }
        for a in &study.amendments {
            let problem = match parse_date(&a.date) {
                None => Some(format!("`{}` is not a valid date", a.date)),
                Some(d) if prereg_date.is_some_and(|p| d < p) => {
                    Some(format!("{} is before the pre-registration date", a.date))
                }
                Some(d) if d > ctx.today => Some(format!("{} is in the future", a.date)),
                Some(_) => None,
            };
            if let Some(problem) = problem {
                r.error(
                    "study.amendment-date",
                    Some(a.line),
                    format!("Amendment date invalid: {problem}"),
                    "Use the date the amendment was made",
                );
            }
        }
    }

    // Result.
    let result_lines = study.result_lines();
    let result_heading = study.section("Result").map(|s| s.heading_line);
    match status {
        Status::Draft | Status::Preregistered if !result_lines.is_empty() => r.error(
            "study.result-unexpected",
            result_lines.first().map(|(l, _)| *l),
            format!("A `{status}` study must not have a Result"),
            "Results are written by `tophat study report` after the pipeline has run",
        ),
        Status::Reported if result_lines.is_empty() => r.error(
            "study.result-missing",
            result_heading,
            "A reported study has an empty Result",
            "Run `tophat study report`",
        ),
        Status::Reported => check_result_against_summary(ctx, r, study, &result_lines),
        Status::Retrospective if !result_lines.is_empty() => {
            if ctx.project.is_file(&summary_path(study)) {
                check_result_against_summary(ctx, r, study, &result_lines);
            } else {
                r.push(
                    "study.result-unverified",
                    Severity::Legacy,
                    result_lines.first().map(|(l, _)| *l),
                    "Retrospective result is hand-written and not backed by a SUMMARY.md",
                    "Reproduce it with a pipeline and `tophat study report` to verify it",
                );
            }
        }
        _ => {}
    }
}

fn summary_path(study: &Study) -> String {
    format!("{STUDIES_DIR}/{}/{SUMMARY_FILE}", study.id)
}

// rq-09eda840
fn check_result_against_summary(
    ctx: &Context<'_>,
    r: &mut Reporter<'_>,
    study: &Study,
    result_lines: &[(u32, String)],
) {
    let summary_file = summary_path(study);
    if !ctx.project.is_file(&summary_file) {
        r.error(
            "study.summary-missing",
            None,
            "The Result must come from SUMMARY.md, which is missing",
            "Run the pipeline's summary stage, then `tophat study report`",
        );
        return;
    }
    let mut sr = Reporter {
        out: &mut *r.out,
        path: summary_file.clone(),
    };
    let summary = match ctx
        .project
        .read_to_string(&summary_file)
        .map_err(|e| e.to_string())
        .and_then(|text| parse_summary_text(&text))
    {
        Ok(summary) => summary,
        Err(message) => {
            sr.error(
                "study.summary-invalid",
                None,
                format!("SUMMARY.md cannot be parsed: {message}"),
                "Regenerate it with the pipeline's summary stage",
            );
            return;
        }
    };
    if summary.study != study.id {
        sr.error(
            "study.summary-invalid",
            None,
            format!("SUMMARY.md is for study `{}`", summary.study),
            "Regenerate it with this study's pipeline",
        );
    }
    for line in &summary.results {
        if !study.variants().contains(&line.variant) {
            sr.error(
                "study.summary-invalid",
                Some(line.line),
                format!("Result line names unlisted variant `{}`", line.variant),
                "List the variant in STUDY.md or regenerate the summary",
            );
        }
    }
    if summary.dirty {
        sr.error(
            "study.summary-dirty",
            Some(summary.dirty_line),
            "SUMMARY.md was produced from uncommitted code (`dirty: true`)",
            "Commit the code, re-run the pipeline, then `tophat study report`",
        );
    }
    let expected: Vec<&str> = summary.results.iter().map(|l| l.raw.trim_end()).collect();
    let actual: Vec<&str> = result_lines.iter().map(|(_, l)| l.trim_end()).collect();
    if expected != actual {
        let first_diff = (0..actual.len().max(expected.len()))
            .find(|&i| actual.get(i) != expected.get(i))
            .unwrap_or(0);
        let line = result_lines
            .get(first_diff)
            .map(|(l, _)| *l)
            .or_else(|| study.section("Result").map(|s| s.heading_line));
        r.error(
            "study.result-mismatch",
            line,
            "The Result differs from the result lines in SUMMARY.md",
            "Do not edit the Result by hand; run `tophat study report`",
        );
    }
}

// rq-aa0fe39a
/// Finds `th:S<n>` references in a file: (scenario number, 1-based line), first occurrence per
/// number.
fn test_references(bytes: &[u8]) -> Vec<(u32, u32)> {
    let mut refs: BTreeMap<u32, u32> = BTreeMap::new();
    let mut line = 1u32;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
        } else if bytes[i..].starts_with(b"th:S") {
            let start = i + 4;
            let digits = bytes[start..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            let end = start + digits;
            let boundary = bytes
                .get(end)
                .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
            if digits > 0 && bytes[start] != b'0' && boundary {
                if let Ok(n) = std::str::from_utf8(&bytes[start..end])
                    .unwrap_or("")
                    .parse::<u32>()
                {
                    refs.entry(n).or_insert(line);
                }
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    refs.into_iter().collect()
}

// rq-aa0fe39a
fn check_tests(ctx: &Context<'_>, r: &mut Reporter<'_>, study: &Study, status: Status) {
    let tests_dir = format!("{STUDIES_DIR}/{}/tests", study.id);
    let files = if ctx.project.is_dir(&tests_dir) {
        ctx.project.walk_files(&tests_dir).unwrap_or_default()
    } else {
        Vec::new()
    };
    let numbers: BTreeSet<u32> = study.scenarios.iter().filter_map(|s| s.number).collect();
    let mut tested = BTreeSet::new();
    let orphan_severity = if status == Status::Draft {
        Severity::Warning
    } else {
        Severity::Error
    };
    for file in files {
        let Ok(bytes) = ctx.project.read(&file) else {
            continue;
        };
        for (n, line) in test_references(&bytes) {
            if numbers.contains(&n) {
                tested.insert(n);
            } else {
                r.out.push(Diagnostic {
                    rule: "study.test-orphan",
                    severity: orphan_severity,
                    path: file.clone(),
                    line: Some(line),
                    message: format!("Test references scenario S{n}, which does not exist"),
                    hint: Some("Point the reference at an existing scenario, or remove it".into()),
                });
            }
        }
    }
    let untested_severity = match status {
        Status::Preregistered => Severity::Warning,
        Status::Reported => Severity::Error,
        _ => return,
    };
    for scenario in &study.scenarios {
        if let Some(n) = scenario.number.filter(|n| !tested.contains(n)) {
            r.push(
                "study.scenario-untested",
                untested_severity,
                Some(scenario.line),
                format!("Scenario S{n} is not referenced by any test"),
                format!("Add `th:S{n}` to a test under `tests/`"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-aa0fe39a
    fn finds_test_references() {
        let text = b"# th:S1\n// th:S12 th:S1\nth:S0 th:S01 th:S3x th:S4_ th:S5.\n";
        assert_eq!(test_references(text), vec![(1, 1), (5, 3), (12, 2)]);
    }

    #[test]
    fn dates() {
        assert!(parse_date("2026-03-01").is_some());
        assert!(parse_date("2026-02-30").is_none());
        assert!(parse_date("2026-3-1").is_none());
    }
}
