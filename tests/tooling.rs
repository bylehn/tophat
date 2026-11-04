//! Tests for the template tooling (rqm/tooling.md).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use xtask::render::{self, Request};

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

struct Temp {
    path: PathBuf,
}

impl Temp {
    fn new(label: &str) -> Temp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tophat-tooling-{}-{}-{}",
            label,
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Temp { path }
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A minimal template repository with an answer set named minimal-claude.
fn mini_template() -> Temp {
    let repo = Temp::new("template");
    repo.write(
        "copier.yml",
        "_subdirectory: template\nproject_name:\n  type: str\n  default: Example Project\n",
    );
    repo.write("template/README.md.jinja", "# {{ project_name }}\n");
    repo.write(
        "template/{{_copier_conf.answers_file}}.jinja",
        "{{ _copier_answers|to_nice_yaml }}\n",
    );
    repo.write(
        "xtask/answers/minimal-claude.yaml",
        "project_name: Example Project\n",
    );
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.org",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo.path)
                .status()
                .unwrap()
                .success()
        );
    }
    repo
}

fn render_with(
    repo: &Path,
    answers: &str,
    out: &Path,
    sets: &[(&str, &str)],
) -> Result<PathBuf, String> {
    render::render(
        repo,
        &Request {
            answers: answers.to_string(),
            out: out.to_path_buf(),
            sets: sets
                .iter()
                .map(|(question, value)| (question.to_string(), value.to_string()))
                .collect(),
        },
    )
    .map_err(|e| e.to_string())
}

#[test] // rq-71619977
fn render_from_the_working_tree() {
    if !common::copier() {
        return;
    }
    let repo = mini_template();
    repo.write("template/README.md.jinja", "changed {{ project_name }}\n");
    let out = Temp::new("out");
    let rendered = render_with(&repo.path, "minimal-claude", &out.join("p"), &[]).unwrap();
    assert_eq!(
        fs::read_to_string(rendered.join("README.md")).unwrap(),
        "changed Example Project\n"
    );
    let answers = fs::read_to_string(rendered.join(".copier-answers.yml")).unwrap();
    assert!(answers.contains("Example Project"), "{answers}");
}

#[test] // rq-b0f89137
fn render_refuses_a_non_empty_output_directory() {
    if !common::copier() {
        return;
    }
    let repo = mini_template();
    let out = Temp::new("out");
    let file = out.join("p/keep.txt");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "unchanged\n").unwrap();
    let err = render_with(&repo.path, "minimal-claude", &out.join("p"), &[]).unwrap_err();
    assert!(err.contains("not empty"), "{err}");
    assert_eq!(fs::read_to_string(&file).unwrap(), "unchanged\n");
}

#[test] // rq-eeadb8c2
fn answers_can_be_overridden() {
    if !common::copier() {
        return;
    }
    let out = Temp::new("out");
    let rendered = render_with(
        &repo_root(),
        "minimal-claude",
        &out.join("p"),
        &[("languages", "[julia]")],
    )
    .unwrap();
    let pixi = fs::read_to_string(rendered.join("pixi.toml")).unwrap();
    assert!(pixi.contains("julia"), "{pixi}");
}

#[test] // rq-9b4b5f1e
fn a_missing_external_program_is_named() {
    let out = Temp::new("out");
    let err = render::render_with_path(
        &repo_root(),
        &Request {
            answers: "minimal-claude".to_string(),
            out: out.join("p"),
            sets: Vec::new(),
        },
        Some(OsStr::new("")),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("copier"), "{err}");
}

