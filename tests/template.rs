//! Tests for the generated project skeleton (rqm/template/project.md): the template is rendered
//! with the real Copier from this repository's working tree.
//!
//! These tests need `copier` on `PATH`; without it they print a skip notice and pass, unless
//! `TOPHAT_REQUIRE_COPIER=1` (see tests/common/mod.rs).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

use tophat::pipeline::{compute_keys, load_pipeline, KeyContext};
use yaml_rust2::{Yaml, YamlLoader};

/// The Background answers of the scenarios.
const BACKGROUND: [(&str, &str); 4] = [
    ("project_name", "Method X"),
    (
        "project_description",
        "Does method X improve affinity prediction?",
    ),
    ("author_name", "A. Scientist"),
    ("author_email", "a@example.org"),
];

/// The project files every render has: the skeleton of rqm/template/project.md and the launcher
/// files of rqm/template/container.md.
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

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch directory, removed on drop.
struct Scratch {
    base: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        static N: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-template-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        Scratch { base }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.base.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Runs `copier copy` from `src` (at `vcs_ref`) into `dst` with the Background answers, overridden
/// and extended by `answers`.
fn copier_copy(src: &Path, vcs_ref: &str, dst: &Path, answers: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("copier");
    cmd.args(["copy", "--defaults", "--quiet", "--vcs-ref", vcs_ref]);
    for (k, v) in BACKGROUND
        .iter()
        .filter(|(k, _)| !answers.iter().any(|(a, _)| a == k))
        .chain(answers)
    {
        cmd.arg("-d").arg(format!("{k}={v}"));
    }
    cmd.arg(src).arg(dst).output().unwrap()
}

/// Renders the working tree's template into `dst`, which must succeed.
fn render(dst: &Path, answers: &[(&str, &str)]) {
    let out = copier_copy(&repo(), "HEAD", dst, answers);
    assert!(
        out.status.success(),
        "copier copy failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Every file below `dir`, as sorted `/`-separated relative paths (skipping `.git/`).
fn files(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                if rel != ".git" {
                    walk(&entry.path(), &rel, out);
                }
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

fn git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn tophat(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(args)
        .current_dir(dir)
        .env_remove("TOPHAT_ROOT_RESULTS")
        .output()
        .unwrap()
}

fn yaml(text: &str) -> Yaml {
    YamlLoader::load_from_str(text)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

/// The conda package URLs of the default environment in a pixi.lock, per platform.
fn lock_packages(lock: &str, platform: &str) -> Vec<String> {
    let doc = yaml(lock);
    doc["environments"]["default"]["packages"][platform]
        .as_vec()
        .unwrap_or_else(|| panic!("no packages for {platform}"))
        .iter()
        .filter_map(|p| p["conda"].as_str().map(str::to_string))
        .collect()
}

fn has_package(urls: &[String], name: &str) -> bool {
    urls.iter().any(|u| {
        u.rsplit('/')
            .next()
            .and_then(|file| file.strip_prefix(&format!("{name}-")))
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
    })
}

/// A directory holding links to the given programs (found on the current PATH), for a PATH that
/// offers exactly them.
fn path_with(scratch: &Scratch, name: &str, programs: &[&str]) -> PathBuf {
    let dir = scratch.path(name);
    fs::create_dir_all(&dir).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    for program in programs {
        let found = std::env::split_paths(&path)
            .map(|d| d.join(program))
            .find(|p| p.is_file())
            .unwrap_or_else(|| panic!("{program} is not on PATH"));
        #[cfg(unix)]
        std::os::unix::fs::symlink(found, dir.join(program)).unwrap();
    }
    dir
}

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A rendered project in a git repository whose hooks are the template's.
fn hooked_project(scratch: &Scratch) -> PathBuf {
    let project = scratch.path("p");
    render(&project, &[]);
    git_ok(&project, &["init", "-q"]);
    git_ok(
        &project,
        &["config", "core.hooksPath", ".tophat/managed/hooks"],
    );
    write(
        &project,
        "PROJECT.md",
        "# Method X\n\n## q-1a2b3c4d · Does X work?\n\nNo status line here.\n",
    );
    project
}

/// Commits everything in `project` with `PATH` set to `path`; returns the commit's output.
fn commit_with_path(project: &Path, path: &Path) -> Output {
    git_ok(project, &["add", "-A"]);
    Command::new(path.join("git"))
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "x",
        ])
        .env("PATH", path)
        .current_dir(project)
        .output()
        .unwrap()
}

/// Runs the CI workflow's "Install the pinned tophat" step in `project`, with `sh`.
fn run_install_step(project: &Path, scratch: &Scratch) -> (Output, PathBuf) {
    let workflow = yaml(&read(project, ".github/workflows/tophat-check.yaml"));
    let steps = workflow["jobs"]["check"]["steps"].as_vec().unwrap();
    let step = steps
        .iter()
        .find(|s| s["name"].as_str() == Some("Install the pinned tophat"))
        .unwrap();
    let runner_temp = scratch.path("runner");
    fs::create_dir_all(&runner_temp).unwrap();
    let github_path = runner_temp.join("github_path");
    let out = Command::new("sh")
        .arg("-c")
        .arg(step["run"].as_str().unwrap())
        .env("RUNNER_TEMP", &runner_temp)
        .env("GITHUB_PATH", &github_path)
        .current_dir(project)
        .output()
        .unwrap();
    (out, runner_temp)
}

// --- Questions ----------------------------------------------------------------------------------

#[test] // rq-911c9187 rq-99c9f467
fn the_slug_is_derived_from_the_project_name() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let answers = yaml(&read(&p, ".copier-answers.yml"));
    assert_eq!(answers["project_slug"].as_str(), Some("method-x"));
    assert!(read(&p, "pixi.toml").contains("name = \"method-x\""));
}

#[test] // rq-195607b5 rq-99c9f467
fn a_malformed_slug_is_rejected() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    let out = copier_copy(&repo(), "HEAD", &p, &[("project_slug", "Method_X")]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("slug"));
    assert!(!p.exists() || files(&p).is_empty());
}

#[test] // rq-db2ca072 rq-99c9f467
fn at_least_one_agent_is_required() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    let out = copier_copy(&repo(), "HEAD", &p, &[("agents", "[]")]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("at least one agent"));
}

