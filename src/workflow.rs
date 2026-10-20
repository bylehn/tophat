//! `tophat status` and `tophat run`: where each study is, its next action, and running its
//! pipeline. Specified in `rqm/workflow.md`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

use crate::check::{run_check, Diagnostic, Project, Severity};
use crate::pipeline::{
    key_status, load_pipeline, qc_status, stale_entries, variant_target, StageInput, Verdict,
};
use crate::question::{parse_project, QuestionStatus, PROJECT_FILE};
use crate::study::{
    parse_study_text, parse_summary, Status, Study, StudyError, STUDIES_DIR, STUDY_FILE,
    SUMMARY_FILE,
};

/// Who may take a next action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// Only a human.
    You,
    /// A human or an agent.
    Anyone,
    /// Nothing to do.
    Nobody,
}

impl Who {
    pub fn as_str(self) -> &'static str {
        match self {
            Who::You => "you",
            Who::Anyone => "anyone",
            Who::Nobody => "nobody",
        }
    }
}

// rq-6ceae812
/// A study's next action: the row of the next-action table that applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextAction {
    pub row: u8,
    pub who: Who,
    pub text: String,
    pub command: Option<String>,
}

impl NextAction {
    fn new(row: u8, who: Who, text: impl Into<String>, command: Option<String>) -> Self {
        NextAction {
            row,
            who,
            text: text.into(),
            command,
        }
    }

    /// The `next …` line printed by `tophat status` and `tophat run`.
    pub fn line(&self) -> String {
        match self.who {
            Who::Nobody => format!("next: {}", self.text),
            who => format!("next ({}): {}", who.as_str(), self.text),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"row": self.row, "who": self.who.as_str(), "text": self.text, "command": self.command})
    }
}

/// Pipeline progress of one variant: `(sealed, total)`, or `None` when the results root is
/// unconfigured.
pub type Progress = Option<(u32, u32)>;

// rq-d511d6c8
/// The per-study part of `tophat status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyStatus {
    pub id: String,
    pub status: Option<String>,
    pub question: Option<String>,
    pub question_text: Option<String>,
    pub superseded_by: Option<String>,
    /// Per listed variant, in `STUDY.md` order; `None` for studies without stages.
    pub pipeline: Option<Vec<(String, Progress)>>,
    pub next: NextAction,
}

/// Project-wide facts shared by every study's evaluation.
struct ProjectView {
    diagnostics: Vec<Diagnostic>,
    questions: crate::question::QuestionTree,
}

impl ProjectView {
    fn load(root: &Path) -> Result<ProjectView, StudyError> {
        let diagnostics = run_check(root, &[]).map_err(|e| StudyError::Io(e.to_string()))?;
        let questions = std::fs::read_to_string(root.join(PROJECT_FILE))
            .map(|t| parse_project(&t))
            .unwrap_or_default();
        Ok(ProjectView {
            diagnostics,
            questions,
        })
    }
}

// rq-b1f6d54f
/// The `error` diagnostics that block a study: those under its directory, under the manifests of
/// datasets its stages reference, or on the roots files.
fn blocking_errors<'a>(root: &Path, view: &'a ProjectView, study: &str) -> Vec<&'a Diagnostic> {
    let study_dir = format!("{STUDIES_DIR}/{study}");
    let datasets: Vec<String> = crate::data::study_dataset_refs(&Project::new(root), study)
        .into_iter()
        .filter_map(|r| r.reference.map(|r| format!("datasets/{}", r.name)))
        .collect();
    let under = |path: &str, dir: &str| path == dir || path.starts_with(&format!("{dir}/"));
    view.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        // Running the pipeline is what resolves a stale summary (row 9), so it never blocks.
        .filter(|d| d.rule != "pipeline.summary-stale")
        .filter(|d| {
            under(&d.path, &study_dir)
                || datasets.iter().any(|dir| under(&d.path, dir))
                || d.path == ".tophat/roots.yaml"
                || d.path == ".tophat/roots.local.yaml"
        })
        .collect()
}

fn read_study(root: &Path, study: &str) -> Result<Option<Study>, StudyError> {
    let rel = format!("{STUDIES_DIR}/{study}/{STUDY_FILE}");
    let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
        return Err(StudyError::StudyNotFound(study.to_string()));
    };
    Ok(parse_study_text(study, &rel, &text).ok())
}

