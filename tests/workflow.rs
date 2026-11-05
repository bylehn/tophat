//! Tests for `tophat status` and `tophat run` (rqm/workflow.md).
//!
//! `tophat run` is tested through the binary with a fake `pixi` that records its arguments. The
//! trial-run scenario needs `TOPHAT_TEST_SNAKEMAKE` (see tests/snakemake.rs).

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use tophat::check::{Severity, run_check};
use tophat::data::{Origin, RegisterOptions, add};
use tophat::pipeline::{
    KeyContext, ProvenanceRecord, SealRequest, compute_keys, generate_rules, load_pipeline, seal,
    summarize, write_summary,
};
use tophat::study::{
    Console, Status, StudyError, exploration_hash, new_study, parse_study_text, preregister,
    record_exploration, report, supersede,
};
use tophat::workflow::{Who, next_action, study_status};

// --- Fixtures -------------------------------------------------------------------------------

const PROJECT: &str = "# Method X

## q-1a2b3c4d · Can X improve affinity prediction?
status: open

### q-9c0d1e2f · Does X rank binders better than docking?
status: open
";

const STAGES: &str = "stages:
  - name: prepare
    run: sh scripts/prepare.sh {in.raw} {out}
    code: [scripts/prepare.sh]
    inputs: {raw: dataset:benchmark-set@v1}
    validate: find {out} -name '*.pdb' | grep -q .
  - name: score
    run: sh scripts/score.sh {item} {out} {params.level}
    code: [scripts/score.sh]
    inputs: {structures: stage:prepare}
    foreach: structures
    validate: test -s {out}/score.txt
    review: Do the scores look plausible for these structures?
    params: {level: b3lyp}
  - name: analyse
    run: sh scripts/analyse.sh {in.scores} {out}
    code: [scripts/analyse.sh]
    inputs: {scores: stage:score}
    result: result.txt
";

const PLAN: [(&str, &str); 4] = [
    ("Question", "Does X rank binders better than docking?"),
    (
        "Primary endpoint",
        "Spearman rho against measured affinity.",
    ),
    (
        "Decision rule",
        "X is better if rho exceeds docking's by 0.1.",
    ),
    (
        "Scenarios",
        "```gherkin\nFeature: f\n\n  @S1\n  Scenario: X ranks better\n    Then rho is higher\n```",
    ),
];

/// A fake pixi: records its arguments and working directory, then either runs a stage command
/// (`run … -- cmd`), runs Snakemake (`run snakemake …`, only when TOPHAT_TEST_REAL_SNAKEMAKE is
/// set), or exits with $FAKE_PIXI_EXIT.
#[cfg(unix)]
const FAKE_PIXI: &str = r#"#!/bin/sh
for a in "$@"; do case "$a" in --) mode=stage;; esac; done
if [ "$mode" = stage ]; then
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done; shift; exec "$@"
fi
if [ -n "$TOPHAT_TEST_REAL_SNAKEMAKE" ]; then shift; exec "$@"; fi
pwd > "$FAKE_PIXI_LOG.cwd"
printf '%s\n' "$@" > "$FAKE_PIXI_LOG"
exit "${FAKE_PIXI_EXIT:-0}"
"#;

struct Env {
    base: PathBuf,
}

