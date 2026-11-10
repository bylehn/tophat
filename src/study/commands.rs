//! The `tophat study` commands: `new`, `preregister`, `report` and `supersede`.

use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use super::edit::EditableDoc;
use super::exploration::{
    EXPLORATION_FILE, Exploration, exploration_hash, exploration_text, explore, write_exploration,
};
use super::rules::{Context, check_study};
use super::{
    Mode, Relation, STUDIES_DIR, STUDY_FILE, SUMMARY_FILE, Status, Study, StudyError,
    frozen_fingerprint, frozen_sections, is_valid_question_id, is_valid_study_id, parse_study_text,
    parse_summary,
};
use crate::check::{Project, Severity};
use crate::fsutil::write_atomic;

/// The terminal a command talks to.
pub struct Console<'a> {
    /// Whether standard input and standard output are both interactive terminals.
    pub interactive: bool,
    pub input: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
}

fn study_dir(root: &Path, id: &str) -> Result<PathBuf, StudyError> {
    if !is_valid_study_id(id) {
        return Err(StudyError::InvalidStudyId(id.to_string()));
    }
    Ok(root.join(STUDIES_DIR).join(id))
}

/// A study loaded for editing: its file path, original text, and parsed form.
struct Loaded {
    path: PathBuf,
    text: String,
    study: Study,
}

fn load(root: &Path, id: &str) -> Result<Loaded, StudyError> {
    let path = study_dir(root, id)?.join(STUDY_FILE);
    let text = fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StudyError::StudyNotFound(id.to_string())
        } else {
            StudyError::Io(format!("{}: {e}", path.display()))
        }
    })?;
    let rel = format!("{STUDIES_DIR}/{id}/{STUDY_FILE}");
    let study = parse_study_text(id, &rel, &text)?;
    Ok(Loaded { path, text, study })
}

fn editable(loaded: &Loaded) -> Result<EditableDoc, StudyError> {
    EditableDoc::new(&loaded.text).map_err(|message| StudyError::FrontMatter {
        path: loaded.path.display().to_string(),
        line: Some(1),
        message,
    })
}

fn require_status(study: &Study, allowed: &[Status]) -> Result<Status, StudyError> {
    match study.status() {
        Some(s) if allowed.contains(&s) => Ok(s),
        status => Err(StudyError::WrongStatus {
            study: study.id.clone(),
            status,
            required: allowed.to_vec(),
        }),
    }
}

fn mode_section_guidance(mode: Mode, name: &str) -> &'static str {
    match (mode, name) {
        (_, "Primary endpoint") => {
            "<!-- The one number that answers the question (for example \"the mean RMSE difference over all\n     signals\"), and how it is measured. If you cannot name it yet, explore first. Locked when you\n     pre-register. -->"
        }
        (_, "Decision rule") => {
            "<!-- How that number answers the question, with every threshold (for example \"method X is\n     better if the 95% interval lies above 0\"). Locked when you pre-register. -->"
        }
        (_, "Deliverables") => {
            "<!-- What is measured, on what data, and published as what dataset or measurement. Locked\n     when you pre-register. -->"
        }
        (_, "Success criteria") => {
            "<!-- The criteria a developed or evaluated method must meet. Locked when you pre-register. -->"
        }
        (_, "Match criterion") => {
            "<!-- What counts as a match with the reproduction target: exact hashes, or stated\n     tolerances on named quantities. Locked when you pre-register. -->"
        }
        (_, "Scenarios") => {
            "<!-- What must be true of the pipeline, one checkable statement per line:\n     - S1: <expectation>\n     - S2: <expectation>\n     Each needs a test under tests/ that mentions th:S1, th:S2, ... (Gherkin blocks tagged @S1\n     also work.) Locked when you pre-register. -->"
        }
        _ => "",
    }
}

