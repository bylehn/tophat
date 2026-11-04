//! Tests for the adoption fixtures (rqm/testing.md).

use std::path::{Path, PathBuf};

use xtask::adoption;

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(fixture: &str) -> adoption::Report {
    adoption::run(
        &repo_root(),
        Path::new(env!("CARGO_BIN_EXE_tophat")),
        &[fixture.to_string()],
    )
    .unwrap()
}

#[test] // rq-f54f3fe1
fn adoption_overwrites_no_user_file() {
    if !common::copier() {
        return;
    }
    let report = run("snakemake-pipeline");
    let text = report.lines.join("\n");
    assert!(!report.failed, "{text}");
    assert!(
        text.contains("snakemake-pipeline skipped .gitignore"),
        "{text}"
    );
    assert!(text.contains("snakemake-pipeline ok"), "{text}");
}

#[test] // rq-fb67f0df
fn adopted_content_is_labelled_legacy() {
    if !common::copier() {
        return;
    }
    let report = run("make-pipeline");
    let text = report.lines.join("\n");
    assert!(!report.failed, "{text}");
    assert!(
        text.contains("make-pipeline legacy data.declared-input"),
        "{text}"
    );
    assert!(
        text.contains("make-pipeline legacy study.result-unverified"),
        "{text}"
    );
    assert!(
        text.contains("make-pipeline lineage [declared, not verified]"),
        "{text}"
    );
}