// --- Layout and ownership -----------------------------------------------------------------------

#[test] // rq-d0e11fee rq-fc2247e5
fn a_new_project_has_the_skeleton_files_and_no_studies_or_datasets() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let rendered = files(&p);
    for f in SKELETON {
        assert!(rendered.iter().any(|r| r == f), "{f} is missing");
    }
    assert!(!p.join("studies").exists() && !p.join("datasets").exists());
}

#[test] // rq-74365fac rq-cafa162c
fn managed_files_carry_the_marker() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let exceptions: Vec<String> = read(&p, ".tophat/managed/ownership-exceptions")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let managed: Vec<String> = files(&p)
        .into_iter()
        .filter(|f| {
            f.starts_with(".tophat/managed/")
                || exceptions.iter().any(|e| common::path_matches(e, f))
        })
        // The worked example is copied into studies/ and then belongs to the project.
        .filter(|f| !f.starts_with(".tophat/managed/examples/"))
        .collect();
    assert!(managed.len() >= 6, "{managed:?}");
    for f in managed.iter().filter(|f| !f.ends_with(".json")) {
        let text = read(&p, f);
        let mut lines: Vec<&str> = text.lines().collect();
        // A file that begins with front matter carries the marker after it.
        if lines.first() == Some(&"---") {
            if let Some(end) = lines.iter().skip(1).position(|l| *l == "---") {
                lines.drain(..=end + 1);
            }
        }
        let head: Vec<String> = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .take(5)
            .map(|l| l.to_string())
            .collect();
        assert!(
            head.iter().any(|l| l.contains("TopHat-managed")),
            "{f} lacks the marker"
        );
    }
}

