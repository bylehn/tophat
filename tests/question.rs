//! Tests for the question tree (rqm/question-tree.md).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use tophat::check::{Diagnostic, Project, Severity, run_check};
use tophat::question::{
    QuestionError, emerge, parse_project, question_rules, render_tree, render_tree_json, stamp,
    stamp_outcome,
};

// --- Fixtures -------------------------------------------------------------------------------

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "question-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".tophat")).unwrap();
        TempProject(dir)
    }

    fn with_project(text: &str) -> Self {
        let p = TempProject::new();
        p.write_project(text);
        p
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn write_project(&self, text: &str) {
        fs::write(self.0.join("PROJECT.md"), text).unwrap();
    }

    fn project_text(&self) -> String {
        fs::read_to_string(self.0.join("PROJECT.md")).unwrap()
    }

    /// Writes a minimal STUDY.md (enough for the question rules) with the given fields.
    fn study(&self, id: &str, question: &str, status: &str, extra: &[&str]) {
        let dir = self.0.join("studies").join(id);
        fs::create_dir_all(&dir).unwrap();
        let mut text = format!(
            "---\nstudy: {id}\ntitle: \"{id}\"\nquestion: {question}\nstatus: {status}\n\
             visibility: public\nvariants: [base]\n"
        );
        for line in extra {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str("---\n");
        fs::write(dir.join("STUDY.md"), text).unwrap();
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The example tree from the specification.
const EXAMPLE: &str = "# Method X for binding affinity

Three-year project funded by ...

## q-1a2b3c4d · Can method X improve affinity prediction for our targets?
status: open

### q-5e6f7a8b · Is X's interaction energy accurate against reference data?
status: answered

**Answer:** Yes. Mean absolute error is 0.8 kcal/mol on the benchmark set.
Evidence: x-energy-accuracy

### q-9c0d1e2f · Does X rank binders better than docking?
status: open

#### q-3b4c5d6e · Does the advantage hold for client targets?
status: dropped

**Reason:** The client withdrew the target set.
";

/// A question heading with a body.
fn q(level: usize, id: &str, text: &str, body: &str) -> String {
    format!("{} {id} · {text}\n{body}\n", "#".repeat(level))
}

fn answered(evidence: &str) -> String {
    format!("status: answered\n\n**Answer:** Yes.\nEvidence: {evidence}\n")
}

const OPEN: &str = "status: open\n";

fn doc(parts: &[String]) -> String {
    format!("# Project\n\n{}", parts.concat())
}

/// Root `q-1a2b3c4d` with children `q-5e6f7a8b` and `q-9c0d1e2f`, with the given bodies.
fn three(root: &str, first: &str, second: &str) -> String {
    doc(&[
        q(2, "q-1a2b3c4d", "Can X improve?", root),
        q(3, "q-5e6f7a8b", "Is X accurate?", first),
        q(3, "q-9c0d1e2f", "Does X beat docking?", second),
    ])
}

fn diagnostics(p: &TempProject) -> Vec<Diagnostic> {
    question_rules(&Project::new(p.root()))
}

fn with_rule<'a>(ds: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    ds.iter().filter(|d| d.rule == rule).collect()
}

fn evidence_rules(ds: &[Diagnostic]) -> Vec<&Diagnostic> {
    ds.iter()
        .filter(|d| d.rule.starts_with("question.evidence"))
        .collect()
}

fn line_of(text: &str, needle: &str) -> u32 {
    text.lines().position(|l| l.contains(needle)).expect(needle) as u32 + 1
}

fn tophat_binary(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

// --- Structure --------------------------------------------------------------------------------

#[test] // rq-e75cad86 rq-cc797bfb
fn a_well_formed_tree_passes_the_check() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    assert_eq!(diagnostics(&p), vec![]);
}

#[test] // rq-5ef3ab11 rq-ba118d75
fn a_project_without_project_md_has_an_empty_tree() {
    let p = TempProject::new();
    assert_eq!(diagnostics(&p), vec![]);
}

#[test] // rq-c18830cc rq-8148f10b
fn unstamped_question_heading() {
    let text = "# P\n\n## Can X improve affinity prediction?\nstatus: open\n";
    let p = TempProject::with_project(text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.unstamped");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(3));
    assert_eq!(found[0].path, "PROJECT.md");
}

#[test] // rq-cfed6415 rq-8148f10b
fn malformed_question_id() {
    let p = TempProject::with_project("# P\n\n## q-12 · Can X improve?\nstatus: open\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.id-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(3));
    assert!(with_rule(&ds, "question.unstamped").is_empty());
}

#[test] // rq-17514314 rq-8148f10b
fn stamped_heading_without_text() {
    let p = TempProject::with_project("# P\n\n## q-1a2b3c4d · \nstatus: open\n");
    assert_eq!(with_rule(&diagnostics(&p), "question.text-empty").len(), 1);
}

#[test] // rq-60a2fa37 rq-8148f10b
fn duplicate_question_ids() {
    let text = doc(&[
        q(2, "q-1a2b3c4d", "A?", OPEN),
        q(3, "q-1a2b3c4d", "B?", OPEN),
    ]);
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.duplicate-id");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(&text, "B?")));
}

#[test] // rq-1a1c0293 rq-8148f10b
fn first_question_is_not_a_level_two_heading() {
    let text = doc(&[q(3, "q-1a2b3c4d", "Sub-question first?", OPEN)]);
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.level-skip");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(&text, "Sub-question first?")));
}

#[test] // rq-a4669866 rq-8148f10b
fn skipped_heading_level() {
    let text = doc(&[
        q(2, "q-1a2b3c4d", "A?", OPEN),
        q(4, "q-5e6f7a8b", "Too deep?", OPEN),
    ]);
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.level-skip");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(&text, "Too deep?")));
}

#[test] // rq-55f9341a rq-8148f10b
fn returning_to_a_shallower_level_is_allowed() {
    let text = doc(&[
        q(2, "q-1a2b3c4d", "A?", OPEN),
        q(3, "q-5e6f7a8b", "B?", OPEN),
        q(4, "q-9c0d1e2f", "C?", OPEN),
        q(2, "q-3b4c5d6e", "D?", OPEN),
    ]);
    let p = TempProject::with_project(&text);
    assert_eq!(diagnostics(&p), vec![]);
}

#[test] // rq-1dd39a2c rq-8148f10b
fn headings_in_code_fences_are_not_questions() {
    let text = format!(
        "{}```markdown\n## Not a question\n```\n",
        doc(&[q(2, "q-1a2b3c4d", "A?", OPEN)])
    );
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    assert!(with_rule(&ds, "question.unstamped").is_empty());
    assert_eq!(ds, vec![]);
}

#[test] // rq-1f183c28 rq-0e66003f
fn parent_is_the_nearest_shallower_question() {
    let tree = parse_project(&doc(&[
        q(2, "q-1a2b3c4d", "A?", OPEN),
        q(3, "q-5e6f7a8b", "B?", OPEN),
        q(4, "q-3b4c5d6e", "C?", OPEN),
        q(3, "q-9c0d1e2f", "D?", OPEN),
    ]));
    let parent_of = |id: &str| {
        let q = &tree.questions[tree.index_of(id).unwrap()];
        q.parent.and_then(|p| tree.questions[p].id.clone())
    };
    assert_eq!(parent_of("q-3b4c5d6e").as_deref(), Some("q-5e6f7a8b"));
    assert_eq!(parent_of("q-9c0d1e2f").as_deref(), Some("q-1a2b3c4d"));
    assert_eq!(parent_of("q-1a2b3c4d"), None);
}

// --- Status, answers and reasons --------------------------------------------------------------

#[test] // rq-86db826d rq-76dbbbfa
fn missing_status_line() {
    let text = doc(&[q(2, "q-1a2b3c4d", "A?", "Some description\n")]);
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.status-missing");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(&text, "A?")));
}

#[test] // rq-a85bdde8 rq-76dbbbfa
fn invalid_status_value() {
    let text = doc(&[q(2, "q-1a2b3c4d", "A?", "status: done\n")]);
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.status-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(&text, "status: done")));
}

#[test] // rq-0e435215 rq-76dbbbfa
fn answered_question_without_an_answer() {
    let p = TempProject::new();
    p.study("x-energy", "q-1a2b3c4d", "reported", &[]);
    p.write_project(&doc(&[q(
        2,
        "q-1a2b3c4d",
        "A?",
        "status: answered\n\nEvidence: x-energy\n",
    )]));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.answer-missing").len(),
        1
    );
}

#[test] // rq-ea89d221 rq-76dbbbfa
fn answered_question_without_evidence() {
    let p = TempProject::with_project(&doc(&[q(
        2,
        "q-1a2b3c4d",
        "A?",
        "status: answered\n\n**Answer:** Yes.\n",
    )]));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-missing").len(),
        1
    );
}

