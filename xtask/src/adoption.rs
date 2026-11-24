//! `cargo xtask adoption`: run the committed adoption fixtures (see `rqm/testing.md`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use sha2::{Digest, Sha256};
use yaml_rust2::{Yaml, YamlLoader};

const DATE: &str = "2026-01-01T00:00:00Z";
const BINARY: &str = "tophat";
const ANSWERS: &str = "xtask/answers/minimal-claude.yaml";

/// One dataset to declare from the fixture's processed data.
struct DatasetRecipe {
    name: String,
    root: String,
    path: String,
    statement: String,
    visibility: String,
}

/// One retrospective study to create for the fixture.
struct StudyRecipe {
    id: String,
    title: String,
    endpoint: String,
    decision: String,
    result: String,
    interpretation: String,
    stage: String,
    output_file: String,
    output_content: String,
}

/// A parsed `case.yaml`.
struct Recipe {
    question: String,
    dataset: DatasetRecipe,
    study: StudyRecipe,
}

/// The result of one fixture.
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
            "tophat-adoption-{}-{}",
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

fn required<'a>(
    hash: &'a yaml_rust2::yaml::Hash,
    key: &str,
    context: &str,
) -> Result<&'a str, String> {
    field(hash, key)
        .and_then(Yaml::as_str)
        .ok_or_else(|| format!("{context}: `{key}` must be a string"))
}