#[test] // rq-cae7fbe8 rq-cafa162c
fn every_project_owned_file_is_skipped_on_update() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let config = yaml(&fs::read_to_string(repo().join("copier.yml")).unwrap());
    let skipped: Vec<&str> = config["_skip_if_exists"]
        .as_vec()
        .unwrap()
        .iter()
        .filter_map(Yaml::as_str)
        .collect();
    let exceptions = read(&p, ".tophat/managed/ownership-exceptions");
    for f in files(&p) {
        let managed = f.starts_with(".tophat/managed/")
            || exceptions.lines().any(|l| common::path_matches(l, &f));
        assert!(
            managed || skipped.iter().any(|s| common::path_matches(s, &f)),
            "{f} is project-owned but not in _skip_if_exists"
        );
    }
}

#[test] // rq-4da71d7f rq-0da9fa10
fn machine_local_state_is_ignored() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    git_ok(&p, &["init", "-q"]);
    let local = [
        ".tophat/roots.local.yaml",
        ".tophat/slurm.local.yaml",
        ".tophat/cache/hashes.json",
        ".tophat/tools/x/y",
        ".tophat/generated/s.smk",
        ".tophat/state/apptainer/x.sif",
        ".snakemake/log",
        ".pixi/envs/x",
    ];
    for f in local {
        write(&p, f, "x\n");
    }
    let status = git_ok(&p, &["status", "--porcelain", "--untracked-files=all"]);
    for f in local {
        assert!(!status.contains(f), "{f} is not ignored:\n{status}");
    }
    assert!(status.contains("PROJECT.md"), "{status}");
}

// --- Files --------------------------------------------------------------------------------------

#[test] // rq-5c6c0574 rq-e660df68
fn project_md_starts_with_an_empty_question_tree() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let text = read(&p, "PROJECT.md");
    assert!(text.starts_with("# Method X\n"), "{text}");
    assert!(text.contains("Does method X improve affinity prediction?"));
    let out = tophat(&p, &["question", "tree", "--json"]);
    assert!(out.status.success());
    let tree: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("\"q-"),
        "{tree}"
    );
}

#[test] // rq-d1fe5c84 rq-cba13e20
fn the_results_root_is_declared_and_defaults_to_the_project() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let out = tophat(&p, &["data", "roots"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!(
            "results\t{}/results\n",
            std::path::absolute(&p).unwrap().display()
        )
    );
    assert!(!p.join(".tophat/roots.local.yaml").exists());
}

#[test] // rq-e9156963 rq-260d84c9
fn python_and_snakemake_are_always_installed() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let toml = read(&p, "pixi.toml");
    assert!(toml.contains("\npython = ") && toml.contains("\nsnakemake-minimal = "));
    assert!(!toml.contains("julia"));
    assert!(toml.contains("channels = [\"conda-forge\", \"bioconda\"]"));
    let lock = read(&p, "pixi.lock");
    for platform in ["linux-64", "linux-aarch64"] {
        let urls = lock_packages(&lock, platform);
        assert!(has_package(&urls, "python"), "{platform}");
        assert!(has_package(&urls, "snakemake-minimal"), "{platform}");
        assert!(!has_package(&urls, "julia"), "{platform}");
    }
}

#[test] // rq-87749f65 rq-260d84c9
fn julia_is_added_when_selected() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[("languages", "[\"julia\"]")]);
    assert!(read(&p, "pixi.toml").contains("\njulia = "));
    let lock = read(&p, "pixi.lock");
    for platform in ["linux-64", "linux-aarch64"] {
        assert!(
            has_package(&lock_packages(&lock, platform), "julia"),
            "{platform}"
        );
    }
}

#[test] // rq-14a1f043 rq-260d84c9
fn the_lock_is_the_templates_pre_solved_lock() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let (a, b) = (scratch.path("a"), scratch.path("b"));
    render(&a, &[]);
    render(&b, &[("project_name", "Another Project")]);
    assert_eq!(read(&a, "pixi.lock"), read(&b, "pixi.lock"));
    assert_eq!(
        read(&a, "pixi.lock"),
        fs::read_to_string(repo().join("template/.locks/python.lock")).unwrap()
    );
    assert_eq!(
        KeyContext::new(&a).environment_hash("default").unwrap(),
        KeyContext::new(&b).environment_hash("default").unwrap()
    );
}

