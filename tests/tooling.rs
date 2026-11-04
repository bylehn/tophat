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
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo.path)
            .status()
            .unwrap()
            .success());
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