#[test] // rq-b358b21a rq-76dbbbfa
fn empty_evidence_list() {
    let p = TempProject::with_project(&doc(&[q(2, "q-1a2b3c4d", "A?", &answered(" , "))]));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-missing").len(),
        1
    );
}

#[test] // rq-9d3f67c8 rq-76dbbbfa
fn dropped_question_without_a_reason() {
    let p = TempProject::with_project(&doc(&[q(2, "q-1a2b3c4d", "A?", "status: dropped\n")]));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.reason-missing").len(),
        1
    );
}

#[test] // rq-427db370 rq-76dbbbfa
fn open_question_needs_neither_answer_nor_evidence() {
    let p = TempProject::with_project(&doc(&[q(2, "q-1a2b3c4d", "A?", OPEN)]));
    assert_eq!(diagnostics(&p), vec![]);
}

// --- Evidence ---------------------------------------------------------------------------------

#[test] // rq-9cadbb01 rq-ef5ab539
fn citing_a_reported_study_about_this_question() {
    let p = TempProject::with_project(&three(OPEN, &answered("x-energy-accuracy"), OPEN));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    assert_eq!(evidence_rules(&diagnostics(&p)), Vec::<&Diagnostic>::new());
}

#[test] // rq-cffe1a61 rq-ef5ab539
fn citing_a_reported_study_about_a_descendant_question() {
    let p = TempProject::with_project(&three(&answered("x-energy-accuracy"), OPEN, OPEN));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    let ds = diagnostics(&p);
    assert!(with_rule(&ds, "question.evidence-unrelated").is_empty());
    assert_eq!(evidence_rules(&ds), Vec::<&Diagnostic>::new());
}

