//! `cargo xtask render-test`: render every answer set and run the render checks of
//! `rqm/testing.md` on each project.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::lint;
use crate::render::{self, Request};

/// The canonical skills every project ships.
const SKILLS: [&str; 8] = [
    "th-adopt",
    "th-data",
    "th-next",
    "th-question",
    "th-quiz",
    "th-run",
    "th-study",
    "th-tool",
];

/// The agent adapter directories.
const ADAPTER_DIRS: [(&str, &str); 3] = [
    ("claude-code", ".claude/skills"),
    ("codex", ".agents/skills"),
    ("opencode", ".opencode/skills"),
];

/// The files every rendered project has (rqm/template/project.md and container.md).
const SKELETON: [&str; 24] = [
    ".copier-answers.yml",
    "th.sh",
    ".tophat/managed/launch/th.sh",
    ".tophat/managed/launch/run-options.sh",
    ".tophat/managed/launch/enter.sh",
    ".tophat/managed/container/Containerfile",
    ".tophat/managed/container/Agent.Containerfile",
    ".tophat/managed/container/versions.yaml",
    ".tophat/managed/hooks/check-container.sh",
    ".tophat/user/container/Containerfile",
    ".tophat/user/podman/run-options",
    "PROJECT.md",
    "README.md",
    "pixi.toml",
    "pixi.lock",
    ".gitignore",
    "exploration/README.md",
    ".github/workflows/tophat-check.yaml",
    ".tophat/.gitignore",
    ".tophat/roots.yaml",
    ".tophat/managed/GUIDE.md",
    ".tophat/managed/tophat-release.yaml",
    ".tophat/managed/hooks/pre-commit",
    ".tophat/managed/ownership-exceptions",
];

/// The command's options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Answer-set names under `xtask/answers/`; empty selects every set.
    pub answers: Vec<String>,
    /// Skip building the container image and the checks that need it.
    pub no_image: bool,
}

/// The result of a run.
#[derive(Debug, Default)]
pub struct Report {
    /// One line per answer set or failure.
    pub lines: Vec<String>,
    pub failed: bool,
    /// Checks that could not run because an external program is missing.
    pub cannot_run: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            writeln!(f, "{line}")?;
        }
        Ok(())
    }
}

/// A temporary directory removed when it is dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Result<Temp, String> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tophat-render-test-{label}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        fs::create_dir_all(path.join("home"))
            .map_err(|e| format!("cannot create {}: {e}", path.join("home").display()))?;
        Ok(Temp(path))
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The `tophat` binary beside this one, built from the workspace.
fn tophat_binary(repo_root: &Path) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("tophat"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("tophat"));
        }
    }
    candidates.push(repo_root.join("target/debug/tophat"));
    if let Some(path) = candidates.iter().find(|path| path.is_file()) {
        Ok(path.clone())
    } else {
        Err(format!(
            "the tophat binary {} does not exist; run `cargo build` first",
            candidates[0].display()
        ))
    }
}

/// Runs a program and returns its output; a missing program is named.
fn output(program: &OsStr, args: &[&str], directory: &Path) -> Result<Output, String> {
    match Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
    {
        Ok(output) => Ok(output),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(format!(
            "{} is not on PATH",
            Path::new(program)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| program.to_string_lossy().into_owned())
        )),
        Err(e) => Err(format!("cannot run {}: {e}", program.to_string_lossy())),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

fn read(project: &Path, rel: &str) -> Result<String, String> {
    fs::read_to_string(project.join(rel)).map_err(|e| format!("cannot read {rel}: {e}"))
}

/// A file of the answer set, parsed as YAML.
fn answer_set(path: &Path) -> Result<yaml_rust2::Yaml, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let docs = yaml_rust2::YamlLoader::load_from_str(&text)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    docs.into_iter()
        .next()
        .ok_or_else(|| format!("{} is empty", path.display()))
}