impl Env {
    /// A committed project whose study `x-vs-docking` is a draft with its plan written and stages
    /// configured, and which has no check errors.
    fn draft() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-wf-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        let env = Env { base };
        for dir in ["project/.tophat", "raw/bench", "results", "bin"] {
            fs::create_dir_all(env.base.join(dir)).unwrap();
        }
        fs::write(env.base.join("raw/bench/1abc.pdb"), "ATOM 1\n").unwrap();
        fs::write(env.base.join("raw/bench/2xyz.pdb"), "ATOM 22\n").unwrap();
        env.write(".tophat/roots.yaml", "roots:\n  raw: {}\n  results: {}\n");
        env.configure(true);
        env.write(
            "pixi.lock",
            "version: 6\nenvironments:\n  default:\n    packages:\n      linux-64:\n      - conda: https://x/python.conda\n",
        );
        env.write("pixi.toml", "[workspace]\nname = \"p\"\n");
        env.write(
            ".gitignore",
            ".tophat/roots.local.yaml\n.tophat/cache/\n.tophat/generated/\n.snakemake/\n",
        );
        env.write("PROJECT.md", PROJECT);
        add(
            &env.root(),
            &RegisterOptions {
                name: "benchmark-set".into(),
                root: "raw".into(),
                path: "bench".into(),
                visibility: "public".into(),
                origin: Origin::Raw {
                    source: "test".into(),
                    date: None,
                    license: None,
                },
            },
        )
        .unwrap();
        env.new_study("x-vs-docking");
        env.write("studies/x-vs-docking/config/base.yaml", STAGES);
        env.write(
            "studies/x-vs-docking/config/pbe.yaml",
            "stages: [{name: score, params: {level: pbe}}]\n",
        );
        env.edit(
            "studies/x-vs-docking/STUDY.md",
            "variants:\n  - base\n",
            "variants:\n  - base\n  - pbe\n",
        );
        env.write(
            "studies/x-vs-docking/scripts/prepare.sh",
            "cp \"$1\"/*.pdb \"$2\"/\n",
        );
        env.write(
            "studies/x-vs-docking/scripts/score.sh",
            "wc -c < \"$1\" > \"$2/score.txt\"\n",
        );
        env.write(
            "studies/x-vs-docking/scripts/analyse.sh",
            "echo \"n=$(ls \"$1\"/items | wc -l)\" > \"$2/result.txt\"\n",
        );
        env.write("studies/x-vs-docking/tests/test_s1.sh", "# th:S1\n");
        env.git(&["init", "-q"]);
        env.commit();
        env
    }

    /// As [`Env::draft`], then pre-registered (through the interactive command) and committed.
    fn preregistered() -> Self {
        let env = Env::draft();
        env.preregister("x-vs-docking");
        env
    }

    /// A draft study created by `tophat study new` with its plan written.
    fn new_study(&self, id: &str) {
        new_study(&self.root(), id, "q-9c0d1e2f", false).unwrap();
        let rel = format!("studies/{id}/STUDY.md");
        for (section, content) in PLAN {
            let text = self.read(&rel);
            let heading = format!("## {section}\n\n");
            let start = text.find(&heading).unwrap() + heading.len();
            let end = text[start..]
                .find("\n## ")
                .map_or(text.len(), |i| start + i);
            self.write(
                &rel,
                &format!("{}{content}\n{}", &text[..start], &text[end..]),
            );
        }
    }

    fn preregister(&self, id: &str) {
        let mut input = Cursor::new(format!("{id}\n").into_bytes());
        let mut out = Vec::new();
        preregister(
            &self.root(),
            id,
            chrono::Local::now().date_naive(),
            &mut Console {
                interactive: true,
                input: &mut input,
                out: &mut out,
            },
        )
        .unwrap();
        self.commit();
    }

    fn root(&self) -> PathBuf {
        self.base.join("project")
    }

    fn configure(&self, results: bool) {
        let mut text = format!("roots:\n  raw: {}\n", self.base.join("raw").display());
        if results {
            text.push_str(&format!(
                "  results: {}\n",
                self.base.join("results").display()
            ));
        }
        self.write(".tophat/roots.local.yaml", &text);
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root().join(rel)).unwrap()
    }

    fn edit(&self, rel: &str, from: &str, to: &str) {
        let text = self.read(rel);
        assert!(text.contains(from), "{rel} lacks {from:?}");
        self.write(rel, &text.replacen(from, to, 1));
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", "fixture"]);
    }

    fn keys(&self, variant: &str) -> BTreeMap<String, String> {
        let config = load_pipeline(&self.root(), "x-vs-docking", variant).unwrap();
        compute_keys(&config, &KeyContext::new(&self.root()))
            .unwrap()
            .keys
    }

    fn out_dir(&self, stage: &str, key: &str) -> PathBuf {
        self.base
            .join("results")
            .join("x-vs-docking")
            .join(stage)
            .join(key)
    }

    fn seal(&self, stage: &str, variant: &str, item: Option<&str>) {
        seal(
            &self.root(),
            &SealRequest {
                study: "x-vs-docking".into(),
                stage: stage.into(),
                key: self.keys(variant)[stage].clone(),
                variant: Some(variant.into()),
                item: item.map(str::to_string),
                trial: false,
            },
        )
        .unwrap();
    }

    /// Seals stages of a variant by hand, up to and including `last`.
    fn run_by_hand(&self, variant: &str, last: &str) {
        let keys = self.keys(variant);
        let prepare = self.out_dir("prepare", &keys["prepare"]);
        if ProvenanceRecord::read(&prepare).is_none() {
            fs::create_dir_all(&prepare).unwrap();
            fs::write(prepare.join("1abc.pdb"), "ATOM 1\n").unwrap();
            fs::write(prepare.join("2xyz.pdb"), "ATOM 22\n").unwrap();
            self.seal("prepare", variant, None);
        }
        if last == "prepare" {
            return;
        }
        let score = self.out_dir("score", &keys["score"]);
        for item in ["1abc", "2xyz"] {
            fs::create_dir_all(score.join("items").join(item)).unwrap();
            fs::write(score.join("items").join(item).join("score.txt"), "1\n").unwrap();
            self.seal("score", variant, Some(item));
        }
        self.seal("score", variant, None);
        if last == "score" {
            return;
        }
        let analyse = self.out_dir("analyse", &keys["analyse"]);
        fs::create_dir_all(&analyse).unwrap();
        fs::write(analyse.join("result.txt"), format!("rho {variant}\n")).unwrap();
        self.seal("analyse", variant, None);
    }

    fn verdict(&self, variant: &str, verdict: &str) {
        let key = self.keys(variant)["score"].clone();
        self.write(
            &format!("studies/x-vs-docking/qc/score/{key}.md"),
            &format!("---\nstage: score\nkey: {key}\nverdict: {verdict}\ndate: \"2026-10-20\"\n---\n\nok\n"),
        );
    }

    /// Runs every stage by hand, records pass verdicts, writes the summary, and commits.
    fn complete(&self) {
        for v in ["base", "pbe"] {
            self.run_by_hand(v, "score");
            self.verdict(v, "pass");
            self.run_by_hand(v, "analyse");
        }
        write_summary(
            &self.root(),
            &summarize(&self.root(), "x-vs-docking").unwrap(),
        )
        .unwrap();
        self.commit();
    }

    fn next(&self) -> tophat::workflow::NextAction {
        next_action(&self.root(), "x-vs-docking").unwrap()
    }

    /// Runs the tophat binary with the fake pixi first on PATH.
    fn tophat(&self, args: &[&str], extra_env: &[(&str, &str)]) -> (i32, String, String) {
        let bin = self.base.join("bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let pixi = bin.join("pixi");
            if !pixi.exists() {
                fs::write(&pixi, FAKE_PIXI).unwrap();
                fs::set_permissions(&pixi, fs::Permissions::from_mode(0o755)).unwrap();
                std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tophat"), bin.join("tophat"))
                    .unwrap();
            }
        }
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
            .args(args)
            .current_dir(self.root())
            .env("PATH", path)
            .env("FAKE_PIXI_LOG", self.pixi_log())
            .envs(extra_env.iter().copied())
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn pixi_log(&self) -> PathBuf {
        self.base.join("pixi-args.txt")
    }

    /// The arguments the fake pixi was started with, or `None` if it was not started.
    fn pixi_args(&self) -> Option<Vec<String>> {
        fs::read_to_string(self.pixi_log())
            .ok()
            .map(|t| t.lines().map(str::to_string).collect())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn status_of(env: &Env) -> tophat::workflow::StudyStatus {
    study_status(&env.root(), &["x-vs-docking".to_string()])
        .unwrap()
        .remove(0)
}

#[test]
fn the_baseline_project_has_no_check_errors() {
    for env in [Env::draft(), Env::preregistered()] {
        let errors: Vec<_> = run_check(&env.root(), &[])
            .unwrap()
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{errors:#?}");
    }
}

// --- Next action ------------------------------------------------------------------------------

#[test] // rq-a3f2b786 rq-956cc0ad rq-bf93285a
fn a_fresh_draft_needs_its_plan_written() {
    let env = Env::draft();
    new_study(&env.root(), "fresh-study", "q-9c0d1e2f", false).unwrap();
    let next = next_action(&env.root(), "fresh-study").unwrap();
    assert_eq!((next.row, next.who), (3, Who::Anyone));
    assert_eq!(next.text, "Write the plan in STUDY.md");
}

#[test] // rq-2ec453d8 rq-956cc0ad
fn a_complete_draft_waits_for_the_human_to_pre_register() {
    let env = Env::draft();
    let next = env.next();
    assert_eq!((next.row, next.who), (4, Who::You));
    assert_eq!(
        next.command.as_deref(),
        Some("tophat study preregister x-vs-docking")
    );
    assert_eq!(
        next.line(),
        "next (you): Pre-register: tophat study preregister x-vs-docking"
    );
}

#[test] // rq-5a59811a rq-956cc0ad
fn check_errors_come_first() {
    let env = Env::draft();
    env.edit(
        "studies/x-vs-docking/STUDY.md",
        "status: draft",
        "status: finished",
    );
    let next = env.next();
    assert_eq!(next.row, 2);
    assert_eq!(
        next.command.as_deref(),
        Some("tophat check studies/x-vs-docking")
    );
}

#[test] // rq-4d4f5fd4 rq-b1f6d54f
fn errors_in_another_study_do_not_block_this_one() {
    let env = Env::draft();
    fs::create_dir_all(env.root().join("studies/other-study")).unwrap();
    assert!(
        run_check(&env.root(), &[])
            .unwrap()
            .iter()
            .any(|d| d.path == "studies/other-study")
    );
    assert_eq!(env.next().row, 4);
}

#[test] // rq-ee88aae5 rq-b1f6d54f
fn errors_in_a_referenced_dataset_block_the_study() {
    let env = Env::preregistered();
    env.edit(
        "datasets/benchmark-set/v1.yaml",
        "content_hash: \"sha256:",
        "content_hash: \"sha256:f",
    );
    assert_eq!(env.next().row, 2);
}

#[test] // rq-eebf1864 rq-956cc0ad
fn preregistered_study_without_stages() {
    let env = Env::preregistered();
    env.write("studies/x-vs-docking/config/base.yaml", "stages: []\n");
    env.write("studies/x-vs-docking/config/pbe.yaml", "stages: []\n");
    let next = env.next();
    assert_eq!(
        (next.row, next.text.as_str()),
        (5, "Add stages to config/base.yaml")
    );
}

#[test] // rq-cde76c0f rq-956cc0ad
fn a_configured_results_location_that_does_not_exist() {
    let env = Env::preregistered();
    let missing = env.base.join("no-such-results");
    env.write(
        ".tophat/roots.local.yaml",
        &format!(
            "roots:\n  raw: {}\n  results: {}\n",
            env.base.join("raw").display(),
            missing.display()
        ),
    );
    let status = status_of(&env);
    assert_eq!(status.next.row, 6, "{:?}", status.next);
    assert!(
        status.next.text.contains(&missing.display().to_string()),
        "{:?}",
        status.next
    );
}

#[test] // rq-1bd248f0 rq-956cc0ad rq-6733b7c0
fn the_default_results_folder_needs_no_configuration() {
    let env = Env::preregistered();
    env.configure(false);
    assert!(!env.root().join("results").exists());
    let status = status_of(&env);
    assert_ne!(status.next.row, 6, "{:?}", status.next);
    let (_, out, _) = env.tophat(&["status", "x-vs-docking"], &[]);
    assert!(
        out.contains("pipeline: base 0/3 sealed, pbe 0/3 sealed"),
        "{out}"
    );
}

#[test] // rq-9bb38d37 rq-956cc0ad
fn waiting_for_a_qc_verdict() {
    let env = Env::preregistered();
    env.run_by_hand("pbe", "score");
    let next = env.next();
    assert_eq!((next.row, next.who), (9, Who::You));
    assert_eq!(
        next.command.as_deref(),
        Some("tophat stage qc x-vs-docking score --variant pbe")
    );
}

#[test] // rq-032bd279 rq-956cc0ad rq-c5b9e99a
fn tools_not_fetched_on_this_machine() {
    let env = Env::preregistered();
    let repo = env.base.join("method-x");
    fs::create_dir_all(&repo).unwrap();
    let git_in = |args: &[&str]| {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git_in(&["init", "-q"]);
    fs::write(repo.join("tool.sh"), "echo tool\n").unwrap();
    git_in(&["add", "-A"]);
    git_in(&["commit", "-q", "-m", "A"]);
    let commit = git_in(&["rev-parse", "HEAD"]);
    env.write(
        ".tophat/tools.yaml",
        &format!("tools:\n  method-x:\n    url: {}\n", repo.display()),
    );
    let config = env.read("studies/x-vs-docking/config/base.yaml");
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &format!("{config}  - name: extra\n    tool: method-x@{commit}\n    run: sh {{tool}}/tool.sh > {{out}}/x\n"),
    );
    let next = env.next();
    assert_eq!((next.row, next.who), (7, Who::You), "{next:?}");
    assert_eq!(next.command.as_deref(), Some("tophat tools sync"));
    let (code, _, err) = env.tophat(&["run", "x-vs-docking"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("tophat tools sync"), "{err}");
    assert!(env.pixi_args().is_none());
}

#[test] // rq-f3b637ee rq-956cc0ad
fn external_job_waiting_for_the_human() {
    let env = Env::preregistered();
    let config = env.read("studies/x-vs-docking/config/base.yaml");
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &format!(
            "{config}  - name: md\n    run: cp {{in.system}}/1abc.pdb {{out}}/\n    inputs: {{system: stage:prepare}}\n    external: {{}}\n"
        ),
    );
    env.commit();
    env.run_by_hand("base", "prepare");
    let key = env.keys("base")["md"].clone();
    tophat::pipeline::write_job_folder(
        &env.root(),
        &SealRequest {
            study: "x-vs-docking".into(),
            stage: "md".into(),
            key,
            variant: Some("base".into()),
            item: None,
            trial: false,
        },
    )
    .unwrap();
    let next = env.next();
    assert_eq!((next.row, next.who), (8, Who::You), "{next:?}");
    assert_eq!(
        next.command.as_deref(),
        Some("tophat stage instructions x-vs-docking md --variant base")
    );
    assert!(next.line().starts_with("next (you): "), "{}", next.line());
}

#[test] // rq-14542671 rq-956cc0ad
fn failed_qc() {
    let env = Env::preregistered();
    env.run_by_hand("base", "score");
    env.verdict("base", "fail");
    let next = env.next();
    assert_eq!(next.row, 10);
    assert!(
        next.text.contains("score") && next.text.contains("base"),
        "{}",
        next.text
    );
}

#[test] // rq-4b2809d2 rq-956cc0ad rq-02e65d51
fn stages_left_to_run() {
    let env = Env::preregistered();
    env.run_by_hand("base", "prepare");
    let status = status_of(&env);
    let pipeline = status.pipeline.unwrap();
    assert_eq!(pipeline[0], ("base".to_string(), Some((1, 3))));
    assert_eq!(pipeline[1], ("pbe".to_string(), Some((1, 3))));
    assert_eq!(status.next.row, 11);
    assert_eq!(
        status.next.command.as_deref(),
        Some("tophat run x-vs-docking")
    );
}

#[cfg(unix)]
#[test] // rq-b1a89dcb rq-956cc0ad
fn stale_summary_means_run_again() {
    let env = Env::preregistered();
    env.complete();
    assert_eq!(env.next().row, 13);
    let key = env.keys("base")["analyse"].clone();
    env.edit(
        "studies/x-vs-docking/SUMMARY.md",
        &format!("\"base/analyse\": \"{key}\""),
        "\"base/analyse\": \"sk-0000000000000000\"",
    );
    assert_eq!(env.next().row, 11);
    // The stale summary is a check error, yet it does not stop the run that fixes it.
    assert!(
        run_check(&env.root(), &[])
            .unwrap()
            .iter()
            .any(|d| d.rule == "pipeline.summary-stale")
    );
    assert_eq!(env.tophat(&["run", "x-vs-docking"], &[]).0, 0);
}

#[test] // rq-82ac8f57 rq-956cc0ad
fn dirty_summary_means_commit_and_run_again() {
    let env = Env::preregistered();
    let committed = env.read("pixi.toml");
    env.write("pixi.toml", "[workspace]\nname = \"wip\"\n");
    env.run_by_hand("base", "prepare");
    env.write("pixi.toml", &committed);
    env.complete();
    let next = env.next();
    assert_eq!(next.row, 12);
    assert_eq!(next.command.as_deref(), Some("tophat run x-vs-docking"));
}

#[test] // rq-f0049ad3 rq-956cc0ad
fn ready_to_report() {
    let env = Env::preregistered();
    env.complete();
    let next = env.next();
    assert_eq!(next.row, 13);
    assert_eq!(
        next.command.as_deref(),
        Some("tophat study report x-vs-docking")
    );
}

#[test] // rq-13a88566 rq-956cc0ad
fn reported_study_whose_question_is_still_open() {
    let env = Env::preregistered();
    env.complete();
    report(&env.root(), "x-vs-docking").unwrap();
    let next = env.next();
    assert_eq!(next.row, 14);
    assert!(next.text.contains("q-9c0d1e2f"));
}

#[test] // rq-62e72400 rq-956cc0ad
fn reported_study_that_is_cited_is_done() {
    let env = Env::preregistered();
    env.complete();
    report(&env.root(), "x-vs-docking").unwrap();
    env.edit(
        "PROJECT.md",
        "### q-9c0d1e2f · Does X rank binders better than docking?\nstatus: open\n",
        "### q-9c0d1e2f · Does X rank binders better than docking?\nstatus: answered\n\n**Answer:** Yes.\nEvidence: x-vs-docking\n",
    );
    let next = env.next();
    assert_eq!(
        (next.row, next.text.as_str(), next.who),
        (15, "None: done", Who::Nobody)
    );
}

#[test] // rq-dffba6fd rq-6733b7c0
fn superseded_study() {
    let env = Env::preregistered();
    env.new_study("old-study");
    env.preregister("old-study");
    supersede(&env.root(), "old-study", "x-vs-docking").unwrap();
    let (code, out, _) = env.tophat(&["status"], &[]);
    assert_eq!(code, 0);
    assert!(
        out.trim_end()
            .ends_with("old-study  superseded by x-vs-docking"),
        "{out}"
    );
    assert_eq!(next_action(&env.root(), "old-study").unwrap().row, 1);
}

#[test] // rq-2b6e16c6 rq-956cc0ad
fn retrospective_study_without_a_pipeline() {
    let env = Env::draft();
    env.new_study("old-benchmark");
    env.edit(
        "studies/old-benchmark/STUDY.md",
        "status: draft",
        "status: retrospective",
    );
    env.edit(
        "studies/old-benchmark/STUDY.md",
        "## Result\n",
        "## Result\n\n- base/score: rho=0.5 (2024)\n",
    );
    env.edit(
        "PROJECT.md",
        "### q-9c0d1e2f · Does X rank binders better than docking?\nstatus: open\n",
        "### q-9c0d1e2f · Does X rank binders better than docking?\nstatus: answered\n\n**Answer:** Partly.\nEvidence: old-benchmark\n",
    );
    let next = next_action(&env.root(), "old-benchmark").unwrap();
    assert_eq!(next.row, 15, "{next:?}");
}

// --- Status output ----------------------------------------------------------------------------

#[test] // rq-018e0aed rq-6733b7c0
fn project_line() {
    let env = Env::draft();
    env.write(
        "PROJECT.md",
        "# P\n\n## q-1a2b3c4d · A?\nstatus: open\n\n### q-9c0d1e2f · B?\nstatus: open\n\n\
         ### q-3b4c5d6e · C?\nstatus: answered\n\n**Answer:** Yes.\nEvidence: no-such-study\n\n\
         ### q-5e6f7a8b · D?\nstatus: dropped\n\n**Reason:** Out of scope.\n",
    );
    env.edit(
        "studies/x-vs-docking/STUDY.md",
        "status: draft\n",
        "status: draft\nfoo: 1\nbar: 2\n",
    );
    let (code, out, _) = env.tophat(&["status"], &[]);
    assert_eq!(code, 0);
    assert_eq!(
        out.lines().next().unwrap(),
        "questions: 2 open, 1 answered, 1 dropped; check: 1 error(s), 2 warning(s), 0 legacy item(s)"
    );
}

#[test] // rq-682322e9 rq-6733b7c0
fn json_status_for_agents() {
    let env = Env::preregistered();
    env.run_by_hand("pbe", "score");
    let (code, out, _) = env.tophat(&["status", "x-vs-docking", "--json"], &[]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let next = &json["studies"][0]["next"];
    assert_eq!(next["row"], 9);
    assert_eq!(next["who"], "you");
    assert_eq!(
        next["command"],
        "tophat stage qc x-vs-docking score --variant pbe"
    );
    assert!(next["text"].as_str().unwrap().contains("QC verdict"));
    assert_eq!(json["studies"][0]["pipeline"]["pbe"]["sealed"], 2);
}

/// Path → contents for every file below `root`.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                map.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    map
}

#[test] // rq-8a1808bb rq-6733b7c0
fn status_is_read_only() {
    let env = Env::preregistered();
    env.run_by_hand("base", "score");
    let before = snapshot(&env.base);
    let (code, _, _) = env.tophat(&["status"], &[]);
    assert_eq!(code, 0);
    let after = snapshot(&env.base);
    let changed: Vec<_> = after
        .keys()
        .chain(before.keys())
        .filter(|p| before.get(*p) != after.get(*p) && !p.starts_with(env.base.join("bin")))
        .collect();
    assert!(changed.is_empty(), "{changed:?}");
}

#[test] // rq-6e632e7d rq-6733b7c0
fn unknown_study() {
    let env = Env::draft();
    assert_eq!(env.tophat(&["status", "no-such-study"], &[]).0, 1);
}

#[test] // rq-7b3ae395 rq-956cc0ad
fn unparseable_study() {
    let env = Env::draft();
    env.write("studies/broken/STUDY.md", "no front matter\n");
    assert_eq!(next_action(&env.root(), "broken").unwrap().row, 2);
}

// --- tophat run -------------------------------------------------------------------------------

#[cfg(unix)]
#[test] // rq-c8940b0d rq-cdc39471
fn run_a_preregistered_study() {
    let env = Env::preregistered();
    let (code, out, err) = env.tophat(&["run", "x-vs-docking", "--cores", "4"], &[]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        env.pixi_args().unwrap().join(" "),
        "run snakemake -s studies/x-vs-docking/Snakefile --cores 4 summary"
    );
    let cwd = fs::read_to_string(format!("{}.cwd", env.pixi_log().display())).unwrap();
    assert_eq!(
        fs::canonicalize(cwd.trim()).unwrap(),
        fs::canonicalize(env.root()).unwrap()
    );
    assert!(
        out.trim_end()
            .ends_with("next (anyone): Run the pipeline: tophat run x-vs-docking"),
        "{out}"
    );
}

#[cfg(unix)]
#[test] // rq-a3466977 rq-cdc39471
fn run_one_variant() {
    let env = Env::preregistered();
    env.tophat(&["run", "x-vs-docking", "--variant", "pbe"], &[]);
    assert_eq!(env.pixi_args().unwrap().last().unwrap(), "variant_pbe");
}

#[cfg(unix)]
#[test] // rq-24702b2d rq-cdc39471
fn default_cores() {
    let env = Env::preregistered();
    env.tophat(&["run", "x-vs-docking"], &[]);
    let args = env.pixi_args().unwrap();
    let cores = std::thread::available_parallelism()
        .unwrap()
        .get()
        .to_string();
    assert_eq!(
        args[args.iter().position(|a| a == "--cores").unwrap() + 1],
        cores
    );
}

#[cfg(unix)]
#[test] // rq-a9d102bf rq-cdc39471
fn extra_snakemake_arguments_are_passed_through() {
    let env = Env::preregistered();
    env.tophat(
        &["run", "x-vs-docking", "--", "--dry-run", "--printshellcmds"],
        &[],
    );
    assert!(
        env.pixi_args()
            .unwrap()
            .join(" ")
            .ends_with("summary --dry-run --printshellcmds")
    );
}

#[cfg(unix)]
#[test] // rq-d1d30204 rq-cdc39471
fn snakemakes_exit_code_is_returned() {
    let env = Env::preregistered();
    let (code, out, _) = env.tophat(&["run", "x-vs-docking"], &[("FAKE_PIXI_EXIT", "1")]);
    assert_eq!(code, 1);
    assert!(out.contains("next (anyone):"), "{out}");
}

#[cfg(unix)]
#[test] // rq-5e257285 rq-cdc39471
fn stopped_at_a_qc_gate() {
    let env = Env::preregistered();
    env.run_by_hand("base", "score");
    let (code, out, _) = env.tophat(&["run", "x-vs-docking"], &[("FAKE_PIXI_EXIT", "1")]);
    assert_eq!(code, 1);
    assert!(
        out.trim_end().ends_with(
            "next (you): Give a QC verdict: tophat stage qc x-vs-docking score --variant base"
        ),
        "{out}"
    );
}

#[test] // rq-d76deef1 rq-cdc39471
fn draft_refused_without_draft_flag() {
    let env = Env::draft();
    let (code, _, err) = env.tophat(&["run", "x-vs-docking"], &[]);
    assert_eq!(code, 1);
    assert!(
        err.contains("pre-register") && err.contains("--draft"),
        "{err}"
    );
    assert!(env.pixi_args().is_none());
}

#[cfg(unix)]
#[test] // rq-23d4abbf rq-cdc39471 rq-0d5e77fe
fn trial_run_of_a_draft() {
    let env = Env::draft();
    let (code, out, err) = env.tophat(&["run", "x-vs-docking", "--draft"], &[]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(env.pixi_args().unwrap().last().unwrap(), "trial");
    assert!(out.contains("can never be reported"), "{out}");
}

#[cfg(unix)]
#[test] // rq-eb7be7fe rq-cdc39471
fn trial_run_of_one_variant() {
    let env = Env::draft();
    env.tophat(&["run", "x-vs-docking", "--draft", "--variant", "pbe"], &[]);
    assert_eq!(env.pixi_args().unwrap().last().unwrap(), "trial_pbe");
}

#[test] // rq-88a10bea rq-cdc39471
fn draft_flag_on_a_preregistered_study_is_refused() {
    let env = Env::preregistered();
    assert_eq!(env.tophat(&["run", "x-vs-docking", "--draft"], &[]).0, 1);
    assert!(env.pixi_args().is_none());
}

#[test] // rq-d14816cc rq-cdc39471
fn superseded_study_is_refused() {
    let env = Env::preregistered();
    env.new_study("old-study");
    env.write(
        "studies/old-study/config/base.yaml",
        "stages:\n  - name: a\n    run: echo {out}\n",
    );
    env.preregister("old-study");
    supersede(&env.root(), "old-study", "x-vs-docking").unwrap();
    assert_eq!(env.tophat(&["run", "old-study"], &[]).0, 1);
    assert!(env.pixi_args().is_none());
}

#[test] // rq-fd9f1a7c rq-cdc39471
fn check_errors_are_refused() {
    let env = Env::preregistered();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "{in.scores} {out}",
        "{in.scores} {output}",
    );
    let (code, _, err) = env.tophat(&["run", "x-vs-docking"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("pipeline.config-invalid"), "{err}");
    assert!(env.pixi_args().is_none());
}

#[cfg(unix)]
#[test] // rq-2e3b70f6 rq-cdc39471
fn warnings_do_not_block_a_run() {
    let env = Env::preregistered();
    env.edit(
        "studies/x-vs-docking/STUDY.md",
        "visibility: public\n",
        "visibility: public\nnote: wip\n",
    );
    let warnings = run_check(&env.root(), &[]).unwrap();
    assert!(warnings.iter().any(|d| d.rule == "study.field-unknown"));
    assert_eq!(env.tophat(&["run", "x-vs-docking"], &[]).0, 0);
    assert!(env.pixi_args().is_some());
}

#[test] // rq-6e13314c rq-cdc39471
fn unknown_variant() {
    let env = Env::preregistered();
    assert_eq!(
        env.tophat(&["run", "x-vs-docking", "--variant", "nope"], &[])
            .0,
        1
    );
    assert!(env.pixi_args().is_none());
}

#[test] // rq-d17bfed2 rq-cdc39471
fn study_without_stages() {
    let env = Env::preregistered();
    env.write("studies/x-vs-docking/config/base.yaml", "stages: []\n");
    let (code, _, err) = env.tophat(&["run", "x-vs-docking"], &[]);
    assert_eq!(code, 1);
    assert!(err.contains("add stages"), "{err}");
}

#[test] // rq-3d60791d rq-cdc39471
fn pixi_not_available() {
    let env = Env::preregistered();
    let empty = env.base.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["run", "x-vs-docking"])
        .current_dir(env.root())
        .env("PATH", &empty)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("pixi could not be started"));
}

// --- Trial runs -------------------------------------------------------------------------------

#[cfg(unix)]
#[test] // rq-2a741327 rq-0d5e77fe rq-2e420949
fn trial_outputs_are_separate_and_unreportable() {
    let env = Env::draft();
    let Some(snakemake) = common::snakemake() else {
        return;
    };
    let bin = env.base.join("bin");
    let real = [("TOPHAT_TEST_REAL_SNAKEMAKE", "1")];
    env.tophat(&["--version"], &[]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&snakemake, bin.join("snakemake")).unwrap();
    let (code, out, err) = env.tophat(&["run", "x-vs-docking", "--draft", "--cores", "2"], &real);
    assert_eq!(code, 0, "{out}{err}");
    let trial = env.base.join("results/x-vs-docking/.trial");
    for v in ["base", "pbe"] {
        for (stage, key) in env.keys(v) {
            let record = ProvenanceRecord::read(&trial.join(&stage).join(&key)).unwrap();
            assert!(record.trial, "{stage}");
        }
    }
    assert!(!env.root().join("studies/x-vs-docking/SUMMARY.md").exists());
    assert!(!env.root().join("studies/x-vs-docking/qc").exists());
    let exploration = env.read("studies/x-vs-docking/EXPLORATION.md");
    assert!(
        exploration.contains("- base/analyse: n=2 ("),
        "{exploration}"
    );
    assert!(
        exploration.contains("- pbe/analyse: n=2 ("),
        "{exploration}"
    );
    assert!(
        ProvenanceRecord::read(&env.out_dir("prepare", &env.keys("base")["prepare"])).is_none()
    );

    env.preregister("x-vs-docking");
    let (code, _, _) = env.tophat(&["run", "x-vs-docking", "--cores", "2"], &real);
    assert_ne!(code, 0, "stops at the QC gate");
    for v in ["base", "pbe"] {
        env.verdict(v, "pass");
    }
    env.commit();
    let (code, out, err) = env.tophat(&["run", "x-vs-docking", "--cores", "2"], &real);
    assert_eq!(code, 0, "{out}{err}");
    for v in ["base", "pbe"] {
        for (stage, key) in env.keys(v) {
            let record = ProvenanceRecord::read(&env.out_dir(&stage, &key)).unwrap();
            assert!(!record.trial, "{stage}");
        }
    }
    assert!(
        out.trim_end()
            .ends_with("next (anyone): Report: tophat study report x-vs-docking"),
        "{out}"
    );
}

#[test] // rq-256e64ad rq-0d5e77fe
fn direct_snakemake_on_a_draft_only_offers_trial_targets() {
    let env = Env::draft();
    let rules = generate_rules(&env.root(), "x-vs-docking").unwrap();
    assert!(
        rules.contains("rule trial:")
            && rules.contains("rule trial_base:")
            && rules.contains("rule trial_pbe:")
    );
    assert!(!rules.contains("rule summary:") && !rules.contains("rule variant_"));
}

// --- Exploration (rqm/study/exploration.md) ---------------------------------------------------

impl Env {
    /// Registers `benchmark-pilot@v1`, a one-structure pilot of the benchmark set.
    fn add_pilot(&self) {
        fs::create_dir_all(self.base.join("raw/pilot")).unwrap();
        fs::write(self.base.join("raw/pilot/1abc.pdb"), "ATOM 1\n").unwrap();
        add(
            &self.root(),
            &RegisterOptions {
                name: "benchmark-pilot".into(),
                root: "raw".into(),
                path: "pilot".into(),
                visibility: "public".into(),
                origin: Origin::Raw {
                    source: "test".into(),
                    date: None,
                    license: None,
                },
            },
        )
        .unwrap();
    }

    /// Gives stage `prepare` the trial mapping `trial`.
    fn prepare_trial(&self, trial: &str) {
        self.edit(
            "studies/x-vs-docking/config/base.yaml",
            "    inputs: {raw: dataset:benchmark-set@v1}\n",
            &format!("    inputs: {{raw: dataset:benchmark-set@v1}}\n    trial: {trial}\n"),
        );
    }

    fn trial_keys(&self, variant: &str) -> BTreeMap<String, String> {
        let config = load_pipeline(&self.root(), "x-vs-docking", variant)
            .unwrap()
            .for_trial();
        compute_keys(&config, &KeyContext::new(&self.root()))
            .unwrap()
            .keys
    }

    fn trial_seal(&self, stage: &str, variant: &str, item: Option<&str>) {
        seal(
            &self.root(),
            &SealRequest {
                study: "x-vs-docking".into(),
                stage: stage.into(),
                key: self.trial_keys(variant)[stage].clone(),
                variant: Some(variant.into()),
                item: item.map(str::to_string),
                trial: true,
            },
        )
        .unwrap();
    }

    fn trial_dir(&self, stage: &str, key: &str) -> PathBuf {
        self.base
            .join("results")
            .join("x-vs-docking")
            .join(".trial")
            .join(stage)
            .join(key)
    }

    /// Seals every stage of the base variant as a trial, by hand, with result line `result`.
    fn trial_by_hand(&self, result: &str) {
        let keys = self.trial_keys("base");
        let prepare = self.trial_dir("prepare", &keys["prepare"]);
        fs::create_dir_all(&prepare).unwrap();
        fs::write(prepare.join("1abc.pdb"), "ATOM 1\n").unwrap();
        self.trial_seal("prepare", "base", None);
        let score = self.trial_dir("score", &keys["score"]);
        fs::create_dir_all(score.join("items/1abc")).unwrap();
        fs::write(score.join("items/1abc/score.txt"), "1\n").unwrap();
        self.trial_seal("score", "base", Some("1abc"));
        self.trial_seal("score", "base", None);
        let analyse = self.trial_dir("analyse", &keys["analyse"]);
        fs::create_dir_all(&analyse).unwrap();
        fs::write(analyse.join("result.txt"), format!("{result}\n")).unwrap();
        self.trial_seal("analyse", "base", None);
    }

    /// Replaces the pipeline with one stage that generates its result from a seed.
    fn seeded(&self, trial: &str) {
        fs::remove_file(self.root().join("studies/x-vs-docking/config/pbe.yaml")).unwrap();
        self.edit(
            "studies/x-vs-docking/STUDY.md",
            "variants:\n  - base\n  - pbe\n",
            "variants:\n  - base\n",
        );
        self.write(
            "studies/x-vs-docking/config/base.yaml",
            &format!(
                "stages:\n  - name: generate\n    run: echo {{params.seed}} > {{out}}/result.txt\n    params: {{seed: 1}}\n    result: result.txt\n{trial}"
            ),
        );
        let key = self.trial_keys("base")["generate"].clone();
        let dir = self.trial_dir("generate", &key);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("result.txt"), "1\n").unwrap();
        self.trial_seal("generate", "base", None);
    }

    fn exploration(&self) -> String {
        record_exploration(&self.root(), "x-vs-docking").unwrap();
        self.read("studies/x-vs-docking/EXPLORATION.md")
    }

    /// Pre-registers interactively and returns what was printed.
    fn preregister_output(&self) -> String {
        let mut input = Cursor::new(b"x-vs-docking\n".to_vec());
        let mut out = Vec::new();
        preregister(
            &self.root(),
            "x-vs-docking",
            chrono::Local::now().date_naive(),
            &mut Console {
                interactive: true,
                input: &mut input,
                out: &mut out,
            },
        )
        .unwrap();
        self.commit();
        String::from_utf8(out).unwrap()
    }

    fn rules(&self) -> Vec<String> {
        run_check(&self.root(), &[])
            .unwrap()
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| format!("{} {}", d.rule, d.message))
            .collect()
    }
}

