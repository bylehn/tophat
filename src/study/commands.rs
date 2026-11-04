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
    FROZEN_SECTIONS, STUDIES_DIR, STUDY_FILE, SUMMARY_FILE, Status, Study, StudyError,
    frozen_fingerprint, is_valid_question_id, is_valid_study_id, parse_study_text, parse_summary,
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

fn template(id: &str, question: &str, visibility: &str) -> String {
    format!(
        "---
study: {id}
title: \"{id}\"
question: {question}
status: draft
visibility: {visibility}
variants:
  - base
---

# {id}

## Question

<!-- The one question this study answers, in plain words. Locked when you pre-register. -->

## Primary endpoint

<!-- The one number that answers the question (for example \"the mean RMSE difference over all
     signals\"), and how it is measured. If you cannot name it yet, explore first. Locked when you
     pre-register. -->

## Decision rule

<!-- How that number answers the question, with every threshold (for example \"method X is
     better if the 95% interval lies above 0\"). Locked when you pre-register. -->

## Scenarios

<!-- What must be true of the pipeline, one checkable statement per line:
     - S1: <expectation>
     - S2: <expectation>
     Each needs a test under tests/ that mentions th:S1, th:S2, ... (Gherkin blocks tagged @S1
     also work.) Locked when you pre-register. -->

## Variants

<!-- Describe each variant listed in the front matter. `base` uses config/base.yaml; any other
     variant uses config/<variant>.yaml holding only what differs. -->

## Stages

<!-- Describe the pipeline stages. The machine-readable definition lives in config/. -->

## Amendments

<!-- After pre-registration only, one line per change:
     - YYYY-MM-DD — results seen: yes|no — reason -->

## Result

<!-- Written by `tophat study report` from SUMMARY.md. Do not edit by hand. -->

## Interpretation

<!-- Your interpretation of the result. -->
"
    )
}

// rq-f689a538
/// Creates `studies/<id>/` with a draft `STUDY.md`, `config/base.yaml` and an empty `tests/`.
/// Writes nothing if validation fails; removes the partially created directory on I/O failure.
pub fn new_study(
    root: &Path,
    id: &str,
    question: &str,
    private: bool,
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
        fs::write(dir.join(STUDY_FILE), template(id, question, visibility))
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
plan you made in advance rather than by one chosen afterwards. From now on the four sections below
are frozen. If one must change later, the change is recorded as a dated amendment that says
whether you had seen results. The pipeline, its stages, variants and scripts can still change.

The sections to lock:
"
    )?;
    for name in FROZEN_SECTIONS {
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

// rq-ecacda48
/// Copies the result lines of `SUMMARY.md` into the `## Result` section verbatim.
pub fn report(root: &Path, id: &str) -> Result<(), StudyError> {
    let loaded = load(root, id)?;
    let study = &loaded.study;
    let status = require_status(
        study,
        &[
            Status::Preregistered,
            Status::Reported,
            Status::Retrospective,
        ],
    )?;
    let summary_path = study_dir(root, id)?.join(SUMMARY_FILE);
    if status == Status::Retrospective && !summary_path.is_file() {
        return Err(StudyError::Summary(
            "a retrospective study can only be reported from an existing SUMMARY.md".into(),
        ));
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
    if status == Status::Preregistered {
        doc.set_field("status", &["status: reported".to_string()]);
    }
    write_atomic(&loaded.path, &doc.text())?;
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
        &[
            Status::Preregistered,
            Status::Reported,
            Status::Retrospective,
        ],
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
