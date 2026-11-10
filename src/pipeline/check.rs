//! The generated check job: the study's scenario tests, their test key and check record.
//! Specified in `rqm/pipeline/snakemake.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::PipelineError;
use super::keys::{KeyContext, stage_key};
use super::provenance::RECORD_FILE;
use crate::check::Project;
use crate::data::{RESULTS_ROOT, hash_file, load_roots};
use crate::fsutil::write_atomic;
use crate::study::STUDIES_DIR;

/// Version line at the start of every test-key text.
pub const TEST_KEY_FORMAT: &str = "tophat-test-key/1";

/// The executable test files of a study, as `(path relative to the study directory, absolute path)`,
/// sorted by path.
pub fn test_files(root: &Path, study: &str) -> Vec<(String, PathBuf)> {
    let project = Project::new(root);
    let dir = format!("{STUDIES_DIR}/{study}/tests");
    let mut files: Vec<(String, PathBuf)> = project
        .walk_files(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| {
            let name = path
                .strip_prefix(&format!("{dir}/"))
                .unwrap_or(&path)
                .to_string();
            Some((name, root.join(&path)))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// The study's test key, or `None` when it has no test files.
pub fn test_key(root: &Path, study: &str) -> Result<Option<String>, PipelineError> {
    let files = test_files(root, study);
    if files.is_empty() {
        return Ok(None);
    }
    let ctx = KeyContext::new(root);
    let env = ctx
        .environment_hash("default")
        .unwrap_or_else(|_| "unknown".to_string());
    let mut text = format!("{TEST_KEY_FORMAT}\nenv {env}\n");
    for (rel, abs) in &files {
        let (_, sha) =
            hash_file(abs).map_err(|e| PipelineError::Io(format!("{}: {e}", abs.display())))?;
        text.push_str(&format!("test {rel} {sha}\n"));
    }
    Ok(Some(stage_key(&text)))
}

/// The directory of a study's check record for `key`.
pub fn check_record_dir(root: &Path, study: &str, key: &str) -> Result<PathBuf, PipelineError> {
    let roots = load_roots(root).map_err(|e| PipelineError::Data(e.to_string()))?;
    let results = roots
        .location(RESULTS_ROOT)
        .ok_or_else(|| PipelineError::UnconfiguredRoot(RESULTS_ROOT.to_string()))?;
    Ok(results.join(study).join(".check").join(key))
}

/// The path of a study's check record for `key`.
pub fn check_record_path(root: &Path, study: &str, key: &str) -> Result<PathBuf, PipelineError> {
    Ok(check_record_dir(root, study, key)?.join(RECORD_FILE))
}

/// Whether a sealed check record exists for `key`.
pub fn check_record_sealed(root: &Path, study: &str, key: &str) -> bool {
    check_record_path(root, study, key).is_ok_and(|p| p.is_file())
}

/// The check record's `clean` flag, when a record exists.
pub fn check_record_clean(root: &Path, study: &str, key: &str) -> Option<bool> {
    let path = check_record_path(root, study, key).ok()?;
    let text = fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    json.get("clean")?.as_bool()
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

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

// rq-22119ec7 rq-02245feb
/// `tophat study test`: runs every test file in the `default` environment and writes the check
/// record. Not normally run by hand; the generated check job calls it.
pub fn run_tests(root: &Path, study: &str, key: &str) -> Result<PathBuf, PipelineError> {
    let files = test_files(root, study);
    if files.is_empty() {
        return Err(PipelineError::Study(format!(
            "study `{study}` has no test files"
        )));
    }
    let current = test_key(root, study)?.expect("files are not empty");
    if current != key {
        return Err(PipelineError::KeyMismatch {
            expected: key.to_string(),
            actual: current,
        });
    }
    let study_dir = root.join(STUDIES_DIR).join(study);
    for (rel, abs) in &files {
        if !is_executable(abs) {
            return Err(PipelineError::Study(format!(
                "test file `tests/{rel}` is not executable; make it executable with \
                 `chmod +x studies/{study}/tests/{rel}`"
            )));
        }
        let status = Command::new("pixi")
            .current_dir(&study_dir)
            .args(["run", "--frozen", "-e", "default", "--"])
            .arg(abs)
            .status()
            .map_err(|e| PipelineError::Io(format!("cannot start pixi: {e}")))?;
        if !status.success() {
            return Err(PipelineError::Study(format!(
                "test `tests/{rel}` failed; nothing was written"
            )));
        }
    }
    let commit = head_commit(root);
    // A test file is clean when it is committed unchanged.
    let mut unclean_files = Vec::new();
    for (rel, _) in &files {
        let project_path = format!("{STUDIES_DIR}/{study}/tests/{rel}");
        let changed = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["status", "--porcelain", "--", &project_path])
            .output()
            .ok()
            .is_some_and(|o| !o.stdout.is_empty());
        if changed {
            unclean_files.push(project_path);
        }
    }
    let ctx = KeyContext::new(root);
    let env = ctx
        .environment_hash("default")
        .unwrap_or_else(|_| "unknown".to_string());
    let tests: Vec<Value> = files
        .iter()
        .map(|(rel, abs)| {
            let (_, sha256) = hash_file(abs).unwrap_or((0, String::new()));
            json!({"path": rel, "sha256": sha256})
        })
        .collect();
    let record = json!({
        "kind": "check",
        "study": study,
        "key": key,
        "environment": env,
        "tests": tests,
        "commit": commit,
        "clean": unclean_files.is_empty(),
        "unclean_files": unclean_files,
        "generated": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    let dir = check_record_dir(root, study, key)?;
    fs::create_dir_all(&dir).map_err(|e| PipelineError::Io(e.to_string()))?;
    let path = dir.join(RECORD_FILE);
    write_atomic(
        &path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&record).expect("serializes")
        ),
    )
    .map_err(|e| PipelineError::Io(e.to_string()))?;
    Ok(path)
}

/// Whether a summary's recorded check key is stale relative to the study's current test files.
pub fn check_key_stale(
    root: &Path,
    study: &str,
    recorded: Option<&str>,
) -> Result<bool, PipelineError> {
    let current = test_key(root, study)?;
    Ok(match (current.as_deref(), recorded) {
        (None, _) => false,
        (Some(current), Some(recorded)) => current != recorded,
        (Some(_), None) => true,
    })
}