#[test] // rq-a65aea60 rq-52742c3b
fn trial_runs_use_the_trial_inputs() {
    let env = Env::draft();
    env.add_pilot();
    env.prepare_trial("{inputs: {raw: dataset:benchmark-pilot@v1}}");
    env.trial_by_hand("rho=0.42");
    let key = env.trial_keys("base")["prepare"].clone();
    let record = ProvenanceRecord::read(&env.trial_dir("prepare", &key)).unwrap();
    assert!(record.trial);
    assert_eq!(record.inputs[0]["name"], "raw");
    assert_eq!(record.inputs[0]["ref"], "benchmark-pilot@v1");
}

#[test] // rq-a96b7d8f rq-52742c3b
fn the_trial_mapping_does_not_enter_the_locked_key() {
    let env = Env::draft();
    let before = (env.keys("base"), env.keys("pbe"));
    env.prepare_trial("{params: {n: 10}}");
    assert_eq!((env.keys("base"), env.keys("pbe")), before);
    assert_ne!(env.trial_keys("base")["prepare"], before.0["prepare"]);
}

#[test] // rq-864e9f23 rq-52742c3b
fn a_trial_input_must_be_an_input_of_the_stage() {
    let env = Env::draft();
    env.add_pilot();
    env.prepare_trial("{inputs: {other: dataset:benchmark-pilot@v1}}");
    let errors = env.rules();
    assert!(
        errors
            .iter()
            .any(|e| e.starts_with("pipeline.config-invalid") && e.contains("other")),
        "{errors:?}"
    );
}

