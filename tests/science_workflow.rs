//! Tests for the science-workflow additions: study modes, epistemic tags, study links,
//! migration, emerged questions and data splits.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use tophat::check::{Project, Severity, run_check};
use tophat::data::{DatasetRef, Split, Splits, parse_splits, render_splits, split_of};
use tophat::question::{Citation, QuestionStatus, emerge, parse_project};
use tophat::study::{
    Epistemic, Mode, Relation, Status, TagProblem, classify_tag, frozen_fingerprint, link, migrate,
    new_study, parse_study_text, study_rules_at,
};

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "science-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".tophat")).unwrap();
        fs::write(
            dir.join("PROJECT.md"),
            "# Project\n\n## q-1a2b3c4d · Does X beat docking?\nstatus: open\n",
        )
        .unwrap();
        TempProject(dir)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.0.join(rel)).unwrap()
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn today() -> NaiveDate {
    NaiveDate::parse_from_str("2026-10-19", "%Y-%m-%d").unwrap()
}

fn diagnostics(p: &TempProject) -> Vec<tophat::check::Diagnostic> {
    study_rules_at(&Project::new(p.root()), today())
}

fn has_rule(ds: &[tophat::check::Diagnostic], rule: &str) -> bool {
    ds.iter().any(|d| d.rule == rule)
}

// --- Study modes ----------------------------------------------------------------------------

#[test] // rq-2cd827c4
fn create_a_descriptive_study() {
    let p = TempProject::new();
    new_study(
        p.root(),
        "bench-characterisation",
        "q-1a2b3c4d",
        false,
        Mode::Descriptive,
    )
    .unwrap();
    let study = parse_study_text(
        "bench-characterisation",
        "STUDY.md",
        &p.read("studies/bench-characterisation/STUDY.md"),
    )
    .unwrap();
    assert_eq!(study.mode(), Mode::Descriptive);
    assert!(study.section("Deliverables").is_some());
    assert!(study.section("Primary endpoint").is_none());
    assert!(!has_rule(&diagnostics(&p), "study.section-missing"));
}

#[test] // rq-a091b98c
fn creating_a_replication_study_requires_the_replicate_command() {
    let p = TempProject::new();
    let err = new_study(
        p.root(),
        "rerun-score",
        "q-1a2b3c4d",
        false,
        Mode::Replication,
    )
    .unwrap_err();
    assert!(matches!(err, tophat::study::StudyError::WrongMode { .. }));
    assert!(!p.root().join("studies/rerun-score").exists());
}

#[test] // rq-bb7f2503
fn a_study_without_a_mode_is_reported() {
    let p = TempProject::new();
    p.write(
        "studies/study-a/STUDY.md",
        "---\nstudy: study-a\ntitle: \"A\"\nquestion: q-1a2b3c4d\nstatus: draft\n\
         visibility: public\nvariants: [base]\n---\n",
    );
    p.write("studies/study-a/config/base.yaml", "stages: []\n");
    let ds = diagnostics(&p);
    let d = ds.iter().find(|d| d.rule == "study.mode-missing").unwrap();
    assert!(d.hint.as_deref().unwrap().contains("tophat study migrate"));
}

#[test] // rq-45f33b14
fn invalid_mode_value_is_reported() {
    let p = TempProject::new();
    p.write(
        "studies/study-a/STUDY.md",
        "---\nstudy: study-a\ntitle: \"A\"\nquestion: q-1a2b3c4d\nstatus: draft\n\
         mode: hybrid\nvisibility: public\nvariants: [base]\n---\n",
    );
    p.write("studies/study-a/config/base.yaml", "stages: []\n");
    assert!(has_rule(&diagnostics(&p), "study.mode-invalid"));
}

#[test] // rq-b80d58f2
fn the_pre_mode_retrospective_status_is_reported() {
    let p = TempProject::new();
    p.write(
        "studies/old-benchmark/STUDY.md",
        "---\nstudy: old-benchmark\ntitle: \"A\"\nquestion: q-1a2b3c4d\n\
         status: retrospective\nvisibility: public\nvariants: [base]\n---\n",
    );
    p.write("studies/old-benchmark/config/base.yaml", "stages: []\n");
    let ds = diagnostics(&p);
    let d = ds
        .iter()
        .find(|d| d.rule == "study.status-retrospective")
        .unwrap();
    assert!(d.hint.as_deref().unwrap().contains("tophat study migrate"));
}