fn parse_recipe(name: &str, text: &str) -> Result<Recipe, String> {
    let documents =
        YamlLoader::load_from_str(text).map_err(|e| format!("{name}: invalid YAML: {e}"))?;
    let document = documents
        .first()
        .and_then(Yaml::as_hash)
        .ok_or_else(|| format!("{name}: case.yaml must be a mapping"))?;
    let dataset = field(document, "dataset")
        .and_then(Yaml::as_hash)
        .ok_or_else(|| format!("{name}: case.yaml has no `dataset` mapping"))?;
    let study = field(document, "study")
        .and_then(Yaml::as_hash)
        .ok_or_else(|| format!("{name}: case.yaml has no `study` mapping"))?;
    let string = |hash: &yaml_rust2::yaml::Hash, key: &str, default: Option<&str>| {
        field(hash, key)
            .and_then(Yaml::as_str)
            .map(str::to_string)
            .or_else(|| default.map(str::to_string))
            .ok_or_else(|| format!("{name}: `{key}` must be a string"))
    };
    Ok(Recipe {
        question: required(document, "question", name)?.to_string(),
        dataset: DatasetRecipe {
            name: required(dataset, "name", name)?.to_string(),
            root: string(dataset, "root", Some("results"))?,
            path: required(dataset, "path", name)?.to_string(),
            statement: required(dataset, "statement", name)?.to_string(),
            visibility: string(dataset, "visibility", Some("public"))?,
        },
        study: StudyRecipe {
            id: required(study, "id", name)?.to_string(),
            title: required(study, "title", name)?.to_string(),
            endpoint: required(study, "endpoint", name)?.to_string(),
            decision: required(study, "decision", name)?.to_string(),
            result: required(study, "result", name)?.to_string(),
            interpretation: string(study, "interpretation", Some(""))?,
            stage: string(study, "stage", Some("analyse"))?,
            output_file: string(study, "output_file", Some("result.txt"))?,
            output_content: string(study, "output_content", Some("adopted\n"))?,
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

fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Hashes every regular file below `root`, excluding `.git`.
fn hash_tree(root: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(base: &Path, current: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
        for entry in
            fs::read_dir(current).map_err(|e| format!("cannot read {}: {e}", current.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let path = entry.path();
            let file_type = entry.file_type().map_err(|e| e.to_string())?;
            if file_type.is_dir() {
                walk(base, &path, out)?;
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(base)
                    .expect("walked paths are below the base")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(relative, hash_file(&path)?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn git(project: &Path, args: &[&str], dated: bool) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=TopHat Adoption",
            "-c",
            "user.email=adoption@tophat.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(project)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if dated {
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

fn commit(project: &Path, message: &str) -> Result<(), String> {
    git(project, &["add", "-A"], false)?;
    git(project, &["commit", "-q", "-m", message], true)
}

/// The `skip` paths in Copier's status output.
fn skipped_files(output: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(output);
    let mut skipped = Vec::new();
    for line in text.lines() {
        let line = strip_ansi(line);
        let line = line.trim();
        if let Some(path) = line.strip_prefix("skip") {
            let path = path.trim();
            if !path.is_empty() {
                skipped.push(path.to_string());
            }
        }
    }
    skipped.sort();
    skipped.dedup();
    skipped
}

/// Removes ANSI colour sequences from a line.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn copier(repo_root: &Path, project: &Path) -> Result<Vec<String>, String> {
    match Command::new("copier").arg("--version").output() {
        Ok(out) if out.status.success() => {}
        _ => return Err("copier is not on PATH; install Copier 9".into()),
    }
    let answers = repo_root.join(ANSWERS);
    let output = Command::new("copier")
        .args(["copy", "--vcs-ref", "HEAD", "--defaults", "--data-file"])
        .arg(&answers)
        .arg(repo_root)
        .arg(project)
        .output()
        .map_err(|e| format!("copier could not be run: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "copier failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut skipped = skipped_files(&output.stderr);
    skipped.extend(skipped_files(&output.stdout));
    skipped.sort();
    skipped.dedup();
    Ok(skipped)
}

fn tophat(binary: &Path, project: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new(binary);
    command.args(args).current_dir(project);
    for (key, _) in std::env::vars() {
        if key.starts_with("TOPHAT_ROOT_") {
            command.env_remove(key);
        }
    }
    command
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))
}

fn tophat_ok(binary: &Path, project: &Path, args: &[&str]) -> Result<String, String> {
    let output = tophat(binary, project, args)?;
    if !output.status.success() {
        return Err(format!(
            "`{BINARY} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn study_text(study: &StudyRecipe, question: &str) -> String {
    let result_line = format!("- base/{}: {} [post hoc]", study.stage, study.result);
    format!(
        "---\nstudy: {id}\ntitle: \"{title}\"\nquestion: {question}\nstatus: reported\nmode: retrospective\nvisibility: public\nvariants:\n  - base\n---\n\n# {id}\n\n## Question\n\n{question_text}\n\n## Primary endpoint\n\n{endpoint}\n\n## Decision rule\n\n{decision}\n\n## Scenarios\n\n```gherkin\nFeature: adopted work\n\n  @S1\n  Scenario: The adopted analysis reports its result\n    Given the declared dataset\n    When the pipeline completes\n    Then the result is recorded\n```\n\n## Variants\n\n- base\n\n## Stages\n\n{stage}\n\n## Result\n\n{result}\n\n## Interpretation\n\n{interpretation}\n\n## Amendments\n\n",
        id = study.id,
        title = study.title,
        question = question,
        question_text = study.title,
        endpoint = study.endpoint,
        decision = study.decision,
        stage = study.stage,
        result = result_line,
        interpretation = study.interpretation,
    )
}

fn config_text(dataset: &DatasetRecipe, study: &StudyRecipe) -> String {
    format!(
        "stages:\n  - name: {stage}\n    run: sh scripts/adopted.sh {{out}}\n    code: [scripts/adopted.sh]\n    inputs: {{data: \"dataset:{name}@v1\"}}\n    result: {result}\n",
        stage = study.stage,
        name = dataset.name,
        result = study.output_file,
    )
}

fn key_of(output: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(output)
        .map_err(|e| format!("`pipeline keys --json` printed invalid JSON: {e}"))?;
    value
        .as_array()
        .and_then(|stages| stages.first())
        .and_then(|stage| stage.get("key"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "`pipeline keys --json` named no stage key".to_string())
}

/// Adds a question to `PROJECT.md` and returns the ID `tophat question stamp` assigns.
fn add_question(binary: &Path, project: &Path, question: &str) -> Result<String, String> {
    let path = project.join("PROJECT.md");
    let mut text =
        fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    text.push_str(&format!("\n## {question}\nstatus: open\n"));
    write(&path, &text)?;
    let output = tophat_ok(binary, project, &["question", "stamp"])?;
    output
        .split_whitespace()
        .next()
        .filter(|id| id.starts_with("q-"))
        .map(str::to_string)
        .ok_or_else(|| "`question stamp` printed no question ID".to_string())
}

/// Applies the template and performs the adoption mapping for one fixture.
fn adopt(
    fixture: &str,
    directory: &Path,
    repo_root: &Path,
    binary: &Path,
) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(directory.join("case.yaml"))
        .map_err(|e| format!("cannot read {fixture}/case.yaml: {e}"))?;
    let recipe = parse_recipe(fixture, &text)?;
    let temp = Temp::new()?;
    let project = temp.0.join("project");
    copy_tree(&directory.join("repository"), &project)?;
    let recorded = hash_tree(&project)?;
    git(&project, &["init", "-q"], false)?;
    commit(&project, "initial")?;

    let skipped = copier(repo_root, &project)?;
    let after_template = hash_tree(&project)?;
    let mut problems = Vec::new();
    for (path, expected) in &recorded {
        match after_template.get(path) {
            None => problems.push(format!("the template removed {path}")),
            Some(actual) if actual != expected => {
                problems.push(format!("the template overwrote {path}"))
            }
            Some(_) => {}
        }
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }

    let question = add_question(binary, &project, &recipe.question)?;
    tophat_ok(
        binary,
        &project,
        &[
            "data",
            "declare",
            &recipe.dataset.name,
            "--root",
            &recipe.dataset.root,
            "--path",
            &recipe.dataset.path,
            "--statement",
            &recipe.dataset.statement,
            "--visibility",
            &recipe.dataset.visibility,
        ],
    )?;
    tophat_ok(
        binary,
        &project,
        &["study", "new", &recipe.study.id, "--question", &question],
    )?;

    let study_dir = project.join("studies").join(&recipe.study.id);
    write(
        &study_dir.join("STUDY.md"),
        &study_text(&recipe.study, &question),
    )?;
    write(
        &study_dir.join("config/base.yaml"),
        &config_text(&recipe.dataset, &recipe.study),
    )?;
    write(
        &study_dir.join("scripts/adopted.sh"),
        "#!/bin/sh\nprintf 'adopted\\n' > \"$1/result.txt\"\n",
    )?;
    commit(&project, "adopt existing work")?;

    let keys = tophat_ok(
        binary,
        &project,
        &["pipeline", "keys", &recipe.study.id, "--json"],
    )?;
    let key = key_of(&keys)?;
    let output_dir = project
        .join("results")
        .join(&recipe.study.id)
        .join(&recipe.study.stage)
        .join(&key);
    write(
        &output_dir.join(&recipe.study.output_file),
        &recipe.study.output_content,
    )?;
    tophat_ok(
        binary,
        &project,
        &[
            "stage",
            "seal",
            &recipe.study.id,
            &recipe.study.stage,
            "--key",
            &key,
        ],
    )?;

    let check = tophat(binary, &project, &["check", "--json"])?;
    if check.status.code() != Some(0) {
        return Err(format!(
            "`tophat check` failed: {}",
            String::from_utf8_lossy(&check.stdout).trim()
        ));
    }
    let value: Value = serde_json::from_slice(&check.stdout)
        .map_err(|e| format!("`check --json` printed invalid JSON: {e}"))?;
    let mut legacy = Vec::new();
    for diagnostic in value
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if diagnostic.get("severity").and_then(Value::as_str) == Some("legacy") {
            legacy.push(
                diagnostic
                    .get("rule")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
            );
        }
    }
    for rule in ["data.declared-input", "study.result-unverified"] {
        if !legacy.iter().any(|r| r == rule) {
            return Err(format!("`tophat check` did not report legacy `{rule}`"));
        }
    }

    let target = format!("{}/{}@{}", recipe.study.id, recipe.study.stage, key);
    let lineage = tophat_ok(binary, &project, &["data", "lineage", &target])?;
    if !lineage.contains("[declared, not verified]") {
        return Err(format!(
            "`tophat data lineage {target}` does not mark the declared dataset:\n{lineage}"
        ));
    }

    let after_mapping = hash_tree(&project)?;
    for (path, expected) in &recorded {
        if after_mapping.get(path) != Some(expected) {
            return Err(format!("the adoption mapping changed {path}"));
        }
    }

    let mut lines = Vec::new();
    for path in skipped {
        lines.push(format!("{fixture} skipped {path}"));
    }
    for rule in legacy {
        lines.push(format!("{fixture} legacy {rule}"));
    }
    lines.push(format!("{fixture} lineage [declared, not verified]"));
    Ok(lines)
}

/// Runs the adoption fixtures in `repo_root/tests/adoption`. `selected` restricts the run.
// rq-5da5a173
pub fn run(repo_root: &Path, binary: &Path, selected: &[String]) -> Result<Report, String> {
    for program in ["git", "copier"] {
        match Command::new(program).arg("--version").output() {
            Ok(out) if out.status.success() => {}
            _ => return Err(format!("{program} is not on PATH")),
        }
    }
    match Command::new(binary).arg("--version").output() {
        Ok(out) if out.status.success() => {}
        _ => return Err(format!("cannot run {} --version", binary.display())),
    }
    let fixtures_dir = repo_root.join("tests/adoption");
    let mut names: Vec<String> = fs::read_dir(&fixtures_dir)
        .map_err(|e| format!("cannot read {}: {e}", fixtures_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if !selected.is_empty() {
        for name in selected {
            if !names.contains(name) {
                return Err(format!("no adoption fixture named '{name}'"));
            }
        }
        names.retain(|name| selected.contains(name));
    }

    let mut report = Report {
        lines: Vec::new(),
        failed: false,
    };
    for name in names {
        let directory = fixtures_dir.join(&name);
        match adopt(&name, &directory, repo_root, binary) {
            Ok(lines) => {
                report.lines.extend(lines);
                report.lines.push(format!("{name} ok"));
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