#[test] // rq-fd01ec6f rq-ef5ab539
fn citing_a_study_that_does_not_exist() {
    let text = three(OPEN, &answered("no-such-study"), OPEN);
    let p = TempProject::with_project(&text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(&text, "Evidence:")));
}

#[test] // rq-544bf01b rq-ef5ab539
fn citing_a_malformed_identifier() {
    let p = TempProject::with_project(&three(OPEN, &answered("X_Study"), OPEN));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-invalid").len(),
        1
    );
}

#[test] // rq-c7e4a9d5 rq-ef5ab539
fn citing_a_study_that_is_only_preregistered() {
    let p = TempProject::with_project(&three(OPEN, &answered("x-energy-accuracy"), OPEN));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "preregistered", &[]);
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-unreported").len(),
        1
    );
}

#[test] // rq-384cc812 rq-ef5ab539
fn citing_a_draft_study() {
    let p = TempProject::with_project(&three(OPEN, &answered("x-energy-accuracy"), OPEN));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "draft", &[]);
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-unreported").len(),
        1
    );
}

#[test] // rq-9f4d2081 rq-ef5ab539
fn citing_a_superseded_study() {
    let p = TempProject::with_project(&three(OPEN, &answered("x-energy-accuracy"), OPEN));
    p.study(
        "x-energy-accuracy",
        "q-5e6f7a8b",
        "superseded",
        &["superseded_by: x-energy-accuracy-v2"],
    );
    p.study("x-energy-accuracy-v2", "q-5e6f7a8b", "reported", &[]);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-superseded");
    assert_eq!(found.len(), 1);
    assert!(
        found[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("x-energy-accuracy-v2")
    );
}

#[test] // rq-db0d11c1 rq-ef5ab539
fn citing_a_retrospective_study_is_flagged_as_legacy() {
    let p = TempProject::with_project(&three(OPEN, &answered("old-benchmark"), OPEN));
    p.study(
        "old-benchmark",
        "q-5e6f7a8b",
        "reported",
        &["mode: retrospective"],
    );
    let dir = p.root().join("studies/old-benchmark");
    let text = fs::read_to_string(dir.join("STUDY.md")).unwrap()
        + "\n## Result\n\n- base/score: rho=0.5 [post hoc]\n";
    fs::write(dir.join("STUDY.md"), text).unwrap();
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-retrospective");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Legacy);
    assert!(ds.iter().all(|d| d.severity == Severity::Legacy), "{ds:#?}");
}

#[test] // rq-e895c8ff rq-ef5ab539
fn citing_a_study_about_an_unrelated_question() {
    let p = TempProject::with_project(&three(OPEN, &answered("x-vs-docking"), OPEN));
    p.study("x-vs-docking", "q-9c0d1e2f", "reported", &[]);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-unrelated");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test] // rq-01b7434d rq-ef5ab539
fn citing_an_answered_sub_question() {
    let p = TempProject::with_project(&three(
        &answered("q-5e6f7a8b"),
        &answered("x-energy-accuracy"),
        OPEN,
    ));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    assert_eq!(diagnostics(&p), vec![]);
}

#[test] // rq-4d76640c rq-ef5ab539
fn citing_an_open_sub_question() {
    let p = TempProject::with_project(&three(&answered("q-9c0d1e2f"), OPEN, OPEN));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-unanswered").len(),
        1
    );
}

#[test] // rq-4d76640c rq-ef5ab539
fn citing_a_dropped_sub_question() {
    let p = TempProject::with_project(&three(
        &answered("q-9c0d1e2f"),
        OPEN,
        "status: dropped\n\n**Reason:** Out of scope.\n",
    ));
    assert_eq!(
        with_rule(&diagnostics(&p), "question.evidence-unanswered").len(),
        1
    );
}

#[test] // rq-503474ab rq-ef5ab539
fn citing_a_question_that_is_not_a_descendant() {
    let p = TempProject::new();
    p.study("x-vs-docking", "q-9c0d1e2f", "reported", &[]);
    p.write_project(&three(
        OPEN,
        &answered("q-9c0d1e2f"),
        &answered("x-vs-docking"),
    ));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-invalid");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("not a sub-question"));
}

#[test] // rq-13deda96 rq-ef5ab539
fn citing_a_question_that_does_not_exist() {
    let p = TempProject::with_project(&three(&answered("q-deadbeef"), OPEN, OPEN));
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-invalid");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("q-deadbeef"));
}

#[test] // rq-f6dec5d4 rq-ef5ab539
fn mixed_citations_are_each_checked() {
    let p = TempProject::with_project(&three(
        &answered("q-5e6f7a8b, x-vs-docking , no-such-study"),
        &answered("x-energy-accuracy"),
        OPEN,
    ));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    p.study("x-vs-docking", "q-9c0d1e2f", "reported", &[]);
    let ds = diagnostics(&p);
    let evidence = evidence_rules(&ds);
    assert_eq!(evidence.len(), 1, "{evidence:#?}");
    assert_eq!(evidence[0].rule, "question.evidence-invalid");
    assert!(evidence[0].message.contains("no-such-study"));
}

// --- Study links ------------------------------------------------------------------------------

#[test] // rq-fe8b6a81 rq-a147f8cf
fn study_names_a_question_that_does_not_exist() {
    let p = TempProject::with_project(&three(OPEN, OPEN, OPEN));
    p.study("x-vs-docking", "q-deadbeef", "draft", &[]);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.unknown-reference");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/x-vs-docking/STUDY.md");
    assert_eq!(found[0].line, Some(4));
}