/// Whether the study's `config/base.yaml` lists at least one stage (whether or not the
/// configuration is otherwise valid; invalid configurations are reported by `tophat check`).
fn has_stages(root: &Path, study: &str) -> bool {
    std::fs::read_to_string(root.join(STUDIES_DIR).join(study).join("config/base.yaml"))
        .ok()
        .and_then(|t| yaml_rust2::YamlLoader::load_from_str(&t).ok())
        .and_then(|docs| docs.into_iter().next())
        .is_some_and(|doc| doc["stages"].as_vec().is_some_and(|v| !v.is_empty()))
}

/// Pipeline progress per variant.
fn progress(root: &Path, study: &str, variants: &[String]) -> Vec<(String, Progress)> {
    variants
        .iter()
        .map(|v| {
            let p = key_status(root, study, v).ok().and_then(|stages| {
                if stages.iter().any(|s| s.status == "unavailable") {
                    None
                } else {
                    let sealed = stages.iter().filter(|s| s.status == "sealed").count() as u32;
                    Some((sealed, stages.len() as u32))
                }
            });
            (v.clone(), p)
        })
        .collect()
}

// rq-956cc0ad
/// The tool pins of a study (any variant) that cannot be used on this machine yet: unresolved or
/// not cached.
fn missing_tools(root: &Path, variants: &[String], study: &str) -> Vec<String> {
    let project = Project::new(root);
    let mut missing = Vec::new();
    for variant in variants {
        let Some(config) = crate::pipeline::load_variant_lenient(&project, study, variant) else {
            continue;
        };
        for raw in config.stages.iter().filter_map(|s| s.tool.as_deref()) {
            let Some(pin) = crate::tools::ToolPin::parse(raw) else {
                continue;
            };
            let unusable = matches!(
                crate::tools::tool_checkout(root, &pin),
                Err(crate::tools::ToolsError::NotCached { .. }
                    | crate::tools::ToolsError::Unlocked { .. })
            );
            if unusable && !missing.contains(&raw.to_string()) {
                missing.push(raw.to_string());
            }
        }
    }
    missing
}

// rq-9f1e97a5
/// Row 8: an external job whose folder is written for a current key and has not been returned.
fn external_action(root: &Path, study: &str, variants: &[String]) -> Option<NextAction> {
    variants.iter().find_map(|variant| {
        let stage = crate::pipeline::waiting_jobs(root, study, variant)
            .into_iter()
            .next()?;
        let cmd = format!("tophat stage instructions {study} {stage} --variant {variant}");
        Some(NextAction::new(
            8,
            Who::You,
            format!("Run the external job for {stage} ({variant}): {cmd}"),
            Some(cmd),
        ))
    })
}

/// Rows 9 to 12: QC gates, unrun stages, and the summary.
fn pipeline_action(root: &Path, study: &Study, variants: &[String]) -> Option<NextAction> {
    let id = &study.id;
    let run_cmd = format!("tophat run {id}");
    let mut qc_needed: Option<NextAction> = None;
    let mut qc_failed: Option<NextAction> = None;
    let mut unsealed = false;
    let mut has_result_stage = false;
    for variant in variants {
        let Ok(config) = load_pipeline(root, id, variant) else {
            continue;
        };
        let Ok(stages) = key_status(root, id, variant) else {
            continue;
        };
        let state: BTreeMap<&str, (&str, &str)> = stages
            .iter()
            .map(|s| (s.stage.as_str(), (s.key.as_str(), s.status)))
            .collect();
        unsealed |= stages.iter().any(|s| s.status != "sealed");
        has_result_stage |= config.stages.iter().any(|s| s.result.is_some());
        for stage in config.stages.iter().filter(|s| config.is_gated(&s.name)) {
            let (key, sealed) = state[stage.name.as_str()];
            if sealed != "sealed" {
                continue;
            }
            match qc_status(root, id, &stage.name, key) {
                Ok(Verdict::Pass) => {}
                Ok(Verdict::Fail) => {
                    qc_failed.get_or_insert_with(|| {
                        NextAction::new(
                            10,
                            Who::Anyone,
                            format!(
                                "Fix the cause of the failed QC of {} ({variant}); the next run uses a new key",
                                stage.name
                            ),
                            None,
                        )
                    });
                }
                Err(_) => {
                    let consumer_unsealed = config.stages.iter().any(|c| {
                        c.inputs
                            .values()
                            .any(|i| matches!(i, StageInput::Stage(s) if *s == stage.name))
                            && state[c.name.as_str()].1 != "sealed"
                    });
                    if consumer_unsealed {
                        let cmd =
                            format!("tophat stage qc {id} {} --variant {variant}", stage.name);
                        qc_needed.get_or_insert_with(|| {
                            NextAction::new(
                                9,
                                Who::You,
                                format!("Give a QC verdict: {cmd}"),
                                Some(cmd),
                            )
                        });
                    }
                }
            }
        }
    }
    if let Some(action) = qc_needed.or(qc_failed) {
        return Some(action);
    }
    let summary = parse_summary(&root.join(STUDIES_DIR).join(id).join(SUMMARY_FILE)).ok();
    let stale = summary.as_ref().is_some_and(|s| {
        s.keys
            .as_ref()
            .is_some_and(|keys| stale_entries(root, id, keys).map_or(true, |e| !e.is_empty()))
    });
    if unsealed || (has_result_stage && (summary.is_none() || stale)) {
        return Some(NextAction::new(
            11,
            Who::Anyone,
            format!("Run the pipeline: {run_cmd}"),
            Some(run_cmd),
        ));
    }
    if summary.is_some_and(|s| s.dirty) {
        return Some(NextAction::new(
            12,
            Who::Anyone,
            format!("Commit the code listed as unclean, then {run_cmd}"),
            Some(run_cmd),
        ));
    }
    None
}

