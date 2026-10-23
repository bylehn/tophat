//! Tests for the study document format, lifecycle, commands and check rules
//! (rqm/study/format.md).

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use tophat::check::{Diagnostic, Project, Severity, RULE_FAMILIES};
use tophat::cli::{run, Streams, EXIT_OK};
use tophat::study::{
    exploration_hash, frozen_fingerprint, new_study, parse_exploration, parse_study_text,
    preregister, report, study_rules_at, supersede, Console, Status, StudyError, GOVERNED_SECTIONS,
};

// --- Fixtures -------------------------------------------------------------------------------

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "study-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".tophat")).unwrap();
        // The question every fixture study names, so full `tophat check` runs resolve it.
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

    fn file(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.file(rel)).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.file(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn study_text(&self, id: &str) -> String {
        self.read(&format!("studies/{id}/STUDY.md"))
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const TODAY: &str = "2026-10-19";
const PREREG_DATE: &str = "2026-03-01";

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

const SCENARIOS: &str = "```gherkin
Feature: x-vs-docking

  @S1
  Scenario: Method X ranks the benchmark set better than docking
    Given the variant \"base\"
    When the score stage completes
    Then Spearman rho of X against experiment exceeds rho of docking
```";

/// A STUDY.md under construction.
#[derive(Clone)]
struct Doc {
    id: String,
    status: String,
    variants: Vec<String>,
    extra_front: Vec<String>,
    sections: Vec<(String, String)>,
}

impl Doc {
    /// A draft ready for pre-registration.
    fn draft(id: &str) -> Doc {
        let content = |name: &str| match name {
            "Question" => "Does method X rank binders better than docking?",
            "Primary endpoint" => "Spearman rho against measured affinity on the benchmark set.",
            "Decision rule" => "X is better if its rho exceeds docking's by at least 0.1.",
            "Scenarios" => SCENARIOS,
            "Variants" => "- base: default settings",
            "Stages" => "prepare, score, summary",
            _ => "",
        };
        Doc {
            id: id.to_string(),
            status: "draft".into(),
            variants: vec!["base".into()],
            extra_front: Vec::new(),
            sections: GOVERNED_SECTIONS
                .iter()
                .map(|n| (n.to_string(), content(n).to_string()))
                .collect(),
        }
    }

    fn status(mut self, status: &str) -> Doc {
        self.status = status.into();
        self
    }

    fn variants(mut self, variants: &[&str]) -> Doc {
        self.variants = variants.iter().map(|v| v.to_string()).collect();
        self
    }

    fn front(mut self, line: &str) -> Doc {
        self.extra_front.push(line.to_string());
        self
    }

    fn set(mut self, name: &str, content: &str) -> Doc {
        match self.sections.iter_mut().find(|(n, _)| n == name) {
            Some(section) => section.1 = content.to_string(),
            None => self.sections.push((name.to_string(), content.to_string())),
        }
        self
    }

    fn without(mut self, name: &str) -> Doc {
        self.sections.retain(|(n, _)| n != name);
        self
    }

    fn render(&self) -> String {
        let mut text = format!(
            "---\nstudy: {}\ntitle: \"X versus docking\"\nquestion: q-1a2b3c4d\nstatus: {}\n\
             visibility: public\nvariants: [{}]\n",
            self.id,
            self.status,
            self.variants.join(", ")
        );
        for line in &self.extra_front {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str(&format!("---\n\n# {}\n\n", self.id));
        for (name, content) in &self.sections {
            text.push_str(&format!("## {name}\n\n"));
            if !content.is_empty() {
                text.push_str(content);
                text.push_str("\n\n");
            }
        }
        text
    }

    /// Adds a `preregistered` block whose fingerprint matches the current frozen sections.
    fn prereg_block(self) -> Doc {
        let study = parse_study_text(&self.id, "STUDY.md", &self.render()).unwrap();
        let fingerprint = frozen_fingerprint(&study);
        self.front("preregistered:")
            .front(&format!("  date: \"{PREREG_DATE}\""))
            .front(&format!("  frozen_hash: \"{fingerprint}\""))
    }

    fn preregistered(id: &str) -> Doc {
        Doc::draft(id).prereg_block().status("preregistered")
    }

    fn reported(id: &str) -> Doc {
        Doc::preregistered(id)
            .status("reported")
            .set("Result", RESULT_LINES.join("\n").as_str())
    }
}

const RESULT_LINES: [&str; 1] = ["- base/score: n=120, rho=0.61 [0.49, 0.71], p<0.001"];

fn summary_text(id: &str, dirty: bool, lines: &[&str]) -> String {
    format!(
        "---\nstudy: {id}\ncommit: \"0123456789abcdef0123456789abcdef01234567\"\ndirty: {dirty}\n\
         generated: \"2026-10-01T12:00:00Z\"\ntophat_version: \"0.1.0\"\n---\n\n## Results\n\n{}\n",
        lines.join("\n")
    )
}

/// Writes a study: STUDY.md, config/base.yaml and one config file per non-base variant.
fn write_study(p: &TempProject, doc: &Doc) {
    p.write(&format!("studies/{}/STUDY.md", doc.id), &doc.render());
    p.write(&format!("studies/{}/config/base.yaml", doc.id), "# base\n");
    for v in doc.variants.iter().filter(|v| *v != "base") {
        p.write(
            &format!("studies/{}/config/{v}.yaml", doc.id),
            "# override\n",
        );
    }
}

/// Writes a fully consistent reported study: summary and a test covering S1.
fn write_reported(p: &TempProject, id: &str) {
    write_study(p, &Doc::reported(id));
    p.write(
        &format!("studies/{id}/SUMMARY.md"),
        &summary_text(id, false, &RESULT_LINES),
    );
    p.write(&format!("studies/{id}/tests/test_scores.py"), "# th:S1\n");
}

fn diagnostics(p: &TempProject) -> Vec<Diagnostic> {
    study_rules_at(&Project::new(p.root()), date(TODAY))
}

fn with_rule<'a>(ds: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    ds.iter().filter(|d| d.rule == rule).collect()
}

fn assert_clean(ds: &[Diagnostic]) {
    let failing: Vec<_> = ds
        .iter()
        .filter(|d| d.severity != Severity::Legacy)
        .collect();
    assert!(failing.is_empty(), "unexpected diagnostics: {failing:#?}");
}

/// Runs the real binary.
fn tophat_binary(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// Runs `tophat` in-process with the registered rule families.
fn tophat(cwd: &Path, args: &[&str]) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(
        &args,
        cwd,
        RULE_FAMILIES,
        &mut Streams {
            input: &mut std::io::empty(),
            out: &mut out,
            err: &mut err,
            interactive: false,
        },
    );
    (code, String::from_utf8(out).unwrap())
}

/// Runs `preregister` as if a human typed `answer` in an interactive terminal.
fn preregister_typing(p: &TempProject, id: &str, answer: &str) -> (Result<(), StudyError>, String) {
    let mut input = Cursor::new(format!("{answer}\n").into_bytes());
    let mut out = Vec::new();
    let result = preregister(
        p.root(),
        id,
        date(TODAY),
        &mut Console {
            interactive: true,
            input: &mut input,
            out: &mut out,
        },
    );
    (result, String::from_utf8(out).unwrap())
}

// --- tophat study new -------------------------------------------------------------------------

#[test] // rq-52e3a974 rq-f689a538
fn create_a_new_study() {
    let p = TempProject::new();
    new_study(p.root(), "x-vs-docking", "q-1a2b3c4d", false).unwrap();
    let text = p.study_text("x-vs-docking");
    let study = parse_study_text("x-vs-docking", "STUDY.md", &text).unwrap();
    assert_eq!(study.status(), Some(Status::Draft));
    assert_eq!(study.front.visibility.as_ref().unwrap().value, "public");
    assert_eq!(study.variants(), ["base".to_string()]);
    for name in GOVERNED_SECTIONS {
        assert!(study.section(name).is_some(), "missing ## {name}");
    }
    assert!(p
        .read("studies/x-vs-docking/config/base.yaml")
        .contains("stages: []"));
    assert_eq!(
        p.read("studies/x-vs-docking/Snakefile"),
        tophat::pipeline::stub("x-vs-docking")
    );
    assert!(p.file("studies/x-vs-docking/tests").is_dir());
    assert_clean(&diagnostics(&p));
    // Each governed section explains in plain words what goes there.
    for name in ["Question", "Primary endpoint", "Decision rule", "Scenarios"] {
        let section = study.section(name).unwrap();
        let text: String = section
            .content
            .iter()
            .map(|(_, l)| l.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("<!--") && text.contains("Locked when you pre-register"),
            "{name}"
        );
    }
    assert!(text.contains("- S1: <expectation>") && text.contains("explore first"));
}

#[test] // rq-7fcedcf8 rq-f689a538
fn create_a_private_study() {
    let p = TempProject::new();
    let (code, _, stderr) = tophat_binary(
        p.root(),
        &[
            "study",
            "new",
            "client-targets",
            "--question",
            "q-1a2b3c4d",
            "--visibility",
            "private",
        ],
    );
    assert_eq!(code, EXIT_OK, "{stderr}");
    let study = parse_study_text(
        "client-targets",
        "STUDY.md",
        &p.study_text("client-targets"),
    )
    .unwrap();
    assert_eq!(study.front.visibility.unwrap().value, "private");
}

#[test] // rq-7bc95d75 rq-76ac5b2e
fn reject_an_invalid_study_id() {
    let p = TempProject::new();
    assert_eq!(
        new_study(p.root(), "X_vs_Docking", "q-1a2b3c4d", false),
        Err(StudyError::InvalidStudyId("X_vs_Docking".into()))
    );
    assert!(!p.file("studies").exists());
}

#[test] // rq-36072125
fn reject_a_study_id_that_already_exists() {
    let p = TempProject::new();
    new_study(p.root(), "x-vs-docking", "q-1a2b3c4d", false).unwrap();
    p.write("studies/x-vs-docking/STUDY.md", "edited\n");
    assert_eq!(
        new_study(p.root(), "x-vs-docking", "q-1a2b3c4d", false),
        Err(StudyError::StudyExists("x-vs-docking".into()))
    );
    assert_eq!(p.study_text("x-vs-docking"), "edited\n");
}

#[test] // rq-bb0e22f1
fn reject_a_malformed_question_id() {
    let p = TempProject::new();
    assert!(new_study(p.root(), "x-vs-docking", "Q1", false).is_err());
    assert!(!p.file("studies/x-vs-docking").exists());
}

// --- Front matter and structure ---------------------------------------------------------------

#[test] // rq-7effa48d
fn directory_without_study_md() {
    let p = TempProject::new();
    fs::create_dir_all(p.file("studies/empty")).unwrap();
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.missing-file");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/empty");
}

#[test] // rq-8d5c3082 rq-ce7c6815
fn invalid_yaml_front_matter() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").front("broken: [unclosed"));
    let ds = diagnostics(&p);
    let for_a: Vec<_> = ds
        .iter()
        .filter(|d| d.path.starts_with("studies/study-a/"))
        .collect();
    assert_eq!(for_a.len(), 1, "{for_a:#?}");
    assert_eq!(for_a[0].rule, "study.parse-error");
}

#[test] // rq-3e058fe4
fn study_id_does_not_match_directory() {
    let p = TempProject::new();
    let text = Doc::draft("study-a")
        .render()
        .replace("study: study-a", "study: study-b");
    p.write("studies/study-a/STUDY.md", &text);
    p.write("studies/study-a/config/base.yaml", "");
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.id-mismatch").len(), 1);
}

#[test] // rq-38c13472
fn invalid_status_value() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").status("finished"));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.field-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(5));
}

#[test] // rq-f2be6f14
fn unknown_front_matter_field() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").front("statsu: draft"));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.field-unknown");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test] // rq-0d2a06e5 rq-e9d4388a
fn missing_governed_section() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").without("Decision rule"));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.section-missing");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("Decision rule"));
}

#[test] // rq-cd497e41 rq-e9d4388a
fn duplicated_governed_section() {
    let p = TempProject::new();
    let text = format!("{}## Question\n\nAgain?\n", Doc::draft("study-a").render());
    p.write("studies/study-a/STUDY.md", &text);
    p.write("studies/study-a/config/base.yaml", "");
    let second_heading = text
        .lines()
        .enumerate()
        .filter(|(_, l)| *l == "## Question")
        .nth(1)
        .unwrap()
        .0 as u32
        + 1;
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.section-duplicate");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(second_heading));
}

#[test] // rq-a79ad3ba rq-e9d4388a
fn headings_inside_code_fences_are_not_sections() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Stages", "```markdown\n## Result\nnot a section\n```"),
    );
    let ds = diagnostics(&p);
    assert!(with_rule(&ds, "study.section-duplicate").is_empty());
    assert!(with_rule(&ds, "study.result-unexpected").is_empty());
}