fn template(id: &str, question: &str, visibility: &str, mode: Mode) -> String {
    let question_note = if mode.preregisters() {
        " Locked when you pre-register."
    } else {
        ""
    };
    let mut out = format!(
        "---
study: {id}
title: \"{id}\"
question: {question}
status: draft
mode: {mode}
visibility: {visibility}
variants:
  - base
---

# {id}

## Question

<!-- The one question this study answers, in plain words.{}-->

",
        question_note
    );
    for name in mode.mode_sections() {
        out.push_str(&format!(
            "## {name}\n\n{}\n\n",
            mode_section_guidance(mode, name)
        ));
    }
    out.push_str(
        "## Variants

<!-- Describe each variant listed in the front matter. `base` uses config/base.yaml; any other
     variant uses config/<variant>.yaml holding only what differs. -->

## Stages

<!-- Describe the pipeline stages. The machine-readable definition lives in config/. -->

",
    );
    if mode.preregisters() {
        out.push_str(
            "## Amendments

<!-- After pre-registration only, one line per change:
     - YYYY-MM-DD — results seen: yes|no — reason -->

",
        );
    }
    out.push_str(
        "## Result

<!-- Written by `tophat study report`. In retrospective mode, hand-written result lines may be
     used when the study has no pipeline. -->

## Interpretation

<!-- Your interpretation of the result. -->
",
    );
    out
}

// rq-f689a538
/// Creates `studies/<id>/` with a draft `STUDY.md`, `config/base.yaml` and an empty `tests/`.
/// Writes nothing if validation fails; removes the partially created directory on I/O failure.
pub fn new_study(
    root: &Path,
    id: &str,
    question: &str,
    private: bool,
    mode: Mode,
) -> Result<PathBuf, StudyError> {
    if mode == Mode::Replication {
        return Err(StudyError::WrongMode {
            study: id.to_string(),
            mode,
        });
    }
    create_study(root, id, question, private, mode)
}

/// Creates a study directory for any mode, including `replication`.
fn create_study(
    root: &Path,
    id: &str,
    question: &str,
    private: bool,
    mode: Mode,
) -> Result<PathBuf, StudyError> {
    let dir = study_dir(root, id)?;
    if !is_valid_question_id(question) {
        return Err(StudyError::InvalidQuestionId(question.to_string()));
    }
    if dir.exists() {
        return Err(StudyError::StudyExists(id.to_string()));
    }
    fs::create_dir_all(root.join(STUDIES_DIR))?;
    fs::create_dir(&dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            StudyError::StudyExists(id.to_string())
        } else {
            e.into()
        }
    })?;
    let visibility = if private { "private" } else { "public" };
    let populate = || -> std::io::Result<()> {
        fs::create_dir(dir.join("config"))?;
        fs::create_dir(dir.join("tests"))?;
        fs::write(
            dir.join("config/base.yaml"),
            format!(
                "# Pipeline stages of study {id}; see rqm/pipeline/provenance.md for the fields.\nstages: []\n"
            ),
        )?;
        fs::write(dir.join("Snakefile"), crate::pipeline::stub(id))?;
        fs::write(
            dir.join(STUDY_FILE),
            template(id, question, visibility, mode),
        )
    };
    if let Err(e) = populate() {
        let _ = fs::remove_dir_all(&dir);
        return Err(e.into());
    }
    Ok(dir)
}