fn strings(value: &yaml_rust2::Yaml) -> Vec<String> {
    match value {
        yaml_rust2::Yaml::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        yaml_rust2::Yaml::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Every answer set, sorted by name.
fn answer_sets(repo_root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let dir = repo_root.join("xtask/answers");
    let mut sets = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("yaml")) {
            continue;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        sets.push((name, path));
    }
    sets.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(sets)
}

/// Every file below `dir`, as `/`-separated paths relative to `dir`.
fn walk(dir: &Path) -> Result<Vec<String>, String> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out)?;
            } else if path.is_file()
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// A Copier-style path pattern: `*` stays within a segment, a trailing `/**` covers a directory.
fn path_matches(pattern: &str, path: &str) -> bool {
    if let Some(dir) = pattern.strip_suffix("/**") {
        return path.starts_with(&format!("{dir}/"));
    }
    fn glob(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len())
                .take_while(|&i| i == 0 || s[i - 1] != b'/')
                .any(|i| glob(rest, &s[i..])),
            Some((c, rest)) => s.first() == Some(c) && glob(rest, &s[1..]),
        }
    }
    glob(pattern.as_bytes(), path.as_bytes())
}

/// Whether a managed file carries the marker in its first five content lines.
fn has_marker(text: &str) -> bool {
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.first() == Some(&"---")
        && let Some(end) = lines.iter().skip(1).position(|l| *l == "---")
    {
        lines.drain(..=end + 1);
    }
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .any(|l| l.contains("TopHat-managed"))
}

// rq-d0e11fee rq-fc2247e5
fn check_skeleton(project: &Path) -> Result<(), String> {
    for file in SKELETON {
        if !project.join(file).is_file() {
            return Err(format!("{file} is missing"));
        }
    }
    for dir in ["studies", "datasets"] {
        if project.join(dir).exists() {
            return Err(format!("a new project has a {dir}/ directory"));
        }
    }
    Ok(())
}

// rq-74365fac rq-cafa162c
fn check_marker(project: &Path) -> Result<(), String> {
    let exceptions = read(project, ".tophat/managed/ownership-exceptions")?;
    let patterns: Vec<&str> = exceptions
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut managed = 0;
    for file in walk(project)? {
        let is_managed =
            file.starts_with(".tophat/managed/") || patterns.iter().any(|p| path_matches(p, &file));
        if !is_managed || file.starts_with(".tophat/managed/examples/") || file.ends_with(".json") {
            continue;
        }
        managed += 1;
        let content = read(project, &file)?;
        if !has_marker(&content) {
            return Err(format!("{file} lacks the TopHat-managed marker"));
        }
    }
    if managed < 6 {
        return Err(format!("only {managed} managed files were rendered"));
    }
    Ok(())
}

// rq-0da9fa10 rq-4da71d7f
fn check_ignore_rules(project: &Path) -> Result<(), String> {
    let local = read(project, ".tophat/.gitignore")?;
    for rule in [
        "/roots.local.yaml",
        "/slurm.local.yaml",
        "/cache/",
        "/tools/",
        "/generated/",
        "/state/",
    ] {
        if !local.lines().any(|l| l.trim() == rule) {
            return Err(format!(".tophat/.gitignore lacks {rule}"));
        }
    }
    let root = read(project, ".gitignore")?;
    for rule in ["/.snakemake/", "/.pixi/", "/results/"] {
        if !root.lines().any(|l| l.trim() == rule) {
            return Err(format!(".gitignore lacks {rule}"));
        }
    }
    Ok(())
}

// rq-5c6c0574 rq-e660df68
fn check_project_md(project: &Path) -> Result<(), String> {
    let answers = answer_set(&project.join(".copier-answers.yml"))?;
    let name = answers["project_name"].as_str().unwrap_or_default();
    let description = answers["project_description"].as_str().unwrap_or_default();
    let project_md = read(project, "PROJECT.md")?;
    if !project_md.starts_with(&format!("# {name}\n")) {
        return Err("PROJECT.md does not start with the project name".into());
    }
    if !project_md.contains(description) {
        return Err("PROJECT.md does not contain the project description".into());
    }
    Ok(())
}

// rq-d1fe5c84 rq-cba13e20
fn check_roots(project: &Path, tophat: &Path) -> Result<(), String> {
    let roots = read(project, ".tophat/roots.yaml")?;
    if !roots.contains("results: {}") {
        return Err(".tophat/roots.yaml does not declare the results root".into());
    }
    if project.join(".tophat/roots.local.yaml").exists() {
        return Err("a new project has a .tophat/roots.local.yaml".into());
    }
    let out = output(tophat.as_os_str(), &["data", "roots"], project)?;
    if !out.status.success() {
        return Err(format!("`tophat data roots` failed: {}", text(&out.stderr)));
    }
    let stdout = text(&out.stdout);
    let expected = format!(
        "results\t{}/results",
        std::path::absolute(project)
            .map_err(|e| e.to_string())?
            .display()
    );
    if stdout != expected {
        return Err(format!(
            "`tophat data roots` printed {stdout:?}, expected {expected:?}"
        ));
    }
    Ok(())
}