#[test] // rq-246da339 rq-52742c3b
fn a_trial_mapping_holds_only_inputs_and_params() {
    let env = Env::draft();
    env.prepare_trial("{run: echo x}");
    let errors = env.rules();
    assert!(
        errors
            .iter()
            .any(|e| e.starts_with("pipeline.config-invalid") && e.contains("run")),
        "{errors:?}"
    );
}

#[test] // rq-3842f534 rq-45ae65c9 rq-983f740b
fn exploring_on_a_pilot_leaves_the_deciding_data_unseen() {
    let env = Env::draft();
    env.add_pilot();
    env.prepare_trial("{inputs: {raw: dataset:benchmark-pilot@v1}}");
    env.trial_by_hand("rho=0.42");
    let text = env.exploration();
    assert!(
        text.contains("- dataset benchmark-pilot@v1, by stage prepare\n"),
        "{text}"
    );
    assert!(
        text.contains("- dataset benchmark-set@v1: not used by any trial run\n"),
        "{text}"
    );
    assert!(text.contains("- no trial run computed an output of the locked pipeline\n"));
    assert!(text.contains("\ndeciding_data_seen: false\n"), "{text}");
}

#[test] // rq-dc5dfbc8 rq-45ae65c9 rq-983f740b
fn exploring_on_the_deciding_dataset_is_seen() {
    let env = Env::draft();
    env.trial_by_hand("rho=0.42");
    let text = env.exploration();
    assert!(
        text.contains("- dataset benchmark-set@v1: used by a trial run\n"),
        "{text}"
    );
    assert!(text.contains("\ndeciding_data_seen: true\n"), "{text}");
}