// --- Epistemic tags -------------------------------------------------------------------------

#[test] // rq-5cca5697
fn classify_result_tags() {
    let (_, tag) = classify_tag("- base/score: n=1 [pre-specified]").unwrap();
    assert_eq!(tag.epistemic, Epistemic::PreSpecified);
    assert_eq!(tag.split, None);
    let (_, tag) = classify_tag("- base/score: n=1 [exploratory · split:held-out]").unwrap();
    assert_eq!(tag.epistemic, Epistemic::Exploratory);
    assert_eq!(tag.split.as_deref(), Some("held-out"));
    assert_eq!(classify_tag("- base/score: n=1"), Err(TagProblem::Missing));
    assert_eq!(
        classify_tag("- base/score: n=1 [maybe]"),
        Err(TagProblem::Invalid)
    );
    assert!(matches!(
        classify_tag("- base/score: n=1 [pre-specified · split:Held Out]"),
        Err(TagProblem::InvalidSplit(_))
    ));
}

// --- Study links ----------------------------------------------------------------------------

fn write_linked_study(p: &TempProject, id: &str, mode: Mode, extra: &str) {
    p.write(
        &format!("studies/{id}/STUDY.md"),
        &format!(
            "---\nstudy: {id}\ntitle: \"{id}\"\nquestion: q-1a2b3c4d\nstatus: draft\n\
             mode: {}\nvisibility: public\nvariants: [base]\n{extra}---\n",
            mode.as_str()
        ),
    );
    p.write(&format!("studies/{id}/config/base.yaml"), "stages: []\n");
}

#[test] // rq-9c500c8f
fn link_records_a_follow_up_relation() {
    let p = TempProject::new();
    write_linked_study(&p, "study-a", Mode::Confirmatory, "");
    write_linked_study(&p, "study-b", Mode::Confirmatory, "");
    let before = p.read("studies/study-b/STUDY.md");
    link(p.root(), "study-b", Relation::FollowsUp, "study-a").unwrap();
    let after = p.read("studies/study-b/STUDY.md");
    assert!(after.contains("follows_up:\n  - \"study-a\""));
    assert_eq!(
        after.matches("study: study-b").count(),
        before.matches("study: study-b").count()
    );
}

#[test] // rq-c2acb5c8
fn link_keeps_the_list_sorted_and_unique() {
    let p = TempProject::new();
    write_linked_study(&p, "study-a", Mode::Confirmatory, "");
    write_linked_study(&p, "study-b", Mode::Confirmatory, "");
    write_linked_study(
        &p,
        "study-c",
        Mode::Confirmatory,
        "follows_up:\n  - \"study-b\"\n",
    );
    link(p.root(), "study-c", Relation::FollowsUp, "study-a").unwrap();
    link(p.root(), "study-c", Relation::FollowsUp, "study-a").unwrap();
    let text = p.read("studies/study-c/STUDY.md");
    assert!(text.contains("follows_up:\n  - \"study-a\"\n  - \"study-b\""));
}

#[test] // rq-2dd238fe
fn link_refuses_the_study_itself() {
    let p = TempProject::new();
    write_linked_study(&p, "study-a", Mode::Confirmatory, "");
    let before = p.read("studies/study-a/STUDY.md");
    assert!(link(p.root(), "study-a", Relation::Extends, "study-a").is_err());
    assert_eq!(p.read("studies/study-a/STUDY.md"), before);
}

#[test] // rq-98115a64
fn a_dangling_relation_is_reported() {
    let p = TempProject::new();
    write_linked_study(
        &p,
        "study-a",
        Mode::Confirmatory,
        "refutes:\n  - \"gone\"\n",
    );
    assert!(has_rule(&diagnostics(&p), "study.link-invalid"));
}

// --- Migration ------------------------------------------------------------------------------