// rq-e9156963 rq-87749f65 rq-260d84c9
fn check_environment(project: &Path, answers: &yaml_rust2::Yaml) -> Result<(), String> {
    let toml = read(project, "pixi.toml")?;
    for package in ["python", "snakemake-minimal"] {
        if !toml.contains(&format!("\n{package} = ")) {
            return Err(format!("pixi.toml lacks {package}"));
        }
    }
    let julia = strings(&answers["languages"]).iter().any(|l| l == "julia");
    if julia != toml.contains("\njulia = ") {
        return Err(format!(
            "pixi.toml {} julia for languages {:?}",
            if julia { "lacks" } else { "unexpectedly has" },
            strings(&answers["languages"])
        ));
    }
    let lock = read(project, "pixi.lock")?;
    for platform in ["linux-64", "linux-aarch64"] {
        if !lock.contains(platform) {
            return Err(format!("pixi.lock does not cover {platform}"));
        }
    }
    if !lock.contains("snakemake-minimal") {
        return Err("pixi.lock lacks snakemake-minimal".into());
    }
    if julia && !lock.contains("julia") {
        return Err("pixi.lock lacks julia".into());
    }
    Ok(())
}

// rq-99f3ccdf rq-56d220b5
fn check_release_pin(project: &Path) -> Result<(), String> {
    let pin = answer_set(&project.join(".tophat/managed/tophat-release.yaml"))?;
    match pin["version"].as_str() {
        Some("unreleased") => {
            if !pin["url"].is_badvalue() || !pin["sha256"].is_badvalue() {
                return Err("an unreleased pin has a url or sha256".into());
            }
        }
        Some(_) => {
            if pin["url"].as_str().is_none_or(str::is_empty) {
                return Err("the pin has no url".into());
            }
            for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
                let hash = pin["sha256"][target].as_str().unwrap_or_default();
                if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("the pin has no sha256 for {target}"));
                }
            }
        }
        None => return Err("the pin has no version".into()),
    }
    Ok(())
}

// rq-1ec0c367 rq-6fd1f551 rq-4a8d3a7d
fn check_initial_state(project: &Path, tophat: &Path) -> Result<(), String> {
    let check = output(tophat.as_os_str(), &["check", "--strict"], project)?;
    if !check.status.success() {
        return Err(format!(
            "`tophat check --strict` failed: {}{}",
            text(&check.stdout),
            text(&check.stderr)
        ));
    }
    let status = output(tophat.as_os_str(), &["status"], project)?;
    if !status.status.success() {
        return Err(format!("`tophat status` failed: {}", text(&status.stderr)));
    }
    let summary = text(&status.stdout);
    if !summary.contains("0 open, 0 answered, 0 dropped") {
        return Err(format!("`tophat status` printed {summary:?}"));
    }
    Ok(())
}

// rq-42fba780 rq-2b638c5b
fn check_skills(project: &Path, answers: &yaml_rust2::Yaml) -> Result<(), String> {
    for skill in SKILLS {
        let canonical = format!(".tophat/managed/skills/{skill}/SKILL.md");
        if !project.join(&canonical).is_file() {
            return Err(format!("{canonical} is missing"));
        }
        let local = format!(".tophat/user/skills/{skill}/local.md");
        if !project.join(&local).is_file() {
            return Err(format!("{local} is missing"));
        }
    }
    let selected = strings(&answers["agents"]);
    for (agent, dir) in ADAPTER_DIRS {
        let present = SKILLS
            .iter()
            .filter(|skill| project.join(dir).join(skill).join("SKILL.md").is_file())
            .count();
        if selected.iter().any(|a| a == agent) {
            for skill in SKILLS {
                let rel = format!("{dir}/{skill}/SKILL.md");
                if !project.join(&rel).is_file() {
                    return Err(format!("{rel} is missing: {agent} is selected"));
                }
            }
        } else if present != 0 {
            return Err(format!(
                "{dir} has adapters for {agent}, which is not selected"
            ));
        }
    }
    Ok(())
}

