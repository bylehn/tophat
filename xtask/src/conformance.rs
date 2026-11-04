//! `cargo xtask conformance`: run the committed conformance cases against a tophat binary.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};

const DATE: &str = "2026-01-01T00:00:00Z";

/// One step of the deterministic git replay.
enum Step {
    Commit(String),
    Write { path: String, content: String },
    Delete(String),
}

/// One expected diagnostic, compared without its message.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Expected {
    rule: String,
    severity: String,
    path: String,
}

impl std::fmt::Display for Expected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.rule, self.severity, self.path)
    }
}

/// What a case expects.
struct Expect {
    exit: i32,
    diagnostics: BTreeSet<Expected>,
    stdout_contains: Vec<String>,
    stderr_contains: Vec<String>,
}

/// A parsed `case.yaml`.
struct Case {
    name: String,
    since: String,
    steps: Vec<Step>,
    command: Vec<String>,
    expect: Expect,
}

/// The result of one case.
pub struct Report {
    pub lines: Vec<String>,
    pub failed: bool,
}

/// A temporary directory removed when dropped.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Result<Temp, String> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tophat-conformance-{}-{}",
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

fn string(value: &Yaml) -> Option<&str> {
    value.as_str()
}

fn field<'a>(hash: &'a yaml_rust2::yaml::Hash, key: &str) -> Option<&'a Yaml> {
    hash.get(&Yaml::String(key.to_string()))
}

fn array(value: &Yaml) -> Result<&Vec<Yaml>, String> {
    value
        .as_vec()
        .ok_or_else(|| "expected a YAML list".to_string())
}

fn strings(value: &Yaml) -> Result<Vec<String>, String> {
    array(value)?
        .iter()
        .map(|v| {
            string(v)
                .map(str::to_string)
                .ok_or_else(|| "expected a YAML string".to_string())
        })
        .collect()
}

fn contained(value: Option<&Yaml>) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(Yaml::String(s)) => Ok(vec![s.clone()]),
        Some(other) => strings(other),
    }
}

fn parse_case(name: &str, text: &str) -> Result<Case, String> {
    let documents =
        YamlLoader::load_from_str(text).map_err(|e| format!("{name}: invalid YAML: {e}"))?;
    let document = documents
        .first()
        .and_then(Yaml::as_hash)
        .ok_or_else(|| format!("{name}: case.yaml must be a mapping"))?;
    let since = field(document, "since")
        .and_then(string)
        .ok_or_else(|| format!("{name}: case.yaml has no `since`"))?
        .to_string();
    let mut steps = Vec::new();
    if let Some(value) = field(document, "git") {
        for item in array(value)? {
            let hash = item
                .as_hash()
                .ok_or_else(|| format!("{name}: every git step must be a mapping"))?;
            if let Some(message) = field(hash, "commit").and_then(string) {
                steps.push(Step::Commit(message.to_string()));
            } else if let Some(write) = field(hash, "write").and_then(Yaml::as_hash) {
                let path = field(write, "path")
                    .and_then(string)
                    .ok_or_else(|| format!("{name}: a write step needs `path`"))?;
                let content = field(write, "content")
                    .and_then(string)
                    .ok_or_else(|| format!("{name}: a write step needs `content`"))?;
                steps.push(Step::Write {
                    path: path.to_string(),
                    content: content.to_string(),
                });
            } else if let Some(path) = field(hash, "delete").and_then(string) {
                steps.push(Step::Delete(path.to_string()));
            } else {
                return Err(format!(
                    "{name}: a git step must be `commit`, `write` or `delete`"
                ));
            }
        }
    }
    let command = match field(document, "command") {
        Some(value) => strings(value)?,
        None => vec!["check".to_string(), "--json".to_string()],
    };
    let expected = field(document, "expect")
        .and_then(Yaml::as_hash)
        .ok_or_else(|| format!("{name}: case.yaml has no `expect` mapping"))?;
    let exit = match field(expected, "exit") {
        None => 0,
        Some(value) => value
            .as_i64()
            .ok_or_else(|| format!("{name}: `expect.exit` must be an integer"))?
            as i32,
    };
    let mut diagnostics = BTreeSet::new();
    if let Some(value) = field(expected, "diagnostics") {
        for item in array(value)? {
            let hash = item
                .as_hash()
                .ok_or_else(|| format!("{name}: a diagnostic must be a mapping"))?;
            let part = |key: &str| {
                field(hash, key)
                    .and_then(string)
                    .map(str::to_string)
                    .ok_or_else(|| format!("{name}: a diagnostic needs `{key}`"))
            };
            diagnostics.insert(Expected {
                rule: part("rule")?,
                severity: part("severity")?,
                path: part("path")?,
            });
        }
    }
    Ok(Case {
        name: name.to_string(),
        since,
        steps,
        command,
        expect: Expect {
            exit,
            diagnostics,
            stdout_contains: contained(field(expected, "stdout_contains"))?,
            stderr_contains: contained(field(expected, "stderr_contains"))?,
        },
    })
}

