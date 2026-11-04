//! `cargo xtask render`: render the template from the working tree with a named answer set.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One render request.
#[derive(Debug, Clone)]
pub struct Request {
    /// An answer-set name under `xtask/answers/`, or a path to a YAML file.
    pub answers: String,
    /// The directory to render into.
    pub out: PathBuf,
    /// Question overrides, applied after the answer set.
    pub sets: Vec<(String, String)>,
}

/// A render that could not run or was refused.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn error(message: impl Into<String>) -> Error {
    Error(message.into())
}

/// Resolves an answer-set name or path.
fn answer_file(repo_root: &Path, answers: &str) -> Result<PathBuf, Error> {
    let given = Path::new(answers);
    if given.is_file() {
        return Ok(given.to_path_buf());
    }
    let named = repo_root
        .join("xtask/answers")
        .join(format!("{answers}.yaml"));
    if named.is_file() {
        Ok(named)
    } else {
        Err(error(format!(
            "no answer set named '{answers}' in xtask/answers, and no such file"
        )))
    }
}

/// Refuses a non-empty output directory.
fn check_output(out: &Path) -> Result<(), Error> {
    match fs::read_dir(out) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(error(format!(
                    "the output directory {} is not empty",
                    out.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(error(format!(
                "the output directory {} cannot be read: {e}",
                out.display()
            )))
        }
    }
    Ok(())
}

/// Runs copier with an explicit PATH, or the inherited PATH when `path` is `None`.
fn copier(path: Option<&OsStr>) -> Result<Command, Error> {
    let mut check = Command::new("copier");
    if let Some(path) = path {
        check.env("PATH", path);
    }
    match check.arg("--version").output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            return Err(error(format!(
                "copier --version failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
        Err(_) => return Err(error("copier is not on PATH; install Copier 9")),
    }
    let mut command = Command::new("copier");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.args(["copy", "--vcs-ref", "HEAD", "--defaults", "--quiet"]);
    Ok(command)
}

/// Renders with the inherited PATH.
// rq-8fd5db8d
pub fn render(repo_root: &Path, request: &Request) -> Result<PathBuf, Error> {
    render_with_path(repo_root, request, std::env::var_os("PATH").as_deref())
}

/// Renders with the given PATH, so tests can simulate a missing copier without changing the
/// environment.
pub fn render_with_path(
    repo_root: &Path,
    request: &Request,
    path: Option<&OsStr>,
) -> Result<PathBuf, Error> {
    let answers = answer_file(repo_root, &request.answers)?;
    check_output(&request.out)?;
    fs::create_dir_all(&request.out).map_err(|e| {
        error(format!(
            "the output directory {} cannot be created: {e}",
            request.out.display()
        ))
    })?;
    let mut command = copier(path)?;
    command.arg("--data-file").arg(&answers);
    for (question, value) in &request.sets {
        command.arg("--data").arg(format!("{question}={value}"));
    }
    command.arg(repo_root).arg(&request.out);
    let output = command
        .output()
        .map_err(|e| error(format!("copier could not be run: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(error(format!("copier failed: {}", stderr.trim())));
    }
    Ok(request.out.clone())
}

/// The `--set question=value` parser.
pub fn parse_set(value: &str) -> Result<(String, String), Error> {
    let Some((question, answer)) = value.split_once('=') else {
        return Err(error(format!(
            "--set needs <question>=<value>, not '{value}'"
        )));
    };
    if question.is_empty() {
        return Err(error("--set needs a question name before '='"));
    }
    Ok((question.to_string(), answer.to_string()))
}