#[test] // rq-9a610110
fn extra_sections_are_allowed() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Background", "Some history."),
    );
    let ds = diagnostics(&p);
    assert!(ds.iter().all(|d| !d.message.contains("Background")));
    assert_clean(&ds);
}

// --- Scenarios and tests ----------------------------------------------------------------------

fn scenario_block(tag_lines: &[&str]) -> String {
    let mut s = String::from("```gherkin\nFeature: f\n");
    for (i, tag) in tag_lines.iter().enumerate() {
        if !tag.is_empty() {
            s.push_str(&format!("  {tag}\n"));
        }
        s.push_str(&format!(
            "  Scenario: scenario {i}\n    Then something holds\n\n"
        ));
    }
    s.push_str("```");
    s
}

#[test] // rq-520893f5 rq-04595081
fn scenario_without_a_tag() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Scenarios", &scenario_block(&["@S1", ""])),
    );
    let text = p.study_text("study-a");
    let line = text
        .lines()
        .position(|l| l.contains("Scenario: scenario 1"))
        .unwrap() as u32
        + 1;
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.scenario-untagged");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line));
}

#[test] // rq-2b94ea79 rq-04595081
fn invalid_scenario_tag() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Scenarios", &scenario_block(&["@S01"])),
    );
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.scenario-tag-invalid").len(), 1);
    assert!(with_rule(&ds, "study.scenario-untagged").is_empty());
}

