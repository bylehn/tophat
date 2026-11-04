//! `cargo xtask eval`: run the skill evaluations (see `rqm/testing.md`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};

use crate::render::{self, Request};

/// Options of one `cargo xtask eval` run.
pub struct Options {
    pub agent: String,
    pub evals: Vec<String>,
    pub repeat: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            agent: "claude-code".into(),
            evals: Vec::new(),
            repeat: 3,
        }
    }
}

/// The result of one evaluation run.
pub struct Report {
    pub lines: Vec<String>,
    pub failed: bool,
}

/// A scripted human reply.
struct Reply {
    when: String,
    say: String,
}

/// One mechanical check.
enum Check {
    Unchanged {
        path: String,
        section: Option<String>,
    },
    Changed {
        path: String,
        section: Option<String>,
    },
    NotRun(Vec<String>),
    Ran(Vec<String>),
    Contains {
        path: String,
        text: String,
    },
    Project(String),
}

/// One setup step.
enum Step {
    Write { path: String, content: String },
    Delete(String),
    Commit(String),
    Mkdir(String),
    CopyExample(String),
    Tophat(Vec<String>),
}

/// A parsed `evals/<name>.yaml`.
struct Definition {
    scenario: String,
    answers: String,
    setup: Vec<Step>,
    prompt: String,
    replies: Vec<Reply>,
    max_turns: usize,
    checks: Vec<Check>,
}

/// A temporary directory removed when dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Result<Temp, String> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tophat-eval-{label}-{}-{}",
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

fn field<'a>(hash: &'a yaml_rust2::yaml::Hash, key: &str) -> Option<&'a Yaml> {
    hash.get(&Yaml::String(key.to_string()))
}

fn string(value: &Yaml) -> Option<&str> {
    value.as_str()
}

fn required<'a>(
    hash: &'a yaml_rust2::yaml::Hash,
    key: &str,
    context: &str,
) -> Result<&'a str, String> {
    field(hash, key)
        .and_then(string)
        .ok_or_else(|| format!("{context}: `{key}` must be a string"))
}

fn strings(value: &Yaml, context: &str) -> Result<Vec<String>, String> {
    value
        .as_vec()
        .ok_or_else(|| format!("{context}: expected a YAML list"))?
        .iter()
        .map(|v| {
            string(v)
                .map(str::to_string)
                .ok_or_else(|| format!("{context}: expected a YAML string"))
        })
        .collect()
}

fn parse_step(name: &str, value: &Yaml) -> Result<Step, String> {
    let hash = value
        .as_hash()
        .ok_or_else(|| format!("{name}: every setup step must be a mapping"))?;
    if let Some(path) = field(hash, "write") {
        if let Some(content) = field(hash, "content").and_then(string) {
            let path = path
                .as_str()
                .ok_or_else(|| format!("{name}: `write` must name a path"))?;
            return Ok(Step::Write {
                path: path.to_string(),
                content: content.to_string(),
            });
        }
        let write = path
            .as_hash()
            .ok_or_else(|| format!("{name}: `write` must be a mapping or `path: content`"))?;
        return Ok(Step::Write {
            path: required(write, "path", name)?.to_string(),
            content: required(write, "content", name)?.to_string(),
        });
    }
    if let Some(path) = field(hash, "delete").and_then(string) {
        return Ok(Step::Delete(path.to_string()));
    }
    if let Some(message) = field(hash, "commit").and_then(string) {
        return Ok(Step::Commit(message.to_string()));
    }
    if let Some(path) = field(hash, "mkdir").and_then(string) {
        return Ok(Step::Mkdir(path.to_string()));
    }
    if let Some(example) = field(hash, "copy-example").and_then(string) {
        return Ok(Step::CopyExample(example.to_string()));
    }
    if let Some(command) = field(hash, "tophat") {
        let args = match command {
            Yaml::String(line) => line.split_whitespace().map(str::to_string).collect(),
            other => strings(other, name)?,
        };
        return Ok(Step::Tophat(args));
    }
    Err(format!(
        "{name}: a setup step needs `write`, `delete`, `commit`, `mkdir`, `copy-example` or `tophat`"
    ))
}