// rq-bcb83d0a
/// Pre-registers a draft study after interactive confirmation by a human. `today` is the
/// pre-registration date.
pub fn preregister(
    root: &Path,
    id: &str,
    today: NaiveDate,
    console: &mut Console<'_>,
) -> Result<(), StudyError> {
    let loaded = load(root, id)?;
    if !console.interactive {
        return Err(StudyError::NotInteractive);
    }
    let mode = loaded.study.mode();
    if !mode.preregisters() {
        return Err(StudyError::WrongMode {
            study: id.to_string(),
            mode,
        });
    }
    require_status(&loaded.study, &[Status::Draft])?;

    let fingerprint = frozen_fingerprint(&loaded.study);
    // rq-fea6913a
    let exploration = explore(root, id)?;
    let (exploration_text, _) = exploration_text(root, &exploration);
    let mut doc = editable(&loaded)?;
    doc.set_field("status", &["status: preregistered".to_string()]);
    doc.set_field(
        "preregistered",
        &[
            "preregistered:".to_string(),
            format!("  date: \"{}\"", today.format("%Y-%m-%d")),
            format!("  frozen_hash: \"{fingerprint}\""),
            format!(
                "  exploration_hash: \"{}\"",
                exploration_hash(&exploration_text)
            ),
            format!("  deciding_data_seen: {}", exploration.deciding_data_seen),
        ],
    );
    let new_text = doc.text();

    // Evaluate the study as it would be after pre-registration.
    let rel = format!("{STUDIES_DIR}/{id}/{STUDY_FILE}");
    let hypothetical = parse_study_text(id, &rel, &new_text)?;
    let project = Project::new(root);
    let ctx = Context::load(&project, today);
    let problems: Vec<_> = check_study(&ctx, &hypothetical)
        .into_iter()
        // EXPLORATION.md is written only on confirmation.
        .filter(|d| d.severity == Severity::Error && !d.rule.starts_with("study.exploration-"))
        .collect();
    if !problems.is_empty() {
        return Err(StudyError::NotReady(problems));
    }

    let out = &mut *console.out;
    // rq-bcb83d0a
    writeln!(
        out,
        "Pre-registering study `{id}`.

Pre-registration locks the plan before you see the result, so that the result is judged by the
plan you made in advance rather than by one chosen afterwards. From now on the sections below
are frozen. If one must change later, the change is recorded as a dated amendment that says
whether you had seen results. The pipeline, its stages, variants and scripts can still change.

The sections to lock:
"
    )?;
    for name in frozen_sections(mode) {
        writeln!(out, "## {name}")?;
        if let Some(section) = loaded.study.section(name) {
            for (_, line) in &section.content {
                writeln!(out, "{line}")?;
            }
        }
    }
    // rq-db0c3b99
    writeln!(
        out,
        "\nWhat the exploration saw: {}. This is recorded in {EXPLORATION_FILE}.",
        exploration.account()
    )?;
    if exploration.deciding_data_seen {
        writeln!(
            out,
            "The trial runs already saw the deciding data (the data the locked plan will compute \
             from). That is allowed, but the result will be labelled as informed by its deciding \
             data."
        )?;
    }
    write!(out, "\nType the study ID (`{id}`) to confirm: ")?;
    out.flush()?;
    let mut answer = String::new();
    console.input.read_line(&mut answer)?;
    if answer.trim() != id {
        return Err(StudyError::ConfirmationMismatch);
    }

    write_atomic(&Exploration::path(root, id), &exploration_text)?;
    write_atomic(&loaded.path, &new_text)?;
    writeln!(
        console.out,
        "Study `{id}` is pre-registered. Commit {STUDY_FILE} and {EXPLORATION_FILE} now, before \
         running any answer-stage computation."
    )?;
    Ok(())
}

// rq-bc7ec152 rq-e3a73fb1
/// `tophat study exploration`: writes `EXPLORATION.md` of a draft study. Returns the record and
/// whether the file changed.
pub fn record_exploration(root: &Path, id: &str) -> Result<(Exploration, bool), StudyError> {
    let loaded = load(root, id)?;
    let mode = loaded.study.mode();
    if !mode.preregisters() {
        return Err(StudyError::WrongMode {
            study: id.to_string(),
            mode,
        });
    }
    match loaded.study.status() {
        Some(Status::Draft) => {}
        Some(_) => return Err(StudyError::ExplorationLocked(id.to_string())),
        None => {
            require_status(&loaded.study, &[Status::Draft])?;
        }
    }
    let exploration = explore(root, id)?;
    let changed = write_exploration(root, &exploration)?;
    Ok((exploration, changed))
}

/// Whether the study's `config/base.yaml` lists at least one stage.
fn study_has_stages(root: &Path, id: &str) -> bool {
    fs::read_to_string(
        root.join(STUDIES_DIR)
            .join(id)
            .join("config")
            .join("base.yaml"),
    )
    .ok()
    .and_then(|t| yaml_rust2::YamlLoader::load_from_str(&t).ok())
    .and_then(|docs| docs.into_iter().next())
    .is_some_and(|doc| doc["stages"].as_vec().is_some_and(|v| !v.is_empty()))
}