#[test] // rq-2b94ea79 rq-04595081
fn ordinary_tags_starting_with_s_are_not_scenario_tags() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Scenarios", &scenario_block(&["@Slow @S1 @Smoke"])),
    );
    assert_clean(&diagnostics(&p));
}

#[test] // rq-67b9306b rq-04595081
fn duplicate_scenario_numbers() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Scenarios", &scenario_block(&["@S2", "@S2"])),
    );
    let text = p.study_text("study-a");
    let second = text
        .lines()
        .position(|l| l.contains("Scenario: scenario 1"))
        .unwrap() as u32
        + 1;
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.scenario-duplicate");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(second));
}

#[test] // rq-3c295093 rq-04595081
fn scenarios_as_a_checklist() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set(
            "Scenarios",
            "- S1: The endpoint is computed from all items.\n- S2: No item is skipped.\n- A note that is not a scenario.",
        ),
    );
    p.write("studies/study-a/tests/test_scores.py", "# th:S1\n# th:S2\n");
    let ds = diagnostics(&p);
    assert!(
        !ds.iter()
            .any(|d| d.rule.starts_with("study.scenario") || d.rule == "study.test-orphan"),
        "{ds:#?}"
    );
    let study = parse_study_text("study-a", "STUDY.md", &p.study_text("study-a")).unwrap();
    let numbers: Vec<Option<u32>> = study.scenarios.iter().map(|s| s.number).collect();
    assert_eq!(numbers, [Some(1), Some(2)]);
    assert_eq!(
        study.scenarios[0].title,
        "The endpoint is computed from all items."
    );
}

#[test] // rq-de619d22 rq-04595081
fn checklist_and_gherkin_numbers_are_unique_together() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set(
            "Scenarios",
            &format!("- S1: one\n\n{}", scenario_block(&["@S1"])),
        ),
    );
    assert_eq!(
        with_rule(&diagnostics(&p), "study.scenario-duplicate").len(),
        1
    );
}

