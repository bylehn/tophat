//! Tests for `tophat check` (rqm/check.md).
//!
//! The real rule families are specified by other features, so the scenarios that need diagnostics
//! use test rule families that react to marker text in fixture `STUDY.md` files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use tophat::check::{Diagnostic, Project, RuleFamily, Severity};
use tophat::cli::{EXIT_FAILED, EXIT_OK, EXIT_UNABLE, Streams, run};

// --- Fixtures -------------------------------------------------------------------------------

/// A temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "check-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Creates an empty TopHat project in a fresh temporary directory.
fn project() -> TempDir {
    let dir = TempDir::new();
    fs::create_dir(dir.path().join(".tophat")).unwrap();
    dir
}

/// Writes `studies/<id>/STUDY.md` with the given content.
fn study(project: &TempDir, id: &str, content: &str) {
    let dir = project.path().join("studies").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("STUDY.md"), content).unwrap();
}

// --- Test rule families -----------------------------------------------------------------------

fn d(rule: &'static str, severity: Severity, path: String, line: Option<u32>) -> Diagnostic {
    Diagnostic {
        rule,
        severity,
        path,
        line,
        message: format!("{rule} found"),
        hint: line.map(|_| format!("Fix {rule}")),
    }
}

/// Emulates a study rule family: reports one parse error for a STUDY.md whose front matter is
/// invalid (and skips its other rules), and otherwise one diagnostic per marker line:
/// `ERROR`, `WARNING`, `LEGACY`, `FROZEN`, and `NOLOC-ERROR` (no line, no hint).
fn marker_family(project: &Project) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Ok(ids) = project.list_dir("studies") else {
        return out;
    };
    for id in ids {
        let path = format!("studies/{id}/STUDY.md");
        let Ok(text) = project.read_to_string(&path) else {
            continue;
        };
        if text.starts_with("---\n: invalid") {
            out.push(d("study.parse-error", Severity::Error, path, Some(1)));
            continue;
        }
        for (i, line) in text.lines().enumerate() {
            let line_no = Some(i as u32 + 1);
            let found = match line.trim() {
                "ERROR" => Some(("test.error", Severity::Error, line_no)),
                "WARNING" => Some(("test.warning", Severity::Warning, line_no)),
                "LEGACY" => Some(("test.legacy", Severity::Legacy, line_no)),
                "FROZEN" => Some(("study.frozen-modified", Severity::Error, line_no)),
                "NOLOC-ERROR" => Some(("test.noloc", Severity::Error, None)),
                _ => None,
            };
            if let Some((rule, severity, line)) = found {
                out.push(d(rule, severity, path.clone(), line));
            }
        }
    }
    out
}

/// Emulates a cross-reference rule: `superseded_by: <id>` must name a study whose STUDY.md lists
/// this study under `supersedes: <id>`.
fn supersede_family(project: &Project) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Ok(ids) = project.list_dir("studies") else {
        return out;
    };
    for id in ids {
        let path = format!("studies/{id}/STUDY.md");
        let Ok(text) = project.read_to_string(&path) else {
            continue;
        };
        let Some(target) = text.lines().find_map(|l| l.strip_prefix("superseded_by: ")) else {
            continue;
        };
        let backlinked = project
            .read_to_string(&format!("studies/{target}/STUDY.md"))
            .map(|t| t.lines().any(|l| l == format!("supersedes: {id}")))
            .unwrap_or(false);
        if !backlinked {
            out.push(d(
                "study.supersede-backlink-missing",
                Severity::Error,
                path,
                None,
            ));
        }
    }
    out
}

const FAMILIES: &[RuleFamily] = &[marker_family, supersede_family];

// --- Running ----------------------------------------------------------------------------------

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs `tophat <args>` in-process from `cwd` with the test rule families.
fn tophat(cwd: &Path, args: &[&str]) -> Outcome {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(
        &args,
        cwd,
        FAMILIES,
        &mut Streams {
            input: &mut std::io::empty(),
            out: &mut out,
            err: &mut err,
            interactive: false,
        },
    );
    Outcome {
        code,
        stdout: String::from_utf8(out).unwrap(),
        stderr: String::from_utf8(err).unwrap(),
    }
}

/// Runs the real `tophat` binary from `cwd`.
fn tophat_binary(cwd: &Path, args: &[&str]) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn json(outcome: &Outcome) -> serde_json::Value {
    serde_json::from_str(&outcome.stdout).expect("stdout is a single JSON value")
}

// --- Project discovery ------------------------------------------------------------------------

#[test] // rq-23b830b6 rq-abbd00cd
fn check_runs_from_the_project_root() {
    let p = project();
    let out = tophat_binary(p.path(), &["check"]);
    assert_eq!(out.code, EXIT_OK, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .ends_with("0 error(s), 0 warning(s), 0 legacy item(s)\n")
    );
}

