//! The `pipeline` rule family (configuration rules).

use std::collections::BTreeSet;

use super::config::{config_path, load_base, load_variant, ConfigProblem};
use super::generate::{snakefile_path, stub};
use super::keys::KeyContext;
use super::qc::qc_file_problems;
use super::summary::stale_entries;
use crate::check::{Diagnostic, Project, Severity};
use crate::study::{
    parse_study_text, parse_summary, Status, STUDIES_DIR, STUDY_FILE, SUMMARY_FILE,
};

fn diagnostic(rule: &'static str, problem: ConfigProblem, hint: &str) -> Diagnostic {
    Diagnostic {
        rule,
        severity: Severity::Error,
        path: problem.file,
        line: problem.line,
        message: problem.message,
        hint: Some(hint.to_string()),
    }
}

// rq-55bef4eb rq-22c3d944
/// The `pipeline` rule family.
pub fn pipeline_rules(project: &Project) -> Vec<Diagnostic> {
    let ctx = KeyContext::new(project.root());
    let mut out = Vec::new();
    for study in project.list_dir(STUDIES_DIR).unwrap_or_default() {
        let mut problems = Vec::new();
        let Some(base) = load_base(project, &study, &mut problems) else {
            out.extend(
                problems.into_iter().map(|p| {
                    diagnostic("pipeline.config-invalid", p, "Fix the stage configuration")
                }),
            );
            continue;
        };
        if base.is_empty() && problems.is_empty() {
            continue;
        }
        if !base.is_empty() {
            check_execution(project, &study, &base, &mut out);
        }

        let variants: Vec<String> = project
            .list_dir(&format!("{STUDIES_DIR}/{study}/config"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| f.strip_suffix(".yaml").map(str::to_string))
            .filter(|v| v != "base")
            .collect();
        let mut configs = Vec::new();
        for variant in &variants {
            if let Some(config) = load_variant(project, &study, variant, &mut problems) {
                configs.push(config);
            }
        }
        let mut seen = BTreeSet::new();
        for p in problems {
            if seen.insert((p.file.clone(), p.line, p.message.clone())) {
                out.push(diagnostic(
                    "pipeline.config-invalid",
                    p,
                    "See rqm/pipeline/provenance.md for the stage fields",
                ));
            }
        }

        // Code and environments, checked on the base stages and on any stage a variant changed.
        let base_file = config_path(&study, "base");
        let mut stages: Vec<_> = base.iter().collect();
        for config in &configs {
            stages.extend(config.stages.iter().filter(|s| s.file != base_file));
        }
        let mut reported = BTreeSet::new();
        for stage in stages {
            if let Err(message) = ctx.code_files(&study, stage) {
                if reported.insert((stage.file.clone(), stage.name.clone(), "code")) {
                    out.push(diagnostic(
                        "pipeline.code-missing",
                        ConfigProblem {
                            file: stage.file.clone(),
                            line: stage.line,
                            message: format!("stage `{}`: {message}", stage.name),
                        },
                        "List files or directories that exist inside the project",
                    ));
                }
            }
            if let Err(message) = ctx.environment_hash(&stage.env) {
                if reported.insert((stage.file.clone(), stage.name.clone(), "env")) {
                    out.push(diagnostic(
                        "pipeline.env-missing",
                        ConfigProblem {
                            file: stage.file.clone(),
                            line: stage.line,
                            message: format!("stage `{}`: {message}", stage.name),
                        },
                        "Add the environment to pixi.toml and run `pixi lock`",
                    ));
                }
            }
        }
    }
    out
}

fn error(
    rule: &'static str,
    severity: Severity,
    path: String,
    line: Option<u32>,
    message: String,
    hint: &str,
) -> Diagnostic {
    Diagnostic {
        rule,
        severity,
        path,
        line,
        message,
        hint: Some(hint.to_string()),
    }
}

// rq-7b0b2ae8
/// The execution rules: the Snakefile stub, QC verdict files, and summary staleness.
fn check_execution(
    project: &Project,
    study: &str,
    base: &[super::config::StageConfig],
    out: &mut Vec<Diagnostic>,
) {
    let snakefile = snakefile_path(study);
    match project.read_to_string(&snakefile) {
        Err(_) => out.push(error(
            "pipeline.snakefile-missing",
            Severity::Error,
            snakefile,
            None,
            "The study has stages but no Snakefile".into(),
            &format!("Run `tophat pipeline stub {study}`"),
        )),
        Ok(text) if text != stub(study) => out.push(error(
            "pipeline.snakefile-modified",
            Severity::Error,
            snakefile,
            None,
            "The Snakefile differs from the generated stub; rules come from config/ only".into(),
            &format!("Run `tophat pipeline stub {study}` and express changes in config/"),
        )),
        Ok(_) => {}
    }

    let all: Vec<String> = base.iter().map(|s| s.name.clone()).collect();
    let gated: Vec<String> = base
        .iter()
        .filter(|s| s.qc)
        .map(|s| s.name.clone())
        .collect();
    for (path, message) in qc_file_problems(project.root(), study, &all, &gated) {
        out.push(error(
            "pipeline.qc-invalid",
            Severity::Error,
            path,
            None,
            format!("Invalid QC verdict file: {message}"),
            "Verdicts are written by `tophat stage qc`",
        ));
    }

    let summary_path = format!("{STUDIES_DIR}/{study}/{SUMMARY_FILE}");
    let Ok(summary) = parse_summary(&project.root().join(&summary_path)) else {
        return;
    };
    let Some(keys) = summary.keys else { return };
    let Ok(stale) = stale_entries(project.root(), study, &keys) else {
        return;
    };
    if stale.is_empty() {
        return;
    }
    let study_file = format!("{STUDIES_DIR}/{study}/{STUDY_FILE}");
    let status = project
        .read_to_string(&study_file)
        .ok()
        .and_then(|t| parse_study_text(study, &study_file, &t).ok())
        .and_then(|s| s.status());
    let severity = match status {
        Some(Status::Preregistered | Status::Reported) => Severity::Error,
        _ => Severity::Warning,
    };
    out.push(error(
        "pipeline.summary-stale",
        severity,
        summary_path,
        None,
        format!(
            "SUMMARY.md was produced under keys that no longer match the configuration: {}",
            stale.join(", ")
        ),
        &format!("Run `tophat run {study}` to bring the results up to date"),
    ));
}