#[test] // rq-6ef37d58 rq-a147f8cf
fn study_names_a_question_when_project_md_is_missing() {
    let p = TempProject::new();
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.unknown-reference");
    assert_eq!(found.len(), 1);
    assert!(found[0].path.contains("x-vs-docking"));
}

#[test] // rq-dd539d53 rq-a147f8cf
fn study_names_a_question_with_sub_questions() {
    let p = TempProject::with_project(&three(OPEN, OPEN, OPEN));
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    assert_eq!(diagnostics(&p), vec![]);
}

#[test] // rq-91713e05 rq-a147f8cf
fn study_with_a_malformed_question_field_is_left_to_the_study_rules() {
    let p = TempProject::with_project(&three(OPEN, OPEN, OPEN));
    p.study("x-vs-docking", "Q1", "draft", &[]);
    let ds = run_check(p.root(), &[]).unwrap();
    assert!(
        ds.iter()
            .any(|d| d.rule == "study.field-invalid" && d.message.contains("question"))
    );
    assert!(with_rule(&ds, "question.unknown-reference").is_empty());
}

#[test] // rq-cc797bfb rq-ceffcdab
fn project_md_that_is_not_utf8_is_a_parse_error() {
    let p = TempProject::new();
    fs::write(p.root().join("PROJECT.md"), [0xff, 0xfe, b'#']).unwrap();
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    let ds = diagnostics(&p);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].rule, "question.parse-error");
}

// --- tophat question stamp --------------------------------------------------------------------

fn is_question_id(s: &str) -> bool {
    s.len() == 10
        && s.starts_with("q-")
        && s[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test] // rq-940d0a4b rq-1513da11
fn stamp_assigns_ids_and_status_lines() {
    let p = TempProject::with_project(
        "# P\n\n## Can X improve affinity prediction?\nWhy it matters.\n",
    );
    let (code, stdout, stderr) = tophat_binary(p.root(), &["question", "stamp"]);
    assert_eq!(code, 0, "{stderr}");
    let text = p.project_text();
    let lines: Vec<&str> = text.lines().collect();
    let heading = lines[2];
    let id = heading
        .strip_prefix("## ")
        .and_then(|h| h.strip_suffix(" · Can X improve affinity prediction?"))
        .expect(heading);
    assert!(is_question_id(id), "{id}");
    assert_eq!(lines[3], "status: open");
    assert_eq!(lines[4], "Why it matters.");
    assert_eq!(stdout, format!("{id} Can X improve affinity prediction?\n"));
    assert_eq!(diagnostics(&p), vec![]);
}

#[test] // rq-2c9834f8 rq-1513da11
fn stamp_keeps_existing_ids_and_status_lines() {
    let text = "# P\n\n## q-1a2b3c4d · A?\nstatus: answered\n";
    let p = TempProject::with_project(text);
    assert_eq!(stamp(p.root()), Ok(vec![]));
    assert_eq!(p.project_text(), text);
}

#[test] // rq-dc4c3c26 rq-1513da11
fn stamp_adds_a_missing_status_line_to_a_stamped_question() {
    let p = TempProject::with_project("# P\n\n## q-1a2b3c4d · A?\n\nDescription.\n");
    assert_eq!(stamp(p.root()), Ok(vec![]));
    assert_eq!(
        p.project_text(),
        "# P\n\n## q-1a2b3c4d · A?\nstatus: open\n\nDescription.\n"
    );
}

#[test] // rq-ba94061b rq-ef3d43f8
fn stamp_produces_unique_ids() {
    let text: String = (0..50).map(|i| format!("## Question {i}?\n")).collect();
    let p = TempProject::with_project(&format!("# P\n\n{text}"));
    let assigned = stamp(p.root()).unwrap();
    assert_eq!(assigned.len(), 50);
    let tree = parse_project(&p.project_text());
    let mut ids: Vec<String> = tree
        .questions
        .iter()
        .map(|q| q.id.clone().unwrap())
        .collect();
    assert!(ids.iter().all(|id| is_question_id(id)));
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 50);
}