#[test] // rq-8889d0ed rq-abbd00cd
fn check_runs_from_a_subdirectory_of_the_project() {
    let p = project();
    fs::write(
        p.path().join("PROJECT.md"),
        "# Project\n\n## q-1a2b3c4d · Can X improve?\nstatus: open\n",
    )
    .unwrap();
    let created = tophat_binary(
        p.path(),
        &["study", "new", "x-vs-docking", "--question", "q-1a2b3c4d"],
    );
    assert_eq!(created.code, EXIT_OK, "stderr: {}", created.stderr);
    let out = tophat_binary(&p.path().join("studies/x-vs-docking"), &["check"]);
    assert_eq!(out.code, EXIT_OK, "stderr: {}", out.stderr);
}

#[test] // rq-580c8189 rq-35e01b00
fn check_outside_any_tophat_project() {
    let dir = TempDir::new();
    for args in [&["check"][..], &["check", "--json"][..]] {
        let out = tophat_binary(dir.path(), args);
        assert_eq!(out.code, EXIT_UNABLE);
        assert!(out.stderr.contains("no TopHat project found"));
        assert!(out.stdout.is_empty());
    }
}

// --- Severities and exit codes ----------------------------------------------------------------

#[test] // rq-b17553a6 rq-35e01b00
fn an_error_fails_the_check() {
    let p = project();
    study(&p, "a", "ERROR\n");
    assert_eq!(tophat(p.path(), &["check"]).code, EXIT_FAILED);
}

#[test] // rq-0bdbef43 rq-192a5ff9
fn a_warning_does_not_fail_the_check_by_default() {
    let p = project();
    study(&p, "a", "WARNING\n");
    let out = tophat(p.path(), &["check"]);
    assert!(
        out.stdout
            .contains("studies/a/STUDY.md:1: warning[test.warning]: test.warning found")
    );
    assert_eq!(out.code, EXIT_OK);
}

#[test] // rq-74adaff3 rq-1bce095c
fn a_warning_fails_the_check_in_strict_mode() {
    let p = project();
    study(&p, "a", "WARNING\n");
    assert_eq!(tophat(p.path(), &["check", "--strict"]).code, EXIT_FAILED);
}

#[test] // rq-4151372b rq-192a5ff9
fn legacy_items_never_fail_the_check_even_in_strict_mode() {
    let p = project();
    study(&p, "a", "LEGACY\n");
    let out = tophat(p.path(), &["check", "--strict"]);
    let legacy_section = out.stdout.split("Legacy\n").nth(1).expect("Legacy section");
    assert!(legacy_section.contains("legacy[test.legacy]"));
    assert_eq!(out.code, EXIT_OK);
}

// --- Robustness -------------------------------------------------------------------------------

#[test] // rq-a8be8316 rq-e54fa7fd
fn an_unparseable_file_does_not_stop_other_files_being_checked() {
    let p = project();
    study(&p, "a", "---\n: invalid yaml\nFROZEN\n---\n");
    study(&p, "b", "FROZEN\n");
    let out = tophat(p.path(), &["check", "--json"]);
    let doc = json(&out);
    let diags = doc["diagnostics"].as_array().unwrap();
    let for_a: Vec<_> = diags
        .iter()
        .filter(|d| d["path"] == "studies/a/STUDY.md")
        .collect();
    assert_eq!(for_a.len(), 1);
    assert_eq!(for_a[0]["rule"], "study.parse-error");
    assert!(
        diags
            .iter()
            .any(|d| d["path"] == "studies/b/STUDY.md" && d["rule"] == "study.frozen-modified")
    );
    assert_eq!(out.code, EXIT_FAILED);
}

/// Path → (contents, modification time) for every file and directory under `root`.
///
/// Directory modification times are deliberately not recorded: Windows updates them as a side
/// effect of unrelated filesystem activity, so they are not a reliable signal that the check
/// changed the working tree. Added or removed entries are still detected by the key set.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, Option<SystemTime>)> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                stack.push(path.clone());
                map.insert(path, (Vec::new(), None));
            } else {
                map.insert(
                    path.clone(),
                    (fs::read(&path).unwrap(), meta.modified().ok()),
                );
            }
        }
    }
    map
}

#[test] // rq-9bca6bbf rq-39ebb522
fn check_does_not_modify_the_working_tree() {
    let p = project();
    study(&p, "a", "ERROR\nWARNING\nLEGACY\n");
    let before = snapshot(p.path());
    let out = tophat(p.path(), &["check"]);
    assert_eq!(out.code, EXIT_FAILED);
    // The real binary (with the registered rule families) must not write either.
    let out = tophat_binary(p.path(), &["check", "--json"]);
    assert_ne!(out.code, EXIT_UNABLE, "stderr: {}", out.stderr);
    assert_eq!(snapshot(p.path()), before);
}

// --- Scope ------------------------------------------------------------------------------------

#[test] // rq-e57fc88d rq-cf68a3b8
fn scoped_check_reports_only_diagnostics_under_the_given_path() {
    let p = project();
    study(&p, "a", "ERROR\n");
    study(&p, "b", "ERROR\n");
    let out = tophat(p.path(), &["check", "studies/a"]);
    assert!(out.stdout.contains("studies/a/STUDY.md"));
    assert!(!out.stdout.contains("studies/b/"));
    assert!(
        out.stdout
            .ends_with("1 error(s), 0 warning(s), 0 legacy item(s)\n")
    );
    assert_eq!(out.code, EXIT_FAILED);
}