/// Copies a directory tree.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|e| {
                format!(
                    "cannot copy {} to {}: {e}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn git(project: &Path, args: &[&str], date: bool) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=TopHat Conformance",
            "-c",
            "user.email=conformance@tophat.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(project)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if date {
        command
            .env("GIT_AUTHOR_DATE", DATE)
            .env("GIT_COMMITTER_DATE", DATE);
    }
    let output = command
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn replay(project: &Path, steps: &[Step]) -> Result<(), String> {
    git(project, &["init", "-q"], false)?;
    for step in steps {
        match step {
            Step::Commit(message) => {
                git(project, &["add", "-A"], false)?;
                git(project, &["commit", "-q", "-m", message], true)?;
            }
            Step::Write { path, content } => {
                let target = project.join(path);
                fs::create_dir_all(target.parent().unwrap())
                    .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
                fs::write(&target, content)
                    .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
            }
            Step::Delete(path) => {
                let target = project.join(path);
                if target.is_dir() {
                    fs::remove_dir_all(&target)
                        .map_err(|e| format!("cannot delete {}: {e}", target.display()))?;
                } else if target.exists() {
                    fs::remove_file(&target)
                        .map_err(|e| format!("cannot delete {}: {e}", target.display()))?;
                }
            }
        }
    }
    Ok(())
}

/// Runs a case's command with the roots configured.
fn run_case(
    project: &Path,
    roots: &Path,
    binary: &Path,
    command: &[String],
) -> Result<Output, String> {
    let mut process = Command::new(binary);
    process.args(command).current_dir(project);
    for (key, _) in std::env::vars() {
        if key.starts_with("TOPHAT_ROOT_") {
            process.env_remove(key);
        }
    }
    if roots.is_dir() {
        for entry in fs::read_dir(roots).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_uppercase();
            let name = name.replace(|c: char| !c.is_ascii_alphanumeric(), "_");
            process.env(format!("TOPHAT_ROOT_{name}"), entry.path());
        }
    }
    process
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))
}

fn diagnostics(output: &Output) -> Result<BTreeSet<Expected>, String> {
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("`check --json` printed invalid JSON: {e}"))?;
    let items = value
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or_else(|| "`check --json` output has no diagnostics list".to_string())?;
    let mut set = BTreeSet::new();
    for item in items {
        let text = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("a diagnostic has no `{key}`"))
        };
        set.insert(Expected {
            rule: text("rule")?,
            severity: text("severity")?,
            path: text("path")?,
        });
    }
    Ok(set)
}

fn checks_diagnostics(command: &[String]) -> bool {
    command.first().is_some_and(|c| c == "check") && command.iter().any(|c| c == "--json")
}