#[test] // rq-e4d78a92 rq-04595081
fn an_invalid_checklist_number() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").set("Scenarios", "- S01: one\n"));
    let ds = diagnostics(&p);
    assert_eq!(
        with_rule(&ds, "study.scenario-tag-invalid").len(),
        1,
        "{ds:#?}"
    );
}

#[test] // rq-af111715 rq-04595081
fn gaps_in_scenario_numbers_are_allowed() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Scenarios", &scenario_block(&["@S1", "@S3"])),
    );
    assert_clean(&diagnostics(&p));
}

#[test] // rq-6a843b0a rq-aa0fe39a
fn test_reference_in_a_julia_file_counts_as_a_test() {
    let p = TempProject::new();
    write_study(&p, &Doc::reported("study-a"));
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &RESULT_LINES),
    );
    p.write(
        "studies/study-a/tests/test_scores.jl",
        "# th:S1\n@test true\n",
    );
    let ds = diagnostics(&p);
    assert!(with_rule(&ds, "study.scenario-untested").is_empty());
    assert_clean(&ds);
}

fn with_two_scenarios(doc: Doc) -> Doc {
    doc.set("Scenarios", &scenario_block(&["@S1", "@S2"]))
}

#[test] // rq-3cc4a23d rq-aa0fe39a
fn untested_scenario_in_a_preregistered_study_warns() {
    let p = TempProject::new();
    write_study(
        &p,
        &with_two_scenarios(Doc::draft("study-a"))
            .prereg_block()
            .status("preregistered"),
    );
    p.write("studies/study-a/tests/t.py", "# th:S1\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.scenario-untested");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    assert!(found[0].message.contains("S2"));
}

#[test] // rq-1931327f rq-aa0fe39a
fn untested_scenario_in_a_reported_study_is_an_error() {
    let p = TempProject::new();
    let doc = with_two_scenarios(Doc::draft("study-a"))
        .prereg_block()
        .status("reported")
        .set("Result", RESULT_LINES[0]);
    write_study(&p, &doc);
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &RESULT_LINES),
    );
    p.write("studies/study-a/tests/t.py", "# th:S1\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.scenario-untested");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("S2"));
}

#[test] // rq-b8e989cd rq-aa0fe39a
fn orphan_test_reference_in_a_draft_warns() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    p.write("studies/study-a/tests/sub/t.R", "x <- 1\n# th:S9\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.test-orphan");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    assert_eq!(found[0].path, "studies/study-a/tests/sub/t.R");
    assert_eq!(found[0].line, Some(2));
}

#[test] // rq-9197f110 rq-aa0fe39a
fn orphan_test_reference_after_preregistration_is_an_error() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    p.write("studies/study-a/tests/t.sh", "# th:S1 th:S9\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.test-orphan");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Error);
    assert_eq!(found[0].path, "studies/study-a/tests/t.sh");
}

// --- Variants and config ----------------------------------------------------------------------

#[test] // rq-d4308b0e rq-e5eda0dd
fn missing_variant_config() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").variants(&["base", "b3lyp"]));
    fs::remove_file(p.file("studies/study-a/config/b3lyp.yaml")).unwrap();
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.config-missing").len(), 1);
}

#[test] // rq-4838d3a2 rq-e5eda0dd
fn unlisted_variant_config() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    p.write("studies/study-a/config/old.yaml", "# stale\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.config-unlisted");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/study-a/config/old.yaml");
}

#[test] // rq-f678e5d8 rq-e5eda0dd
fn missing_base_config() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    fs::remove_file(p.file("studies/study-a/config/base.yaml")).unwrap();
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.config-base-missing").len(), 1);
}

// --- Pre-registration -------------------------------------------------------------------------

/// The draft text with the edits `preregister` is specified to make.
/// The draft as pre-registration writes it, with the exploration fields of the study's
/// `EXPLORATION.md` (which pre-registration writes too).
fn expected_preregistered(p: &TempProject, draft: &str, date: &str, fingerprint: &str) -> String {
    let with_status = draft.replacen("status: draft\n", "status: preregistered\n", 1);
    let close = with_status[4..].find("---\n").unwrap() + 4;
    let id = with_status
        .lines()
        .find_map(|l| l.strip_prefix("study: "))
        .unwrap();
    let exploration = p.read(&format!("studies/{id}/EXPLORATION.md"));
    let seen = parse_exploration(&exploration).unwrap().1;
    format!(
        "{}preregistered:\n  date: \"{date}\"\n  frozen_hash: \"{fingerprint}\"\n  \
         exploration_hash: \"{}\"\n  deciding_data_seen: {seen}\n{}",
        &with_status[..close],
        exploration_hash(&exploration),
        &with_status[close..]
    )
}

#[test] // rq-9c53f79c rq-bcb83d0a
fn preregister_a_ready_draft_interactively() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    let before = p.study_text("study-a");
    let (result, output) = preregister_typing(&p, "study-a", "study-a");
    result.unwrap();
    assert!(output.contains("## Decision rule"));
    assert!(output.contains("Commit STUDY.md"));

    let after = p.study_text("study-a");
    let study = parse_study_text("study-a", "STUDY.md", &after).unwrap();
    assert_eq!(study.status(), Some(Status::Preregistered));
    let block = study.front.preregistered.as_ref().unwrap();
    assert_eq!(block.value.date.as_deref(), Some(TODAY));
    let fingerprint = frozen_fingerprint(&study);
    assert_eq!(
        block.value.frozen_hash.as_deref(),
        Some(fingerprint.as_str())
    );
    assert_eq!(
        after,
        expected_preregistered(&p, &before, TODAY, &fingerprint)
    );
    assert_clean(
        &diagnostics(&p)
            .into_iter()
            .filter(|d| d.rule != "study.scenario-untested")
            .collect::<Vec<_>>(),
    );
}