#[test] // rq-06b39dff rq-485c120f
fn the_guide_starts_with_exploring_and_explains_its_terms() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let guide = read(&p, ".tophat/managed/GUIDE.md");
    let explore = guide
        .find("## 2. Explore first")
        .expect("an explore section");
    let plan = guide.find("## 3. The plan").expect("a plan section");
    assert!(explore < plan);
    assert!(guide.contains("explore, plan, lock, run, report"));
    let glossary = &guide[guide.find("## Glossary").expect("a glossary")..];
    for term in [
        "Pre-register",
        "Primary endpoint",
        "Decision rule",
        "Scenario",
    ] {
        assert!(glossary.contains(&format!("- **{term}**")), "{term}");
    }
}

#[test] // rq-99f3ccdf rq-56d220b5 rq-72b63b68
fn an_unreleased_pin_says_so() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let pin = yaml(&read(&p, ".tophat/managed/tophat-release.yaml"));
    assert_eq!(pin["version"].as_str(), Some("unreleased"));
    assert!(pin["url"].is_badvalue() && pin["sha256"].is_badvalue());
    let (out, _) = run_install_step(&p, &scratch);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("No tophat release is pinned"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// --- Hook and CI --------------------------------------------------------------------------------

#[test] // rq-05e3b95c rq-a35593b1
fn the_hook_reports_check_errors_without_blocking() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let project = hooked_project(&scratch);
    let bin = path_with(&scratch, "bin", &["git", "sh"]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tophat"), bin.join("tophat")).unwrap();
    let out = commit_with_path(&project, &bin);
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    assert!(log.contains("question.status-missing"), "{log}");
    assert!(log.contains("CI will reject this commit"), "{log}");
}

#[test] // rq-47e127bc rq-a35593b1
fn the_hook_uses_the_container_on_the_host() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let project = hooked_project(&scratch);
    let bin = path_with(&scratch, "bin", &["git", "sh"]);
    executable(&bin.join("podman"), "#!/bin/sh\nexit 0\n");
    // A stand-in for the launcher: it runs the check as the container would.
    executable(
        &project.join("th.sh"),
        &format!(
            "#!/bin/sh\necho \"th.sh $*\"\nexec '{}' check\n",
            env!("CARGO_BIN_EXE_tophat")
        ),
    );
    let out = commit_with_path(&project, &bin);
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    assert!(log.contains("th.sh --check"), "{log}");
    assert!(log.contains("question.status-missing"), "{log}");
}

#[test] // rq-c0d8a5d1 rq-a35593b1
fn the_hook_says_when_the_check_could_not_run() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let project = hooked_project(&scratch);
    let bin = path_with(&scratch, "bin", &["git", "sh"]);
    executable(&bin.join("podman"), "#!/bin/sh\nexit 0\n");
    // A stand-in for the launcher refusing to start, as it does without a development binary.
    executable(
        &project.join("th.sh"),
        "#!/bin/sh\necho 'th.sh: no tophat release is pinned' >&2\nexit 2\n",
    );
    let out = commit_with_path(&project, &bin);
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    assert!(log.contains("no tophat release is pinned"), "{log}");
    assert!(log.contains("could not run (exit status 2)"), "{log}");
    assert!(!log.contains("CI will reject"), "{log}");
}

#[test] // rq-26afc755 rq-a35593b1
fn the_hook_is_skipped_without_tophat_or_a_container_runtime() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let project = hooked_project(&scratch);
    let bin = path_with(&scratch, "bin", &["git", "sh"]);
    let out = commit_with_path(&project, &bin);
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    assert!(log.contains("check skipped"), "{log}");
}