/// Runs one case and returns its one-line result.
fn execute(
    case: &Case,
    case_dir: &Path,
    temp: &Path,
    binary: &Path,
) -> Result<(bool, Vec<String>), String> {
    let work = temp.join(&case.name);
    copy_tree(case_dir, &work)?;
    let project = work.join("project");
    let roots = work.join("roots");
    replay(&project, &case.steps)?;
    let output = run_case(&project, &roots, binary, &case.command)?;
    let exit = output.status.code().unwrap_or(-1);
    let mut problems = Vec::new();
    if exit != case.expect.exit {
        problems.push(format!("expected exit {}, got {exit}", case.expect.exit));
    }
    if checks_diagnostics(&case.command) {
        let actual = diagnostics(&output)?;
        for missing in case.expect.diagnostics.difference(&actual) {
            problems.push(format!("missing: {missing}"));
        }
        for unexpected in actual.difference(&case.expect.diagnostics) {
            problems.push(format!("unexpected: {unexpected}"));
        }
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in &case.expect.stdout_contains {
        if !stdout.contains(expected) {
            problems.push(format!("stdout does not contain {expected:?}"));
        }
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in &case.expect.stderr_contains {
        if !stderr.contains(expected) {
            problems.push(format!("stderr does not contain {expected:?}"));
        }
    }
    if problems.is_empty() {
        Ok((true, Vec::new()))
    } else {
        let mut lines = vec![format!("{} FAILED", case.name)];
        lines.extend(problems.into_iter().map(|p| format!("  {p}")));
        Ok((false, lines))
    }
}

fn version(text: &str) -> Option<Vec<u64>> {
    let last = text.split_whitespace().last()?;
    let numbers: Vec<u64> = last
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if numbers.is_empty() {
        None
    } else {
        Some(numbers)
    }
}

fn newer(a: &[u64], b: &[u64]) -> bool {
    for i in 0..a.len().max(b.len()) {
        let left = a.get(i).copied().unwrap_or(0);
        let right = b.get(i).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

/// Replays a case's git steps in a temporary directory and returns the resulting HEAD, so tests
/// can check that the replay is deterministic.
pub fn replay_head(case_dir: &Path) -> Result<String, String> {
    let text = fs::read_to_string(case_dir.join("case.yaml"))
        .map_err(|e| format!("cannot read case.yaml: {e}"))?;
    let case = parse_case("case", &text)?;
    let temp = Temp::new()?;
    let work = temp.0.join("case");
    copy_tree(case_dir, &work)?;
    replay(&work.join("project"), &case.steps)?;
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(work.join("project"))
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Runs the conformance suite. `selected` restricts it to the named cases.
// rq-f5dee2b3
pub fn run(repo_root: &Path, binary: &Path, selected: &[String]) -> Result<Report, String> {
    run_dir(&repo_root.join("tests/conformance"), binary, selected)
}

/// Runs the cases in `cases_dir`, for tests that need a temporary suite.
pub fn run_dir(cases_dir: &Path, binary: &Path, selected: &[String]) -> Result<Report, String> {
    let version_output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {} --version: {e}", binary.display()))?;
    if !version_output.status.success() {
        return Err(format!("{} --version failed", binary.display()));
    }
    let binary_version = version(&String::from_utf8_lossy(&version_output.stdout))
        .ok_or_else(|| format!("cannot parse the version of {}", binary.display()))?;

    let mut names: Vec<String> = fs::read_dir(cases_dir)
        .map_err(|e| format!("cannot read {}: {e}", cases_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if !selected.is_empty() {
        for name in selected {
            if !names.contains(name) {
                return Err(format!("no conformance case named '{name}'"));
            }
        }
        names.retain(|name| selected.contains(name));
    }

    let temp = Temp::new()?;
    let mut report = Report {
        lines: Vec::new(),
        failed: false,
    };
    for name in names {
        let case_dir = cases_dir.join(&name);
        let text = fs::read_to_string(case_dir.join("case.yaml"))
            .map_err(|e| format!("cannot read {name}/case.yaml: {e}"))?;
        let case = parse_case(&name, &text)?;
        let since =
            version(&case.since).ok_or_else(|| format!("{name}: `since` is not a version"))?;
        if newer(&since, &binary_version) {
            report.lines.push(format!("{name} skipped"));
            continue;
        }
        match execute(&case, &case_dir, &temp.0, binary) {
            Ok((true, _)) => report.lines.push(format!("{name} ok")),
            Ok((false, lines)) => {
                report.failed = true;
                report.lines.extend(lines);
            }
            Err(e) => {
                report.failed = true;
                report.lines.push(format!("{name} FAILED"));
                report.lines.push(format!("  {e}"));
            }
        }
    }
    Ok(report)
}