#[test] // rq-014c049f
fn migrate_adds_the_mode_of_an_existing_study() {
    let p = TempProject::new();
    write_linked_study(&p, "study-a", Mode::Confirmatory, "");
    let text = p
        .read("studies/study-a/STUDY.md")
        .replace("mode: confirmatory\n", "");
    p.write("studies/study-a/STUDY.md", &text);
    assert!(migrate(p.root(), "study-a").unwrap());
    let migrated = p.read("studies/study-a/STUDY.md");
    assert!(migrated.contains("mode: confirmatory"));
    assert!(!has_rule(&diagnostics(&p), "study.mode-missing"));
}

#[test] // rq-c3ccc5f9
fn migrate_converts_a_reported_pre_mode_retrospective_study() {
    let p = TempProject::new();
    p.write(
        "studies/old-benchmark/STUDY.md",
        "---\nstudy: old-benchmark\ntitle: \"A\"\nquestion: q-1a2b3c4d\n\
         status: retrospective\nvisibility: public\nvariants: [base]\n---\n\n\
         ## Question\n\nWhy?\n\n## Result\n\n- base/score: rho=0.5 [post hoc]\n",
    );
    p.write("studies/old-benchmark/config/base.yaml", "stages: []\n");
    assert!(migrate(p.root(), "old-benchmark").unwrap());
    let text = p.read("studies/old-benchmark/STUDY.md");
    assert!(text.contains("mode: retrospective"));
    assert!(text.contains("status: reported"));
    let study = parse_study_text("old-benchmark", "STUDY.md", &text).unwrap();
    assert_eq!(study.mode(), Mode::Retrospective);
    assert_eq!(study.status(), Some(Status::Reported));
}

// --- Emerged questions ----------------------------------------------------------------------

#[test] // rq-f36436b8
fn an_emerged_question_needs_neither_answer_nor_evidence() {
    let tree = parse_project(
        "# P\n\n## q-1a2b3c4d · A?\nstatus: open\n\n\
         ### q-9c0d1e2f · B?\nstatus: emerged\n\nEmerged from: x-vs-docking\n",
    );
    assert_eq!(
        tree.questions[1].parsed_status(),
        Some(QuestionStatus::Emerged)
    );
    assert_eq!(tree.questions[1].origin.as_ref().unwrap().0, "x-vs-docking");
}

#[test] // rq-94b72fce
fn a_citation_carries_its_label() {
    let tree = parse_project(
        "# P\n\n## q-1a2b3c4d · A?\nstatus: answered\n\n**Answer:** Yes.\n\
         Evidence: outlier-dig [exploratory], q-1a2b3c4d\n",
    );
    let (citations, _) = tree.questions[0].evidence.as_ref().unwrap();
    assert_eq!(
        citations[0],
        Citation {
            target: "outlier-dig".into(),
            label: Some("exploratory".into())
        }
    );
    assert_eq!(citations[1].label, None);
}

#[test] // rq-4044efe1
fn emerge_records_a_question_born_from_a_study() {
    let p = TempProject::new();
    write_linked_study(&p, "x-vs-docking", Mode::Confirmatory, "");
    let (id, text) = emerge(
        p.root(),
        "What causes the outlier structures?",
        "x-vs-docking",
        None,
    )
    .unwrap();
    assert!(id.starts_with("q-"));
    assert_eq!(text, "What causes the outlier structures?");
    let tree = parse_project(&p.read("PROJECT.md"));
    let q = tree.index_of(&id).map(|i| &tree.questions[i]).unwrap();
    assert_eq!(q.parsed_status(), Some(QuestionStatus::Emerged));
    assert_eq!(q.origin.as_ref().unwrap().0, "x-vs-docking");
}

// --- Data splits ----------------------------------------------------------------------------

#[test] // rq-9f70c6e0 rq-f9dd9929
fn split_registry_round_trips() {
    let splits = Splits {
        splits: vec![Split {
            name: "holdout".into(),
            datasets: vec![DatasetRef::parse("bench-holdout@v1").unwrap()],
            partition: Some("bench".into()),
            source: Some(DatasetRef::parse("bench@v2").unwrap()),
        }],
    };
    let text = render_splits(&splits);
    assert_eq!(parse_splits(&text).unwrap(), splits);
}