#[test] // rq-019ede34 rq-45ae65c9
fn an_identical_trial_computation_is_seen() {
    let env = Env::draft();
    env.seeded("");
    let text = env.exploration();
    assert!(
        text.contains("- a trial run computed an output of the locked pipeline\n"),
        "{text}"
    );
    assert!(text.contains("\ndeciding_data_seen: true\n"), "{text}");
}

#[test] // rq-89cbcbc0 rq-45ae65c9
fn a_different_trial_seed_keeps_synthetic_data_unseen() {
    let env = Env::draft();
    env.seeded("    trial: {params: {seed: 2}}\n");
    let text = env.exploration();
    assert!(text.contains("- no trial run computed an output of the locked pipeline\n"));
    assert!(text.contains("\ndeciding_data_seen: false\n"), "{text}");
}

#[test] // rq-ce8f7bac rq-983f740b rq-36d7d1e3 rq-d0c8b971
fn trial_results_and_exploration_files_are_listed() {
    let env = Env::draft();
    env.add_pilot();
    env.prepare_trial("{inputs: {raw: dataset:benchmark-pilot@v1}}");
    env.trial_by_hand("rho=0.42");
    env.write("exploration/x-vs-docking/notes.ipynb", "{}\n");
    let exploration = tophat::study::explore(&env.root(), "x-vs-docking").unwrap();
    assert!(!exploration.deciding_data_seen);
    let text = env.exploration();
    let key = &env.trial_keys("base")["analyse"];
    assert!(
        text.contains(&format!("- base/analyse: rho=0.42 ({key})\n")),
        "{text}"
    );
    let pbe = &env.trial_keys("pbe")["analyse"];
    assert!(
        text.contains(&format!("- pbe/analyse: not run ({pbe})\n")),
        "{text}"
    );
    let sha = tophat::data::hash_file(&env.root().join("exploration/x-vs-docking/notes.ipynb"))
        .unwrap()
        .1;
    assert!(
        text.contains(&format!(
            "- exploration/x-vs-docking/notes.ipynb sha256:{sha}\n"
        )),
        "{text}"
    );
}