#[test] // rq-d19b87ac rq-1513da11
fn stamp_leaves_malformed_ids_for_the_user() {
    let p = TempProject::with_project("# P\n\n## q-12 · A?\nstatus: open\n\n## B?\nstatus: open\n");
    let (code, stdout, stderr) = tophat_binary(p.root(), &["question", "stamp"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("q-12"), "{stderr}");
    assert!(stdout.contains(" B?"));
    let text = p.project_text();
    assert!(text.contains("## q-12 · A?\n"));
    assert!(!text.contains("## B?"));
}

#[test] // rq-24fdff0d rq-1513da11
fn stamp_leaves_duplicated_ids_for_the_user() {
    let text = "# P\n\n## q-1a2b3c4d · A?\nstatus: open\n\n### q-1a2b3c4d · B?\nstatus: open\n";
    let p = TempProject::with_project(text);
    assert_eq!(
        stamp(p.root()),
        Err(QuestionError::Unresolved(vec!["q-1a2b3c4d".into()]))
    );
    assert_eq!(p.project_text(), text);
}

#[test] // rq-a9a0b08d rq-1513da11
fn stamp_preserves_unrelated_content() {
    let original = "# Title\r\n\r\nPreamble <!-- note -->\r\n\r\n```text\r\n## Not a question\r\n```\r\n\r\n\
                    ## First?\r\nBody text.\r\n\r\n### q-1a2b3c4d · Second?\r\nstatus: open\r\n";
    let p = TempProject::with_project(original);
    let outcome = stamp_outcome(p.root()).unwrap();
    assert_eq!(outcome.assigned.len(), 1);
    let id = &outcome.assigned[0].0;
    let expected = original.replace(
        "## First?\r\nBody text.",
        &format!("## {id} · First?\r\nstatus: open\r\nBody text."),
    );
    assert_eq!(p.project_text(), expected);
}

#[test] // rq-6106e112 rq-1513da11
fn stamp_ignores_headings_in_code_fences() {
    let text = "# P\n\n## q-1a2b3c4d · A?\nstatus: open\n\n```\n## Not a question\n```\n";
    let p = TempProject::with_project(text);
    assert_eq!(stamp(p.root()), Ok(vec![]));
    assert_eq!(p.project_text(), text);
}

#[test] // rq-7eba13df rq-1513da11
fn stamp_without_project_md() {
    let p = TempProject::new();
    assert_eq!(stamp(p.root()), Err(QuestionError::ProjectFileMissing));
    assert!(!p.root().join("PROJECT.md").exists());
}

// --- tophat question tree ---------------------------------------------------------------------

#[test] // rq-c62bd382 rq-5d07156c
fn tree_shows_questions_and_attached_studies() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    p.study("x-vs-docking", "q-9c0d1e2f", "draft", &[]);
    let (code, stdout, stderr) = tophat_binary(p.root(), &["question", "tree"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        "q-1a2b3c4d [open] Can method X improve affinity prediction for our targets?
  q-5e6f7a8b [answered] Is X's interaction energy accurate against reference data?
    study x-energy-accuracy [reported]
  q-9c0d1e2f [open] Does X rank binders better than docking?
    study x-vs-docking [draft]
    q-3b4c5d6e [dropped] Does the advantage hold for client targets?
"
    );
}

#[test] // rq-41dc6a2d rq-5d07156c
fn tree_as_json() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    let doc: serde_json::Value =
        serde_json::from_str(&render_tree_json(p.root()).unwrap()).unwrap();
    let questions = doc["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 4);
    let ids: Vec<_> = questions
        .iter()
        .map(|q| q["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["q-1a2b3c4d", "q-5e6f7a8b", "q-9c0d1e2f", "q-3b4c5d6e"]
    );
    assert_eq!(questions[3]["level"], 4);
    assert_eq!(questions[3]["parent"], "q-9c0d1e2f");
    assert!(questions[0]["parent"].is_null());
    assert_eq!(
        questions[1]["studies"],
        serde_json::json!([{"id": "x-energy-accuracy", "status": "reported"}])
    );
}

#[test] // rq-7d741e69 rq-5d07156c
fn tree_does_not_validate() {
    let p = TempProject::with_project("# P\n\n## B?\n");
    assert_eq!(render_tree(p.root()).unwrap(), "(no id) [?] B?\n");
    let (code, stdout, _) = tophat_binary(p.root(), &["question", "tree"]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "(no id) [?] B?\n");
}

// --- Emerged questions, labels and emerge (science workflow) --------------------------------

/// Writes a reported study with the given mode and `## Result` lines.
fn reported_study(p: &TempProject, id: &str, question: &str, mode: &str, result: &str) {
    p.study(id, question, "reported", &[&format!("mode: {mode}")]);
    let dir = p.root().join("studies").join(id);
    let text =
        fs::read_to_string(dir.join("STUDY.md")).unwrap() + &format!("\n## Result\n\n{result}\n");
    fs::write(dir.join("STUDY.md"), text).unwrap();
}

#[test] // rq-e75df1d9
fn emerged_question_without_an_origin() {
    let p = TempProject::with_project("# P\n\n## q-1a2b3c4d · A?\nstatus: emerged\n");
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.origin-missing");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(3));
}

#[test] // rq-1b7579af
fn emerged_question_naming_a_missing_study() {
    let text = "# P\n\n## q-1a2b3c4d · A?\nstatus: emerged\n\nEmerged from: no-such-study\n";
    let p = TempProject::with_project(text);
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.origin-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(line_of(text, "Emerged from")));
}

#[test] // rq-273c5387
fn an_emerged_question_is_answered_like_an_open_one() {
    let p = TempProject::with_project(&three(
        OPEN,
        &answered("x-energy-accuracy"),
        "status: answered\n\n**Answer:** Yes.\nEvidence: outlier-dig [exploratory]\n\nEmerged from: x-vs-docking\n",
    ));
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    reported_study(
        &p,
        "outlier-dig",
        "q-9c0d1e2f",
        "exploratory",
        "- base/analyse: rho=0.42 [exploratory]",
    );
    let ds = diagnostics(&p);
    assert!(
        !ds.iter().any(|d| d.rule.starts_with("question.")
            && d.line == Some(line_of(&p.project_text(), "q-9c0d1e2f"))),
        "{ds:#?}"
    );
}

#[test] // rq-6dc70288
fn citing_an_emerged_sub_question() {
    let p = TempProject::with_project(&three(
        &answered("q-9c0d1e2f"),
        OPEN,
        "status: emerged\n\nEmerged from: x-vs-docking\n",
    ));
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "question.evidence-unanswered").len(), 1);
}