// rq-ecacda48
/// Copies the result lines of `SUMMARY.md` into the `## Result` section verbatim, or reports a
/// retrospective study's hand-written result.
pub fn report(root: &Path, id: &str) -> Result<(), StudyError> {
    let loaded = load(root, id)?;
    let study = &loaded.study;
    let mode = study.mode();
    let status = require_status(
        study,
        &[Status::Preregistered, Status::Reported, Status::Draft],
    )?;
    if status == Status::Draft && mode.preregisters() {
        return Err(StudyError::WrongStatus {
            study: id.to_string(),
            status: Some(status),
            required: vec![Status::Preregistered, Status::Reported],
        });
    }
    let has_stages = study_has_stages(root, id);
    let summary_path = study_dir(root, id)?.join(SUMMARY_FILE);

    if has_stages && mode == Mode::Exploratory && !summary_path.is_file() {
        return Err(StudyError::Summary(
            "an exploratory study without stages has nothing to report from".into(),
        ));
    }
    if !summary_path.is_file() {
        return match mode {
            Mode::Retrospective if !has_stages => report_hand_written(root, &loaded, status),
            _ => Err(StudyError::Summary(
                "the study has no SUMMARY.md to report from".into(),
            )),
        };
    }

    let summary = parse_summary(&summary_path)?;
    if summary.study != id {
        return Err(StudyError::Summary(format!(
            "it is for study `{}`, not `{id}`",
            summary.study
        )));
    }
    if summary.dirty {
        return Err(StudyError::Summary(
            "the pipeline ran from uncommitted code (`dirty: true`); commit the code and re-run \
             the pipeline first"
                .into(),
        ));
    }
    if let Some(line) = summary
        .results
        .iter()
        .find(|l| !study.variants().contains(&l.variant))
    {
        return Err(StudyError::Summary(format!(
            "line {} names variant `{}`, which is not listed in STUDY.md",
            line.line, line.variant
        )));
    }

    if let Some(keys) = &summary.keys {
        match crate::pipeline::stale_entries(root, id, keys) {
            Ok(stale) if stale.is_empty() => {}
            Ok(stale) => {
                return Err(StudyError::Summary(format!(
                    "the summary is stale: {} no longer match the configuration; run the pipeline again",
                    stale.join(", ")
                )));
            }
            Err(e) => {
                return Err(StudyError::Summary(format!(
                    "cannot check whether the summary is current: {e}"
                )));
            }
        }
    }
    match crate::pipeline::check_key_stale(root, id, summary.check.as_deref()) {
        Ok(false) => {}
        Ok(true) => {
            return Err(StudyError::Summary(
                "the summary is stale: the study's test files changed; run the pipeline again"
                    .into(),
            ));
        }
        Err(e) => {
            return Err(StudyError::Summary(format!(
                "cannot check whether the summary is current: {e}"
            )));
        }
    }
    // rq-ecacda48
    if let Some((stage, variant)) = crate::pipeline::missing_reviews(root, id, study.variants())
        .into_iter()
        .next()
    {
        return Err(StudyError::Summary(format!(
            "stage `{stage}` ({variant}) has a review question and no `pass` verdict for its \
             current key; a human gives it with `tophat stage qc {id} {stage} --variant {variant}`"
        )));
    }
    let unreported = crate::data::unreported_dependencies(&Project::new(root), study);
    if !unreported.is_empty() {
        let refs: Vec<String> = unreported.iter().map(|r| r.to_string()).collect();
        return Err(StudyError::Summary(format!(
            "the study depends on unreported work: {}; report the producing studies first, or \
             accept the dependency with an amendment containing `accept <name>@v<n>`",
            refs.join(", ")
        )));
    }

    let structure_error = |message: &str| StudyError::Structure {
        path: loaded.path.display().to_string(),
        message: message.to_string(),
    };
    let result_sections: Vec<_> = study.sections_named("Result").collect();
    let heading = match result_sections.as_slice() {
        [one] => one.heading_line,
        [] => return Err(structure_error("there is no `## Result` section")),
        _ => return Err(structure_error("`## Result` appears more than once")),
    };
    let next_heading = study
        .sections
        .iter()
        .map(|s| s.heading_line)
        .find(|&l| l > heading);

    let mut content = vec![String::new()];
    content.extend(summary.results.iter().map(|l| l.raw.clone()));
    if next_heading.is_some() {
        content.push(String::new());
    }
    let mut doc = editable(&loaded)?;
    doc.replace_section_content(heading, next_heading, &content);
    if status != Status::Reported {
        doc.set_field("status", &["status: reported".to_string()]);
    }
    write_atomic(&loaded.path, &doc.text())?;
    Ok(())
}