#[test] // rq-c69454bd rq-bcb83d0a
fn preregistration_explains_what_it_locks() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    let (result, output) = preregister_typing(&p, "study-a", "study-a");
    result.unwrap();
    let confirm = output.find("Type the study ID").unwrap();
    let explained = &output[..confirm];
    for phrase in [
        "locks the plan before you see the result",
        "dated amendment",
        "The pipeline, its stages, variants and scripts can still change",
    ] {
        assert!(explained.contains(phrase), "{phrase}: {output}");
    }
}

#[test] // rq-54c9eb81 rq-bcb83d0a
fn preregistration_refused_without_an_interactive_terminal() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    let before = p.study_text("study-a");
    let mut input = Cursor::new(b"study-a\n".to_vec());
    let mut out = Vec::new();
    let result = preregister(
        p.root(),
        "study-a",
        date(TODAY),
        &mut Console {
            interactive: false,
            input: &mut input,
            out: &mut out,
        },
    );
    assert_eq!(result, Err(StudyError::NotInteractive));
    assert_eq!(p.study_text("study-a"), before);

    // The real binary with piped standard input refuses too.
    let (code, _, stderr) = tophat_binary(p.root(), &["study", "preregister", "study-a"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("interactive terminal"));
    assert_eq!(p.study_text("study-a"), before);
}

#[test] // rq-fdd4cac9 rq-bcb83d0a
fn preregistration_aborted_by_a_wrong_confirmation() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    let before = p.study_text("study-a");
    let (result, _) = preregister_typing(&p, "study-a", "study-b");
    assert_eq!(result, Err(StudyError::ConfirmationMismatch));
    assert_eq!(p.study_text("study-a"), before);
}

#[test] // rq-9334c74c rq-bcb83d0a
fn preregistration_refused_when_the_draft_is_not_ready() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Decision rule", "<!-- TODO -->"),
    );
    let before = p.study_text("study-a");
    let (result, output) = preregister_typing(&p, "study-a", "study-a");
    match result {
        Err(StudyError::NotReady(ds)) => {
            assert!(ds.iter().any(|d| d.rule == "study.section-empty"))
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(!output.contains("Type the study ID"));
    assert_eq!(p.study_text("study-a"), before);
}

#[test] // rq-65465008 rq-bcb83d0a
fn preregistration_refused_for_a_non_draft_study() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    let (result, _) = preregister_typing(&p, "study-a", "study-a");
    assert!(matches!(
        result,
        Err(StudyError::WrongStatus {
            status: Some(Status::Reported),
            ..
        })
    ));
}

// --- Frozen sections --------------------------------------------------------------------------

fn edit_study(p: &TempProject, id: &str, from: &str, to: &str) {
    let text = p.study_text(id);
    assert!(text.contains(from), "fixture lacks {from:?}");
    p.write(
        &format!("studies/{id}/STUDY.md"),
        &text.replacen(from, to, 1),
    );
}

#[test] // rq-a3e1bf1f rq-08c7718c
fn editing_a_frozen_section_is_detected() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    assert!(with_rule(&diagnostics(&p), "study.frozen-modified").is_empty());
    edit_study(&p, "study-a", "Spearman rho against", "Pearson r against");
    assert_eq!(
        with_rule(&diagnostics(&p), "study.frozen-modified").len(),
        1
    );
}

#[test] // rq-a046955d rq-08c7718c
fn editing_a_scenario_is_detected() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    edit_study(
        &p,
        "study-a",
        "Then Spearman rho of X",
        "Then Kendall tau of X",
    );
    assert_eq!(
        with_rule(&diagnostics(&p), "study.frozen-modified").len(),
        1
    );
}

#[test] // rq-cb65dab4 rq-08c7718c
fn editing_a_non_frozen_section_is_allowed() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    edit_study(
        &p,
        "study-a",
        "prepare, score, summary",
        "prepare, filter, score, summary",
    );
    edit_study(
        &p,
        "study-a",
        "## Interpretation\n",
        "## Interpretation\n\nLooks promising.\n",
    );
    assert!(with_rule(&diagnostics(&p), "study.frozen-modified").is_empty());
}

#[test] // rq-2e00ed22 rq-08c7718c
fn whitespace_and_line_ending_changes_do_not_alter_the_fingerprint() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    let text = p
        .study_text("study-a")
        .replace("better than docking?", "better than docking?   \t")
        .replace('\n', "\r\n");
    p.write("studies/study-a/STUDY.md", &text);
    let ds = diagnostics(&p);
    assert!(with_rule(&ds, "study.frozen-modified").is_empty());
    assert!(with_rule(&ds, "study.parse-error").is_empty());
}