#[test] // rq-e3a73fb1 rq-bc7ec152
fn exploration_only_for_drafts() {
    let env = Env::preregistered();
    assert!(matches!(
        record_exploration(&env.root(), "x-vs-docking"),
        Err(StudyError::ExplorationLocked(_))
    ));
    let (code, _, err) = env.tophat(&["study", "exploration", "x-vs-docking"], &[]);
    assert_eq!(code, 1);
    assert!(
        err.contains("recorded in EXPLORATION.md when the plan was locked"),
        "{err}"
    );
}

#[test] // rq-b3af0612 rq-79590bc8 rq-bc7ec152
fn rewriting_an_unchanged_exploration_changes_nothing() {
    let env = Env::draft();
    env.trial_by_hand("rho=0.42");
    let rel = "studies/x-vs-docking/EXPLORATION.md";
    let text = env.exploration();
    let generated = text.lines().find(|l| l.starts_with("generated:")).unwrap();
    let old = text.replace(generated, "generated: \"2026-01-01T00:00:00Z\"");
    env.write(rel, &old);
    let (code, out, err) = env.tophat(&["study", "exploration", "x-vs-docking"], &[]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("Unchanged: studies/x-vs-docking/EXPLORATION.md"),
        "{out}"
    );
    assert!(out.contains("the deciding data"), "{out}");
    assert_eq!(env.read(rel), old);
}