// rq-956cc0ad
fn evaluate(root: &Path, view: &ProjectView, study_id: &str) -> Result<StudyStatus, StudyError> {
    let study = read_study(root, study_id)?;
    let fix = |id: &str| {
        let cmd = format!("tophat check {STUDIES_DIR}/{id}");
        NextAction::new(2, Who::Anyone, format!("Fix the errors: {cmd}"), Some(cmd))
    };
    let Some(study) = study else {
        return Ok(StudyStatus {
            id: study_id.to_string(),
            status: None,
            question: None,
            question_text: None,
            superseded_by: None,
            pipeline: None,
            next: fix(study_id),
        });
    };
    let status = study.status();
    let question = study.front.question.as_ref().map(|f| f.value.clone());
    let question_text = question.as_ref().and_then(|q| {
        view.questions
            .index_of(q)
            .map(|i| view.questions.questions[i].text.clone())
    });
    let superseded_by = study.front.superseded_by.as_ref().map(|f| f.value.clone());
    let variants = study.variants().to_vec();
    let stages = has_stages(root, study_id);
    let pipeline = stages.then(|| progress(root, study_id, &variants));
    let mut result = StudyStatus {
        id: study_id.to_string(),
        status: study.front.status.as_ref().map(|f| f.value.clone()),
        question: question.clone(),
        question_text,
        superseded_by: superseded_by.clone(),
        pipeline,
        next: NextAction::new(15, Who::Nobody, "None: done", None),
    };

    let id = study_id;
    let next = if status == Some(Status::Superseded) {
        NextAction::new(
            1,
            Who::Nobody,
            format!(
                "None: superseded by {}",
                superseded_by.as_deref().unwrap_or("?")
            ),
            None,
        )
    } else if !blocking_errors(root, view, id).is_empty() {
        fix(id)
    } else if status == Some(Status::Draft)
        && (["Question", "Primary endpoint", "Decision rule"]
            .iter()
            .any(|s| study.section_is_empty(s))
            || study.scenarios.is_empty())
    {
        NextAction::new(3, Who::Anyone, "Write the plan in STUDY.md", None)
    } else if status == Some(Status::Draft) {
        let cmd = format!("tophat study preregister {id}");
        NextAction::new(4, Who::You, format!("Pre-register: {cmd}"), Some(cmd))
    } else if let Some(action) = (|| {
        if status == Some(Status::Retrospective) && !stages {
            return None;
        }
        if !stages {
            return Some(NextAction::new(
                5,
                Who::Anyone,
                "Add stages to config/base.yaml",
                None,
            ));
        }
        let roots = crate::data::load_roots(root).ok();
        if roots
            .as_ref()
            .and_then(|r| r.location(crate::data::RESULTS_ROOT))
            .is_none()
        {
            return Some(NextAction::new(
                6,
                Who::Anyone,
                "Configure the results data root",
                None,
            ));
        }
        if !missing_tools(root, &variants, id).is_empty() {
            let cmd = "tophat tools sync".to_string();
            return Some(NextAction::new(
                7,
                Who::Anyone,
                format!("Fetch tools: {cmd}"),
                Some(cmd),
            ));
        }
        external_action(root, id, &variants).or_else(|| pipeline_action(root, &study, &variants))
    })() {
        action
    } else if status == Some(Status::Preregistered) {
        let cmd = format!("tophat study report {id}");
        NextAction::new(13, Who::Anyone, format!("Report: {cmd}"), Some(cmd))
    } else if let Some(q) = question.as_ref().filter(|q| {
        let tree = &view.questions;
        let open = tree
            .index_of(q)
            .is_some_and(|i| tree.questions[i].parsed_status() == Some(QuestionStatus::Open));
        let cited = tree.questions.iter().any(|other| {
            other.parsed_status() == Some(QuestionStatus::Answered)
                && other
                    .evidence
                    .as_ref()
                    .is_some_and(|(citations, _)| citations.iter().any(|c| c == id))
        });
        open && !cited
    }) {
        NextAction::new(
            14,
            Who::Anyone,
            format!("Answer question {q} in PROJECT.md, citing this study"),
            None,
        )
    } else {
        NextAction::new(15, Who::Nobody, "None: done", None)
    };
    result.next = next;
    Ok(result)
}