#[test] // rq-a5a6c56e
fn an_emerged_question_is_dropped_like_an_open_one() {
    let p = TempProject::with_project(&three(
        OPEN,
        OPEN,
        "status: dropped\n\n**Reason:** Out of scope.\n\nEmerged from: x-vs-docking\n",
    ));
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    let ds = diagnostics(&p);
    assert!(
        !ds.iter().any(|d| d.rule.starts_with("question.")
            && d.line == Some(line_of(&p.project_text(), "q-9c0d1e2f"))),
        "{ds:#?}"
    );
}

#[test] // rq-ea31a640
fn an_answered_question_keeps_a_valid_origin_line() {
    let p = TempProject::with_project(&three(
        OPEN,
        OPEN,
        &format!(
            "{}\nEmerged from: x-vs-docking\n",
            answered("x-energy-accuracy")
        ),
    ));
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    reported_study(
        &p,
        "x-energy-accuracy",
        "q-9c0d1e2f",
        "confirmatory",
        "- base/score: n=120, rho=0.61 [pre-specified]",
    );
    let ds = diagnostics(&p);
    assert!(
        !ds.iter().any(|d| d.rule.starts_with("question.")
            && d.line == Some(line_of(&p.project_text(), "q-9c0d1e2f"))),
        "{ds:#?}"
    );
}

#[test] // rq-944a6f44
fn a_study_may_address_an_emerged_question() {
    let p = TempProject::with_project(
        "# P\n\n## q-1a2b3c4d · A?\nstatus: open\n\n### q-7d8e9f0a · B?\nstatus: emerged\n\nEmerged from: x-vs-docking\n",
    );
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    p.study("outlier-dig", "q-7d8e9f0a", "draft", &[]);
    let ds = diagnostics(&p);
    assert!(
        with_rule(&ds, "question.unknown-reference").is_empty(),
        "{ds:#?}"
    );
}

#[test] // rq-d47955a2
fn citing_an_exploratory_study_with_its_label() {
    let p = TempProject::with_project(&three(OPEN, OPEN, &answered("outlier-dig [exploratory]")));
    reported_study(
        &p,
        "outlier-dig",
        "q-9c0d1e2f",
        "exploratory",
        "- base/analyse: rho=0.42 [exploratory]",
    );
    assert!(evidence_rules(&diagnostics(&p)).is_empty());
}

