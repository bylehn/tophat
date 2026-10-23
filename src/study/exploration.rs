//! The exploration record of a study, `EXPLORATION.md`: what its trial runs and scratch work saw
//! before the plan was locked, and whether that included the data that decides the question.
//! Specified in `rqm/study/exploration.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{StudyError, STUDIES_DIR};
use crate::pipeline::{
    compute_keys, load_pipeline, output_dir, KeyContext, ProvenanceRecord, StageInput,
};

/// File name of the exploration record, in the study directory.
pub const EXPLORATION_FILE: &str = "EXPLORATION.md";

/// The trial result of one result stage in one variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialResult {
    pub variant: String,
    pub stage: String,
    /// The current trial key of the stage.
    pub key: String,
    /// The result line, or `None` when no trial output is sealed for the key.
    pub line: Option<String>,
}

// rq-36d7d1e3
/// The content of `EXPLORATION.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exploration {
    pub study: String,
    pub trial_results: Vec<TrialResult>,
    /// Dataset versions consumed by sealed trial records, with the stages that consumed them.
    pub datasets_used: BTreeMap<String, BTreeSet<String>>,
    /// Files below `exploration/<study>/`: path and SHA-256.
    pub files: Vec<(String, String)>,
    /// The dataset versions of the locked pipeline, and whether a trial run used each.
    pub deciding: BTreeMap<String, bool>,
    /// Whether a trial run sealed an output under the key of a stage of the locked pipeline.
    pub computed_locked_output: bool,
    pub deciding_data_seen: bool,
}

/// The stage-level trial records of a study, from `<results>/<study>/.trial/<stage>/<key>/`.
fn trial_records(results: &Path, study: &str) -> Vec<ProvenanceRecord> {
    let mut records = Vec::new();
    let Ok(stages) = fs::read_dir(results.join(study).join(".trial")) else {
        return records;
    };
    for stage in stages.flatten() {
        let Ok(keys) = fs::read_dir(stage.path()) else {
            continue;
        };
        for key in keys.flatten() {
            if let Some(record) = ProvenanceRecord::read(&key.path()) {
                if record.trial && record.study == study {
                    records.push(record);
                }
            }
        }
    }
    records.sort_by(|a, b| (&a.stage, &a.key).cmp(&(&b.stage, &b.key)));
    records
}