#[test] // rq-930cacde rq-08c7718c
fn reordering_sections_does_not_alter_the_fingerprint() {
    let p = TempProject::new();
    let doc = Doc::preregistered("study-a");
    let scenarios = doc
        .sections
        .iter()
        .find(|(n, _)| n == "Scenarios")
        .unwrap()
        .clone();
    let mut reordered = doc.clone();
    reordered.sections.retain(|(n, _)| n != "Scenarios");
    reordered.sections.insert(0, scenarios);
    write_study(&p, &reordered);
    assert!(with_rule(&diagnostics(&p), "study.frozen-modified").is_empty());
}

#[test] // rq-d4227e28 rq-76ac5b2e
fn renaming_a_preregistered_study_is_detected() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    fs::rename(p.file("studies/study-a"), p.file("studies/study-b")).unwrap();
    edit_study(&p, "study-b", "study: study-a", "study: study-b");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.frozen-modified");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/study-b/STUDY.md");
}

#[test] // rq-ea9c4659 rq-f28e6ab8
fn preregistration_block_on_a_draft() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").prereg_block());
    assert_eq!(
        with_rule(&diagnostics(&p), "study.prereg-unexpected").len(),
        1
    );
}

#[test] // rq-a97cc02c rq-f28e6ab8
fn missing_preregistration_block_on_a_preregistered_study() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a").status("preregistered"));
    assert_eq!(with_rule(&diagnostics(&p), "study.prereg-missing").len(), 1);
}

// --- Amendments -------------------------------------------------------------------------------

fn amendment_diagnostics(ds: &[Diagnostic]) -> Vec<&Diagnostic> {
    ds.iter()
        .filter(|d| d.rule.starts_with("study.amendment"))
        .collect()
}

#[test] // rq-ec84dc2b rq-fd31255d
fn valid_amendment() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a").set(
            "Amendments",
            "- 2026-04-02 — results seen: no — Fixed unit conversion in the filter script",
        ),
    );
    assert!(amendment_diagnostics(&diagnostics(&p)).is_empty());
}

#[test] // rq-214c0020 rq-fd31255d
fn malformed_amendment() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a").set("Amendments", "- changed the filter"),
    );
    let line = p
        .study_text("study-a")
        .lines()
        .position(|l| l == "- changed the filter")
        .unwrap() as u32
        + 1;
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.amendment-format");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line));
}

#[test] // rq-68f43997 rq-fd31255d
fn amendment_dated_before_preregistration() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a")
            .set("Amendments", "- 2026-02-15 — results seen: no — Early fix"),
    );
    assert_eq!(with_rule(&diagnostics(&p), "study.amendment-date").len(), 1);
}

#[test] // rq-70b0f2e4 rq-fd31255d
fn amendment_dated_in_the_future() {
    let p = TempProject::new();
    let today = chrono::Local::now().date_naive();
    let tomorrow = today.succ_opt().unwrap().format("%Y-%m-%d").to_string();
    write_study(
        &p,
        &Doc::preregistered("study-a").set(
            "Amendments",
            &format!("- {tomorrow} — results seen: no — Time travel"),
        ),
    );
    let ds = study_rules_at(&Project::new(p.root()), today);
    assert_eq!(with_rule(&ds, "study.amendment-date").len(), 1);
    // The registered family uses today's local date as well.
    let (code, out) = tophat(p.root(), &["check"]);
    assert_eq!(code, 1);
    assert!(out.contains("study.amendment-date"));
}

#[test] // rq-a32e7ed9 rq-fd31255d
fn amendment_on_a_draft() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::draft("study-a").set("Amendments", "- 2026-04-02 — results seen: no — Too early"),
    );
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.amendment-unexpected").len(), 1);
    assert_eq!(amendment_diagnostics(&ds).len(), 1);
}

// --- Reporting --------------------------------------------------------------------------------

const TWO_LINES: [&str; 2] = [
    "- base/score: n=120, rho=0.61 [0.49, 0.71], p<0.001",
    "- b3lyp/score: n=120, rho=0.58 [0.45, 0.69], p<0.001",
];

#[test] // rq-46e1add4 rq-ecacda48
fn report_copies_result_lines_verbatim() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a").variants(&["base", "b3lyp"]),
    );
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &TWO_LINES),
    );
    let before = p.study_text("study-a");
    report(p.root(), "study-a").unwrap();
    let after = p.study_text("study-a");

    let expected = before
        .replacen("status: preregistered\n", "status: reported\n", 1)
        .replacen(
            "## Result\n\n## Interpretation",
            &format!("## Result\n\n{}\n\n## Interpretation", TWO_LINES.join("\n")),
            1,
        );
    assert_eq!(after, expected);
    let study = parse_study_text("study-a", "STUDY.md", &after).unwrap();
    assert_eq!(study.status(), Some(Status::Reported));
    let lines: Vec<String> = study.result_lines().into_iter().map(|(_, l)| l).collect();
    assert_eq!(lines, TWO_LINES);
}

#[test] // rq-330e2712 rq-ecacda48
fn report_refuses_a_dirty_summary() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", true, &RESULT_LINES),
    );
    let before = p.study_text("study-a");
    match report(p.root(), "study-a") {
        Err(StudyError::Summary(message)) => assert!(message.contains("uncommitted code")),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(p.study_text("study-a"), before);
}

#[test] // rq-ce0bc1c8 rq-ecacda48
fn report_refuses_a_missing_summary() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    let before = p.study_text("study-a");
    assert!(matches!(
        report(p.root(), "study-a"),
        Err(StudyError::Summary(_))
    ));
    assert_eq!(p.study_text("study-a"), before);
}