#[test] // rq-7aef1eb3 rq-fea6913a
fn pre_registration_records_the_exploration() {
    let env = Env::draft();
    env.add_pilot();
    env.prepare_trial("{inputs: {raw: dataset:benchmark-pilot@v1}}");
    env.trial_by_hand("rho=0.42");
    env.commit();
    env.preregister_output();
    let exploration = env.read("studies/x-vs-docking/EXPLORATION.md");
    let study = parse_study_text(
        "x-vs-docking",
        "STUDY.md",
        &env.read("studies/x-vs-docking/STUDY.md"),
    )
    .unwrap();
    let block = &study.front.preregistered.as_ref().unwrap().value;
    assert_eq!(
        block.exploration_hash.as_deref(),
        Some(exploration_hash(&exploration).as_str())
    );
    assert_eq!(block.deciding_data_seen.as_deref(), Some("false"));
    assert!(env.rules().is_empty(), "{:?}", env.rules());
}

#[test] // rq-db0c3b99 rq-fea6913a
fn pre_registration_says_what_the_exploration_saw() {
    let env = Env::draft();
    env.trial_by_hand("rho=0.42");
    env.commit();
    let out = env.preregister_output();
    let before_confirm = &out[..out.find("Type the study ID").unwrap()];
    assert!(
        before_confirm.contains("data used in trial runs: benchmark-set@v1"),
        "{out}"
    );
    assert!(
        before_confirm.contains("already saw the deciding data")
            && before_confirm.contains("labelled as informed by its deciding"),
        "{out}"
    );
    let study = parse_study_text(
        "x-vs-docking",
        "STUDY.md",
        &env.read("studies/x-vs-docking/STUDY.md"),
    )
    .unwrap();
    assert_eq!(study.status(), Some(Status::Preregistered));
    assert!(study.deciding_data_seen());
}

