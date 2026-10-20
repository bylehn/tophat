//! `tophat study summarize`: writing `SUMMARY.md` from sealed result stages, and detecting stale
//! summaries.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use super::config::StageInput;
use super::provenance::{ProvenanceRecord, ResolvedStage};
use super::PipelineError;
use crate::fsutil::write_atomic;
use crate::study::{
    parse_study_text, ResultLine, Study, Summary, STUDIES_DIR, STUDY_FILE, SUMMARY_FILE,
};

/// Reads and parses a study's `STUDY.md`.
pub(crate) fn read_study(root: &Path, study: &str) -> Result<Study, PipelineError> {
    let rel = format!("{STUDIES_DIR}/{study}/{STUDY_FILE}");
    let text = fs::read_to_string(root.join(&rel))
        .map_err(|_| PipelineError::Study(format!("study `{study}` has no {STUDY_FILE}")))?;
    parse_study_text(study, &rel, &text).map_err(|e| PipelineError::Study(e.to_string()))
}

/// Whether the record of `stage` (and, transitively, every upstream record and fan-out item record)
/// is clean. A missing upstream record counts as unclean.
fn lineage_clean(resolved: &ResolvedStage, stage: &str, seen: &mut BTreeSet<String>) -> bool {
    if !seen.insert(stage.to_string()) {
        return true;
    }
    let Ok(dir) = resolved.out_dir(stage, false) else {
        return false;
    };
    let Some(record) = ProvenanceRecord::read(&dir).filter(|r| r.key == resolved.key(stage)) else {
        return false;
    };
    if !record.clean {
        return false;
    }
    for item in record.items.iter().flatten() {
        if !ProvenanceRecord::read(&dir.join("items").join(&item.name)).is_some_and(|r| r.clean) {
            return false;
        }
    }
    let Ok(config) = resolved.stage(stage) else {
        return false;
    };
    config.inputs.values().all(|input| match input {
        StageInput::Stage(upstream) => lineage_clean(resolved, upstream, seen),
        StageInput::Dataset(_) => true,
    })
}

fn head_commit(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// rq-b9512459 rq-1a4b027f
/// Computes the summary of a study from the sealed outputs of its result stages.
pub fn summarize(root: &Path, study: &str) -> Result<Summary, PipelineError> {
    let doc = read_study(root, study)?;
    let variants = doc.variants().to_vec();
    if variants.is_empty() {
        return Err(PipelineError::Study(format!(
            "study `{study}` lists no variants"
        )));
    }
    let commit = head_commit(root).ok_or_else(|| {
        PipelineError::Io("summaries need a git repository with at least one commit".into())
    })?;

    let mut results = Vec::new();
    let mut keys = BTreeMap::new();
    let mut missing = Vec::new();
    let mut dirty = false;
    for variant in &variants {
        let resolved = ResolvedStage::load(root, study, variant)?;
        let result_stages: Vec<_> = resolved
            .config
            .stages
            .iter()
            .filter(|s| s.result.is_some())
            .collect();
        if result_stages.is_empty() {
            return Err(PipelineError::Config {
                path: format!("{STUDIES_DIR}/{study}/config/base.yaml"),
                line: None,
                message: "the study has no result stages (no stage declares `result`)".into(),
            });
        }
        for stage in result_stages {
            let label = format!("{variant}/{}", stage.name);
            let key = resolved.key(&stage.name).to_string();
            let dir = resolved.out_dir(&stage.name, false)?;
            let sealed = ProvenanceRecord::read(&dir).is_some_and(|r| r.key == key && !r.trial);
            let text = fs::read_to_string(dir.join(stage.result.as_ref().expect("result stage")))
                .unwrap_or_default();
            let lines: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            match (sealed, lines.as_slice()) {
                (true, [line]) => {
                    dirty |= !lineage_clean(&resolved, &stage.name, &mut BTreeSet::new());
                    results.push(ResultLine {
                        variant: variant.clone(),
                        stage: stage.name.clone(),
                        text: line.to_string(),
                        raw: format!("- {label}: {line}"),
                        line: 0,
                    });
                    keys.insert(label, key);
                }
                (true, _) => {
                    return Err(PipelineError::BadResult(format!(
                        "the result file of {label} must contain exactly one non-empty line"
                    )))
                }
                (false, _) => missing.push(label),
            }
        }
    }
    if !missing.is_empty() {
        return Err(PipelineError::NotSealed(format!(
            "results not sealed under their current keys: {}",
            missing.join(", ")
        )));
    }
    Ok(Summary {
        study: study.to_string(),
        commit,
        dirty,
        dirty_line: 4,
        generated: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        tophat_version: env!("CARGO_PKG_VERSION").to_string(),
        keys: Some(keys),
        results,
    })
}

/// Renders a summary as `SUMMARY.md`.
pub fn render_summary(summary: &Summary) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("strings serialize");
    let mut out = format!(
        "---\nstudy: {}\ncommit: {}\ndirty: {}\ngenerated: {}\ntophat_version: {}\n",
        q(&summary.study),
        q(&summary.commit),
        summary.dirty,
        q(&summary.generated),
        q(&summary.tophat_version)
    );
    if let Some(keys) = &summary.keys {
        out.push_str("keys:\n");
        for (label, key) in keys {
            out.push_str(&format!("  {}: {}\n", q(label), q(key)));
        }
    }
    out.push_str("---\n\n## Results\n\n");
    for line in &summary.results {
        out.push_str(&line.raw);
        out.push('\n');
    }
    out
}

/// Writes `SUMMARY.md`, leaving the file untouched when only the `generated` timestamp would change.
/// Returns whether the file was written.
pub fn write_summary(root: &Path, summary: &Summary) -> Result<bool, PipelineError> {
    let path = root
        .join(STUDIES_DIR)
        .join(&summary.study)
        .join(SUMMARY_FILE);
    let text = render_summary(summary);
    let without_generated = |t: &str| {
        t.lines()
            .filter(|l| !l.starts_with("generated:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if fs::read_to_string(&path)
        .is_ok_and(|old| without_generated(&old) == without_generated(&text))
    {
        return Ok(false);
    }
    write_atomic(&path, &text).map_err(|e| PipelineError::Io(e.to_string()))?;
    Ok(true)
}

/// The `<variant>/<stage>` entries of a summary's `keys` that no longer match the current
/// configuration: a different key, a stage that is no longer a result stage, or a variant that no
/// longer exists. Fails if the configuration cannot be loaded.
pub fn stale_entries(
    root: &Path,
    study: &str,
    keys: &BTreeMap<String, String>,
) -> Result<Vec<String>, PipelineError> {
    let mut stale = Vec::new();
    let mut by_variant: BTreeMap<&str, Vec<(&str, &String)>> = BTreeMap::new();
    for (label, key) in keys {
        match label.split_once('/') {
            Some((variant, stage)) => by_variant.entry(variant).or_default().push((stage, key)),
            None => stale.push(label.clone()),
        }
    }
    for (variant, entries) in by_variant {
        let resolved = match ResolvedStage::load(root, study, variant) {
            Ok(r) => r,
            Err(PipelineError::UnknownVariant(_)) => {
                stale.extend(
                    entries
                        .iter()
                        .map(|(stage, _)| format!("{variant}/{stage}")),
                );
                continue;
            }
            Err(e) => return Err(e),
        };
        for (stage, key) in entries {
            let current = resolved
                .config
                .stage(stage)
                .filter(|s| s.result.is_some())
                .map(|_| resolved.key(stage));
            if current != Some(key.as_str()) {
                stale.push(format!("{variant}/{stage}"));
            }
        }
    }
    Ok(stale)
}