#[test] // rq-d4c74a32 rq-72b63b68
fn ci_verifies_the_binary_before_running_the_check() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let workflow = yaml(&read(&p, ".github/workflows/tophat-check.yaml"));
    assert!(!workflow["on"]["push"].is_badvalue() && !workflow["on"]["pull_request"].is_badvalue());
    let names: Vec<&str> = workflow["jobs"]["check"]["steps"]
        .as_vec()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert_eq!(names, ["Install the pinned tophat", "tophat check"]);

    // A release served from a local directory.
    let release = scratch.path("release");
    fs::create_dir_all(&release).unwrap();
    let binary = release.join("tophat-x86_64-unknown-linux-musl");
    fs::copy(env!("CARGO_BIN_EXE_tophat"), &binary).unwrap();
    let (_, sha256) = tophat::data::hash_file(&binary).unwrap();
    let pin = |hash: &str| {
        format!(
            "# TopHat-managed\nversion: 0.1.0\nurl: file://{}\nsha256:\n  x86_64-unknown-linux-musl: {hash}\n  aarch64-unknown-linux-musl: {hash}\n",
            release.display()
        )
    };
    write(&p, ".tophat/managed/tophat-release.yaml", &pin(&sha256));
    let (out, runner) = run_install_step(&p, &scratch);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let installed = runner.join("tophat-bin/tophat");
    assert!(installed.is_file());
    assert!(fs::read_to_string(runner.join("github_path"))
        .unwrap()
        .contains("tophat-bin"));

    // A wrong hash stops the job before the check.
    write(
        &p,
        ".tophat/managed/tophat-release.yaml",
        &pin(&"0".repeat(64)),
    );
    let (out, _) = run_install_step(&p, &scratch);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("SHA-256"));
}

// --- Initial state ------------------------------------------------------------------------------

#[test] // rq-1ec0c367 rq-4a8d3a7d
fn a_new_project_passes_the_check() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let out = tophat(&p, &["check", "--strict", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test] // rq-6fd1f551 rq-4a8d3a7d
fn a_new_project_has_nothing_to_do_but_ask_the_question() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    let out = tophat(&p, &["status", "--json"]);
    assert!(out.status.success());
    let status: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        status["project"]["questions"],
        serde_json::json!({"open": 0, "answered": 0, "dropped": 0})
    );
    assert_eq!(status["studies"], serde_json::json!([]));
    let text = String::from_utf8_lossy(&tophat(&p, &["status"]).stdout).into_owned();
    assert!(text.contains("0 open, 0 answered, 0 dropped"), "{text}");
}

#[test] // rq-83643799 rq-4a8d3a7d
fn generation_runs_no_commands() {
    if !common::copier() {
        return;
    }
    let config = yaml(&fs::read_to_string(repo().join("copier.yml")).unwrap());
    assert!(config["_tasks"].is_badvalue());
    // Every render in these tests runs without --trust.
    let scratch = Scratch::new();
    render(&scratch.path("p"), &[]);
}