// rq-bf93285a
/// Evaluates the next-action table for one study.
pub fn next_action(root: &Path, study: &str) -> Result<NextAction, StudyError> {
    let view = ProjectView::load(root)?;
    evaluate(root, &view, study).map(|s| s.next)
}

// rq-02e65d51
/// The data behind `tophat status`: every study when `studies` is empty, otherwise those named.
pub fn study_status(root: &Path, studies: &[String]) -> Result<Vec<StudyStatus>, StudyError> {
    let view = ProjectView::load(root)?;
    let ids: Vec<String> = if studies.is_empty() {
        Project::new(root)
            .list_dir(STUDIES_DIR)
            .unwrap_or_default()
            .into_iter()
            .filter(|id| root.join(STUDIES_DIR).join(id).join(STUDY_FILE).is_file())
            .collect()
    } else {
        studies.to_vec()
    };
    ids.iter().map(|id| evaluate(root, &view, id)).collect()
}

/// Counts of questions by status, and of check diagnostics by severity.
pub fn project_summary(root: &Path) -> Result<Value, StudyError> {
    let view = ProjectView::load(root)?;
    let count_q = |s: QuestionStatus| {
        view.questions
            .questions
            .iter()
            .filter(|q| q.parsed_status() == Some(s))
            .count()
    };
    let count_d = |s: Severity| view.diagnostics.iter().filter(|d| d.severity == s).count();
    Ok(json!({
        "questions": {
            "open": count_q(QuestionStatus::Open),
            "answered": count_q(QuestionStatus::Answered),
            "dropped": count_q(QuestionStatus::Dropped),
        },
        "check": {
            "errors": count_d(Severity::Error),
            "warnings": count_d(Severity::Warning),
            "legacy": count_d(Severity::Legacy),
        },
    }))
}

// rq-6733b7c0
/// Renders `tophat status` as text.
pub fn render_status(project: &Value, studies: &[StudyStatus]) -> String {
    let q = &project["questions"];
    let c = &project["check"];
    let mut out = format!(
        "questions: {} open, {} answered, {} dropped; check: {} error(s), {} warning(s), {} legacy item(s)\n",
        q["open"], q["answered"], q["dropped"], c["errors"], c["warnings"], c["legacy"]
    );
    let (superseded, active): (Vec<_>, Vec<_>) = studies
        .iter()
        .partition(|s| s.status.as_deref() == Some("superseded"));
    for s in &active {
        out.push('\n');
        let mut header = vec![s.id.clone(), s.status.clone().unwrap_or_else(|| "?".into())];
        if let Some(q) = &s.question {
            header.push(q.clone());
        }
        if let Some(text) = &s.question_text {
            header.push(text.clone());
        }
        out.push_str(&header.join("  "));
        out.push('\n');
        if let Some(pipeline) = &s.pipeline {
            let parts: Vec<String> = pipeline
                .iter()
                .map(|(v, p)| match p {
                    Some((sealed, total)) => format!("{v} {sealed}/{total} sealed"),
                    None => format!("{v} unavailable"),
                })
                .collect();
            out.push_str(&format!("  pipeline: {}\n", parts.join(", ")));
        }
        out.push_str(&format!("  {}\n", s.next.line()));
    }
    if !superseded.is_empty() {
        out.push('\n');
        for s in superseded {
            out.push_str(&format!(
                "{}  superseded by {}\n",
                s.id,
                s.superseded_by.as_deref().unwrap_or("?")
            ));
        }
    }
    out
}