#[test] // rq-e57fc88d rq-cf68a3b8
fn scope_paths_are_relative_to_the_current_directory() {
    let p = project();
    study(&p, "a", "ERROR\n");
    study(&p, "ab", "ERROR\n");
    let out = tophat(&p.path().join("studies"), &["check", "a"]);
    assert!(out.stdout.contains("studies/a/STUDY.md"));
    assert!(!out.stdout.contains("studies/ab/"));
}

#[test] // rq-170877ff rq-e54fa7fd
fn scoped_check_still_resolves_cross_references_project_wide() {
    let p = project();
    study(&p, "a", "superseded_by: b\n");
    study(&p, "b", "supersedes: a\n");
    let out = tophat(p.path(), &["check", "studies/a"]);
    assert!(!out.stdout.contains("supersede-backlink-missing"));
    assert_eq!(out.code, EXIT_OK);

    // Control: without the back-link the diagnostic is reported in the same scope.
    study(&p, "b", "\n");
    let out = tophat(p.path(), &["check", "studies/a"]);
    assert!(out.stdout.contains("supersede-backlink-missing"));
}

#[test] // rq-451aca52 rq-35e01b00
fn scope_path_outside_the_project() {
    let p = project();
    let outside = TempDir::new();
    let out = tophat(p.path(), &["check", outside.path().to_str().unwrap()]);
    assert_eq!(out.code, EXIT_UNABLE);
    assert!(out.stderr.contains("outside the TopHat project"));
    assert!(out.stdout.is_empty());

    let out = tophat(p.path(), &["check", "../elsewhere"]);
    assert_eq!(out.code, EXIT_UNABLE);
}

#[test] // rq-35e01b00
fn invalid_arguments_cannot_run() {
    let p = project();
    let out = tophat(p.path(), &["check", "--bogus"]);
    assert_eq!(out.code, EXIT_UNABLE);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.contains("--bogus"));
}

// --- Output -----------------------------------------------------------------------------------

#[test] // rq-d47ba6b9 rq-b757d2fc
fn json_output_structure() {
    let p = project();
    study(&p, "a", "\nERROR\n");
    let out = tophat(p.path(), &["check", "--json"]);
    let doc = json(&out);
    assert_eq!(doc["format_version"], 1);
    assert_eq!(doc["tophat_version"], env!("CARGO_PKG_VERSION"));
    let diags = doc["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    let keys: Vec<&str> = diags[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = vec!["rule", "severity", "path", "line", "message", "hint"];
    expected.sort();
    let mut keys_sorted = keys.clone();
    keys_sorted.sort();
    assert_eq!(keys_sorted, expected);
    assert_eq!(diags[0]["severity"], "error");
    assert_eq!(diags[0]["line"], 2);
    assert_eq!(
        doc["summary"],
        serde_json::json!({"errors": 1, "warnings": 0, "legacy": 0})
    );
}

#[test] // rq-f94476f9 rq-b757d2fc
fn json_omits_absent_optional_fields() {
    let p = project();
    study(&p, "a", "NOLOC-ERROR\n");
    let doc = json(&tophat(p.path(), &["check", "--json"]));
    let diag = doc["diagnostics"][0].as_object().unwrap();
    assert!(!diag.contains_key("line"));
    assert!(!diag.contains_key("hint"));
}

#[test] // rq-68a6dd72 rq-b757d2fc
fn output_is_deterministic() {
    let p = project();
    study(&p, "b", "WARNING\nERROR\n");
    study(&p, "a", "LEGACY\nERROR\nNOLOC-ERROR\n");
    study(&p, "c", "superseded_by: missing\nERROR\n");
    let first = tophat(p.path(), &["check", "--json"]).stdout;
    let second = tophat(p.path(), &["check", "--json"]).stdout;
    assert_eq!(first, second);

    let doc: serde_json::Value = serde_json::from_str(&first).unwrap();
    let keys: Vec<(String, i64, String, String)> = doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_string(),
                d["line"].as_i64().unwrap_or(-1),
                d["rule"].as_str().unwrap().to_string(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert_eq!(keys.len(), 7);
}

#[test] // rq-11b56000 rq-b757d2fc
fn human_output_separates_legacy_items() {
    let p = project();
    study(&p, "a", "LEGACY\n");
    study(&p, "b", "ERROR\n");
    let out = tophat(p.path(), &["check"]);
    let error_at = out
        .stdout
        .find("studies/b/STUDY.md:1: error[test.error]: test.error found")
        .expect("error line");
    let legacy_header = out.stdout.find("\nLegacy\n").expect("Legacy section");
    let legacy_at = out
        .stdout
        .find("studies/a/STUDY.md:1: legacy[test.legacy]")
        .expect("legacy line");
    assert!(error_at < legacy_header && legacy_header < legacy_at);
    assert!(out.stdout.contains("    hint: Fix test.error\n"));
    assert!(!out.stdout.contains('\\'));
}