// rq-ae6ec611 rq-44085c48
/// Reports a retrospective study without stages from its hand-written result lines.
fn report_hand_written(root: &Path, loaded: &Loaded, status: Status) -> Result<(), StudyError> {
    let study = &loaded.study;
    let result_sections: Vec<_> = study.sections_named("Result").collect();
    if result_sections.len() != 1 {
        return Err(StudyError::Structure {
            path: loaded.path.display().to_string(),
            message: "there must be exactly one `## Result` section".into(),
        });
    }
    let lines = study.result_lines();
    if lines.is_empty() {
        return Err(StudyError::Summary(
            "a retrospective study without stages needs hand-written result lines".into(),
        ));
    }
    for (line_no, raw) in &lines {
        let valid = raw
            .strip_prefix("- ")
            .and_then(|rest| rest.split_once(": ").map(|(key, _)| key))
            .and_then(|key| key.split_once('/'))
            .is_some_and(|(variant, stage)| {
                study.variants().iter().any(|v| v == variant)
                    && crate::study::matches_id_pattern(stage)
            });
        let tag = super::summary::classify_tag(raw);
        let is_post_hoc =
            matches!(tag, Ok((_, ref t)) if t.epistemic == crate::study::Epistemic::PostHoc);
        if !valid || !is_post_hoc {
            return Err(StudyError::Summary(format!(
                "line {line_no} is not a hand-written result line of a listed variant tagged \
                 `post hoc`: {raw}"
            )));
        }
    }
    if status != Status::Reported {
        let mut doc = editable(loaded)?;
        doc.set_field("status", &["status: reported".to_string()]);
        write_atomic(&loaded.path, &doc.text())?;
    }
    let _ = root;
    Ok(())
}

// rq-9f2c5cd1
/// Marks `old` as superseded by `new` and links both studies.
pub fn supersede(root: &Path, old: &str, new: &str) -> Result<(), StudyError> {
    study_dir(root, old)?;
    study_dir(root, new)?;
    if old == new {
        return Err(StudyError::SameStudy(old.to_string()));
    }
    let old_loaded = load(root, old)?;
    let new_loaded = load(root, new)?;
    require_status(
        &old_loaded.study,
        &[Status::Preregistered, Status::Reported],
    )?;

    let mut old_doc = editable(&old_loaded)?;
    old_doc.set_field("status", &["status: superseded".to_string()]);
    old_doc.set_field("superseded_by", &[format!("superseded_by: \"{new}\"")]);

    let existing = new_loaded
        .study
        .front
        .supersedes
        .as_ref()
        .map(|f| f.value.clone())
        .unwrap_or_default();
    if !existing.iter().any(|s| s == old) {
        let mut lines = vec!["supersedes:".to_string()];
        lines.extend(
            existing
                .iter()
                .map(String::as_str)
                .chain([old])
                .map(|s| format!("  - \"{s}\"")),
        );
        let mut new_doc = editable(&new_loaded)?;
        new_doc.set_field("supersedes", &lines);
        write_atomic(&new_loaded.path, &new_doc.text())?;
    }
    write_atomic(&old_loaded.path, &old_doc.text())?;
    Ok(())
}