/// Renders `tophat status --json`.
pub fn status_json(project: &Value, studies: &[StudyStatus]) -> Value {
    json!({
        "project": project,
        "studies": studies.iter().map(|s| json!({
            "id": s.id,
            "status": s.status,
            "question": s.question,
            "pipeline": s.pipeline.as_ref().map(|p| p.iter().map(|(v, progress)| (v.clone(), match progress {
                Some((sealed, total)) => json!({"sealed": sealed, "total": total}),
                None => Value::Null,
            })).collect::<serde_json::Map<_, _>>()),
            "next": s.next.to_json(),
        })).collect::<Vec<_>>(),
    })
}

/// Options of `tophat run`.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub variant: Option<String>,
    pub cores: Option<usize>,
    pub draft: bool,
    pub snakemake_args: Vec<String>,
}

/// Why `tophat run` refused to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

// rq-cdc39471
/// Checks that a study may be run and returns the Snakemake command line (after `pixi`).
pub fn run_command(root: &Path, study: &str, options: &RunOptions) -> Result<Vec<String>, Refusal> {
    let refuse = |m: String| Err(Refusal(m));
    let doc = match read_study(root, study) {
        Err(_) => return refuse(format!("study `{study}` does not exist")),
        Ok(None) => {
            return refuse(format!(
                "{STUDIES_DIR}/{study}/{STUDY_FILE} cannot be parsed; run `tophat check`"
            ))
        }
        Ok(Some(doc)) => doc,
    };
    let status = doc.status();
    if status == Some(Status::Superseded) {
        return refuse(format!(
            "study `{study}` is superseded; run the study that replaced it"
        ));
    }
    if !has_stages(root, study) {
        return refuse(format!(
            "study `{study}` has no stages: add stages to config/base.yaml"
        ));
    }
    match (status == Some(Status::Draft), options.draft) {
        (true, false) => {
            return refuse(format!(
                "study `{study}` is a draft. Results seen before pre-registration cannot be reported, so \
                 pre-register first (`tophat study preregister {study}`), or pass --draft for a trial run \
                 whose results are kept apart and can never be reported"
            ))
        }
        (false, true) => {
            return refuse(format!(
                "--draft is only for draft studies; study `{study}` is not a draft"
            ))
        }
        _ => {}
    }
    if let Some(v) = &options.variant {
        if !doc.variants().contains(v) {
            return refuse(format!("variant `{v}` is not listed in STUDY.md"));
        }
    }
    let missing = missing_tools(root, doc.variants(), study);
    if !missing.is_empty() {
        return refuse(format!(
            "these tool versions are not on this machine yet: {}; run `tophat tools sync` first",
            missing.join(", ")
        ));
    }
    let view = ProjectView::load(root).map_err(|e| Refusal(e.to_string()))?;
    let errors = blocking_errors(root, &view, study);
    if !errors.is_empty() {
        let list: Vec<String> = errors
            .iter()
            .map(|d| {
                let line = d.line.map(|l| format!(":{l}")).unwrap_or_default();
                format!("  {}{line}: [{}] {}", d.path, d.rule, d.message)
            })
            .collect();
        return refuse(format!(
            "study `{study}` has check errors; fix them first:\n{}",
            list.join("\n")
        ));
    }
    let cores = options
        .cores
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let target = match (&options.variant, options.draft) {
        (None, false) => "summary".to_string(),
        (None, true) => "trial".to_string(),
        (Some(v), trial) => variant_target(v, trial),
    };
    let mut args = vec![
        "run".to_string(),
        "snakemake".to_string(),
        "-s".to_string(),
        format!("{STUDIES_DIR}/{study}/Snakefile"),
        "--cores".to_string(),
        cores.to_string(),
        target,
    ];
    args.extend(options.snakemake_args.iter().cloned());
    Ok(args)
}

/// Runs `pixi` with the given arguments from the project root, attached to the terminal.
/// Returns its exit code, or `None` if it could not be started.
pub fn run_pixi(root: &Path, args: &[String]) -> Option<i32> {
    let status = Command::new("pixi")
        .args(args)
        .current_dir(root)
        .status()
        .ok()?;
    Some(status.code().unwrap_or(1))
}
