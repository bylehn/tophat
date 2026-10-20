//! Human QC gates: verdict files, `tophat stage qc` and `tophat stage qc-status`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use yaml_rust2::{Yaml, YamlLoader};

use super::provenance::{ProvenanceRecord, ResolvedStage};
use super::PipelineError;
use crate::fsutil::write_atomic;
use crate::markdown::{numbered_lines, split_front_matter};
use crate::study::{Console, STUDIES_DIR};

/// A QC verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
        }
    }
}

/// A parsed verdict file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictFile {
    pub stage: String,
    pub key: String,
    pub verdict: Verdict,
    pub date: String,
}

/// Project-relative path of the verdict file for a stage key.
pub fn verdict_path(study: &str, stage: &str, key: &str) -> String {
    format!("{STUDIES_DIR}/{study}/qc/{stage}/{key}.md")
}

/// Parses a verdict file's text.
pub fn parse_verdict(text: &str) -> Result<VerdictFile, String> {
    let lines = numbered_lines(text);
    let split = split_front_matter(&lines)?;
    let yaml: String = split.front.iter().map(|(_, l)| format!("{l}\n")).collect();
    let doc = YamlLoader::load_from_str(&yaml)
        .map_err(|e| format!("invalid YAML: {e}"))?
        .into_iter()
        .next()
        .unwrap_or(Yaml::Null);
    let field = |key: &str| -> Result<String, String> {
        match &doc[key] {
            Yaml::String(s) => Ok(s.clone()),
            _ => Err(format!("`{key}` is missing or not text")),
        }
    };
    let verdict = match field("verdict")?.as_str() {
        "pass" => Verdict::Pass,
        "fail" => Verdict::Fail,
        other => return Err(format!("`verdict: {other}` must be pass or fail")),
    };
    let date = field("date")?;
    if date.len() != 10 || NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err("`date` must be YYYY-MM-DD".into());
    }
    Ok(VerdictFile {
        stage: field("stage")?,
        key: field("key")?,
        verdict,
        date,
    })
}

// rq-872c3d40
/// The verdict recorded for a stage key: `Ok(verdict)`, or `Err` describing why there is none.
pub fn qc_status(root: &Path, study: &str, stage: &str, key: &str) -> Result<Verdict, String> {
    let path = verdict_path(study, stage, key);
    let text = fs::read_to_string(root.join(&path))
        .map_err(|_| format!("no QC verdict: {path} does not exist"))?;
    let verdict = parse_verdict(&text).map_err(|e| format!("{path}: {e}"))?;
    if verdict.stage != stage || verdict.key != key {
        return Err(format!(
            "{path}: stage or key disagrees with the file's location"
        ));
    }
    Ok(verdict.verdict)
}