// rq-6dd4dd02 rq-4a032ea7
fn check_lint(project: &Path, tophat: &Path) -> Result<(), String> {
    let help = output(tophat.as_os_str(), &["--help"], project)?;
    if !help.status.success() {
        return Err(format!("`tophat --help` failed: {}", text(&help.stderr)));
    }
    let usage = String::from_utf8_lossy(&help.stdout);
    let findings = lint::lint_skills(project, &usage).map_err(|e| e.to_string())?;
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join("; "))
    }
}

// rq-351008a7
fn check_pixi(project: &Path) -> Result<(), String> {
    let pixi = output(OsStr::new("pixi"), &["install", "--frozen"], project)?;
    if pixi.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`pixi install --frozen` failed: {}{}",
            text(&pixi.stdout),
            text(&pixi.stderr)
        ))
    }
}

/// The environment a launcher run gets: a home of its own and the workspace binary.
fn launcher_env(command: &mut Command, temp: &Path, tophat: &Path) {
    // The release workflow points TOPHAT_TEST_BINARY at the binary it built, so the same render
    // checks run against the release instead of the workspace build.
    let binary = std::env::var_os("TOPHAT_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| tophat.to_path_buf());
    command
        .env("HOME", temp.join("home"))
        .env("TOPHAT_DEV_BINARY", binary);
    for (key, _) in std::env::vars() {
        if key.starts_with("TOPHAT_ROOT_")
            || key.starts_with("GIT_")
            || key == "EMAIL"
            || key == "TOPHAT_GPU"
            || key == "XDG_CONFIG_HOME"
        {
            command.env_remove(key);
        }
    }
}

/// Runs `th.sh` with the launcher environment.
fn th(temp: &Path, project: &Path, tophat: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("sh");
    command.args(["./th.sh"]).args(args).current_dir(project);
    launcher_env(&mut command, temp, tophat);
    command
        .output()
        .map_err(|e| format!("cannot run th.sh: {e}"))
}

/// Runs `podman` with the launcher environment, so it uses the same rootless image storage as the
/// `th.sh` that built the image.
fn podman(temp: &Path, project: &Path, tophat: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("podman");
    command.args(args).current_dir(project);
    launcher_env(&mut command, temp, tophat);
    command
        .output()
        .map_err(|e| format!("cannot run podman: {e}"))
}

/// The image name of the project.
fn image_name(project: &Path) -> Result<String, String> {
    let id = read(project, ".tophat/project-id")?;
    Ok(format!("localhost/tophat-{}-project:latest", id.trim()))
}

// rq-351008a7 rq-5fa25ddc rq-099a04fb
fn check_image(
    project: &Path,
    temp: &Path,
    tophat: &Path,
    answers: &yaml_rust2::Yaml,
) -> Result<(), String> {
    let version = output(OsStr::new("podman"), &["--version"], project)?;
    if !version.status.success() {
        return Err(format!(
            "`podman --version` failed: {}",
            text(&version.stderr)
        ));
    }
    let build = th(temp, project, tophat, &["--check"])?;
    if !build.status.success() {
        return Err(format!(
            "the image build or its check failed: {}{}",
            text(&build.stdout),
            text(&build.stderr)
        ));
    }
    let image = image_name(project)?;
    let workspace = std::path::absolute(project)
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    let mount = format!("{workspace}:/work");
    let in_image = |script: &str| -> Result<String, String> {
        // The pixi-install check has already installed the environment in the project on the host,
        // and pixi environments cannot be relocated: an anonymous volume (removed with the
        // container) hides the host's `.pixi/` so pixi installs its own at `/work/.pixi`.
        let out = podman(
            temp,
            project,
            tophat,
            &[
                "run",
                "--rm",
                "-v",
                &mount,
                "-v",
                "/work/.pixi",
                "-w",
                "/work",
                &image,
                "sh",
                "-c",
                script,
            ],
        )?;
        if !out.status.success() {
            return Err(format!(
                "`{script}` in the image failed: {}{}",
                text(&out.stdout),
                text(&out.stderr)
            ));
        }
        Ok(text(&out.stdout))
    };
    if in_image("tophat --version")?.is_empty() {
        return Err("`tophat --version` in the image printed nothing".into());
    }
    in_image("pixi run snakemake --version")?;
    if strings(&answers["languages"]).iter().any(|l| l == "julia") {
        in_image("pixi run julia --version")?;
    }
    Ok(())
}

// rq-22ab7a81 rq-1cc9f489
fn check_launcher(project: &Path, temp: &Path, tophat: &Path) -> Result<(), String> {
    let first = th(temp, project, tophat, &["--dry-run"])?;
    if !first.status.success() {
        return Err(format!(
            "`th.sh --dry-run` failed: {}{}",
            text(&first.stdout),
            text(&first.stderr)
        ));
    }
    let id = read(project, ".tophat/project-id")?;
    let id = id.trim();
    let uuid = id.len() == 36
        && id.split('-').map(str::len).eq([8, 4, 4, 4, 12])
        && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if !uuid {
        return Err(format!(".tophat/project-id is not a UUID: {id:?}"));
    }
    let second = th(temp, project, tophat, &["--dry-run"])?;
    if !second.status.success() || read(project, ".tophat/project-id")?.trim() != id {
        return Err("a second dry run changed .tophat/project-id".into());
    }
    let session = text(&first.stdout)
        .lines()
        .find(|l| l.starts_with("podman run --rm -it"))
        .ok_or("the dry run printed no session command")?
        .to_string();
    if !session.contains(&format!("localhost/tophat-{id}-project:latest")) {
        return Err("the session command does not name the project image".into());
    }
    let workspace = std::path::absolute(project)
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    if !session.contains(&format!("{workspace}:/work")) {
        return Err("the session command does not bind the workspace at /work".into());
    }
    for forbidden in [
        ".ssh",
        ".gitconfig",
        ".git-credentials",
        ".netrc",
        "SSH_AUTH_SOCK",
    ] {
        if session.contains(forbidden) {
            return Err(format!("the session command contains {forbidden}"));
        }
    }
    Ok(())
}

// rq-5b3d8f35
fn check_launcher_unknown_option(project: &Path, temp: &Path, tophat: &Path) -> Result<(), String> {
    let out = th(temp, project, tophat, &["--frobnicate"])?;
    if out.status.success() {
        return Err("an unknown option was accepted".into());
    }
    if !text(&out.stderr).contains("Usage:") {
        return Err("an unknown option did not print the usage text".into());
    }
    Ok(())
}

// rq-7fb69dfd
fn check_gpu(project: &Path, temp: &Path) -> Result<(), String> {
    let cases = [
        ("nvidia", true, true, false),
        ("nvidia-no-cdi", true, false, false),
        ("amd", false, false, true),
        ("none", false, false, false),
    ];
    for (expected, nvidia, cdi, amd) in cases {
        let root = temp.join(format!("gpu-{expected}"));
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        if nvidia {
            write_file(&root, "dev/nvidiactl", "")?;
        }
        if cdi {
            write_file(&root, "etc/cdi/nvidia.yaml", "name: nvidia.com/gpu\n")?;
        }
        if amd {
            write_file(&root, "dev/kfd", "")?;
            write_file(&root, "dev/dri/renderD128", "")?;
        }
        let script = project.join(".tophat/managed/launch/gpu-detect.sh");
        let out = Command::new("sh")
            .arg(&script)
            .arg(&root)
            .current_dir(project)
            .output()
            .map_err(|e| format!("cannot run gpu-detect.sh: {e}"))?;
        let detected = text(&out.stdout);
        if !out.status.success() || detected != expected {
            return Err(format!(
                "gpu-detect.sh printed {detected:?} for a simulated {expected} host"
            ));
        }
    }
    Ok(())
}

/// Writes a file below `root`, creating its parent directory.
fn write_file(root: &Path, rel: &str, content: &str) -> Result<(), String> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, content).map_err(|e| e.to_string())
}