fn parse_check(name: &str, value: &Yaml) -> Result<Check, String> {
    let hash = value
        .as_hash()
        .ok_or_else(|| format!("{name}: every check must be a mapping"))?;
    let target = |key: &str| -> Result<(String, Option<String>), String> {
        let raw = field(hash, key)
            .and_then(string)
            .ok_or_else(|| format!("{name}: `{key}` must be a string"))?;
        Ok(match raw.split_once('#') {
            Some((path, section)) => (path.to_string(), Some(section.to_string())),
            None => (raw.to_string(), None),
        })
    };
    if field(hash, "unchanged").is_some() {
        let (path, section) = target("unchanged")?;
        return Ok(Check::Unchanged { path, section });
    }
    if field(hash, "changed").is_some() {
        let (path, section) = target("changed")?;
        return Ok(Check::Changed { path, section });
    }
    for (key, run) in [("not_run", false), ("ran", true)] {
        if let Some(value) = field(hash, key) {
            let prefixes = match value {
                Yaml::String(line) => vec![line.clone()],
                other => strings(other, name)?,
            };
            return Ok(if run {
                Check::Ran(prefixes)
            } else {
                Check::NotRun(prefixes)
            });
        }
    }
    if let Some(value) = field(hash, "contains") {
        let mapping = value
            .as_hash()
            .ok_or_else(|| format!("{name}: `contains` must be a mapping"))?;
        return Ok(Check::Contains {
            path: required(mapping, "path", name)?.to_string(),
            text: required(mapping, "text", name)?.to_string(),
        });
    }
    if let Some(value) = field(hash, "tophat_check").and_then(string) {
        return Ok(Check::Project(value.to_string()));
    }
    Err(format!(
        "{name}: a check needs `unchanged`, `changed`, `not_run`, `ran`, `contains` or `tophat_check`"
    ))
}