/// Every regular file below `dir`, as sorted project-relative paths with their SHA-256.
fn exploration_files(root: &Path, rel: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pending = vec![rel.to_string()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{dir}/{name}");
            match entry.file_type() {
                Ok(t) if t.is_dir() => pending.push(path),
                Ok(t) if t.is_file() => {
                    if let Ok((_, sha256)) = crate::data::hash_file(&entry.path()) {
                        out.push((path, sha256));
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// The first non-empty line of a stage's result file.
fn result_line(dir: &Path, file: &str) -> Option<String> {
    fs::read_to_string(dir.join(file))
        .ok()?
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

// rq-d0c8b971 rq-45ae65c9
/// Computes the exploration record of a study from its configuration, its trial records and
/// `exploration/<study>/`, without writing anything.
pub fn explore(root: &Path, study: &str) -> Result<Exploration, StudyError> {
    let doc = super::parse_study(&root.join(STUDIES_DIR).join(study))?;
    let roots = crate::data::load_roots(root).map_err(|e| StudyError::Io(e.to_string()))?;
    let results = roots
        .location(crate::data::RESULTS_ROOT)
        .ok_or_else(|| StudyError::Io("the results root has no location".into()))?
        .to_path_buf();
    let records = trial_records(&results, study);
    let trial_keys: BTreeSet<(String, String)> = records
        .iter()
        .map(|r| (r.stage.clone(), r.key.clone()))
        .collect();
    let mut datasets_used: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for record in &records {
        for input in &record.inputs {
            if input["kind"] == "dataset" {
                if let Some(r) = input["ref"].as_str() {
                    datasets_used
                        .entry(r.to_string())
                        .or_default()
                        .insert(record.stage.clone());
                }
            }
        }
    }

    let ctx = KeyContext::new(root);
    let mut trial_results = Vec::new();
    let mut deciding: BTreeMap<String, bool> = BTreeMap::new();
    let mut computed_locked_output = false;
    for variant in doc.variants() {
        // A variant whose configuration or keys cannot be computed is reported by `tophat check`.
        let Ok(locked) = load_pipeline(root, study, variant) else {
            continue;
        };
        if let Ok(keys) = compute_keys(&locked, &ctx) {
            for (stage, key) in &keys.keys {
                if trial_keys.contains(&(stage.clone(), key.clone())) {
                    computed_locked_output = true;
                }
            }
        }
        for stage in &locked.stages {
            for input in stage.inputs.values() {
                if let StageInput::Dataset(r) = input {
                    let r = r.to_string();
                    let used = datasets_used.contains_key(&r);
                    deciding.insert(r, used);
                }
            }
        }
        let trial = locked.for_trial();
        let Ok(trial_keys) = compute_keys(&trial, &ctx) else {
            continue;
        };
        for stage in trial.stages.iter().filter(|s| s.result.is_some()) {
            let key = trial_keys.keys[&stage.name].clone();
            let dir = output_dir(&results, study, &stage.name, &key, true);
            let sealed = ProvenanceRecord::read(&dir).is_some_and(|r| r.key == key && r.trial);
            let line = sealed
                .then(|| result_line(&dir, stage.result.as_deref().expect("a result stage")))
                .flatten();
            trial_results.push(TrialResult {
                variant: variant.clone(),
                stage: stage.name.clone(),
                key,
                line,
            });
        }
    }
    let deciding_data_seen = computed_locked_output || deciding.values().any(|used| *used);
    Ok(Exploration {
        study: study.to_string(),
        trial_results,
        datasets_used,
        files: exploration_files(root, &format!("exploration/{study}")),
        deciding,
        computed_locked_output,
        deciding_data_seen,
    })
}

impl Exploration {
    /// The text of `EXPLORATION.md`, stamped with `generated`.
    pub fn render(&self, generated: &str) -> String {
        let mut s = format!(
            "---\nstudy: {}\ngenerated: \"{generated}\"\ntophat_version: \"{}\"\ndeciding_data_seen: {}\n---\n\n",
            self.study,
            env!("CARGO_PKG_VERSION"),
            self.deciding_data_seen
        );
        let list = |items: Vec<String>| {
            if items.is_empty() {
                "- none\n".to_string()
            } else {
                items.iter().map(|i| format!("- {i}\n")).collect()
            }
        };
        s.push_str("## Trial results\n\n");
        s.push_str(&list(
            self.trial_results
                .iter()
                .map(|r| {
                    let line = r.line.as_deref().unwrap_or("not run");
                    format!("{}/{}: {line} ({})", r.variant, r.stage, r.key)
                })
                .collect(),
        ));
        s.push_str("\n## Data used in trial runs\n\n");
        s.push_str(&list(
            self.datasets_used
                .iter()
                .map(|(r, stages)| {
                    let stages: Vec<&str> = stages.iter().map(String::as_str).collect();
                    let word = if stages.len() == 1 { "stage" } else { "stages" };
                    format!("dataset {r}, by {word} {}", stages.join(", "))
                })
                .collect(),
        ));
        s.push_str("\n## Exploration files\n\n");
        s.push_str(&list(
            self.files
                .iter()
                .map(|(path, sha)| format!("{path} sha256:{sha}"))
                .collect(),
        ));
        s.push_str("\n## Deciding data\n\n");
        for (r, used) in &self.deciding {
            let how = if *used {
                "used by a trial run"
            } else {
                "not used by any trial run"
            };
            s.push_str(&format!("- dataset {r}: {how}\n"));
        }
        s.push_str(if self.computed_locked_output {
            "- a trial run computed an output of the locked pipeline\n"
        } else {
            "- no trial run computed an output of the locked pipeline\n"
        });
        s
    }

    /// The path of the study's `EXPLORATION.md`.
    pub fn path(root: &Path, study: &str) -> std::path::PathBuf {
        root.join(STUDIES_DIR).join(study).join(EXPLORATION_FILE)
    }

    /// One-line account of what the exploration saw, for pre-registration.
    pub fn account(&self) -> String {
        let run = self
            .trial_results
            .iter()
            .filter(|r| r.line.is_some())
            .count();
        let data: Vec<&str> = self.datasets_used.keys().map(String::as_str).collect();
        format!(
            "{run} trial result(s); data used in trial runs: {}; {} exploration file(s)",
            if data.is_empty() {
                "none".to_string()
            } else {
                data.join(", ")
            },
            self.files.len()
        )
    }
}

/// The text without its `generated` line, to compare two renderings.
fn without_generated(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with("generated:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text to write for `exploration`, and whether it differs from the file on disk. The file's
/// own text is kept when only its `generated` stamp would change.
pub fn exploration_text(root: &Path, exploration: &Exploration) -> (String, bool) {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let text = exploration.render(&now);
    match fs::read_to_string(Exploration::path(root, &exploration.study)) {
        Ok(old) if without_generated(&old) == without_generated(&text) => (old, false),
        _ => (text, true),
    }
}

// rq-79590bc8 rq-b3af0612
/// Writes `EXPLORATION.md` atomically, unless only its `generated` stamp would change. Returns
/// whether the file changed.
pub fn write_exploration(root: &Path, exploration: &Exploration) -> Result<bool, StudyError> {
    let (text, changed) = exploration_text(root, exploration);
    if changed {
        crate::fsutil::write_atomic(&Exploration::path(root, &exploration.study), &text)?;
    }
    Ok(changed)
}

/// `sha256:` and the SHA-256 of `text`.
pub fn exploration_hash(text: &str) -> String {
    format!(
        "sha256:{}",
        crate::data::to_hex(&Sha256::digest(text.as_bytes()))
    )
}

/// The front matter of an `EXPLORATION.md`: `(study, deciding_data_seen)`.
pub fn parse_exploration(text: &str) -> Result<(String, bool), String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("it must start with front matter (`---`)".into());
    }
    let mut study = None;
    let mut seen = None;
    for line in lines.by_ref() {
        if line == "---" {
            break;
        }
        if let Some(v) = line.strip_prefix("study:") {
            study = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("deciding_data_seen:") {
            seen = match v.trim() {
                "true" => Some(true),
                "false" => Some(false),
                other => {
                    return Err(format!(
                        "`deciding_data_seen: {other}` is not true or false"
                    ))
                }
            };
        }
    }
    match (study, seen) {
        (Some(study), Some(seen)) => Ok((study, seen)),
        _ => Err("the front matter needs `study` and `deciding_data_seen`".into()),
    }
}