#[cfg(unix)]
#[test] // rq-cb49acfd
fn lock_check_detects_a_stale_pre_solved_lock() {
    use std::os::unix::fs::PermissionsExt;

    if !common::copier() {
        return;
    }
    let bin = Temp::new("pixi-bin");
    let pixi = bin.join("pixi");
    fs::write(
        &pixi,
        "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'pixi 0.82.1' ;;\n  lock)\n    if [ \"$2\" = --check ] && grep -q julia pixi.toml; then exit 1; fi ;;\nesac\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&pixi, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(std::iter::once(bin.path.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let root = repo_root();
    let before = (
        fs::read(root.join("template/.locks/python.lock")).unwrap(),
        fs::read(root.join("template/.locks/python-julia.lock")).unwrap(),
    );
    let outcomes = xtask::lock::lock_with_path(&root, true, Some(path.as_os_str())).unwrap();
    let status = |name: &str| {
        outcomes
            .iter()
            .find(|o| o.selection == name)
            .unwrap()
            .status
    };
    assert_eq!(status("none"), xtask::lock::Status::Ok);
    assert_eq!(status("julia"), xtask::lock::Status::Stale);
    assert!(xtask::lock::any_stale(&outcomes));
    assert_eq!(
        before,
        (
            fs::read(root.join("template/.locks/python.lock")).unwrap(),
            fs::read(root.join("template/.locks/python-julia.lock")).unwrap(),
        )
    );
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of the repository's template and answer sets, so a test can break the template.
fn template_repo() -> Temp {
    let repo = Temp::new("template-repo");
    let root = repo_root();
    fs::copy(root.join("copier.yml"), repo.join("copier.yml")).unwrap();
    copy_dir(&root.join("template"), &repo.join("template"));
    copy_dir(&root.join("xtask/answers"), &repo.join("xtask/answers"));
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.org",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo.path)
                .status()
                .unwrap()
                .success()
        );
    }
    repo
}

#[test] // rq-2e07e31f
fn skill_lint_reports_findings_with_their_rule() {
    if !common::copier() {
        return;
    }
    let out = Temp::new("lint");
    let project = render_with(&repo_root(), "minimal-claude", &out.join("p"), &[]).unwrap();
    fs::remove_file(project.join(".claude/skills/th-study/SKILL.md")).unwrap();
    let out = Command::new("cargo")
        .args(["run", "--quiet", "--package", "xtask", "--", "lint-skills"])
        .arg(&project)
        .args(["--tophat", env!("CARGO_BIN_EXE_tophat")])
        .current_dir(repo_root())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("skill.adapter-missing") && line.contains("th-study")),
        "{stdout}"
    );
}

#[test] // rq-5040a08a
fn render_test_names_an_unknown_answer_set() {
    let err = xtask::render_test::run(
        &repo_root(),
        &xtask::render_test::Options {
            answers: vec!["no-such-set".to_string()],
            no_image: true,
        },
    )
    .unwrap_err();
    assert!(err.contains("no-such-set"), "{err}");
}

#[test] // rq-cb8f92b1
fn a_failing_render_check_names_the_answer_set_and_step() {
    if !common::copier() {
        return;
    }
    let repo = template_repo();
    let ignore = repo.join("template/.tophat/.gitignore");
    let text = fs::read_to_string(&ignore)
        .unwrap()
        .lines()
        .filter(|line| *line != "/cache/")
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&ignore, format!("{text}\n")).unwrap();
    let report = xtask::render_test::run(
        &repo.path,
        &xtask::render_test::Options {
            answers: vec!["minimal-claude".to_string()],
            no_image: true,
        },
    )
    .unwrap();
    assert!(report.failed, "{:?}", report.lines);
    let all = report.lines.join("\n");
    assert!(all.contains("minimal-claude"), "{all}");
    assert!(all.contains("ignore-rules"), "{all}");
}

#[test] // rq-5fa25ddc
fn a_julia_project_has_julia_in_its_image() {
    if !common::copier() {
        return;
    }
    let podman = Command::new("podman")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    let pixi = Command::new("pixi")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !podman || !pixi {
        eprintln!("skipped: install Podman and pixi to run the image render test");
        return;
    }
    let report = xtask::render_test::run(
        &repo_root(),
        &xtask::render_test::Options {
            answers: vec!["julia-all-agents".to_string()],
            no_image: false,
        },
    )
    .unwrap();
    assert!(!report.failed, "{:?}", report.lines);
}