fn parse_definition(name: &str, text: &str) -> Result<Definition, String> {
    let documents =
        YamlLoader::load_from_str(text).map_err(|e| format!("{name}: invalid YAML: {e}"))?;
    let document = documents
        .first()
        .and_then(Yaml::as_hash)
        .ok_or_else(|| format!("{name}: eval must be a mapping"))?;
    let setup = match field(document, "setup") {
        None | Some(Yaml::Null) => Vec::new(),
        Some(value) => value
            .as_vec()
            .ok_or_else(|| format!("{name}: `setup` must be a list"))?
            .iter()
            .map(|step| parse_step(name, step))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let replies = match field(document, "replies") {
        None | Some(Yaml::Null) => Vec::new(),
        Some(value) => value
            .as_vec()
            .ok_or_else(|| format!("{name}: `replies` must be a list"))?
            .iter()
            .map(|item| {
                let hash = item
                    .as_hash()
                    .ok_or_else(|| format!("{name}: every reply must be a mapping"))?;
                Ok(Reply {
                    when: required(hash, "when", name)?.to_string(),
                    say: required(hash, "say", name)?.to_string(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let checks = match field(document, "checks") {
        None | Some(Yaml::Null) => Vec::new(),
        Some(value) => value
            .as_vec()
            .ok_or_else(|| format!("{name}: `checks` must be a list"))?
            .iter()
            .map(|check| parse_check(name, check))
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok(Definition {
        scenario: required(document, "scenario", name)?.to_string(),
        answers: required(document, "answers", name)?.to_string(),
        setup,
        prompt: required(document, "prompt", name)?.to_string(),
        replies,
        max_turns: field(document, "max_turns")
            .and_then(Yaml::as_i64)
            .unwrap_or(30)
            .try_into()
            .map_err(|_| format!("{name}: `max_turns` must be positive"))?,
        checks,
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

fn git(project: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=TopHat Eval",
            "-c",
            "user.email=eval@tophat.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(project)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
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

fn commit(project: &Path, message: &str) -> Result<(), String> {
    if !project.join(".git").is_dir() {
        git(project, &["init", "-q"])?;
    }
    git(project, &["add", "-A"])?;
    git(project, &["commit", "-q", "-m", message])
}

fn tophat(binary: &Path, project: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(binary)
        .args(args)
        .current_dir(project)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    if !output.status.success() {
        return Err(format!(
            "`tophat {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Finds a stamped question ID in `PROJECT.md`.
fn first_question(project: &Path) -> Option<String> {
    let text = fs::read_to_string(project.join("PROJECT.md")).ok()?;
    text.split_whitespace()
        .find(|token| {
            token.len() == 10
                && token.starts_with("q-")
                && token[2..].chars().all(|c| c.is_ascii_hexdigit())
        })
        .map(str::to_string)
}

fn setup(project: &Path, repo_root: &Path, binary: &Path, steps: &[Step]) -> Result<(), String> {
    for step in steps {
        match step {
            Step::Write { path, content } => {
                let target = project.join(path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
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
            Step::Commit(message) => commit(project, message)?,
            Step::Mkdir(path) => {
                let target = project.join(path);
                fs::create_dir_all(&target)
                    .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
            }
            Step::CopyExample(name) => {
                let from = repo_root
                    .join("template/.tophat/managed/examples")
                    .join(name);
                if !from.is_dir() {
                    return Err(format!("no worked example named '{name}'"));
                }
                let to = project.join("studies").join(name);
                copy_tree(&from, &to)?;
                let question = first_question(project);
                if let Some(question) = question {
                    let study = to.join("STUDY.md");
                    let text = fs::read_to_string(&study)
                        .map_err(|e| format!("cannot read {}: {e}", study.display()))?;
                    let text =
                        text.replace("question: q-00000000", &format!("question: {question}"));
                    fs::write(&study, text)
                        .map_err(|e| format!("cannot write {}: {e}", study.display()))?;
                }
            }
            Step::Tophat(args) => {
                let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
                tophat(binary, project, &borrowed)?;
            }
        }
    }
    Ok(())
}

fn agents(name: &str) -> Result<&'static str, String> {
    match name {
        "claude-code" => Ok("claude"),
        "codex" => Ok("codex"),
        "opencode" => Ok("opencode"),
        other => Err(format!(
            "unknown agent '{other}'; use claude-code, codex or opencode"
        )),
    }
}

/// One parsed agent turn.
#[derive(Default)]
struct Transcript {
    text: String,
    commands: Vec<String>,
    questions: Vec<String>,
    session: Option<String>,
}

fn collect(value: &Value, out: &mut Transcript) {
    match value {
        Value::Object(map) => {
            if let Some(command) = map.get("command").and_then(Value::as_str) {
                out.commands.push(command.to_string());
            }
            if let Some(question) = map.get("question").and_then(Value::as_str) {
                out.questions.push(question.to_string());
            }
            if let Some(session) = map.get("session_id").and_then(Value::as_str) {
                out.session = Some(session.to_string());
            }
            if map.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    out.text.push_str(text);
                    out.text.push('\n');
                }
            }
            if let Some(Value::Object(message)) = map.get("message") {
                collect(&Value::Object(message.clone()), out);
            }
            for value in map.values() {
                collect(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, out);
            }
        }
        _ => {}
    }
}

fn parse_transcript(bytes: &[u8]) -> Transcript {
    let mut out = Transcript::default();
    for line in String::from_utf8_lossy(bytes).lines() {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        collect(&value, &mut out);
    }
    if out.questions.is_empty() {
        for line in out.text.lines() {
            let line = line.trim();
            if line.ends_with('?') {
                out.questions.push(line.to_string());
            }
        }
    }
    out.commands.sort();
    out.commands.dedup();
    out
}

/// Matches a reply pattern: an optional `(?i)` prefix and a substring.
fn matches(pattern: &str, question: &str) -> bool {
    let (pattern, insensitive) = match pattern.strip_prefix("(?i)") {
        Some(rest) => (rest, true),
        None => (pattern, false),
    };
    if insensitive {
        question.to_lowercase().contains(&pattern.to_lowercase())
    } else {
        question.contains(pattern)
    }
}

fn reply(definition: &Definition, question: &str) -> Option<String> {
    definition
        .replies
        .iter()
        .find(|reply| matches(&reply.when, question))
        .map(|reply| reply.say.clone())
}

struct AgentContext<'a> {
    agent: &'a str,
    program: &'a Path,
    project: &'a Path,
    state: &'a Path,
    repeat: usize,
}

fn run_agent(
    context: &AgentContext<'_>,
    turn: usize,
    prompt: &str,
    session: Option<&str>,
) -> Result<String, String> {
    let mut command = Command::new(context.program);
    match context.agent {
        "claude-code" => {
            command.args([
                "--print",
                prompt,
                "--output-format",
                "stream-json",
                "--verbose",
                "--dangerously-skip-permissions",
            ]);
            if let Some(session) = session {
                command.args(["--resume", session]);
            }
        }
        "codex" => {
            command.args(["exec", "--json", "--skip-git-repo-check", prompt]);
        }
        "opencode" => {
            command.args(["run", "--format", "json", prompt]);
        }
        _ => unreachable!("validated above"),
    }
    let output = command
        .current_dir(context.project)
        .env("TOPHAT_EVAL_PROJECT", context.project)
        .env("TOPHAT_EVAL_STATE", context.state)
        .env("TOPHAT_EVAL_TURN", turn.to_string())
        .env("TOPHAT_EVAL_REPEAT", context.repeat.to_string())
        .output()
        .map_err(|e| format!("cannot run agent {}: {e}", context.agent))?;
    if !output.status.success() {
        return Err(format!(
            "agent {} failed: {}",
            context.agent,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The section body of `name` in a `STUDY.md` text.
fn section(text: &str, name: &str) -> String {
    let marker = format!("## {name}");
    let mut lines = text.lines().skip_while(|line| line.trim_end() != marker);
    if lines.next().is_none() {
        return String::new();
    }
    let body: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    body.join("\n").trim().to_string()
}

fn read_target(project: &Path, path: &str, section_name: &Option<String>) -> String {
    let text = fs::read_to_string(project.join(path)).unwrap_or_default();
    match section_name {
        Some(name) => section(&text, name),
        None => text,
    }
}

fn tophat_check(binary: &Path, project: &Path, expected: &str) -> Result<Option<String>, String> {
    let output = Command::new(binary)
        .args(["check", "--json"])
        .current_dir(project)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("`check --json` printed invalid JSON: {e}"))?;
    let diagnostics = value
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if expected == "no-errors" {
        let failing = diagnostics
            .iter()
            .any(|d| d.get("severity").and_then(Value::as_str) == Some("error"));
        if output.status.success() && !failing {
            Ok(None)
        } else {
            Ok(Some("`tophat check` reported errors".into()))
        }
    } else if diagnostics
        .iter()
        .any(|d| d.get("rule").and_then(Value::as_str) == Some(expected))
    {
        Ok(None)
    } else {
        Ok(Some(format!("`tophat check` did not report `{expected}`")))
    }
}

fn evaluate(
    project: &Path,
    binary: &Path,
    definition: &Definition,
    transcript: &Transcript,
    baseline: &BTreeMap<usize, String>,
) -> Result<(), String> {
    for (index, check) in definition.checks.iter().enumerate() {
        match check {
            Check::Unchanged { path, section } => {
                let now = read_target(project, path, section);
                if baseline.get(&index) != Some(&now) {
                    return Err(format!("{path} changed"));
                }
            }
            Check::Changed { path, section } => {
                let now = read_target(project, path, section);
                if baseline.get(&index) == Some(&now) {
                    return Err(format!("{path} did not change"));
                }
            }
            Check::NotRun(prefixes) => {
                for prefix in prefixes {
                    if let Some(command) = transcript
                        .commands
                        .iter()
                        .find(|command| command.starts_with(prefix))
                    {
                        return Err(format!("forbidden command `{prefix}` ran: {command}"));
                    }
                }
            }
            Check::Ran(prefixes) => {
                for prefix in prefixes {
                    if !transcript
                        .commands
                        .iter()
                        .any(|command| command.starts_with(prefix))
                    {
                        return Err(format!("expected command `{prefix}` did not run"));
                    }
                }
            }
            Check::Contains { path, text } => {
                let actual = fs::read_to_string(project.join(path)).unwrap_or_default();
                if !actual.contains(text) {
                    return Err(format!("{path} does not contain {text:?}"));
                }
            }
            Check::Project(expected) => {
                if let Some(problem) = tophat_check(binary, project, expected)? {
                    return Err(problem);
                }
            }
        }
    }
    Ok(())
}

fn baseline(project: &Path, definition: &Definition) -> BTreeMap<usize, String> {
    let mut out = BTreeMap::new();
    for (index, check) in definition.checks.iter().enumerate() {
        match check {
            Check::Unchanged { path, section } | Check::Changed { path, section } => {
                out.insert(index, read_target(project, path, section));
            }
            _ => {}
        }
    }
    out
}

fn execute(
    repo_root: &Path,
    binary: &Path,
    definition: &Definition,
    agent: &str,
    program: &Path,
    repeat: usize,
) -> Result<(), String> {
    let temp = Temp::new("run")?;
    let project = temp.0.join("project");
    let request = Request {
        answers: definition.answers.clone(),
        out: project.clone(),
        sets: Vec::new(),
    };
    render::render(repo_root, &request).map_err(|e| e.to_string())?;
    setup(&project, repo_root, binary, &definition.setup)?;
    let baseline = baseline(&project, definition);

    let state = temp.0.join("state");
    fs::create_dir_all(&state).map_err(|e| format!("cannot create {}: {e}", state.display()))?;
    let context = AgentContext {
        agent,
        program,
        project: &project,
        state: &state,
        repeat,
    };

    let mut transcript = Transcript::default();
    let mut turn = 0usize;
    let mut next = definition.prompt.clone();
    let mut conversation = Vec::new();
    loop {
        turn += 1;
        if turn > definition.max_turns {
            return Err(format!("reached max_turns {}", definition.max_turns));
        }
        let raw = run_agent(&context, turn, &next, transcript.session.as_deref())?;
        let parsed = parse_transcript(raw.as_bytes());
        transcript.text.push_str(&parsed.text);
        transcript.commands.extend(parsed.commands.iter().cloned());
        transcript.commands.sort();
        transcript.commands.dedup();
        if transcript.session.is_none() {
            transcript.session = parsed.session.clone();
        }
        if parsed.questions.is_empty() {
            break;
        }
        let mut replies = Vec::new();
        for question in &parsed.questions {
            match reply(definition, question) {
                Some(say) => replies.push(say),
                None => return Err(format!("unanswered question: {question}")),
            }
        }
        let say = replies.join("\n");
        conversation.push(format!("Assistant: {}\nHuman: {say}", parsed.text.trim()));
        next = if agent == "claude-code" && transcript.session.is_some() {
            say
        } else {
            format!("{}\nHuman: {say}", conversation.join("\n"))
        };
    }
    evaluate(&project, binary, definition, &transcript, &baseline)
}

/// Runs the eval definitions in `evals_dir`. `program` overrides the agent executable for tests.
// rq-03d2afb2
pub fn run_dir(
    repo_root: &Path,
    evals_dir: &Path,
    binary: &Path,
    options: &Options,
    program: Option<&Path>,
) -> Result<Report, String> {
    agents(&options.agent)?;
    let configured = program
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("TOPHAT_EVAL_AGENT").map(PathBuf::from));
    let program = configured.unwrap_or_else(|| PathBuf::from(agents(&options.agent).unwrap()));

    let mut names: Vec<String> = fs::read_dir(evals_dir)
        .map_err(|e| format!("cannot read {}: {e}", evals_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|t| t.is_file())
                && entry.path().extension().is_some_and(|e| e == "yaml")
        })
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    if !options.evals.is_empty() {
        for name in &options.evals {
            if !names.contains(name) {
                return Err(format!("no eval named '{name}'"));
            }
        }
        names.retain(|name| options.evals.contains(name));
    }

    let mut report = Report {
        lines: Vec::new(),
        failed: false,
    };
    for name in names {
        let path = evals_dir.join(format!("{name}.yaml"));
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let definition = parse_definition(&name, &text)?;
        let repeat = options.repeat.max(1);
        let mut passed = 0;
        let mut problems = Vec::new();
        for index in 0..repeat {
            match execute(
                repo_root,
                binary,
                &definition,
                &options.agent,
                &program,
                index,
            ) {
                Ok(()) => passed += 1,
                Err(e) => problems.push(e),
            }
        }
        if passed == repeat {
            report.lines.push(format!("{name} ok"));
        } else {
            report.failed = true;
            report
                .lines
                .push(format!("{name} FAILED {passed}/{repeat}"));
            report
                .lines
                .extend(problems.into_iter().map(|p| format!("  {p}")));
        }
        report
            .lines
            .push(format!("{name} scenario {}", definition.scenario));
    }
    Ok(report)
}

/// Runs `repo_root/evals` with the repository's answer sets and the binary at `binary`.
pub fn run(repo_root: &Path, binary: &Path, options: &Options) -> Result<Report, String> {
    run_dir(repo_root, &repo_root.join("evals"), binary, options, None)
}
