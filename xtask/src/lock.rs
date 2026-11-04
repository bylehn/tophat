//! `cargo xtask lock`: regenerate or check the template's pre-solved pixi locks.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::render::{self, Request};

/// One language selection and the pre-solved lock it uses.
const SELECTIONS: [(&str, &str); 2] = [
    ("none", "template/.locks/python.lock"),
    ("julia", "template/.locks/python-julia.lock"),
];

/// What happened to one selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Stale,
    Updated,
    Unchanged,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Stale => "stale",
            Status::Updated => "updated",
            Status::Unchanged => "unchanged",
        }
    }
}

/// The result for one selection.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub selection: String,
    pub status: Status,
    pub lock: PathBuf,
}

/// A temporary directory removed when it is dropped.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Result<Temp, String> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tophat-lock-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        Ok(Temp(path))
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs pixi with the given PATH.
fn pixi(
    path: Option<&OsStr>,
    directory: &Path,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let mut command = Command::new("pixi");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.args(args).current_dir(directory);
    match command.output() {
        Ok(output) => Ok(output),
        Err(_) => Err("pixi is not on PATH; install pixi".to_string()),
    }
}

/// Replaces `path` with `bytes`, but only when the content differs.
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    if fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(false);
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&temporary, bytes)
        .map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        format!("cannot replace {}: {e}", path.display())
    })?;
    Ok(true)
}

/// Runs the lock command with the inherited PATH.
// rq-6fc05e55
pub fn lock(repo_root: &Path, check: bool) -> Result<Vec<Outcome>, String> {
    lock_with_path(repo_root, check, std::env::var_os("PATH").as_deref())
}

/// Runs the lock command with an explicit PATH, so tests can supply a fake pixi.
pub fn lock_with_path(
    repo_root: &Path,
    check: bool,
    path: Option<&OsStr>,
) -> Result<Vec<Outcome>, String> {
    let temp = Temp::new()?;
    let mut outcomes = Vec::new();
    for (index, (selection, relative_lock)) in SELECTIONS.iter().enumerate() {
        let out = temp.0.join(format!("{index}-{selection}"));
        let mut request = Request {
            answers: "minimal-claude".to_string(),
            out,
            sets: Vec::new(),
        };
        if *selection == "julia" {
            request
                .sets
                .push(("languages".to_string(), "[julia]".to_string()));
        }
        let project =
            render::render_with_path(repo_root, &request, path).map_err(|e| e.to_string())?;
        let lock = repo_root.join(relative_lock);
        if check {
            let output = pixi(path, &project, &["lock", "--check"])?;
            let status = if output.status.success() {
                Status::Ok
            } else {
                Status::Stale
            };
            outcomes.push(Outcome {
                selection: selection.to_string(),
                status,
                lock,
            });
        } else {
            let output = pixi(path, &project, &["lock"])?;
            if !output.status.success() {
                return Err(format!(
                    "pixi lock failed for selection '{selection}': {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            let bytes = fs::read(project.join("pixi.lock")).map_err(|e| {
                format!("the rendered project has no pixi.lock for selection '{selection}': {e}")
            })?;
            let changed = write_if_changed(&lock, &bytes)?;
            outcomes.push(Outcome {
                selection: selection.to_string(),
                status: if changed {
                    Status::Updated
                } else {
                    Status::Unchanged
                },
                lock,
            });
        }
    }
    Ok(outcomes)
}

/// Whether any outcome means the checked locks are stale.
pub fn any_stale(outcomes: &[Outcome]) -> bool {
    outcomes.iter().any(|o| o.status == Status::Stale)
}