#[test] // rq-9e04442c
fn citing_a_confirmatory_studys_exploratory_result() {
    let p = TempProject::with_project(&three(OPEN, OPEN, &answered("x-vs-docking [exploratory]")));
    reported_study(
        &p,
        "x-vs-docking",
        "q-9c0d1e2f",
        "confirmatory",
        "- base/score: n=120, rho=0.61 [pre-specified]\n- pbe/analyse: rho=0.42 [exploratory]",
    );
    assert!(evidence_rules(&diagnostics(&p)).is_empty());
}

#[test] // rq-9447b114
fn a_label_the_study_has_no_result_line_for() {
    let p = TempProject::with_project(&three(OPEN, OPEN, &answered("x-vs-docking [post hoc]")));
    reported_study(
        &p,
        "x-vs-docking",
        "q-9c0d1e2f",
        "confirmatory",
        "- base/score: n=120, rho=0.61 [pre-specified]",
    );
    let ds = diagnostics(&p);
    let found = with_rule(&ds, "question.evidence-tag-mismatch");
    assert_eq!(found.len(), 1);
    assert!(found[0].hint.as_deref().unwrap().contains("pre-specified"));
}

#[test] // rq-f5e9098c
fn a_label_that_is_not_a_citation_label() {
    let p = TempProject::with_project(&three(
        OPEN,
        OPEN,
        &answered("x-vs-docking [pre-specified]"),
    ));
    p.study("x-vs-docking", "q-9c0d1e2f", "reported", &[]);
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "question.evidence-invalid").len(), 1);
}

#[test] // rq-c333e981
fn a_question_citation_carries_no_label() {
    let p = TempProject::with_project(&three(
        &answered("q-5e6f7a8b [exploratory]"),
        &answered("x-energy-accuracy"),
        OPEN,
    ));
    p.study("x-energy-accuracy", "q-5e6f7a8b", "reported", &[]);
    let ds = diagnostics(&p);
    assert_eq!(with_rule(&ds, "question.evidence-invalid").len(), 1);
}

#[test] // rq-df111cbc
fn citing_a_retrospective_study_with_a_verified_summary() {
    let p = TempProject::with_project(&three(OPEN, OPEN, &answered("old-benchmark [post hoc]")));
    reported_study(
        &p,
        "old-benchmark",
        "q-9c0d1e2f",
        "retrospective",
        "- base/score: n=60, MAE=0.9 kcal/mol [post hoc]",
    );
    let dir = p.root().join("studies/old-benchmark");
    fs::write(
        dir.join("SUMMARY.md"),
        "---\nstudy: old-benchmark\ncommit: \"0123456789abcdef0123456789abcdef01234567\"\ndirty: false\ngenerated: \"2026-10-01T12:00:00Z\"\ntophat_version: \"0.1.0\"\n---\n\n## Results\n\n- base/score: n=60, MAE=0.9 kcal/mol [post hoc]\n",
    )
    .unwrap();
    let ds = diagnostics(&p);
    assert!(
        with_rule(&ds, "question.evidence-retrospective").is_empty(),
        "{ds:#?}"
    );
}

#[test] // rq-a7910365
fn an_untagged_result_line_is_left_to_the_study_rules() {
    let p = TempProject::with_project(&three(OPEN, OPEN, &answered("x-vs-docking")));
    reported_study(
        &p,
        "x-vs-docking",
        "q-9c0d1e2f",
        "retrospective",
        "- base/score: n=120, rho=0.61",
    );
    let ds = run_check(p.root(), &[]).unwrap();
    assert!(ds.iter().any(|d| d.rule == "study.tag-missing"));
    assert!(with_rule(&ds, "question.evidence-tag-mismatch").is_empty());
}

#[test] // rq-e03f0826
fn tree_shows_an_origin_line_for_a_missing_study() {
    let p = TempProject::with_project(
        "# P\n\n## q-1a2b3c4d · A?\nstatus: emerged\n\nEmerged from: no-such-study\n",
    );
    let out = render_tree(p.root()).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    let idx = lines.iter().position(|l| l.contains("q-1a2b3c4d")).unwrap();
    assert_eq!(lines[idx + 1].trim(), "emerged from no-such-study [?]");
}

#[test] // rq-1176e768
fn emerge_from_a_study_that_does_not_exist() {
    let p = TempProject::with_project(EXAMPLE);
    let before = p.project_text();
    let err = emerge(p.root(), "A?", "no-such-study", None).unwrap_err();
    assert!(matches!(err, QuestionError::OriginStudyInvalid(s) if s == "no-such-study"));
    assert_eq!(p.project_text(), before);
}