#[test] // rq-5b211090 rq-4d6751e2
fn editing_the_locked_exploration_is_an_error() {
    let env = Env::preregistered();
    assert!(env.rules().is_empty(), "{:?}", env.rules());
    env.edit(
        "studies/x-vs-docking/EXPLORATION.md",
        "## Trial results",
        "## Trial results\n\nNothing to see.",
    );
    assert!(
        env.rules()
            .iter()
            .any(|e| e.starts_with("study.exploration-modified"))
    );
    fs::remove_file(env.root().join("studies/x-vs-docking/EXPLORATION.md")).unwrap();
    assert!(
        env.rules()
            .iter()
            .any(|e| e.starts_with("study.exploration-modified") && e.contains("missing"))
    );
}

#[test] // rq-647bcecc rq-4d6751e2
fn an_invalid_exploration_file_is_reported() {
    let env = Env::draft();
    env.write(
        "studies/x-vs-docking/EXPLORATION.md",
        "---\nstudy: other-study\ndeciding_data_seen: false\n---\n",
    );
    assert!(
        env.rules()
            .iter()
            .any(|e| e.starts_with("study.exploration-invalid") && e.contains("other-study"))
    );
    env.write("studies/x-vs-docking/EXPLORATION.md", "no front matter\n");
    assert!(
        env.rules()
            .iter()
            .any(|e| e.starts_with("study.exploration-invalid"))
    );
}

#[test] // rq-643d82cb rq-3a21c28f
fn a_result_informed_by_its_deciding_data_is_labelled() {
    let env = Env::draft();
    env.trial_by_hand("rho=0.42");
    env.commit();
    env.preregister_output();
    env.complete();
    assert!(
        env.read("studies/x-vs-docking/SUMMARY.md")
            .contains("\ndeciding_data_seen: true\n")
    );
    let (code, out, err) = env.tophat(&["study", "report", "x-vs-docking"], &[]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("the plan was informed by results on the deciding data"),
        "{out}"
    );
    let key = env.keys("base")["analyse"].clone();
    let tree = tophat::data::lineage(&env.root(), &format!("x-vs-docking/analyse@{key}"))
        .unwrap()
        .render();
    assert!(
        tree.lines()
            .next()
            .unwrap()
            .ends_with("[plan informed by its deciding data]"),
        "{tree}"
    );
    let (_, status, _) = env.tophat(&["status"], &[]);
    assert!(
        status.contains("x-vs-docking  reported (plan informed by its deciding data)"),
        "{status}"
    );
}

#[test] // rq-6ee5a634 rq-4d6751e2
fn the_summary_must_carry_the_studys_label() {
    let env = Env::draft();
    env.trial_by_hand("rho=0.42");
    env.commit();
    env.preregister_output();
    env.complete();
    assert!(env.rules().is_empty(), "{:?}", env.rules());
    env.edit(
        "studies/x-vs-docking/SUMMARY.md",
        "deciding_data_seen: true\n",
        "deciding_data_seen: false\n",
    );
    assert!(
        env.rules()
            .iter()
            .any(|e| e.starts_with("study.exploration-label-mismatch"))
    );
}

#[test] // rq-2048dd2b rq-3a21c28f
fn studies_pre_registered_without_an_exploration_record() {
    let env = Env::preregistered();
    let rel = "studies/x-vs-docking/STUDY.md";
    let text = env.read(rel);
    let stripped: String = text
        .lines()
        .filter(|l| {
            !l.starts_with("  exploration_hash:") && !l.starts_with("  deciding_data_seen:")
        })
        .map(|l| format!("{l}\n"))
        .collect();
    env.write(rel, &stripped);
    fs::remove_file(env.root().join("studies/x-vs-docking/EXPLORATION.md")).unwrap();
    env.complete();
    let all = run_check(&env.root(), &[]).unwrap();
    assert!(
        !all.iter().any(|d| d.rule.starts_with("study.exploration-")),
        "{all:?}"
    );
    assert!(!status_of(&env).informed);
}
