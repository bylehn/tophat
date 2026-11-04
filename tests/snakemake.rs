//! Tests for Snakemake pipelines, QC gates and summaries (rqm/pipeline/snakemake.md).
//!
//! Scenarios that run Snakemake need `TOPHAT_TEST_SNAKEMAKE` set to a Snakemake executable; they
//! pass trivially (printing a notice) when it is unset. A fake `pixi` on `PATH` runs the stage
//! commands directly.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::NaiveDate;
use tophat::check::{Diagnostic, Project, Severity};
use tophat::data::{Origin, RegisterOptions, add, retire};
use tophat::pipeline::{
    KeyContext, PipelineError, ProvenanceRecord, SealRequest, Verdict, compute_keys,
    generate_rules, items_of, load_pipeline, pipeline_rules, qc, qc_status, seal, stub, summarize,
    write_summary,
};
use tophat::study::{Console, StudyError, new_study, parse_summary, report};

// --- Fixtures -------------------------------------------------------------------------------

const BASE: &str = "stages:
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

const PREPARE: &str = r#"if [ -n "$TOPHAT_TEST_FAIL_PREPARE" ] && [ -e "$TOPHAT_TEST_FAIL_PREPARE" ]; then
  echo partial > "$2/partial.txt"; exit 1
fi
cp "$1"/*.pdb "$2"/
"#;

const SCORE: &str = r#"case "$1" in *2xyz*)
  if [ -n "$TOPHAT_TEST_FAIL_ITEM" ] && [ -e "$TOPHAT_TEST_FAIL_ITEM" ]; then exit 1; fi;;
esac
wc -c < "$1" > "$2/score.txt"; echo "$3" >> "$2/score.txt"
"#;

const ANALYSE: &str = r#"n=$(ls "$1"/items | wc -l); echo "n=$n level=$(tail -1 "$1"/items/1abc/score.txt)" > "$2/result.txt"
"#;

#[cfg(unix)]
const FAKE_PIXI: &str = "#!/bin/sh\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";

struct Env {
    base: PathBuf,
}

impl Env {
    /// A committed project with preregistered study `x-vs-docking` (variants base and pbe).
    fn new() -> Self {
        Env::with_status("preregistered")
    }

    fn with_status(status: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-smk-{}-{}",
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
        env.set_status(status);
        env.write("studies/x-vs-docking/config/base.yaml", BASE);
        env.write(
            "studies/x-vs-docking/config/pbe.yaml",
            "stages: [{name: score, params: {level: pbe}}]\n",
        );
        env.write("studies/x-vs-docking/scripts/prepare.sh", PREPARE);
        env.write("studies/x-vs-docking/scripts/score.sh", SCORE);
        env.write("studies/x-vs-docking/scripts/analyse.sh", ANALYSE);
        env.write("studies/x-vs-docking/Snakefile", &stub("x-vs-docking"));
        env.git(&["init", "-q"]);
        env.commit();
        env
    }

    fn set_status(&self, status: &str) {
        self.write(
            "studies/x-vs-docking/STUDY.md",
            &format!(
                "---\nstudy: x-vs-docking\nstatus: {status}\nvisibility: public\nvariants: [base, pbe]\n---\n\n## Result\n\n"
            ),
        );
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

    fn head(&self) -> String {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(self.root())
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn keys(&self, variant: &str) -> BTreeMap<String, String> {
        let config = load_pipeline(&self.root(), "x-vs-docking", variant).unwrap();
        compute_keys(&config, &KeyContext::new(&self.root()))
            .unwrap()
            .keys
    }

    fn out_dir(&self, stage: &str, key: &str) -> PathBuf {
        self.base.join("results/x-vs-docking").join(stage).join(key)
    }

    fn rules(&self) -> String {
        generate_rules(&self.root(), "x-vs-docking").unwrap()
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        pipeline_rules(&Project::new(self.root()))
    }

    fn seal(
        &self,
        stage: &str,
        variant: &str,
        item: Option<&str>,
    ) -> Result<PathBuf, PipelineError> {
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
    }

    /// Produces and seals every stage of a variant by hand (no Snakemake). `before_item` runs
    /// before each item is sealed.
    fn run_by_hand(&self, variant: &str, result: &str, before_item: &dyn Fn(&str)) {
        let keys = self.keys(variant);
        let prepare = self.out_dir("prepare", &keys["prepare"]);
        if ProvenanceRecord::read(&prepare).is_none() {
            fs::create_dir_all(&prepare).unwrap();
            fs::write(prepare.join("1abc.pdb"), "ATOM 1\n").unwrap();
            fs::write(prepare.join("2xyz.pdb"), "ATOM 22\n").unwrap();
            self.seal("prepare", variant, None).unwrap();
        }
        let score = self.out_dir("score", &keys["score"]);
        for item in ["1abc", "2xyz"] {
            fs::create_dir_all(score.join("items").join(item)).unwrap();
            fs::write(
                score.join("items").join(item).join("score.txt"),
                format!("{item}\n"),
            )
            .unwrap();
            before_item(item);
            self.seal("score", variant, Some(item)).unwrap();
        }
        self.seal("score", variant, None).unwrap();
        let analyse = self.out_dir("analyse", &keys["analyse"]);
        fs::create_dir_all(&analyse).unwrap();
        fs::write(analyse.join("result.txt"), format!("{result}\n")).unwrap();
        self.seal("analyse", variant, None).unwrap();
    }

    fn run_all_by_hand(&self) {
        self.run_by_hand("base", "rho=0.61", &|_| {});
        self.run_by_hand("pbe", "rho=0.58", &|_| {});
    }

    fn verdict(&self, variant: &str, verdict: &str) {
        let key = self.keys(variant)["score"].clone();
        self.write(
            &format!("studies/x-vs-docking/qc/score/{key}.md"),
            &format!("---\nstage: score\nkey: {key}\nverdict: {verdict}\ndate: \"2026-10-20\"\n---\n\nok\n"),
        );
    }

    /// Runs real Snakemake, or returns `None` when the test is skipped (see `common::snakemake`).
    fn snakemake(&self, extra_env: &[(&str, &Path)]) -> Option<(bool, String)> {
        let snakemake = common::snakemake()?;
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
        let out = Command::new(snakemake)
            .args([
                "-s",
                "studies/x-vs-docking/Snakefile",
                "--cores",
                "2",
                // Keep independent jobs running after one fails, so the two variants and the
                // items within a fan-out reach the same state on every scheduler interleaving.
                "--keep-going",
            ])
            .env("PATH", path)
            .envs(extra_env.iter().map(|(k, v)| (*k, v.as_os_str())))
            .current_dir(self.root())
            .output()
            .unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Some((out.status.success(), log))
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Stage jobs that ran, from Snakemake's "Job stats" (target rules excluded).
fn stage_jobs(log: &str) -> BTreeMap<String, u32> {
    let mut jobs = BTreeMap::new();
    let mut in_stats = false;
    for line in log.lines() {
        if line.starts_with("Job stats:") {
            in_stats = true;
            continue;
        }
        if in_stats {
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.as_slice() {
                [name, count] if count.parse::<u32>().is_ok() => {
                    if !["total", "tophat_default", "summary"].contains(name)
                        && !name.starts_with("variant_")
                    {
                        *jobs.entry(name.to_string()).or_default() += count.parse::<u32>().unwrap();
                    }
                }
                [] => in_stats = false,
                _ => {}
            }
        }
    }
    jobs
}

fn with_rule<'a>(ds: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    ds.iter().filter(|d| d.rule == rule).collect()
}

fn hex(key: &str) -> &str {
    key.trim_start_matches("sk-")
}

// --- Running the Snakemake-backed tests -----------------------------------------------------

#[test] // rq-b55aac3a rq-20e3189f
fn snakemake_tests_cannot_silently_pass_in_ci() {
    let result = common::snakemake_from(None, Some("1".into()));
    assert!(
        result
            .as_ref()
            .is_err_and(|m| m.contains("Snakemake is required")),
        "{result:?}"
    );
    // With Snakemake given, the requirement is met.
    assert_eq!(
        common::snakemake_from(Some("/usr/bin/snakemake".into()), Some("1".into())),
        Ok(Some("/usr/bin/snakemake".into()))
    );
}

#[test] // rq-20c233fd rq-20e3189f
fn snakemake_tests_skip_locally_without_snakemake() {
    assert_eq!(common::snakemake_from(None, None), Ok(None));
    assert_eq!(
        common::snakemake_from(Some("".into()), Some("0".into())),
        Ok(None)
    );
}

// --- Stub -------------------------------------------------------------------------------------

#[test] // rq-b4eb6e53 rq-fbc524b4
fn new_studies_get_the_stub() {
    let env = Env::new();
    env.write("PROJECT.md", "# P\n\n## q-1a2b3c4d · Q?\nstatus: open\n");
    new_study(&env.root(), "y-study", "q-1a2b3c4d", false).unwrap();
    assert_eq!(env.read("studies/y-study/Snakefile"), stub("y-study"));
    assert!(
        env.read("studies/y-study/config/base.yaml")
            .contains("stages: []")
    );
}

#[test] // rq-46470647 rq-7b0b2ae8
fn edited_snakefile_is_rejected() {
    let env = Env::new();
    assert_eq!(env.diagnostics(), vec![]);
    env.write(
        "studies/x-vs-docking/Snakefile",
        &format!(
            "{}\nrule extra:\n    shell: \"true\"\n",
            stub("x-vs-docking")
        ),
    );
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.snakefile-modified").len(),
        1
    );
}

#[test] // rq-c698a75a rq-7b0b2ae8
fn missing_snakefile_for_a_study_with_stages() {
    let env = Env::new();
    fs::remove_file(env.root().join("studies/x-vs-docking/Snakefile")).unwrap();
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.snakefile-missing").len(),
        1
    );
}

#[test] // rq-3123c559 rq-c7d50a4a
fn stub_can_be_restored() {
    let env = Env::new();
    env.write("studies/x-vs-docking/Snakefile", "# edited\n");
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["pipeline", "stub", "x-vs-docking"])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        env.read("studies/x-vs-docking/Snakefile"),
        stub("x-vs-docking")
    );
    assert!(with_rule(&env.diagnostics(), "pipeline.snakefile-modified").is_empty());
}

// --- Generation -------------------------------------------------------------------------------

#[test] // rq-f9fd06bf rq-5782f3d0 rq-aa6553ad
fn shared_keys_produce_one_job() {
    let env = Env::new();
    let rules = env.rules();
    let (base, pbe) = (env.keys("base"), env.keys("pbe"));
    assert_eq!(base["prepare"], pbe["prepare"]);
    assert_eq!(
        rules
            .matches(&format!("prepare_{}:", hex(&base["prepare"])))
            .count(),
        1
    );
    for stage in ["score", "analyse"] {
        assert_ne!(base[stage], pbe[stage]);
        for keys in [&base, &pbe] {
            assert_eq!(
                rules
                    .matches(&format!("rule {stage}_{}:", hex(&keys[stage])))
                    .count(),
                1,
                "{stage}"
            );
        }
    }
}

#[test] // rq-d77701f7 rq-aa6553ad
fn generated_rules_are_deterministic() {
    let env = Env::new();
    assert_eq!(env.rules(), env.rules());
}

#[test] // rq-d66af500 rq-5782f3d0
fn rule_outputs_and_inputs_follow_the_keys() {
    let env = Env::new();
    let rules = env.rules();
    let base = env.keys("base");
    let prepare_record = env
        .out_dir("prepare", &base["prepare"])
        .join("provenance.json");
    assert!(rules.contains(&format!("\"{}\"", prepare_record.display())));
    let analyse = rules
        .split(&format!("rule analyse_{}:", hex(&base["analyse"])))
        .nth(1)
        .unwrap();
    let analyse = analyse.split("\nrule ").next().unwrap();
    let score_record = env.out_dir("score", &base["score"]).join("provenance.json");
    assert!(analyse.contains(&format!("ancient(\"{}\")", score_record.display())));
    assert!(
        analyse.contains(&format!(
            "qc=[(\"score\", \"{}\", \"base\")]",
            base["score"]
        )),
        "{analyse}"
    );
    assert!(
        !rules.contains("rule qc_"),
        "verdict files are never Snakemake outputs"
    );
}

#[test] // rq-9c43f967 rq-bdfef246
fn study_md_is_never_a_rule_input() {
    let env = Env::new();
    assert!(!env.rules().contains("STUDY.md"));
}

#[test] // rq-1a269752 rq-654fa516
fn generation_fails_on_an_invalid_configuration() {
    let env = Env::new();
    let text = env
        .read("studies/x-vs-docking/config/base.yaml")
        .replace("{in.raw} {out}", "{in.raw} {output}");
    env.write("studies/x-vs-docking/config/base.yaml", &text);
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["pipeline", "rules", "x-vs-docking", "--output", "rules.smk"])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("{output}"));
    assert!(!env.root().join("rules.smk").exists());
}

#[test] // rq-a27155bc rq-654fa516
fn generated_outputs_default_to_the_projects_results_folder() {
    let env = Env::new();
    env.configure(false);
    let rules = generate_rules(&env.root(), "x-vs-docking").unwrap();
    let results = format!(
        "{}/results/",
        std::path::absolute(env.root()).unwrap().display()
    );
    let outputs: Vec<&str> = rules
        .lines()
        .filter(|l| l.trim_start().starts_with('"') && l.contains("provenance.json"))
        .collect();
    assert!(!outputs.is_empty(), "{rules}");
    assert!(outputs.iter().all(|l| l.contains(&results)), "{outputs:?}");
}

#[test] // rq-dbe124d9 rq-654fa516
fn generation_fails_on_a_retired_dataset() {
    let env = Env::new();
    retire(
        &env.root(),
        "benchmark-set@v1",
        "bad",
        NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
    )
    .unwrap();
    match generate_rules(&env.root(), "x-vs-docking") {
        Err(PipelineError::Data(m)) => assert!(m.contains("benchmark-set@v1"), "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test] // rq-809df566 rq-a44c3b76
fn upstream_of_a_fan_out_is_a_checkpoint() {
    let env = Env::new();
    let rules = env.rules();
    let base = env.keys("base");
    assert!(rules.contains(&format!("checkpoint prepare_{}:", hex(&base["prepare"]))));
    assert!(rules.contains(&format!("rule score_{}_item:", hex(&base["score"]))));
    assert!(rules.contains(&format!("rule score_{}:", hex(&base["score"]))));
}

#[test] // rq-1339368c
fn draft_studies_only_get_trial_targets() {
    let env = Env::with_status("draft");
    let rules = env.rules();
    assert!(rules.contains("rule trial:") && rules.contains("rule trial_base:"));
    assert!(!rules.contains("rule summary:") && !rules.contains("rule variant_base:"));
    assert!(rules.contains("/.trial/"));
    assert!(!rules.contains("qc=[(\"score\""));
}

// --- Running (requires Snakemake) -------------------------------------------------------------

#[test] // rq-167d5ad5 rq-1339368c
fn end_to_end_run_produces_sealed_outputs_and_a_summary() {
    let env = Env::new();
    env.verdict("base", "pass");
    env.verdict("pbe", "pass");
    env.commit();
    let Some((ok, log)) = env.snakemake(&[]) else {
        return;
    };
    assert!(ok, "{log}");
    for variant in ["base", "pbe"] {
        for (stage, key) in env.keys(variant) {
            assert!(
                ProvenanceRecord::read(&env.out_dir(&stage, &key)).is_some(),
                "{variant}/{stage}"
            );
        }
    }
    let summary = env.read("studies/x-vs-docking/SUMMARY.md");
    assert!(
        summary.contains("- base/analyse: n=2 level=b3lyp\n- pbe/analyse: n=2 level=pbe\n"),
        "{summary}"
    );
    assert!(summary.contains("dirty: false"));
    report(&env.root(), "x-vs-docking").unwrap();
}

#[test] // rq-90b7b2f4 rq-5782f3d0
fn a_second_run_does_nothing() {
    let env = Env::new();
    env.verdict("base", "pass");
    env.verdict("pbe", "pass");
    env.commit();
    let Some((ok, log)) = env.snakemake(&[]) else {
        return;
    };
    assert!(ok, "{log}");
    let summary = env.read("studies/x-vs-docking/SUMMARY.md");
    let (ok, log) = env.snakemake(&[]).unwrap();
    assert!(ok, "{log}");
    assert_eq!(stage_jobs(&log), BTreeMap::new(), "{log}");
    assert_eq!(env.read("studies/x-vs-docking/SUMMARY.md"), summary);
}

#[test] // rq-59f7b61a rq-5782f3d0
fn changing_a_parameter_re_runs_only_what_depends_on_it() {
    let env = Env::new();
    env.verdict("base", "pass");
    env.verdict("pbe", "pass");
    env.commit();
    let Some((ok, log)) = env.snakemake(&[]) else {
        return;
    };
    assert!(ok, "{log}");
    let old_pbe = env.keys("pbe");
    let old_record = fs::read_to_string(
        env.out_dir("analyse", &old_pbe["analyse"])
            .join("provenance.json"),
    )
    .unwrap();
    env.write(
        "studies/x-vs-docking/config/pbe.yaml",
        "stages: [{name: score, params: {level: pbe0}}]\n",
    );
    env.verdict("pbe", "pass");
    env.commit();
    let (ok, log) = env.snakemake(&[]).unwrap();
    assert!(ok, "{log}");
    let new_pbe = env.keys("pbe");
    let base = env.keys("base");
    let jobs = stage_jobs(&log);
    assert!(!jobs.keys().any(|j| j.starts_with("prepare_")), "{jobs:?}");
    assert!(
        !jobs
            .keys()
            .any(|j| j.contains(hex(&base["score"])) || j.contains(hex(&base["analyse"]))),
        "{jobs:?}"
    );
    assert!(
        jobs.contains_key(&format!("analyse_{}", hex(&new_pbe["analyse"]))),
        "{jobs:?}"
    );
    assert!(ProvenanceRecord::read(&env.out_dir("analyse", &new_pbe["analyse"])).is_some());
    assert_eq!(
        fs::read_to_string(
            env.out_dir("analyse", &old_pbe["analyse"])
                .join("provenance.json")
        )
        .unwrap(),
        old_record
    );
}

#[test] // rq-1b09b7f5 rq-a44c3b76
fn a_failed_item_is_retried_alone() {
    let env = Env::new();
    env.verdict("base", "pass");
    env.verdict("pbe", "pass");
    env.commit();
    let marker = env.base.join("fail-item");
    fs::write(&marker, "").unwrap();
    let Some((ok, log)) = env.snakemake(&[("TOPHAT_TEST_FAIL_ITEM", &marker)]) else {
        return;
    };
    assert!(!ok);
    let base = env.keys("base");
    let item_1abc = env
        .out_dir("score", &base["score"])
        .join("items/1abc/provenance.json");
    assert!(item_1abc.exists(), "{log}");
    let sealed_1abc = fs::read_to_string(&item_1abc).unwrap();
    fs::remove_file(&marker).unwrap();
    let (ok, log) = env
        .snakemake(&[("TOPHAT_TEST_FAIL_ITEM", &marker)])
        .unwrap();
    assert!(ok, "{log}");
    let jobs = stage_jobs(&log);
    assert!(!jobs.keys().any(|j| j.starts_with("prepare_")), "{jobs:?}");
    // In the base variant, 1abc was sealed before the failure, so only 2xyz re-runs. (The pbe
    // variant may not have started before Snakemake stopped at the first failure.)
    assert_eq!(
        jobs.get(&format!("score_{}_item", hex(&base["score"]))),
        Some(&1),
        "only the failed item re-runs: {jobs:?}"
    );
    assert_eq!(fs::read_to_string(&item_1abc).unwrap(), sealed_1abc);
}

#[test] // rq-8a9e97ed rq-5782f3d0
fn partial_output_of_a_failed_run_is_discarded() {
    let env = Env::new();
    env.verdict("base", "pass");
    env.verdict("pbe", "pass");
    env.commit();
    let marker = env.base.join("fail-prepare");
    fs::write(&marker, "").unwrap();
    let Some((ok, _)) = env.snakemake(&[("TOPHAT_TEST_FAIL_PREPARE", &marker)]) else {
        return;
    };
    assert!(!ok);
    let dir = env.out_dir("prepare", &env.keys("base")["prepare"]);
    assert!(dir.join("partial.txt").exists());
    fs::remove_file(&marker).unwrap();
    let (ok, log) = env
        .snakemake(&[("TOPHAT_TEST_FAIL_PREPARE", &marker)])
        .unwrap();
    assert!(ok, "{log}");
    assert!(!dir.join("partial.txt").exists());
    assert!(dir.join("1abc.pdb").exists());
}

// --- Fan-out items ----------------------------------------------------------------------------

#[test] // rq-db3f2ac8 rq-a44c3b76
fn items_are_the_upstream_output_files() {
    let env = Env::new();
    let dir = env.base.join("upstream");
    fs::create_dir_all(dir.join("subdir")).unwrap();
    for f in ["1abc.pdb", "2xyz.pdb", ".hidden", "provenance.json"] {
        fs::write(dir.join(f), "x").unwrap();
    }
    let names: Vec<String> = items_of(&dir)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["1abc", "2xyz"]);
}

#[test] // rq-1d07e2ec rq-a44c3b76
fn duplicate_item_names_fail_the_gather_job() {
    let env = Env::new();
    let prepare = env.out_dir("prepare", &env.keys("base")["prepare"]);
    fs::create_dir_all(&prepare).unwrap();
    fs::write(prepare.join("1abc.pdb"), "a").unwrap();
    fs::write(prepare.join("1abc.cif"), "b").unwrap();
    env.seal("prepare", "base", None).unwrap();
    assert_eq!(
        env.seal("score", "base", None),
        Err(PipelineError::BadItems(vec!["1abc".into()]))
    );
}

#[test] // rq-543d641b rq-a44c3b76
fn fan_out_over_a_dataset() {
    let env = Env::new();
    let text = env
        .read("studies/x-vs-docking/config/base.yaml")
        .replace(
            "    inputs: {raw: dataset:benchmark-set@v1}\n",
            "    inputs: {raw: dataset:benchmark-set@v1}\n    foreach: raw\n",
        )
        .replace("{in.raw} {out}", "{item} {out}");
    env.write("studies/x-vs-docking/config/base.yaml", &text);
    let rules = env.rules();
    let key = env.keys("base")["prepare"].clone();
    assert!(rules.contains(&format!("rule prepare_{}_item:", hex(&key))));
    assert!(
        rules.contains("for item in [\"1abc\", \"2xyz\"]"),
        "{rules}"
    );
    // The dataset's items are known now, so prepare's own gather needs no checkpoint call.
    let gather = rules
        .split(&format!("prepare_{}:\n", hex(&key)))
        .nth(1)
        .unwrap();
    let gather_input = gather.lines().nth(1).unwrap();
    assert!(
        gather_input.contains("[\"1abc\", \"2xyz\"]"),
        "{gather_input}"
    );
    assert!(!gather_input.contains("_tophat_checkpoint_items"));
}

// --- QC gates ---------------------------------------------------------------------------------

#[test] // rq-29ef4225 rq-dc1e521d
fn only_stages_with_a_review_question_are_gated() {
    let env = Env::new();
    let item_rule = |env: &Env| {
        let rules = env.rules();
        let base = env.keys("base");
        let rule = rules
            .split(&format!("rule score_{}_item:", hex(&base["score"])))
            .nth(1)
            .unwrap()
            .split("\nrule ")
            .next()
            .unwrap()
            .to_string();
        (rules, base, rule)
    };
    let (rules, base, rule) = item_rule(&env);
    assert!(rule.contains("qc=[]"), "{rule}");
    let analyse = rules
        .split(&format!("rule analyse_{}:", hex(&base["analyse"])))
        .nth(1)
        .unwrap();
    assert!(analyse.contains(&format!("(\"score\", \"{}\", \"base\")", base["score"])));

    let config = env.read("studies/x-vs-docking/config/base.yaml");
    let line = "    inputs: {raw: dataset:benchmark-set@v1}\n";
    assert!(config.contains(line));
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &config.replacen(
            line,
            &format!("{line}    review: Are all structures present and intact?\n"),
            1,
        ),
    );
    let (_, base, rule) = item_rule(&env);
    assert!(
        rule.contains(&format!(
            "qc=[(\"prepare\", \"{}\", \"base\")]",
            base["prepare"]
        )),
        "{rule}"
    );
}

#[test] // rq-78b231e3 rq-dc1e521d
fn a_stage_that_nothing_consumes_is_never_gated() {
    let env = Env::new();
    // No job's verdict list (`qc=[...]`) names analyse.
    for line in env.rules().lines() {
        if let Some(list) = line.split("qc=[").nth(1) {
            let list = list.split(']').next().unwrap();
            assert!(!list.contains("\"analyse\""), "{line}");
        }
    }
    env.run_by_hand("base", "rho=0.61", &|_| {});
    let (result, _) = qc_typing(&env, "analyse", "pass\nok\n", true);
    assert!(matches!(result, Err(PipelineError::Qc(m)) if m.contains("no QC gate")));
}

#[test] // rq-41afeb5b rq-dc1e521d rq-ecacda48
fn a_reviewed_stage_that_nothing_consumes_gates_the_report() {
    let env = Env::new();
    let config = env.read("studies/x-vs-docking/config/base.yaml");
    let line = "    result: result.txt\n";
    assert!(config.contains(line));
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &config.replacen(
            line,
            &format!("{line}    review: Does the result line make sense?\n"),
            1,
        ),
    );
    env.commit();
    summarized(&env);
    env.verdict("base", "pass");
    env.verdict("pbe", "pass");
    let before = env.read("studies/x-vs-docking/STUDY.md");
    match report(&env.root(), "x-vs-docking") {
        Err(StudyError::Summary(m)) => assert!(
            m.contains("tophat stage qc x-vs-docking analyse --variant base"),
            "{m}"
        ),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(env.read("studies/x-vs-docking/STUDY.md"), before);
}

#[test] // rq-a30a3eff rq-dc1e521d rq-5782f3d0
fn a_failing_validation_stops_the_stage() {
    let env = Env::new();
    let config = env.read("studies/x-vs-docking/config/base.yaml");
    let line = "    validate: find {out} -name '*.pdb' | grep -q .\n";
    assert!(config.contains(line));
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &config.replacen(
            line,
            "    validate: sh -c 'echo validation found no structures >&2; exit 1'\n",
            1,
        ),
    );
    env.commit();
    let Some((ok, log)) = env.snakemake(&[]) else {
        return;
    };
    assert!(!ok);
    assert!(log.contains("validation found no structures"), "{log}");
    assert!(log.contains("Validation of stage prepare failed"), "{log}");
    let keys = env.keys("base");
    assert!(ProvenanceRecord::read(&env.out_dir("prepare", &keys["prepare"])).is_none());
    assert!(!env.out_dir("score", &keys["score"]).exists());
}

#[test] // rq-c705fcd3 rq-dc1e521d
fn missing_verdict_stops_the_pipeline_before_downstream_stages() {
    let env = Env::new();
    let Some((ok, log)) = env.snakemake(&[]) else {
        return;
    };
    assert!(!ok);
    assert!(
        log.contains("tophat stage qc x-vs-docking score --variant"),
        "{log}"
    );
    for variant in ["base", "pbe"] {
        let keys = env.keys(variant);
        assert!(ProvenanceRecord::read(&env.out_dir("score", &keys["score"])).is_some());
        assert!(ProvenanceRecord::read(&env.out_dir("analyse", &keys["analyse"])).is_none());
    }
}

/// Runs `qc` as if a human typed `answers` in an interactive terminal.
fn qc_typing(
    env: &Env,
    stage: &str,
    answers: &str,
    interactive: bool,
) -> (Result<PathBuf, PipelineError>, String) {
    let mut input = Cursor::new(answers.as_bytes().to_vec());
    let mut out = Vec::new();
    let result = qc(
        &env.root(),
        "x-vs-docking",
        stage,
        "base",
        NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        &mut Console {
            interactive,
            input: &mut input,
            out: &mut out,
        },
    );
    (result, String::from_utf8(out).unwrap())
}

#[test] // rq-88bd99cd rq-6c24fee9
fn the_qc_prompt_asks_the_review_question() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    let (result, output) = qc_typing(&env, "score", "pass\nok\n", true);
    result.unwrap();
    let question = output
        .find("Do the scores look plausible for these structures?")
        .expect("the review question is shown");
    assert!(
        question < output.find("Verdict (pass or fail)").unwrap(),
        "{output}"
    );
}

#[test] // rq-79e78b37 rq-6c24fee9 rq-872c3d40
fn record_a_passing_verdict_interactively() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    let (result, output) = qc_typing(&env, "score", "pass\nDistributions look fine\n", true);
    let path = result.unwrap();
    let key = env.keys("base")["score"].clone();
    assert_eq!(
        path,
        env.root()
            .join(format!("studies/x-vs-docking/qc/score/{key}.md"))
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!(
            "---\nstage: score\nkey: {key}\nverdict: pass\ndate: \"2026-10-20\"\n---\n\nDistributions look fine\n"
        )
    );
    assert!(output.contains("items:   2"));
    assert_eq!(
        qc_status(&env.root(), "x-vs-docking", "score", &key),
        Ok(Verdict::Pass)
    );
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["stage", "qc-status", "x-vs-docking", "score", "--key", &key])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test] // rq-83c71c3c rq-6c24fee9
fn qc_refused_without_an_interactive_terminal() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    let (result, _) = qc_typing(&env, "score", "pass\nok\n", false);
    assert!(matches!(result, Err(PipelineError::NotInteractive(m)) if m.contains("human")));
    assert!(!env.root().join("studies/x-vs-docking/qc").exists());
}

#[test] // rq-933f4cf3 rq-6c24fee9
fn qc_refused_for_an_unsealed_output() {
    let env = Env::new();
    let (result, _) = qc_typing(&env, "score", "pass\nok\n", true);
    assert!(matches!(result, Err(PipelineError::Qc(m)) if m.contains("has not finished")));
}

#[test] // rq-6315ba95 rq-6c24fee9
fn qc_refused_for_a_stage_without_a_gate() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    let (result, _) = qc_typing(&env, "prepare", "pass\nok\n", true);
    assert!(matches!(result, Err(PipelineError::Qc(m)) if m.contains("no QC gate")));
}

#[test] // rq-0c5735ed rq-6c24fee9
fn existing_verdicts_are_not_overwritten() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    env.verdict("base", "fail");
    let before = env.read(&format!(
        "studies/x-vs-docking/qc/score/{}.md",
        env.keys("base")["score"]
    ));
    let (result, _) = qc_typing(&env, "score", "pass\nok\n", true);
    assert!(matches!(result, Err(PipelineError::Qc(m)) if m.contains("already exists")));
    assert_eq!(
        env.read(&format!(
            "studies/x-vs-docking/qc/score/{}.md",
            env.keys("base")["score"]
        )),
        before
    );
}

#[test] // rq-4d7646da rq-6c24fee9
fn unrecognised_verdict_aborts() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    let (result, _) = qc_typing(&env, "score", "maybe\nok\n", true);
    assert!(matches!(result, Err(PipelineError::Qc(_))));
    assert!(!env.root().join("studies/x-vs-docking/qc").exists());
}

#[test] // rq-e88cc29b rq-dc1e521d
fn failing_verdict_blocks_downstream_stages() {
    let env = Env::new();
    env.verdict("base", "fail");
    env.verdict("pbe", "pass");
    env.commit();
    let Some((ok, log)) = env.snakemake(&[]) else {
        return;
    };
    assert!(!ok);
    let key = env.keys("base")["score"].clone();
    assert!(log.contains(&format!("qc/score/{key}.md")), "{log}");
    assert!(
        ProvenanceRecord::read(&env.out_dir("analyse", &env.keys("base")["analyse"])).is_none()
    );
    assert!(!env.root().join("studies/x-vs-docking/SUMMARY.md").exists());
}

#[test] // rq-bf30c589 rq-7b0b2ae8
fn invalid_qc_file() {
    let env = Env::new();
    env.verdict("base", "ok");
    let ds = env.diagnostics();
    assert_eq!(with_rule(&ds, "pipeline.qc-invalid").len(), 1);
}

#[test] // rq-2c0af82b rq-7b0b2ae8
fn qc_file_for_a_stage_without_a_gate() {
    let env = Env::new();
    let key = env.keys("base")["prepare"].clone();
    env.write(
        &format!("studies/x-vs-docking/qc/prepare/{key}.md"),
        &format!("---\nstage: prepare\nkey: {key}\nverdict: pass\ndate: \"2026-10-20\"\n---\n"),
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.qc-invalid");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("no QC gate"));
}

// --- Summaries --------------------------------------------------------------------------------

#[test] // rq-d9fd0437 rq-1a4b027f rq-b9512459 rq-23fa4a9a
fn summarize_sealed_results() {
    let env = Env::new();
    env.run_all_by_hand();
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["study", "summarize", "x-vs-docking"])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary = parse_summary(&env.root().join("studies/x-vs-docking/SUMMARY.md")).unwrap();
    let lines: Vec<&str> = summary.results.iter().map(|l| l.raw.as_str()).collect();
    assert_eq!(
        lines,
        ["- base/analyse: rho=0.61", "- pbe/analyse: rho=0.58"]
    );
    let keys = summary.keys.unwrap();
    assert_eq!(keys["base/analyse"], env.keys("base")["analyse"]);
    assert_eq!(keys["pbe/analyse"], env.keys("pbe")["analyse"]);
    assert_eq!(summary.commit, env.head());
    assert!(!summary.dirty);
}

#[test] // rq-960f50c6 rq-1a4b027f
fn summary_is_dirty_when_an_upstream_record_is_unclean() {
    let env = Env::new();
    let committed = env.read("pixi.toml");
    env.write("pixi.toml", "[workspace]\nname = \"wip\"\n");
    let prepare = env.out_dir("prepare", &env.keys("base")["prepare"]);
    fs::create_dir_all(&prepare).unwrap();
    fs::write(prepare.join("1abc.pdb"), "a").unwrap();
    fs::write(prepare.join("2xyz.pdb"), "b").unwrap();
    env.seal("prepare", "base", None).unwrap();
    env.write("pixi.toml", &committed);
    env.run_all_by_hand();
    assert!(summarize(&env.root(), "x-vs-docking").unwrap().dirty);
}

#[test] // rq-9f3b24de rq-1a4b027f
fn summary_is_dirty_when_one_fan_out_item_record_is_unclean() {
    let env = Env::new();
    let committed = env.read("pixi.toml");
    let root = env.root();
    env.run_by_hand("base", "rho=0.61", &|item| {
        let text = if item == "2xyz" {
            "[workspace]\nname = \"wip\"\n"
        } else {
            committed.as_str()
        };
        fs::write(root.join("pixi.toml"), text).unwrap();
    });
    env.write("pixi.toml", &committed);
    env.run_by_hand("pbe", "rho=0.58", &|_| {});
    assert!(summarize(&env.root(), "x-vs-docking").unwrap().dirty);
}

#[test] // rq-df68d9f7 rq-23fa4a9a
fn summarize_refuses_missing_outputs() {
    let env = Env::new();
    env.run_by_hand("base", "rho=0.61", &|_| {});
    match summarize(&env.root(), "x-vs-docking") {
        Err(PipelineError::NotSealed(m)) => assert!(
            m.contains("pbe/analyse") && !m.contains("base/analyse"),
            "{m}"
        ),
        other => panic!("unexpected {other:?}"),
    }
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["study", "summarize", "x-vs-docking"])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!env.root().join("studies/x-vs-docking/SUMMARY.md").exists());
}

#[test] // rq-d953c801 rq-23fa4a9a
fn summarize_refuses_a_study_without_result_stages() {
    let env = Env::new();
    let text = env
        .read("studies/x-vs-docking/config/base.yaml")
        .replace("    result: result.txt\n", "");
    env.write("studies/x-vs-docking/config/base.yaml", &text);
    match summarize(&env.root(), "x-vs-docking") {
        Err(PipelineError::Config { message, .. }) => assert!(message.contains("no result stages")),
        other => panic!("unexpected {other:?}"),
    }
}

fn summarized(env: &Env) {
    env.run_all_by_hand();
    let summary = summarize(&env.root(), "x-vs-docking").unwrap();
    write_summary(&env.root(), &summary).unwrap();
}

#[test] // rq-262c5552 rq-7b0b2ae8
fn summary_goes_stale_when_the_code_changes() {
    let env = Env::new();
    summarized(&env);
    env.set_status("reported");
    assert!(with_rule(&env.diagnostics(), "pipeline.summary-stale").is_empty());
    env.write(
        "studies/x-vs-docking/scripts/analyse.sh",
        "echo changed > \"$2/result.txt\"\n",
    );
    env.commit();
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.summary-stale");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Error);
}

#[test] // rq-a029874e rq-7b0b2ae8
fn stale_summary_in_a_draft_is_only_a_warning() {
    let env = Env::new();
    summarized(&env);
    env.set_status("draft");
    env.write(
        "studies/x-vs-docking/scripts/analyse.sh",
        "echo changed > \"$2/result.txt\"\n",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.summary-stale");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test] // rq-9acfe92b rq-ecacda48
fn report_refuses_a_stale_summary() {
    let env = Env::new();
    summarized(&env);
    env.write(
        "studies/x-vs-docking/scripts/analyse.sh",
        "echo changed > \"$2/result.txt\"\n",
    );
    env.commit();
    match report(&env.root(), "x-vs-docking") {
        Err(StudyError::Summary(m)) => assert!(m.contains("stale"), "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test] // rq-1a4b027f
fn summarize_does_not_rewrite_for_a_new_timestamp_only() {
    let env = Env::new();
    summarized(&env);
    let before = env.read("studies/x-vs-docking/SUMMARY.md");
    let again = summarize(&env.root(), "x-vs-docking").unwrap();
    assert!(!write_summary(&env.root(), &again).unwrap());
    assert_eq!(env.read("studies/x-vs-docking/SUMMARY.md"), before);
}