/// Runs one check and records a failure naming the answer set and the check.
fn run_check<F>(report: &mut Report, set: &str, check: &str, f: F) -> Result<bool, String>
where
    F: FnOnce() -> Result<(), String>,
{
    match f() {
        Ok(()) => Ok(true),
        Err(detail) => {
            report.failed = true;
            report.lines.push(format!("{set}: {check} FAILED"));
            report.lines.push(format!("  {detail}"));
            Ok(false)
        }
    }
}

// rq-5040a08a
/// Renders the selected answer sets and runs the render checks on each.
pub fn run(repo_root: &Path, options: &Options) -> Result<Report, String> {
    let all = answer_sets(repo_root)?;
    let selected: Vec<(String, PathBuf)> = if options.answers.is_empty() {
        all
    } else {
        let mut out = Vec::new();
        for name in &options.answers {
            let found = all
                .iter()
                .find(|(candidate, _)| candidate == name)
                .ok_or_else(|| format!("no answer set named '{name}' in xtask/answers"))?;
            out.push(found.clone());
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out.dedup_by(|a, b| a.0 == b.0);
        out
    };
    if selected.is_empty() {
        return Err("there are no answer sets in xtask/answers".into());
    }

    let tophat = tophat_binary(repo_root)?;
    let temp = Temp::new("run")?;
    let mut report = Report::default();
    let have_pixi = Command::new("pixi").arg("--version").output().is_ok();
    let have_podman = Command::new("podman").arg("--version").output().is_ok();

    for (set, path) in selected {
        let answers = answer_set(&path)?;
        let out = temp.0.join(&set);
        let project = render::render(
            repo_root,
            &Request {
                answers: path.to_string_lossy().into_owned(),
                out: out.clone(),
                sets: Vec::new(),
            },
        )
        .map_err(|e| e.to_string())?;

        let checks: Vec<(&str, bool)> = vec![
            (
                "skeleton",
                run_check(&mut report, &set, "skeleton", || check_skeleton(&project))?,
            ),
            (
                "managed-marker",
                run_check(&mut report, &set, "managed-marker", || {
                    check_marker(&project)
                })?,
            ),
            (
                "ignore-rules",
                run_check(&mut report, &set, "ignore-rules", || {
                    check_ignore_rules(&project)
                })?,
            ),
            (
                "project-md",
                run_check(&mut report, &set, "project-md", || {
                    check_project_md(&project)
                })?,
            ),
            (
                "roots",
                run_check(&mut report, &set, "roots", || {
                    check_roots(&project, &tophat)
                })?,
            ),
            (
                "environment",
                run_check(&mut report, &set, "environment", || {
                    check_environment(&project, &answers)
                })?,
            ),
            (
                "release-pin",
                run_check(&mut report, &set, "release-pin", || {
                    check_release_pin(&project)
                })?,
            ),
            (
                "initial-state",
                run_check(&mut report, &set, "initial-state", || {
                    check_initial_state(&project, &tophat)
                })?,
            ),
            (
                "skills",
                run_check(&mut report, &set, "skills", || {
                    check_skills(&project, &answers)
                })?,
            ),
            (
                "lint-skills",
                run_check(&mut report, &set, "lint-skills", || {
                    check_lint(&project, &tophat)
                })?,
            ),
            (
                "gpu-detection",
                run_check(&mut report, &set, "gpu-detection", || {
                    check_gpu(&project, &temp.0)
                })?,
            ),
        ];
        let mut ok = checks.iter().all(|(_, ok)| *ok);
        if ok {
            if have_pixi {
                ok &= run_check(&mut report, &set, "pixi-install", || check_pixi(&project))?;
            } else {
                report
                    .cannot_run
                    .push("pixi is not on PATH; the pixi-install check did not run".into());
            }
        }
        if ok && !options.no_image {
            if have_podman {
                ok &= run_check(&mut report, &set, "image", || {
                    check_image(&project, &temp.0, &tophat, &answers)
                })?;
                if ok {
                    ok &= run_check(&mut report, &set, "launcher", || {
                        check_launcher(&project, &temp.0, &tophat)
                            .and_then(|_| check_launcher_unknown_option(&project, &temp.0, &tophat))
                    })?;
                }
            } else {
                report.cannot_run.push(
                    "podman is not on PATH; the image and launcher checks did not run".into(),
                );
            }
        }
        if ok {
            report.lines.push(format!("{set} ok"));
        }
    }

    if !report.failed
        && let Some(message) = report.cannot_run.first()
    {
        return Err(message.clone());
    }
    Ok(report)
}