#[test] // rq-8848775c
fn emerge_with_an_empty_text() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-vs-docking", "q-9c0d1e2f", "draft", &[]);
    let before = p.project_text();
    let err = emerge(p.root(), "", "x-vs-docking", None).unwrap_err();
    assert!(matches!(err, QuestionError::TextInvalid));
    assert_eq!(p.project_text(), before);
}

#[test] // rq-cdad3c4b
fn emerge_without_project_md() {
    let p = TempProject::new();
    p.study("x-vs-docking", "q-9c0d1e2f", "draft", &[]);
    let err = emerge(p.root(), "A?", "x-vs-docking", None).unwrap_err();
    assert!(matches!(err, QuestionError::ProjectFileMissing));
    assert!(!p.root().join("PROJECT.md").exists());
}

#[test] // rq-9ff67f40
fn emerge_from_a_study_whose_question_is_not_in_the_tree() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-vs-docking", "q-deadbeef", "draft", &[]);
    let err = emerge(p.root(), "A?", "x-vs-docking", None).unwrap_err();
    assert!(matches!(err, QuestionError::ParentInvalid(_)));
}

#[test] // rq-6c48cc09
fn emerge_places_the_question_as_the_parents_last_sub_question() {
    let p = TempProject::with_project(&format!(
        "# P\n\n{}{}{}",
        q(2, "q-1a2b3c4d", "A?", OPEN),
        q(3, "q-5e6f7a8b", "B?", OPEN),
        q(2, "q-9c0d1e2f", "C?", OPEN)
    ));
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    let (id, _) = emerge(p.root(), "Why did B behave oddly?", "x-vs-docking", None).unwrap();
    let text = p.project_text();
    let pos_new = text.find(&id).unwrap();
    let pos_b_body = text.find("B?").unwrap();
    let pos_c = text.find("q-9c0d1e2f").unwrap();
    assert!(pos_b_body < pos_new && pos_new < pos_c);
}

#[test] // rq-7853bb0a
fn emerge_with_an_explicit_parent() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-vs-docking", "q-9c0d1e2f", "draft", &[]);
    let (id, _) = emerge(
        p.root(),
        "Is the reference data reliable?",
        "x-vs-docking",
        Some("q-1a2b3c4d"),
    )
    .unwrap();
    let tree = parse_project(&p.project_text());
    let i = tree.index_of(&id).unwrap();
    assert_eq!(tree.questions[i].level, 3);
    let parent = tree.questions[i].parent.unwrap();
    assert_eq!(tree.questions[parent].id.as_deref(), Some("q-1a2b3c4d"));
}

#[test] // rq-9c8407f2
fn emerge_at_the_maximum_depth() {
    let mut text = String::from("# P\n\n");
    let ids = [
        "q-1a2b3c4d",
        "q-5e6f7a8b",
        "q-9c0d1e2f",
        "q-3b4c5d6e",
        "q-7d8e9f0a",
    ];
    for (i, id) in ids.iter().enumerate() {
        text.push_str(&q(i + 2, id, "Q?", OPEN));
    }
    let p = TempProject::with_project(&text);
    p.study("x-vs-docking", "q-7d8e9f0a", "draft", &[]);
    let before = p.project_text();
    let err = emerge(p.root(), "A?", "x-vs-docking", None).unwrap_err();
    assert!(matches!(err, QuestionError::ParentTooDeep));
    assert_eq!(p.project_text(), before);
}

#[test] // rq-6248ce74
fn emerge_preserves_the_rest_of_the_file() {
    let original = "# Title\n\n<!-- comment -->\n\n## q-1a2b3c4d · A?\nstatus: open\n\n```\n## not a question\n```\n";
    let crlf = original.replace('\n', "\r\n");
    let p = TempProject::with_project(&crlf);
    p.study("x-vs-docking", "q-1a2b3c4d", "draft", &[]);
    let (id, _) = emerge(p.root(), "A?", "x-vs-docking", None).unwrap();
    let after = p.project_text();
    assert!(after.contains(&id));
    let inserted = after.lines().find(|l| l.contains(&id)).unwrap().to_string();
    assert!(after.contains(&inserted.replace('\n', "\r\n")) || after.contains(&inserted));
    assert!(after.starts_with("# Title\r\n"));
    assert!(after.contains("<!-- comment -->"));
    assert!(after.contains("## not a question"));
}

#[test] // rq-c7e4904d
fn emerged_questions_get_distinct_ids() {
    let p = TempProject::with_project(EXAMPLE);
    p.study("x-vs-docking", "q-9c0d1e2f", "draft", &[]);
    let (id1, _) = emerge(p.root(), "A?", "x-vs-docking", None).unwrap();
    let (id2, _) = emerge(p.root(), "B?", "x-vs-docking", None).unwrap();
    assert_ne!(id1, id2);
    let tree = parse_project(&p.project_text());
    assert!(tree.index_of(&id1).is_some());
    assert!(tree.index_of(&id2).is_some());
}
