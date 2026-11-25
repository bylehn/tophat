//! `cargo xtask release-pin`: write the template's pinned `tophat` release.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const TARGETS: [&str; 2] = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

/// One `cargo xtask release-pin` request.
pub struct Request {
    pub version: String,
    pub url: String,
    /// `<target>=<path>` pairs.
    pub binaries: Vec<String>,
}

/// The outcome of a release-pin run.
pub struct Report {
    pub lines: Vec<String>,
    pub failed: bool,
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Whether a start failure means the binary cannot run on this host at all: `ENOEXEC` on
/// Unix, i.e. it was built for another architecture.
#[cfg(unix)]
fn cannot_run_here(e: &std::io::Error) -> bool {
    const ENOEXEC: i32 = 8;
    e.raw_os_error() == Some(ENOEXEC)
}

#[cfg(not(unix))]
fn cannot_run_here(_e: &std::io::Error) -> bool {
    false
}

/// The version a runnable binary reports, or `None` when it cannot be read here. A binary for
/// another architecture never starts (`ENOEXEC`) and is skipped by design; any other failure
/// is usually transient — a loaded runner can refuse to fork (`EAGAIN`) or kill the child —
/// so it is retried briefly before giving up.
fn reported_version(path: &Path) -> Option<String> {
    for attempt in 0..3 {
        let version = match Command::new(path).arg("--version").output() {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                text.split_whitespace().last().map(str::to_string)
            }
            Ok(_) => None,
            Err(e) if cannot_run_here(&e) => return None,
            Err(_) => None,
        };
        if version.is_some() || attempt == 2 {
            return version;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

/// Replaces `path` atomically when the content differs.
fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if fs::read_to_string(path).is_ok_and(|current| current == text) {
        return Ok(false);
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&temporary, text)
        .map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        format!("cannot replace {}: {e}", path.display())
    })?;
    Ok(true)
}

fn render(version: &str, url: &str, hashes: &BTreeMap<String, String>) -> String {
    let mut out = String::from(
        "# TopHat-managed: the tophat release this project uses. `copier update` changes it.\n",
    );
    out.push_str(&format!("version: {version}\n"));
    out.push_str(&format!("url: {url}\n"));
    out.push_str("sha256:\n");
    for target in TARGETS {
        out.push_str(&format!("  {target}: {}\n", hashes[target]));
    }
    out
}

// rq-be3c6abc
/// Hashes the binaries and writes the template's `tophat-release.yaml`.
pub fn release_pin(repo_root: &Path, request: &Request) -> Result<Report, String> {
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    for pair in &request.binaries {
        let (target, path) = pair
            .split_once('=')
            .ok_or_else(|| format!("--binary needs <target>=<path>, not '{pair}'"))?;
        if !TARGETS.contains(&target) {
            return Err(format!(
                "unknown target '{target}'; use x86_64-unknown-linux-musl or aarch64-unknown-linux-musl"
            ));
        }
        if paths
            .insert(target.to_string(), PathBuf::from(path))
            .is_some()
        {
            return Err(format!("target '{target}' is given more than once"));
        }
    }

    let mut problems = Vec::new();
    for target in TARGETS {
        if !paths.contains_key(target) {
            problems.push(format!("missing target {target}"));
        }
    }
    if !problems.is_empty() {
        return Ok(Report {
            lines: problems,
            failed: true,
        });
    }

    let mut hashes = BTreeMap::new();
    let mut lines = Vec::new();
    for target in TARGETS {
        let path = &paths[target];
        hashes.insert(target.to_string(), sha256(path)?);
        if let Some(found) = reported_version(path)
            && found != request.version
        {
            problems.push(format!(
                "{} reports version {found}, not {}",
                target, request.version
            ));
        }
        lines.push(format!("{target} {}", hashes[target]));
    }
    if !problems.is_empty() {
        return Ok(Report {
            lines: problems,
            failed: true,
        });
    }

    let pin = repo_root.join("template/.tophat/managed/tophat-release.yaml");
    let text = render(&request.version, &request.url, &hashes);
    let changed = write_if_changed(&pin, &text)?;
    lines.push(format!(
        "{} {}",
        pin.display(),
        if changed { "written" } else { "unchanged" }
    ));
    Ok(Report {
        lines,
        failed: false,
    })
}
