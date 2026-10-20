//! The `pipeline` rule family (configuration rules).

use std::collections::BTreeSet;

use super::config::{config_path, load_base, load_variant, ConfigProblem};
use super::keys::KeyContext;
use crate::check::{Diagnostic, Project, Severity};
use crate::study::STUDIES_DIR;

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