#[test] // rq-919c4647 rq-ecacda48
fn report_refuses_a_summary_for_another_study() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-b", false, &RESULT_LINES),
    );
    assert!(matches!(
        report(p.root(), "study-a"),
        Err(StudyError::Summary(_))
    ));
}

#[test] // rq-13aae895 rq-ecacda48
fn report_refuses_a_draft() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &RESULT_LINES),
    );
    assert!(matches!(
        report(p.root(), "study-a"),
        Err(StudyError::WrongStatus {
            status: Some(Status::Draft),
            ..
        })
    ));
}

#[test] // rq-e7c71a2b rq-ecacda48
fn re_reporting_refreshes_the_result() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    let new_line = "- base/score: n=130, rho=0.63 [0.52, 0.72], p<0.001";
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &[new_line]),
    );
    report(p.root(), "study-a").unwrap();
    let study = parse_study_text("study-a", "STUDY.md", &p.study_text("study-a")).unwrap();
    assert_eq!(study.status(), Some(Status::Reported));
    let lines: Vec<String> = study.result_lines().into_iter().map(|(_, l)| l).collect();
    assert_eq!(lines, vec![new_line.to_string()]);
    assert_clean(&diagnostics(&p));
}

#[test] // rq-d6677222 rq-09eda840
fn hand_edited_result_is_detected() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    assert_clean(&diagnostics(&p));
    edit_study(&p, "study-a", "rho=0.61 [", "rho=0.71 [");
    let line = p
        .study_text("study-a")
        .lines()
        .position(|l| l.contains("rho=0.71"))
        .unwrap() as u32
        + 1;
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.result-mismatch");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line));
}

#[test] // rq-14d17a56 rq-09eda840
fn reported_study_whose_summary_became_dirty() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", true, &RESULT_LINES),
    );
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.summary-dirty").len(), 1);
    assert!(with_rule(&ds, "study.result-mismatch").is_empty());
}

#[test] // rq-be41ffc1 rq-f28e6ab8
fn result_in_a_preregistered_study() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a").set("Result", RESULT_LINES[0]),
    );
    assert_eq!(
        with_rule(&diagnostics(&p), "study.result-unexpected").len(),
        1
    );
}

#[test] // rq-f2135c24 rq-f28e6ab8
fn empty_result_in_a_reported_study() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a").status("reported"));
    assert_eq!(with_rule(&diagnostics(&p), "study.result-missing").len(), 1);
}

#[test] // rq-588d371f rq-09eda840
fn summary_line_with_an_unlisted_variant() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    let lines = [RESULT_LINES[0], "- pbe/score: n=10, rho=0.3"];
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &lines),
    );
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.summary-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/study-a/SUMMARY.md");
}

// --- Retrospective studies --------------------------------------------------------------------

fn retrospective(id: &str) -> Doc {
    Doc::draft(id)
        .status("retrospective")
        .set("Result", "- base/score: rho=0.5 (from the 2024 report)")
}

#[test] // rq-c55140e8 rq-f28e6ab8
fn retrospective_study_with_hand_written_result_is_flagged_as_legacy() {
    let p = TempProject::new();
    write_study(&p, &retrospective("old-benchmark"));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.result-unverified");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Legacy);
    assert_clean(&ds);
    let (code, out) = tophat(p.root(), &["check", "--strict"]);
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.contains("legacy[study.result-unverified]"));
}

#[test] // rq-c985c1d6 rq-09eda840
fn retrospective_study_with_a_pipeline_uses_strict_results() {
    let p = TempProject::new();
    write_study(&p, &retrospective("old-benchmark"));
    p.write(
        "studies/old-benchmark/SUMMARY.md",
        &summary_text("old-benchmark", false, &RESULT_LINES),
    );
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "study.result-mismatch").len(), 1);
    assert!(with_rule(&ds, "study.result-unverified").is_empty());
}

#[test] // rq-c65bf680 rq-f28e6ab8
fn retrospective_study_may_have_empty_decision_rule_and_scenarios() {
    let p = TempProject::new();
    write_study(
        &p,
        &retrospective("old-benchmark")
            .set("Decision rule", "")
            .set("Scenarios", ""),
    );
    let ds = diagnostics(&p);
    assert!(with_rule(&ds, "study.section-empty").is_empty());
    assert!(with_rule(&ds, "study.no-scenarios").is_empty());
    assert_clean(&ds);
}

#[test] // rq-647102ae rq-f28e6ab8
fn retrospective_study_cannot_carry_a_preregistration_block() {
    let p = TempProject::new();
    write_study(&p, &retrospective("old-benchmark").prereg_block());
    assert_eq!(
        with_rule(&diagnostics(&p), "study.prereg-unexpected").len(),
        1
    );
}

#[test] // rq-3a79a3f7 rq-ecacda48
fn retrospective_study_can_be_reported_from_a_summary() {
    let p = TempProject::new();
    write_study(&p, &retrospective("old-benchmark"));
    p.write(
        "studies/old-benchmark/SUMMARY.md",
        &summary_text("old-benchmark", false, &RESULT_LINES),
    );
    report(p.root(), "old-benchmark").unwrap();
    let study =
        parse_study_text("old-benchmark", "STUDY.md", &p.study_text("old-benchmark")).unwrap();
    assert_eq!(study.status(), Some(Status::Retrospective));
    let lines: Vec<String> = study.result_lines().into_iter().map(|(_, l)| l).collect();
    assert_eq!(lines, RESULT_LINES);
    assert_clean(&diagnostics(&p));
}

