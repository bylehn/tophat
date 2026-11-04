//! Tests for the conformance runner and the required conformance cases (rqm/testing.md).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use xtask::conformance;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn passing_project(case: &Path) {
    let project = case.join("project");
    fs::create_dir_all(project.join(".tophat")).unwrap();
    fs::write(project.join("PROJECT.md"), "# P\n\nA project.\n").unwrap();
    fs::write(
        project.join(".tophat/roots.yaml"),
        "roots:\n  results: {}\n",
    )
    .unwrap();
}

fn write_case(case: &Path, expect: &str) {
    fs::create_dir_all(case).unwrap();
    fs::write(
        case.join("case.yaml"),
        format!(
            "description: test\nsince: 0.1.0\ngit:\n  - commit: initial\ncommand: [check, --json]\nexpect:\n{expect}"
        ),
    )
    .unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "tophat-conformance-test-{}-{}-{}",
        label,
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_tophat")
}

fn run(cases: &Path, selected: &[&str]) -> conformance::Report {
    let selected: Vec<String> = selected.iter().map(|s| s.to_string()).collect();
    conformance::run_dir(cases, Path::new(binary()), &selected).unwrap()
}

#[test] // rq-4c6f3b16
fn a_valid_case_passes() {
    let cases = temp_dir("valid");
    let case = cases.join("valid");
    passing_project(&case);
    write_case(&case, "  exit: 0\n  diagnostics: []\n");
    let report = run(&cases, &[]);
    assert!(!report.failed);
    assert!(
        report.lines.contains(&"valid ok".to_string()),
        "{:?}",
        report.lines
    );
}

#[test] // rq-23b93182
fn a_missing_expected_diagnostic_fails_the_case() {
    let cases = temp_dir("missing");
    let case = cases.join("missing");
    passing_project(&case);
    write_case(
        &case,
        "  exit: 0\n  diagnostics:\n    - {rule: data.file-changed, severity: error, path: datasets/x/v1.yaml}\n",
    );
    let report = run(&cases, &[]);
    assert!(report.failed);
    let text = report.lines.join("\n");
    assert!(text.contains("missing FAILED"), "{text}");
    assert!(text.contains("missing: data.file-changed"), "{text}");
}

#[test] // rq-a819acba
fn an_unexpected_diagnostic_fails_the_case() {
    let cases = temp_dir("unexpected");
    let case = cases.join("leak");
    copy_tree(
        &repo_root().join("tests/conformance/visibility-leak"),
        &case,
    );
    write_case(&case, "  exit: 1\n  diagnostics: []\n");
    let report = run(&cases, &[]);
    assert!(report.failed);
    let text = report.lines.join("\n");
    assert!(text.contains("leak FAILED"), "{text}");
    assert!(text.contains("unexpected: data.visibility-leak"), "{text}");
}

#[test] // rq-58513972
fn messages_are_not_compared() {
    let cases = repo_root().join("tests/conformance");
    let report = run(&cases, &["dataset-file-changed"]);
    assert!(!report.failed, "{:?}", report.lines);
    assert!(
        report
            .lines
            .contains(&"dataset-file-changed ok".to_string())
    );
}

#[test] // rq-067bd77a
fn replayed_commits_are_reproducible() {
    let case = repo_root().join("tests/conformance/fresh-project");
    let first = conformance::replay_head(&case).unwrap();
    let second = conformance::replay_head(&case).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), 40);
}

#[test] // rq-5958144d
fn cases_newer_than_the_binary_are_skipped() {
    let cases = temp_dir("newer");
    let case = cases.join("newer");
    passing_project(&case);
    fs::write(
        case.join("case.yaml"),
        "description: newer\nsince: 9.0.0\ngit:\n  - commit: initial\ncommand: [check, --json]\nexpect:\n  exit: 0\n  diagnostics: []\n",
    )
    .unwrap();
    let report = run(&cases, &[]);
    assert!(!report.failed);
    assert!(report.lines.contains(&"newer skipped".to_string()));
}

#[test] // rq-513a430f
fn commands_other_than_check_are_compared_by_exit_code_and_output() {
    let cases = repo_root().join("tests/conformance");
    let report = run(&cases, &["external-key-mismatch"]);
    assert!(!report.failed, "{:?}", report.lines);
    assert!(
        report
            .lines
            .contains(&"external-key-mismatch ok".to_string())
    );
}

#[test] // rq-97bc377b
fn the_required_cases_are_present() {
    let cases = repo_root().join("tests/conformance");
    let expected: BTreeSet<&str> = [
        "fresh-project",
        "reported-fanout-qc",
        "published-dataset-consumer",
        "retrospective-declared-dataset",
        "external-ingested",
        "frozen-section-edited",
        "scenario-untested",
        "summary-dirty",
        "dataset-file-changed",
        "missing-qc-verdict",
        "unknown-question",
        "unpublished-stage-input",
        "dataset-retired",
        "producer-unreported",
        "visibility-leak",
        "external-key-mismatch",
        "external-output-mismatch",
        "external-script-modified",
    ]
    .into_iter()
    .collect();
    let actual: BTreeSet<String> = fs::read_dir(&cases)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        actual,
        expected.iter().map(|s| s.to_string()).collect(),
        "unexpected or missing cases"
    );
    for name in &expected {
        assert!(cases.join(name).join("case.yaml").is_file(), "{name}");
        assert!(cases.join(name).join("project").is_dir(), "{name}");
    }
}
