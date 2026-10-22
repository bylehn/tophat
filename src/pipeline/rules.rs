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
    // rq-85215682
    let declared = match crate::tools::declared_tools(project.root())
        .and_then(|d| crate::tools::read_lock(project.root()).map(|_| d))
    {
        Ok(declared) => Some(declared),
        Err(e) => {
            let path = if e.to_string().starts_with(crate::tools::LOCK_FILE) {
                crate::tools::LOCK_FILE
            } else {
                crate::tools::TOOLS_FILE
            };
            out.push(Diagnostic {
                rule: "pipeline.tools-invalid",
                severity: Severity::Error,
                path: path.to_string(),
                line: None,
                message: e.to_string(),
                hint: Some("Fix the file; the lock is written by `tophat tools sync`".into()),
            });
            None
        }
    };
    // rq-3898af16
    if let Err(message) = super::external::slurm_defaults(project.root()) {
        out.push(Diagnostic {
            rule: "pipeline.slurm-defaults-invalid",
            severity: Severity::Error,
            path: super::external::SLURM_DEFAULTS_FILE.to_string(),
            line: None,
            message,
            hint: Some(
                "Write `sbatch:` followed by a list of options such as `- --account=e123`".into(),
            ),
        });
    }
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
        // Stages gated in the base or any variant.
        let base_config = super::config::PipelineConfig {
            study: study.clone(),
            variant: "base".into(),
            stages: base.clone(),
        };
        let gated: BTreeSet<String> = base
            .iter()
            .map(|s| s.name.clone())
            .filter(|name| {
                std::iter::once(&base_config)
                    .chain(configs.iter())
                    .any(|c| c.is_gated(name))
            })
            .collect();
        let mut reported = BTreeSet::new();
        for stage in stages {
            // rq-0a10fc96
            let check_missing = stage.check.as_deref().is_none_or(|c| c.trim().is_empty());
            if gated.contains(&stage.name)
                && check_missing
                && reported.insert((stage.file.clone(), stage.name.clone(), "check"))
            {
                let mut d = diagnostic(
                    "pipeline.qc-check-missing",
                    ConfigProblem {
                        file: stage.file.clone(),
                        line: stage.line,
                        message: format!(
                            "stage `{}` has a QC gate but no `check` saying what to look for in its output",
                            stage.name
                        ),
                    },
                    "Add `check:` with what the human should verify before passing this stage",
                );
                d.severity = Severity::Warning;
                out.push(d);
            }
            let reason_missing = stage
                .qc_reason
                .as_deref()
                .is_none_or(|r| r.trim().is_empty());
            if stage.qc_skip
                && reason_missing
                && reported.insert((stage.file.clone(), stage.name.clone(), "qc"))
            {
                out.push(diagnostic(
                    "pipeline.qc-reason-missing",
                    ConfigProblem {
                        file: stage.file.clone(),
                        line: stage.line,
                        message: format!(
                            "stage `{}` declares `qc: skip` without a `qc_reason`",
                            stage.name
                        ),
                    },
                    "Write why this stage's output needs no human verdict in `qc_reason`",
                ));
            }
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
            if !reported.insert((stage.file.clone(), stage.name.clone(), "env")) {
                continue;
            }
            let at = |message: String| ConfigProblem {
                file: stage.file.clone(),
                line: stage.line,
                message: format!("stage `{}`: {message}", stage.name),
            };
            match &stage.tool {
                // An external stage's software comes from the cluster.
                None if stage.external.is_some() => {}
                None => {
                    if let Err(message) = ctx.environment_hash(&stage.env) {
                        out.push(diagnostic(
                            "pipeline.env-missing",
                            at(message),
                            "Add the environment to pixi.toml and run `pixi lock`",
                        ));
                    }
                }
                Some(raw) => {
                    out.extend(tool_diagnostics(project, &ctx, stage, raw, &declared, &at))
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
    // A verdict file is valid for a stage that is gated in at least one variant.
    let mut configs = vec![super::config::PipelineConfig {
        study: study.to_string(),
        variant: "base".into(),
        stages: base.to_vec(),
    }];
    for variant in project
        .list_dir(&format!("{STUDIES_DIR}/{study}/config"))
        .unwrap_or_default()
        .iter()
        .filter_map(|f| f.strip_suffix(".yaml"))
        .filter(|v| *v != "base")
    {
        if let Some(config) = load_variant(project, study, variant, &mut Vec::new()) {
            configs.push(config);
        }
    }
    let gated: Vec<String> = all
        .iter()
        .filter(|name| configs.iter().any(|c| c.is_gated(name)))
        .cloned()
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

// rq-85215682
/// The diagnostics of a tool stage's pin, cache and environment.
fn tool_diagnostics(
    project: &Project,
    ctx: &KeyContext,
    stage: &super::config::StageConfig,
    raw: &str,
    declared: &Option<std::collections::BTreeMap<String, String>>,
    at: &dyn Fn(String) -> ConfigProblem,
) -> Vec<Diagnostic> {
    use crate::tools::{resolve_pin, tool_checkout, ToolPin, ToolsError};
    let one = |rule, severity, message: String, hint: &str| {
        let mut d = diagnostic(rule, at(message), hint);
        d.severity = severity;
        vec![d]
    };
    let Some(pin) = ToolPin::parse(raw) else {
        return one(
            "pipeline.tool-pin-invalid",
            Severity::Error,
            format!("`{raw}` is not a tool pin"),
            "Use <tool>@<7 to 40 lowercase hex characters of a commit>; branches and tags are not pins",
        );
    };
    let Some(declared) = declared else {
        return Vec::new();
    };
    if !declared.contains_key(&pin.name) {
        return one(
            "pipeline.tool-unknown",
            Severity::Error,
            format!("tool `{}` is not declared", pin.name),
            "Declare it with its git URL in .tophat/tools.yaml",
        );
    }
    if let Err(e) = resolve_pin(project.root(), &pin) {
        return one(
            "pipeline.tool-unlocked",
            Severity::Error,
            e.to_string(),
            "Run `tophat tools sync`",
        );
    }
    match tool_checkout(project.root(), &pin) {
        Err(e @ ToolsError::NotCached { .. }) => one(
            "pipeline.tool-not-cached",
            Severity::Warning,
            e.to_string(),
            "Run `tophat tools sync` on this machine before running the study",
        ),
        Err(e @ ToolsError::NotPristine { .. }) => one(
            "pipeline.tool-not-pristine",
            Severity::Error,
            e.to_string(),
            "Tool code is never changed from a study; change it through the tool's own repository",
        ),
        Err(e) => one("pipeline.tools-invalid", Severity::Error, e.to_string(), "Run `tophat tools sync`"),
        Ok(_) => match ctx.stage_environment_hash(stage) {
            Ok(_) => Vec::new(),
            Err(message) => one(
                "pipeline.env-missing",
                Severity::Error,
                message,
                "Give the tool a pixi environment, or add envs/<tool>/pixi.toml and pixi.lock to the project",
            ),
        },
    }
}