#[test] // rq-f57dc6fa rq-4a8d3a7d
fn a_study_can_be_added_to_a_new_project() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("p");
    render(&p, &[]);
    git_ok(&p, &["init", "-q"]);
    let project_md = read(&p, "PROJECT.md");
    write(
        &p,
        "PROJECT.md",
        &format!("{project_md}\n## Does X improve affinity prediction?\nstatus: open\n"),
    );
    let out = tophat(&p, &["question", "stamp"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stamped = String::from_utf8_lossy(&out.stdout).into_owned();
    let id = stamped.split_whitespace().next().unwrap().to_string();
    assert!(id.starts_with("q-"), "{stamped}");
    let out = tophat(&p, &["study", "new", "x-vs-docking", "--question", &id]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = tophat(&p, &["check"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// --- Updates and adoption -----------------------------------------------------------------------

/// Copies the template sources (copier.yml and template/) into `dst` as a new git repository.
fn template_repo(dst: &Path) {
    fn copy(from: &Path, to: &Path) {
        if from.is_dir() {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                copy(&entry.path(), &to.join(entry.file_name()));
            }
        } else {
            fs::copy(from, to).unwrap();
        }
    }
    fs::create_dir_all(dst).unwrap();
    fs::copy(repo().join("copier.yml"), dst.join("copier.yml")).unwrap();
    copy(&repo().join("template"), &dst.join("template"));
    git_ok(dst, &["init", "-q"]);
    git_ok(dst, &["add", "-A"]);
    git_ok(dst, &["commit", "-qm", "v1"]);
    git_ok(dst, &["tag", "v1"]);
}

#[test] // rq-e7220b72 rq-cafa162c
fn updating_keeps_project_owned_files_and_stage_keys() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let tpl = scratch.path("tpl");
    template_repo(&tpl);
    let p = scratch.path("p");
    let out = copier_copy(&tpl, "v1", &p, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The project's own changes: README, a question, and a study with a stage.
    git_ok(&p, &["init", "-q"]);
    write(&p, "README.md", "# Our own README\n");
    let project_md = read(&p, "PROJECT.md");
    write(
        &p,
        "PROJECT.md",
        &format!("{project_md}\n## Does X work?\nstatus: open\n"),
    );
    let stamped = String::from_utf8_lossy(&tophat(&p, &["question", "stamp"]).stdout).into_owned();
    let id = stamped.split_whitespace().next().unwrap().to_string();
    assert!(tophat(&p, &["study", "new", "x-study", "--question", &id])
        .status
        .success());
    write(
        &p,
        "studies/x-study/config/base.yaml",
        "stages:\n  - name: make\n    run: echo x > {out}/a.txt\n    result: a.txt\n",
    );
    git_ok(&p, &["add", "-A"]);
    git_ok(&p, &["commit", "-qm", "project"]);
    let keys = |p: &Path| {
        let config = load_pipeline(p, "x-study", "base").unwrap();
        compute_keys(&config, &KeyContext::new(p)).unwrap().keys
    };
    let before = keys(&p);
    let owned = ["README.md", "PROJECT.md", "pixi.toml", "pixi.lock"];
    let kept: Vec<String> = owned.iter().map(|f| read(&p, f)).collect();

    // A newer template version that changes those files and the managed release pin.
    for (f, add) in [
        ("template/README.md.jinja", "\nA newer README.\n"),
        ("template/PROJECT.md.jinja", "\nA newer comment.\n"),
        ("template/pixi.toml.jinja", "\n# newer\n"),
        ("template/.locks/python.lock", "# newer\n"),
        (
            "template/.tophat/managed/tophat-release.yaml",
            "# template v2\n",
        ),
    ] {
        let text = fs::read_to_string(tpl.join(f)).unwrap();
        fs::write(tpl.join(f), format!("{text}{add}")).unwrap();
    }
    git_ok(&tpl, &["commit", "-qam", "v2"]);
    git_ok(&tpl, &["tag", "v2"]);

    let out = Command::new("copier")
        .args([
            "update",
            "--defaults",
            "--quiet",
            "--vcs-ref",
            "v2",
            "--conflict",
            "inline",
        ])
        .current_dir(&p)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for (f, text) in owned.iter().zip(&kept) {
        assert_eq!(&read(&p, f), text, "{f} changed");
    }
    assert_eq!(keys(&p), before);
    assert!(read(&p, ".tophat/managed/tophat-release.yaml").contains("# template v2"));
}

#[test] // rq-46aa2f57 rq-cafa162c
fn copying_into_an_existing_repository_overwrites_no_user_file() {
    if !common::copier() {
        return;
    }
    let scratch = Scratch::new();
    let p = scratch.path("existing");
    let own = [
        ("README.md", "# An existing project\n"),
        (".gitignore", "*.o\n"),
        ("pixi.toml", "[workspace]\nname = \"existing\"\n"),
    ];
    for (f, text) in own {
        write(&p, f, text);
    }
    render(&p, &[]);
    for (f, text) in own {
        assert_eq!(read(&p, f), text, "{f} was overwritten");
    }
    assert!(p.join(".tophat/.gitignore").is_file());
    assert!(p.join(".tophat/roots.yaml").is_file());
}