/// Whether `s` is a 40-character lowercase hex commit.
fn is_commit(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// rq-3250dbea
/// Creates a replication study pointed at a target study and commit.
pub fn replicate(
    root: &Path,
    id: &str,
    question: &str,
    private: bool,
    target: &str,
    commit: &str,
) -> Result<PathBuf, StudyError> {
    if !is_valid_study_id(target)
        || !root
            .join(STUDIES_DIR)
            .join(target)
            .join(STUDY_FILE)
            .is_file()
    {
        return Err(StudyError::ReproducesInvalid(format!(
            "target study `{target}` does not exist"
        )));
    }
    if !is_commit(commit) {
        return Err(StudyError::ReproducesInvalid(format!(
            "`{commit}` is not a 40-character lowercase hex commit"
        )));
    }
    // The target's keys at that commit are what the reproduction compares against.
    let rel = format!("{STUDIES_DIR}/{target}/{SUMMARY_FILE}");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("{commit}:{rel}")])
        .output()
        .map_err(|e| StudyError::ReproducesInvalid(format!("cannot read git history: {e}")))?;
    let text = output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        .ok_or_else(|| {
            StudyError::ReproducesInvalid(format!("`{rel}` cannot be read at commit `{commit}`"))
        })?;
    let summary = super::summary::parse_summary_text(&text).map_err(|e| {
        StudyError::ReproducesInvalid(format!("`{rel}` at `{commit}` cannot be parsed: {e}"))
    })?;
    if summary.study != target {
        return Err(StudyError::ReproducesInvalid(format!(
            "`{rel}` at `{commit}` is for study `{}`, not `{target}`",
            summary.study
        )));
    }

    let dir = create_study(root, id, question, private, Mode::Replication)?;
    let path = dir.join(STUDY_FILE);
    let result = (|| -> Result<(), StudyError> {
        let text = fs::read_to_string(&path)?;
        let mut doc = EditableDoc::new(&text).map_err(|message| StudyError::FrontMatter {
            path: path.display().to_string(),
            line: Some(1),
            message,
        })?;
        doc.set_field(
            "reproduces",
            &[format!("reproduces: \"{target}@{commit}\"")],
        );
        write_atomic(&path, &doc.text())?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

// rq-a8e3636b
/// Records a relation from `study` to `other` in the study's front matter.
pub fn link(root: &Path, study: &str, relation: Relation, other: &str) -> Result<(), StudyError> {
    let invalid = || StudyError::LinkInvalid {
        study: study.to_string(),
        relation,
        other: other.to_string(),
    };
    let loaded = load(root, study)?;
    if !is_valid_study_id(other)
        || other == study
        || !root
            .join(STUDIES_DIR)
            .join(other)
            .join(STUDY_FILE)
            .is_file()
    {
        return Err(invalid());
    }
    let mut existing: Vec<String> = loaded
        .study
        .relation(relation)
        .map(|f| f.value.clone())
        .unwrap_or_default();
    existing.push(other.to_string());
    existing.sort();
    existing.dedup();
    let mut lines = vec![format!("{}:", relation.field())];
    lines.extend(existing.iter().map(|s| format!("  - \"{s}\"")));
    let mut doc = editable(&loaded)?;
    doc.set_field(relation.field(), &lines);
    write_atomic(&loaded.path, &doc.text())?;
    Ok(())
}

// rq-9bc9d54b
/// Migrates one pre-mode study to an explicit mode. Returns whether the file changed.
pub fn migrate(root: &Path, id: &str) -> Result<bool, StudyError> {
    let loaded = load(root, id)?;
    let status_value = loaded.study.front.status.as_ref().map(|f| f.value.clone());
    let mode_value = loaded.study.front.mode.as_ref().map(|f| f.value.clone());
    if let Some(mode) = mode_value {
        if Mode::parse(&mode).is_none() {
            return Err(StudyError::ModeInvalid(mode));
        }
        if status_value.as_deref() == Some("retrospective") {
            return Err(StudyError::Migration(format!(
                "study `{id}` has `status: retrospective` and already carries `mode: {mode}`"
            )));
        }
        return Ok(false);
    }
    match status_value.as_deref() {
        Some("retrospective") => {
            let status = if loaded.study.result_lines().is_empty() {
                "draft"
            } else {
                "reported"
            };
            let mut doc = editable(&loaded)?;
            doc.set_field("mode", &["mode: retrospective".to_string()]);
            doc.set_field("status", &[format!("status: {status}")]);
            write_atomic(&loaded.path, &doc.text())?;
            Ok(true)
        }
        Some(s @ ("draft" | "preregistered" | "reported" | "superseded")) => {
            let _ = s;
            let mut doc = editable(&loaded)?;
            doc.set_field("mode", &["mode: confirmatory".to_string()]);
            write_atomic(&loaded.path, &doc.text())?;
            Ok(true)
        }
        other => Err(StudyError::Migration(format!(
            "study `{id}` has status `{}`, which cannot be migrated",
            other.unwrap_or("missing")
        ))),
    }
}