// --- Supersession -----------------------------------------------------------------------------

fn supersede_diagnostics(ds: &[Diagnostic]) -> Vec<&Diagnostic> {
    ds.iter()
        .filter(|d| d.rule.starts_with("study.supersede"))
        .collect()
}

#[test] // rq-78ed7708 rq-9f2c5cd1
fn supersede_a_reported_study() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    write_study(&p, &Doc::draft("study-b"));
    supersede(p.root(), "study-a", "study-b").unwrap();

    let a = parse_study_text("study-a", "STUDY.md", &p.study_text("study-a")).unwrap();
    assert_eq!(a.status(), Some(Status::Superseded));
    assert_eq!(a.front.superseded_by.unwrap().value, "study-b");
    let b = parse_study_text("study-b", "STUDY.md", &p.study_text("study-b")).unwrap();
    assert_eq!(
        b.front.supersedes.unwrap().value,
        vec!["study-a".to_string()]
    );
    let ds = diagnostics(&p);
    assert!(supersede_diagnostics(&ds).is_empty());
    assert_clean(&ds);
}

#[test] // rq-84fe523b rq-f28e6ab8
fn superseded_study_keeps_its_frozen_section_check() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    write_study(&p, &Doc::draft("study-b"));
    supersede(p.root(), "study-a", "study-b").unwrap();
    assert!(with_rule(&diagnostics(&p), "study.frozen-modified").is_empty());
    edit_study(
        &p,
        "study-a",
        "better than docking?",
        "better than anything?",
    );
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.frozen-modified");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/study-a/STUDY.md");
}

#[test] // rq-a9de1b5b rq-9f2c5cd1
fn supersede_refuses_a_draft() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-a"));
    write_study(&p, &Doc::draft("study-b"));
    let (a, b) = (p.study_text("study-a"), p.study_text("study-b"));
    assert!(matches!(
        supersede(p.root(), "study-a", "study-b"),
        Err(StudyError::WrongStatus { .. })
    ));
    assert_eq!(p.study_text("study-a"), a);
    assert_eq!(p.study_text("study-b"), b);
}

#[test] // rq-765490e4 rq-9f2c5cd1
fn supersede_refuses_a_missing_replacement() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    let a = p.study_text("study-a");
    assert_eq!(
        supersede(p.root(), "study-a", "study-b"),
        Err(StudyError::StudyNotFound("study-b".into()))
    );
    assert_eq!(p.study_text("study-a"), a);
}

#[test] // rq-9f2c5cd1
fn supersede_refuses_the_same_study() {
    let p = TempProject::new();
    write_reported(&p, "study-a");
    assert_eq!(
        supersede(p.root(), "study-a", "study-a"),
        Err(StudyError::SameStudy("study-a".into()))
    );
}

#[test] // rq-1aa65ab4
fn superseded_by_target_does_not_exist() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a")
            .status("superseded")
            .front("superseded_by: gone"),
    );
    assert_eq!(
        with_rule(&diagnostics(&p), "study.supersede-target-missing").len(),
        1
    );
}

#[test] // rq-ddb6cfbc
fn missing_back_link() {
    let p = TempProject::new();
    write_study(
        &p,
        &Doc::preregistered("study-a")
            .status("superseded")
            .front("superseded_by: study-b"),
    );
    write_study(&p, &Doc::draft("study-b"));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.supersede-backlink-missing");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/study-a/STUDY.md");
}

#[test] // rq-ada3b70f
fn supersedes_entry_without_a_matching_forward_link() {
    let p = TempProject::new();
    write_study(&p, &Doc::draft("study-b").front("supersedes: [study-a]"));
    write_reported(&p, "study-a");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "study.supersedes-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/study-b/STUDY.md");
}

// --- File safety ------------------------------------------------------------------------------

#[test] // rq-607bbcc0 rq-bcb83d0a
fn commands_preserve_unrelated_content() {
    let p = TempProject::new();
    let doc = Doc::draft("study-a")
        .set(
            "Background",
            "Context   with  odd   spacing.\t\n\n\n<!-- a comment -->",
        )
        .front("# a YAML comment");
    let text = doc
        .render()
        .replace("## Stages\n\n", "## Stages\n\n<!-- stage notes -->\n   \n");
    p.write("studies/study-a/STUDY.md", &text);
    p.write("studies/study-a/config/base.yaml", "");
    let (result, _) = preregister_typing(&p, "study-a", "study-a");
    result.unwrap();
    let after = p.study_text("study-a");
    let study = parse_study_text("study-a", "STUDY.md", &after).unwrap();
    let fingerprint = frozen_fingerprint(&study);
    assert_eq!(
        after,
        expected_preregistered(&p, &text, TODAY, &fingerprint)
    );
}

#[test] // rq-1dd3778a rq-ecacda48
fn commands_write_atomically() {
    let p = TempProject::new();
    write_study(&p, &Doc::preregistered("study-a"));
    p.write(
        "studies/study-a/SUMMARY.md",
        &summary_text("study-a", false, &RESULT_LINES),
    );
    // A directory where the temporary file must go makes creating it fail, even for root.
    fs::create_dir(p.file("studies/study-a/.STUDY.md.tophat-tmp")).unwrap();
    let before = p.study_text("study-a");
    assert!(matches!(
        report(p.root(), "study-a"),
        Err(StudyError::Io(_))
    ));
    assert_eq!(p.study_text("study-a"), before);
}