#[test] // rq-70fdb927
fn split_of_finds_the_unique_split_on_a_consumed_set() {
    let splits = Splits {
        splits: vec![
            Split {
                name: "train".into(),
                datasets: vec![DatasetRef::parse("bench-train@v1").unwrap()],
                partition: Some("bench".into()),
                source: None,
            },
            Split {
                name: "holdout".into(),
                datasets: vec![DatasetRef::parse("bench-holdout@v1").unwrap()],
                partition: Some("bench".into()),
                source: None,
            },
        ],
    };
    let consumed = vec![DatasetRef::parse("bench-holdout@v1").unwrap()];
    assert_eq!(split_of(&splits, &consumed).as_deref(), Some("holdout"));
    let both = vec![
        DatasetRef::parse("bench-holdout@v1").unwrap(),
        DatasetRef::parse("bench-train@v1").unwrap(),
    ];
    assert_eq!(split_of(&splits, &both), None);
}

#[test] // rq-a8e08904
fn an_invalid_split_registry_is_rejected() {
    assert!(parse_splits("splits:\n  holdout:\n    datasets: []\n").is_err());
}

// --- Frozen fingerprint per mode ------------------------------------------------------------

#[test] // rq-f7810e6d
fn descriptive_fingerprint_covers_deliverables() {
    let text = "---\nstudy: bench\ntitle: \"B\"\nquestion: q-1a2b3c4d\nstatus: draft\n\
                mode: descriptive\nvisibility: public\nvariants: [base]\n---\n\n\
                ## Question\n\nWhat is measured?\n\n## Deliverables\n\nA dataset.\n\n\
                ## Scenarios\n\n```gherkin\n@S1\nScenario: s\n```\n";
    let study = parse_study_text("bench", "STUDY.md", text).unwrap();
    let before = frozen_fingerprint(&study);
    let changed = parse_study_text(
        "bench",
        "STUDY.md",
        &text.replace("A dataset.", "A different dataset."),
    )
    .unwrap();
    assert_ne!(before, frozen_fingerprint(&changed));
    assert!(frozen_fingerprint(&study).starts_with("sha256:"));
}

// --- Replication ------------------------------------------------------------------------------

fn git(p: &TempProject, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(p.root())
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test] // rq-0725f8c7
fn replicate_creates_a_study_with_a_pinned_target() {
    let p = TempProject::new();
    write_linked_study(&p, "x-vs-docking", Mode::Confirmatory, "");
    p.write(
        "studies/x-vs-docking/SUMMARY.md",
        "---\nstudy: x-vs-docking\ncommit: \"0123456789abcdef0123456789abcdef01234567\"\n\
         dirty: false\ngenerated: \"2026-10-01T12:00:00Z\"\ntophat_version: \"0.1.0\"\n---\n\n\
         ## Results\n\n- base/score: n=1 [pre-specified]\n",
    );
    git(&p, &["init", "-q"]);
    git(&p, &["config", "user.email", "t@example.com"]);
    git(&p, &["config", "user.name", "T"]);
    git(&p, &["add", "-A"]);
    git(&p, &["commit", "-qm", "target"]);
    let commit = git(&p, &["rev-parse", "HEAD"]);
    tophat::study::replicate(
        p.root(),
        "rerun-score",
        "q-1a2b3c4d",
        false,
        "x-vs-docking",
        &commit,
    )
    .unwrap();
    let text = p.read("studies/rerun-score/STUDY.md");
    assert!(text.contains("mode: replication"));
    assert!(text.contains(&format!("reproduces: \"x-vs-docking@{commit}\"")));
    let study = parse_study_text("rerun-score", "STUDY.md", &text).unwrap();
    assert!(study.section("Match criterion").is_some());
    let ds = diagnostics(&p);
    assert!(!has_rule(&ds, "study.reproduces-missing"));
    assert!(
        !ds.iter()
            .any(|d| d.rule == "study.section-missing" && d.path.contains("rerun-score"))
    );
}