// rq-6c24fee9 rq-dc1e521d
/// Records a human QC verdict for a sealed stage output, interactively.
pub fn qc(
    root: &Path,
    study: &str,
    stage: &str,
    variant: &str,
    today: NaiveDate,
    console: &mut Console<'_>,
) -> Result<PathBuf, PipelineError> {
    if !console.interactive {
        return Err(PipelineError::NotInteractive(
            "QC verdicts must be given by a human in an interactive terminal; run this command yourself".into(),
        ));
    }
    let resolved = ResolvedStage::load(root, study, variant)?;
    resolved.stage(stage)?;
    if !resolved.config.is_gated(stage) {
        return Err(PipelineError::Qc(format!(
            "stage `{stage}` has no QC gate in variant `{variant}`: nothing consumes it, or it \
             declares `qc: skip`"
        )));
    }
    let key = resolved.key(stage).to_string();
    let dir = resolved.out_dir(stage, false)?;
    let Some(record) = ProvenanceRecord::read(&dir).filter(|r| r.key == key && !r.trial) else {
        return Err(PipelineError::Qc(format!(
            "stage `{stage}` ({variant}) has not finished: no sealed output for {key}"
        )));
    };
    let rel = verdict_path(study, stage, &key);
    if root.join(&rel).exists() {
        return Err(PipelineError::Qc(format!(
            "a verdict already exists: {rel}"
        )));
    }

    let mut files = record.outputs.len();
    let mut size: u64 = record.outputs.iter().map(|o| o.size).sum();
    let items = record.items.clone().unwrap_or_default();
    for item in &items {
        if let Some(r) = ProvenanceRecord::read(&dir.join("items").join(&item.name)) {
            files += r.outputs.len();
            size += r.outputs.iter().map(|o| o.size).sum::<u64>();
        }
    }
    let io = |e: std::io::Error| PipelineError::Io(e.to_string());
    let out = &mut *console.out;
    writeln!(out, "QC for {study} / {stage} ({variant})").map_err(io)?;
    writeln!(out, "  key:     {key}").map_err(io)?;
    writeln!(out, "  files:   {files} ({size} bytes)").map_err(io)?;
    writeln!(out, "  items:   {}", items.len()).map_err(io)?;
    writeln!(out, "  clean:   {}", record.clean).map_err(io)?;
    writeln!(out, "  outputs: {}", dir.display()).map_err(io)?;
    write!(out, "\nVerdict (pass or fail): ").map_err(io)?;
    out.flush().map_err(io)?;
    let mut answer = String::new();
    console.input.read_line(&mut answer).map_err(io)?;
    let verdict = match answer.trim() {
        "pass" => Verdict::Pass,
        "fail" => Verdict::Fail,
        other => {
            return Err(PipelineError::Qc(format!(
                "unrecognised verdict `{other}`; nothing was written"
            )))
        }
    };
    write!(console.out, "Note (one line): ").map_err(io)?;
    console.out.flush().map_err(io)?;
    let mut note = String::new();
    console.input.read_line(&mut note).map_err(io)?;

    let text = format!(
        "---\nstage: {stage}\nkey: {key}\nverdict: {}\ndate: \"{}\"\n---\n\n{}\n",
        verdict.as_str(),
        today.format("%Y-%m-%d"),
        note.trim()
    );
    let path = root.join(&rel);
    fs::create_dir_all(path.parent().expect("verdict paths have a parent")).map_err(io)?;
    write_atomic(&path, &text).map_err(io)?;
    writeln!(
        console.out,
        "Recorded `{}` in {rel}. Commit it with the study.",
        verdict.as_str()
    )
    .map_err(io)?;
    Ok(path)
}

/// Problems with the files under a study's `qc/` directory: `(project-relative path, message)`.
/// `qc_stages` are the names of the stages that are gated in at least one variant.
pub(crate) fn qc_file_problems(
    root: &Path,
    study: &str,
    all_stages: &[String],
    qc_stages: &[String],
) -> Vec<(String, String)> {
    let qc_dir = root.join(STUDIES_DIR).join(study).join("qc");
    let mut problems = Vec::new();
    let Ok(stage_dirs) = fs::read_dir(&qc_dir) else {
        return problems;
    };
    let mut entries: Vec<_> = stage_dirs.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let stage = entry.file_name().to_string_lossy().into_owned();
        let rel_dir = format!("{STUDIES_DIR}/{study}/qc/{stage}");
        if !entry.path().is_dir() {
            problems.push((rel_dir, "expected qc/<stage>/<key>.md".into()));
            continue;
        }
        let mut files: Vec<_> = fs::read_dir(entry.path())
            .map(|d| d.filter_map(Result::ok).collect())
            .unwrap_or_default();
        files.sort_by_key(|e| e.file_name());
        for file in files {
            let name = file.file_name().to_string_lossy().into_owned();
            let rel = format!("{rel_dir}/{name}");
            let Some(key) = name.strip_suffix(".md") else {
                problems.push((rel, "expected qc/<stage>/<key>.md".into()));
                continue;
            };
            let parsed = fs::read_to_string(file.path())
                .map_err(|e| e.to_string())
                .and_then(|t| parse_verdict(&t));
            match parsed {
                Err(message) => problems.push((rel, message)),
                Ok(v) if v.stage != stage || v.key != key => problems.push((
                    rel,
                    "`stage` or `key` disagrees with the file's location".into(),
                )),
                Ok(_) if !all_stages.contains(&stage) => {
                    problems.push((rel, format!("there is no stage `{stage}`")))
                }
                Ok(_) if !qc_stages.contains(&stage) => problems.push((
                    rel,
                    format!("stage `{stage}` has no QC gate in any variant"),
                )),
                Ok(_) => {}
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-dc1e521d
    fn parses_verdicts() {
        let text = "---\nstage: score\nkey: sk-0000000000000001\nverdict: pass\ndate: \"2026-10-20\"\n---\n\nok\n";
        let v = parse_verdict(text).unwrap();
        assert_eq!((v.stage.as_str(), v.verdict), ("score", Verdict::Pass));
        assert!(parse_verdict(&text.replace("verdict: pass", "verdict: ok")).is_err());
        assert!(parse_verdict(&text.replace("\"2026-10-20\"", "yesterday")).is_err());
    }
}
