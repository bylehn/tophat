//! Tests for stage configuration, keys and provenance (rqm/pipeline/provenance.md), and for
//! `tophat data publish` (rqm/data.md), which needs stage keys.
//!
//! Fixture projects live in the system temporary directory, outside any enclosing git repository,
//! because cleanliness is determined with git.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use tophat::check::{Diagnostic, Project, Severity};
use tophat::data::{
    DataError, DatasetKind, Origin, RegisterOptions, add, parse_manifest, publish, verify,
};
use tophat::pipeline::{
    KeyContext, PipelineError, ProvenanceRecord, SealRequest, compute_keys, key_status,
    load_pipeline, pipeline_rules, seal, stage_key, stage_key_text,
};

// --- Fixtures -------------------------------------------------------------------------------

const BASE: &str = "stages:
  - name: prepare
    run: julia --project=. scripts/prepare.jl {in.raw} {out}
    code: [scripts/prepare.jl]
    inputs:
      raw: dataset:benchmark-set@v2
    params: {cutoff: 2.0}
    validate: find {out} -name '*.pdb' | grep -q .
  - name: score
    run: python scripts/score.py {item} {out} --level {params.level}
    code: [scripts/score.py, scripts/common/]
    inputs:
      structures: stage:prepare
    foreach: structures
    validate: test -s {out}/score.txt
    review: Do the scores look plausible for these structures?
    params: {level: b3lyp, grid: fine}
    env: chem
    threads: 4
  - name: analyse
    run: Rscript scripts/analyse.R {in.scores} {out}
    code: [scripts/analyse.R]
    inputs:
      scores: stage:score
    result: result.txt
";

const LOCK: &str = "version: 6
environments:
  default:
    packages:
      linux-64:
      - conda: https://conda.example/python-3.12.conda
  chem:
    packages:
      linux-64:
      - conda: https://conda.example/rdkit-2024.conda
      osx-arm64:
      - conda: https://conda.example/rdkit-2024-arm.conda
";

struct Env {
    base: PathBuf,
}

impl Env {
    /// A committed project with study `x-vs-docking` and datasets `benchmark-set@v1..v3`.
    fn new() -> Self {
        let env = Env::bare();
        env.git(&["init", "-q"]);
        env.commit_all();
        env
    }

    /// The same project without a git repository.
    fn bare() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-pipeline-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        let env = Env { base };
        for dir in ["project/.tophat", "raw", "results"] {
            fs::create_dir_all(env.base.join(dir)).unwrap();
        }
        env.write(".tophat/roots.yaml", "roots:\n  raw: {}\n  results: {}\n");
        env.configure(true);
        env.write("pixi.lock", LOCK);
        env.write("pixi.toml", "[workspace]\nname = \"p\"\n");
        env.write(".gitignore", ".tophat/roots.local.yaml\n.tophat/cache/\n");
        for v in 1..=3 {
            let dir = env.base.join(format!("raw/bench/v{v}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("set.csv"), format!("version {v}\n")).unwrap();
            add(
                &env.root(),
                &RegisterOptions {
                    name: "benchmark-set".into(),
                    root: "raw".into(),
                    path: format!("bench/v{v}"),
                    visibility: "public".into(),
                    origin: Origin::Raw {
                        source: "x".into(),
                        date: None,
                        license: None,
                    },
                },
            )
            .unwrap();
        }
        env.write(
            "studies/x-vs-docking/STUDY.md",
            "---\nstudy: x-vs-docking\nvisibility: public\n---\n",
        );
        env.write("studies/x-vs-docking/config/base.yaml", BASE);
        env.write(
            "studies/x-vs-docking/Snakefile",
            &tophat::pipeline::stub("x-vs-docking"),
        );
        env.write(
            "studies/x-vs-docking/scripts/prepare.jl",
            "println(\"prepare\")\n",
        );
        env.write("studies/x-vs-docking/scripts/score.py", "print('score')\n");
        env.write("studies/x-vs-docking/scripts/common/util.py", "X = 1\n");
        env.write("studies/x-vs-docking/scripts/analyse.R", "cat('analyse')\n");
        env
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

    fn edit(&self, rel: &str, from: &str, to: &str) {
        let text = fs::read_to_string(self.root().join(rel)).unwrap();
        assert!(text.contains(from), "{rel} lacks {from:?}");
        self.write(rel, &text.replacen(from, to, 1));
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn commit_all(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", "fixture"]);
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
        self.base
            .join("results")
            .join("x-vs-docking")
            .join(stage)
            .join(key)
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        pipeline_rules(&Project::new(self.root()))
    }

    /// Writes files into the output directory of a stage (base variant key).
    fn outputs(&self, stage: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.out_dir(stage, &self.keys("base")[stage]);
        for (name, text) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn seal(&self, stage: &str, item: Option<&str>) -> Result<PathBuf, PipelineError> {
        self.seal_variant(stage, "base", item)
    }

    fn seal_variant(
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

    /// Produces and seals `prepare` with items `1abc` and `2xyz`.
    fn prepared(&self) -> PathBuf {
        let dir = self.outputs(
            "prepare",
            &[("1abc.pdb", "ATOM 1\n"), ("2xyz.pdb", "ATOM 2\n")],
        );
        self.seal("prepare", None).unwrap();
        dir
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn with_rule<'a>(ds: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    ds.iter().filter(|d| d.rule == rule).collect()
}

// --- Configuration ----------------------------------------------------------------------------

#[test] // rq-bbe1941a rq-55bef4eb rq-081e737a
fn a_valid_configuration_has_no_pipeline_diagnostics() {
    let env = Env::new();
    assert_eq!(env.diagnostics(), vec![]);
}

#[test] // rq-a5fd6b70 rq-22c3d944
fn a_study_without_stages_has_no_pipeline() {
    let env = Env::new();
    env.write("studies/notes-only/config/base.yaml", "stages: []\n");
    env.write("studies/other-notes/config/base.yaml", "# nothing yet\n");
    assert_eq!(env.diagnostics(), vec![]);
}

#[test] // rq-8d671e50 rq-081e737a
fn input_referencing_a_later_stage() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "      raw: dataset:benchmark-set@v2\n",
        "      raw: dataset:benchmark-set@v2\n      scores: stage:analyse\n",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.config-invalid");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].line, Some(2));
    assert_eq!(found[0].path, "studies/x-vs-docking/config/base.yaml");
}

#[test] // rq-19e53a7b rq-6891672b
fn unknown_placeholder() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "{in.raw} {out}",
        "{in.raw} {output}",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.config-invalid");
    assert_eq!(found.len(), 1);
    assert!(
        found[0].message.contains("{output}"),
        "{}",
        found[0].message
    );
}

#[test] // rq-ccaa4e30 rq-6891672b
fn escaped_braces_are_allowed() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "run: julia --project=. scripts/prepare.jl {in.raw} {out}",
        "run: awk '{{print $1}}' {in.raw}/list > {out}/ids",
    );
    assert!(load_pipeline(&env.root(), "x-vs-docking", "base").is_ok());
    assert_eq!(env.diagnostics(), vec![]);
}

#[test] // rq-eda3d443 rq-081e737a
fn a_stage_cannot_be_both_a_tool_stage_and_an_external_stage() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    env: chem\n",
        "    env: chem\n    tool: method-x@a1b2c3d\n    external: {}\n",
    );
    let ds = env.diagnostics();
    assert!(
        with_rule(&ds, "pipeline.config-invalid")
            .iter()
            .any(|d| d.message.contains("`tool`") && d.message.contains("`external`"))
    );
}

#[test] // rq-a5ab4b78 rq-081e737a
fn fan_out_stage_cannot_declare_a_result() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    threads: 4\n",
        "    threads: 4\n    result: r.txt\n",
    );
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.config-invalid").len(),
        1
    );
}

#[test] // rq-6101fec0 rq-22c3d944
fn missing_code_file() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "code: [scripts/prepare.jl]",
        "code: [scripts/missing.jl]",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.code-missing");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("scripts/missing.jl"));
}

#[test] // rq-c3a5cbef rq-22c3d944
fn code_outside_the_project() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "code: [scripts/prepare.jl]",
        "code: [../../../etc/passwd]",
    );
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.code-missing").len(),
        1
    );
}

#[test] // rq-003d3d0f rq-3ac5de89
fn missing_environment() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "env: chem",
        "env: gpu",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.env-missing");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("gpu"));
}

#[test] // rq-180689f2 rq-22c3d944
fn a_consumed_stage_without_validation_is_flagged() {
    let env = Env::new();
    assert!(with_rule(&env.diagnostics(), "pipeline.validate-missing").is_empty());
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    validate: find {out} -name '*.pdb' | grep -q .\n",
        "",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.validate-missing");
    assert_eq!(found.len(), 1, "{ds:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
    assert!(
        found[0].message.contains("`prepare`"),
        "{}",
        found[0].message
    );
    assert_eq!(found[0].line, Some(2));
}

#[test] // rq-0a10fc96 rq-4aa0232e
fn the_validate_command_enters_the_key() {
    let env = Env::new();
    let config = load_pipeline(&env.root(), "x-vs-docking", "base").unwrap();
    let ctx = KeyContext::new(&env.root());
    let text = stage_key_text(&config, "prepare", &ctx).unwrap();
    assert_eq!(
        text.lines().nth(2),
        Some("validate find {out} -name '*.pdb' | grep -q .")
    );
    let before = env.keys("base");
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    validate: find {out} -name '*.pdb' | grep -q .\n",
        "    validate: find {out} -name '*.pdb' | grep -q pdb\n",
    );
    let after = env.keys("base");
    for stage in ["prepare", "score", "analyse"] {
        assert_ne!(before[stage], after[stage], "{stage}");
    }
    let analyse = stage_key_text(&config, "analyse", &ctx).unwrap();
    assert!(!analyse.contains("\nvalidate "), "{analyse}");
}

#[test] // rq-617974f2 rq-37a6119e
fn the_review_question_does_not_enter_the_key() {
    let env = Env::new();
    let before = env.keys("base");
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    review: Do the scores look plausible for these structures?\n",
        "    review: Something else entirely?\n",
    );
    assert_eq!(env.keys("base"), before);
}

#[test] // rq-2a51782f rq-081e737a
fn the_removed_qc_fields_are_rejected() {
    for field in ["qc: skip", "qc_reason: not needed", "check: something"] {
        let env = Env::new();
        env.edit(
            "studies/x-vs-docking/config/base.yaml",
            "    threads: 4\n",
            &format!("    threads: 4\n    {field}\n"),
        );
        let ds = env.diagnostics();
        let found = with_rule(&ds, "pipeline.config-invalid");
        assert_eq!(found.len(), 1, "{field}: {ds:#?}");
        assert!(
            found[0].message.contains("`review`") && found[0].message.contains("`validate`"),
            "{}",
            found[0].message
        );
    }
}

#[test] // rq-fd086402 rq-081e737a
fn an_empty_review_question_is_rejected() {
    let env = Env::new();
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    review: Do the scores look plausible for these structures?\n",
        "    review: ''\n",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.config-invalid");
    assert_eq!(found.len(), 1, "{ds:#?}");
    assert!(
        found[0].message.contains("`review`"),
        "{}",
        found[0].message
    );
}

// --- Variants ---------------------------------------------------------------------------------

#[test] // rq-43b7a9e6 rq-a17aa884 rq-f8dfe570
fn variant_overrides_merge_into_the_base_stage() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/config/pbe.yaml",
        "stages: [{name: score, params: {level: pbe}}]\n",
    );
    let base = load_pipeline(&env.root(), "x-vs-docking", "base").unwrap();
    let pbe = load_pipeline(&env.root(), "x-vs-docking", "pbe").unwrap();
    let score = pbe.stage("score").unwrap();
    assert_eq!(
        serde_json::Value::Object(score.params.clone()),
        serde_json::json!({"level": "pbe", "grid": "fine"})
    );
    assert_eq!(pbe.stage("prepare"), base.stage("prepare"));
    assert_eq!(pbe.stage("analyse"), base.stage("analyse"));
    assert_eq!(score.run, base.stage("score").unwrap().run);
    assert_eq!(env.diagnostics(), vec![]);
}

#[test] // rq-3fd889d8 rq-a17aa884
fn variant_cannot_add_stages() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/config/pbe.yaml",
        "stages: [{name: extra, run: echo}]\n",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.config-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/x-vs-docking/config/pbe.yaml");
}

#[test] // rq-a17aa884 rq-f8dfe570
fn unknown_variant() {
    let env = Env::new();
    assert_eq!(
        load_pipeline(&env.root(), "x-vs-docking", "nope"),
        Err(PipelineError::UnknownVariant("nope".into()))
    );
}

// --- Keys -------------------------------------------------------------------------------------

#[test] // rq-59d28b94 rq-4aa0232e rq-f40ffa2b rq-e6252079
fn key_text_follows_the_specified_construction() {
    let env = Env::new();
    let config = load_pipeline(&env.root(), "x-vs-docking", "base").unwrap();
    let ctx = KeyContext::new(&env.root());
    let text = stage_key_text(&config, "prepare", &ctx).unwrap();
    let script_sha =
        tophat::data::hash_file(&env.root().join("studies/x-vs-docking/scripts/prepare.jl"))
            .unwrap()
            .1;
    let content_hash = parse_manifest(
        &fs::read_to_string(env.root().join("datasets/benchmark-set/v2.yaml")).unwrap(),
    )
    .unwrap()
    .content_hash;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "tophat-stage-key/1");
    assert_eq!(
        lines[1],
        "run julia --project=. scripts/prepare.jl {in.raw} {out}"
    );
    assert_eq!(lines[2], "validate find {out} -name '*.pdb' | grep -q .");
    assert!(lines[3].starts_with("env sha256:"));
    assert_eq!(lines[4..7], ["threads 1", "foreach -", "result -"]);
    assert_eq!(lines[7], format!("code scripts/prepare.jl {script_sha}"));
    assert_eq!(lines[8], r#"params {"cutoff":2.0}"#);
    assert_eq!(
        lines[9],
        format!("input raw dataset benchmark-set@v2 {content_hash}")
    );
    assert!(text.ends_with('\n'));
    assert_eq!(env.keys("base")["prepare"], stage_key(&text));
}

#[test] // rq-6f38a675 rq-4aa0232e
fn changing_a_script_changes_its_key_and_every_downstream_key() {
    let env = Env::new();
    let before = env.keys("base");
    env.write(
        "studies/x-vs-docking/scripts/prepare.jl",
        "println(\"changed\")\n",
    );
    let after = env.keys("base");
    for stage in ["prepare", "score", "analyse"] {
        assert_ne!(before[stage], after[stage], "{stage}");
    }
}

#[test] // rq-e35a8bdd rq-4aa0232e
fn changing_a_parameter_changes_only_that_stage_and_downstream() {
    let env = Env::new();
    let before = env.keys("base");
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "level: b3lyp",
        "level: pbe0",
    );
    let after = env.keys("base");
    assert_eq!(before["prepare"], after["prepare"]);
    assert_ne!(before["score"], after["score"]);
    assert_ne!(before["analyse"], after["analyse"]);
}

#[test] // rq-21dcc0c2 rq-4aa0232e
fn parameter_order_does_not_matter() {
    let env = Env::new();
    let before = env.keys("base");
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "{level: b3lyp, grid: fine}",
        "{grid: fine, level: b3lyp}",
    );
    assert_eq!(before["score"], env.keys("base")["score"]);
}

#[test] // rq-9f5b536b rq-4aa0232e rq-a56c1f8f
fn variants_with_identical_stage_settings_share_a_key() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/config/pbe.yaml",
        "stages: [{name: score, params: {level: pbe}}]\n",
    );
    let (base, pbe) = (env.keys("base"), env.keys("pbe"));
    assert_eq!(base["prepare"], pbe["prepare"]);
    assert_ne!(base["score"], pbe["score"]);
    let dirs = |variant: &str| {
        key_status(&env.root(), "x-vs-docking", variant).unwrap()[0]
            .output
            .clone()
    };
    assert_eq!(dirs("base"), dirs("pbe"));
}

#[test] // rq-85aa702f rq-4aa0232e
fn a_new_dataset_version_changes_the_key() {
    let env = Env::new();
    let before = env.keys("base");
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "benchmark-set@v2",
        "benchmark-set@v3",
    );
    assert_ne!(before["prepare"], env.keys("base")["prepare"]);
}

#[test] // rq-e23a3422 rq-4aa0232e
fn a_code_directory_is_hashed_file_by_file() {
    let env = Env::new();
    let before = env.keys("base");
    env.write("studies/x-vs-docking/scripts/common/util.py", "X = 2\n");
    let after = env.keys("base");
    assert_ne!(before["score"], after["score"]);
    assert_eq!(before["prepare"], after["prepare"]);
}

#[test] // rq-9e24cb69 rq-4aa0232e
fn gating_a_stage_does_not_change_keys() {
    let env = Env::new();
    let before = env.keys("base");
    env.edit(
        "studies/x-vs-docking/config/base.yaml",
        "    params: {cutoff: 2.0}\n",
        "    params: {cutoff: 2.0}\n    review: Do the prepared structures look complete?\n",
    );
    assert_eq!(before, env.keys("base"));
}

#[test] // rq-6d9d6c2d rq-3ac5de89
fn updating_the_environment_changes_keys_of_stages_using_it() {
    let env = Env::new();
    let before = env.keys("base");
    env.edit("pixi.lock", "rdkit-2024.conda", "rdkit-2025.conda");
    let after = env.keys("base");
    assert_ne!(before["score"], after["score"]);
    assert_eq!(before["prepare"], after["prepare"]);
}

#[test] // rq-04f01464 rq-a56c1f8f rq-c8637010
fn output_directory_is_named_by_the_key() {
    let env = Env::new();
    let status = key_status(&env.root(), "x-vs-docking", "base").unwrap();
    let key = env.keys("base")["prepare"].clone();
    assert_eq!(
        status[0].output.as_deref(),
        Some(env.out_dir("prepare", &key).as_path())
    );
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["pipeline", "keys", "x-vs-docking"])
        .current_dir(env.root())
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(&format!(
        "prepare  {key}  missing  {}",
        env.out_dir("prepare", &key).display()
    )));
}

// --- Sealing ----------------------------------------------------------------------------------

#[test] // rq-de9819a0 rq-719acfdc rq-533606d2 rq-9f3a6366
fn seal_a_completed_stage() {
    let env = Env::new();
    let dir = env.outputs(
        "prepare",
        &[("1abc.pdb", "ATOM 1\n"), ("2xyz.pdb", "ATOM 2\n")],
    );
    let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args([
            "stage",
            "seal",
            "x-vs-docking",
            "prepare",
            "--key",
            &env.keys("base")["prepare"],
        ])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let record = ProvenanceRecord::read(&dir).unwrap();
    let json = &record.json;
    assert_eq!(record.key, env.keys("base")["prepare"]);
    let paths: Vec<_> = record.outputs.iter().map(|o| o.path.as_str()).collect();
    assert_eq!(paths, ["1abc.pdb", "2xyz.pdb"]);
    assert_eq!(
        record.outputs[0].sha256,
        tophat::data::hash_file(&dir.join("1abc.pdb")).unwrap().1
    );
    let raw_dir = env.base.join("raw").join("bench").join("v2");
    assert_eq!(
        record.run,
        format!(
            "julia --project=. scripts/prepare.jl '{}' '{}'",
            raw_dir.display(),
            dir.display()
        )
    );
    assert_eq!(json["code"][0]["path"], "scripts/prepare.jl");
    assert_eq!(json["params"], serde_json::json!({"cutoff": 2.0}));
    assert_eq!(json["inputs"][0]["ref"], "benchmark-set@v2");
    assert_eq!(json["env"]["name"], "default");
    assert_eq!(record.commit.as_deref(), Some(env.head().as_str()));
    assert!(record.clean);
    assert!(record.unclean_files.is_empty());
    assert_eq!(json["trial"], false);
    assert!(json["platform"].as_str().unwrap().contains('-'));
}

#[test] // rq-9fa80dbe rq-719acfdc
fn seal_refuses_when_the_configuration_changed_during_the_run() {
    let env = Env::new();
    let started = env.keys("base")["prepare"].clone();
    let dir = env.outputs("prepare", &[("1abc.pdb", "ATOM 1\n")]);
    env.write(
        "studies/x-vs-docking/scripts/prepare.jl",
        "println(\"edited\")\n",
    );
    let result = seal(
        &env.root(),
        &SealRequest {
            study: "x-vs-docking".into(),
            stage: "prepare".into(),
            key: started.clone(),
            ..SealRequest::default()
        },
    );
    assert!(
        matches!(result, Err(PipelineError::KeyMismatch { expected, .. }) if expected == started)
    );
    assert!(!dir.join("provenance.json").exists());
}

#[test] // rq-672050d2 rq-719acfdc
fn seal_refuses_an_empty_output() {
    let env = Env::new();
    fs::create_dir_all(env.out_dir("prepare", &env.keys("base")["prepare"])).unwrap();
    assert!(matches!(
        env.seal("prepare", None),
        Err(PipelineError::NoOutputs(_))
    ));
}

#[test] // rq-a54ab3d8 rq-719acfdc
fn seal_checks_the_result_file() {
    let env = Env::new();
    env.outputs("analyse", &[("result.txt", "rho=0.61\nrho=0.58\n")]);
    assert!(matches!(
        env.seal("analyse", None),
        Err(PipelineError::BadResult(_))
    ));
    env.outputs("analyse", &[("result.txt", "rho=0.61\n")]);
    assert!(env.seal("analyse", None).is_ok());
}

#[test] // rq-383b2154 rq-9f3a6366
fn uncommitted_code_makes_the_record_unclean() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/scripts/prepare.jl",
        "println(\"uncommitted\")\n",
    );
    let dir = env.prepared();
    let record = ProvenanceRecord::read(&dir).unwrap();
    assert!(!record.clean);
    assert_eq!(
        record.unclean_files,
        ["studies/x-vs-docking/scripts/prepare.jl"]
    );
}

#[test] // rq-6a6e86ca rq-9f3a6366
fn untracked_config_makes_the_record_unclean() {
    let env = Env::new();
    env.prepared();
    env.write(
        "studies/x-vs-docking/config/pbe.yaml",
        "stages: [{name: score, params: {level: pbe}}]\n",
    );
    let key = env.keys("pbe")["score"].clone();
    fs::create_dir_all(env.out_dir("score", &key).join("items/1abc")).unwrap();
    fs::write(
        env.out_dir("score", &key).join("items/1abc/score.json"),
        "{}",
    )
    .unwrap();
    let path = env.seal_variant("score", "pbe", Some("1abc")).unwrap();
    let record = ProvenanceRecord::parse(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(!record.clean);
    assert!(
        record
            .unclean_files
            .contains(&"studies/x-vs-docking/config/pbe.yaml".to_string())
    );
}

#[test] // rq-c85070d7 rq-9f3a6366
fn changes_elsewhere_do_not_make_a_record_unclean() {
    let env = Env::new();
    env.write("studies/other-study/scripts/x.py", "print('wip')\n");
    env.write("notes.md", "uncommitted\n");
    let dir = env.prepared();
    assert!(ProvenanceRecord::read(&dir).unwrap().clean);
}

#[test] // rq-9f3a6366
fn ignored_code_is_unclean() {
    let env = Env::new();
    env.write(
        ".gitignore",
        ".tophat/roots.local.yaml\n.tophat/cache/\n*.jl\n",
    );
    env.git(&[
        "rm",
        "-q",
        "--cached",
        "studies/x-vs-docking/scripts/prepare.jl",
    ]);
    env.commit_all();
    let dir = env.prepared();
    let record = ProvenanceRecord::read(&dir).unwrap();
    assert!(!record.clean);
    assert_eq!(
        record.unclean_files,
        ["studies/x-vs-docking/scripts/prepare.jl"]
    );
}

#[test] // rq-29ee60f1 rq-9f3a6366
fn outside_a_git_repository_records_are_unclean() {
    let env = Env::bare();
    let dir = env.prepared();
    let record = ProvenanceRecord::read(&dir).unwrap();
    assert!(!record.clean);
    assert_eq!(record.unclean_files, ["<no git>"]);
    assert_eq!(record.commit, None);
    assert!(record.json["commit"].is_null());
}

#[test] // rq-96ee56c2 rq-719acfdc rq-533606d2
fn seal_a_fan_out_item_and_the_stage() {
    let env = Env::new();
    env.prepared();
    let key = env.keys("base")["score"].clone();
    let dir = env.out_dir("score", &key);
    for item in ["1abc", "2xyz"] {
        fs::create_dir_all(dir.join("items").join(item)).unwrap();
        fs::write(
            dir.join("items").join(item).join("score.json"),
            format!("{{\"{item}\": 1}}"),
        )
        .unwrap();
        env.seal("score", Some(item)).unwrap();
        let record = ProvenanceRecord::read(&dir.join("items").join(item)).unwrap();
        assert_eq!(record.item.as_deref(), Some(item));
        assert!(record.run.contains(&format!("{item}.pdb")));
    }
    env.seal("score", None).unwrap();
    let stage = ProvenanceRecord::read(&dir).unwrap();
    assert!(stage.run.contains("{item}"), "{}", stage.run);
    let items = stage.items.unwrap();
    assert_eq!(
        items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["1abc", "2xyz"]
    );
    let item_record = ProvenanceRecord::read(&dir.join("items/1abc")).unwrap();
    assert_eq!(items[0].outputs_hash, item_record.outputs_hash());
    assert!(stage.outputs.is_empty());
}

#[test] // rq-2f5e3dcb rq-719acfdc
fn stage_level_seal_of_a_fan_out_stage_requires_all_items() {
    let env = Env::new();
    env.prepared();
    let dir = env.out_dir("score", &env.keys("base")["score"]);
    fs::create_dir_all(dir.join("items/1abc")).unwrap();
    fs::write(dir.join("items/1abc/score.json"), "{}").unwrap();
    env.seal("score", Some("1abc")).unwrap();
    assert_eq!(
        env.seal("score", None),
        Err(PipelineError::ItemsUnsealed(vec!["2xyz".into()]))
    );
}

// --- Keys command -----------------------------------------------------------------------------

#[test] // rq-3b86c7d4 rq-c8637010
fn keys_command_reports_sealed_and_missing_outputs() {
    let env = Env::new();
    env.prepared();
    let status = key_status(&env.root(), "x-vs-docking", "base").unwrap();
    let states: Vec<_> = status
        .iter()
        .map(|s| (s.stage.as_str(), s.status))
        .collect();
    assert_eq!(
        states,
        [
            ("prepare", "sealed"),
            ("score", "missing"),
            ("analyse", "missing")
        ]
    );
}

#[test] // rq-11940cec rq-c8637010
fn keys_command_with_the_default_results_root() {
    let env = Env::new();
    env.configure(false);
    let status = key_status(&env.root(), "x-vs-docking", "base").unwrap();
    let results = std::path::absolute(env.root()).unwrap().join("results");
    assert!(
        status.iter().all(|s| s.key.starts_with("sk-")
            && s.output.as_ref().is_some_and(|o| o.starts_with(&results)))
    );
}

// --- Publishing (rqm/data.md) -----------------------------------------------------------------

#[test] // rq-dbb8600d rq-4fc6c2a5
fn publish_a_clean_stage_output() {
    let env = Env::new();
    let dir = env.prepared();
    let reference = publish(&env.root(), "x-vs-docking", "prepare", None, "prepared-set").unwrap();
    assert_eq!(reference.to_string(), "prepared-set@v1");
    let manifest = parse_manifest(
        &fs::read_to_string(env.root().join("datasets/prepared-set/v1.yaml")).unwrap(),
    )
    .unwrap();
    let key = env.keys("base")["prepare"].clone();
    assert_eq!(manifest.kind, DatasetKind::Published);
    assert_eq!(manifest.root, "results");
    assert_eq!(manifest.path, format!("x-vs-docking/prepare/{key}"));
    let produced = manifest.produced_by.clone().unwrap();
    assert_eq!(
        (
            produced.study.as_str(),
            produced.stage.as_str(),
            produced.key.as_str()
        ),
        ("x-vs-docking", "prepare", key.as_str())
    );
    assert_eq!(manifest.visibility, "public");
    let record = ProvenanceRecord::read(&dir).unwrap();
    for output in &record.outputs {
        assert!(
            manifest
                .files
                .iter()
                .any(|f| f.path == output.path && f.sha256 == output.sha256)
        );
    }
    assert_eq!(
        verify(&env.root(), Some("prepared-set@v1"))
            .unwrap()
            .problems,
        0
    );
}

#[test] // rq-510dffe3 rq-4fc6c2a5
fn publishing_an_unsealed_output_fails() {
    let env = Env::new();
    env.outputs("prepare", &[("1abc.pdb", "ATOM 1\n")]);
    assert!(matches!(
        publish(&env.root(), "x-vs-docking", "prepare", None, "prepared-set"),
        Err(DataError::NotSealed(_))
    ));
}

#[test] // rq-1abe7472 rq-4fc6c2a5
fn publishing_a_dirty_output_fails() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/scripts/prepare.jl",
        "println(\"wip\")\n",
    );
    env.prepared();
    assert!(matches!(
        publish(&env.root(), "x-vs-docking", "prepare", None, "prepared-set"),
        Err(DataError::NotClean(_))
    ));
}

#[test] // rq-205358e8 rq-4fc6c2a5
fn publishing_an_output_modified_after_sealing_fails() {
    let env = Env::new();
    let dir = env.prepared();
    fs::write(dir.join("1abc.pdb"), "ATOM X\n").unwrap();
    assert_eq!(
        publish(&env.root(), "x-vs-docking", "prepare", None, "prepared-set"),
        Err(DataError::OutputMismatch(vec!["1abc.pdb".into()]))
    );
    assert!(!env.root().join("datasets/prepared-set").exists());
}

#[test] // rq-4fc6c2a5
fn publish_a_fan_out_stage_includes_items() {
    let env = Env::new();
    env.prepared();
    let dir = env.out_dir("score", &env.keys("base")["score"]);
    for item in ["1abc", "2xyz"] {
        fs::create_dir_all(dir.join("items").join(item)).unwrap();
        fs::write(dir.join("items").join(item).join("score.json"), item).unwrap();
        env.seal("score", Some(item)).unwrap();
    }
    env.seal("score", None).unwrap();
    publish(&env.root(), "x-vs-docking", "score", None, "scores").unwrap();
    let manifest =
        parse_manifest(&fs::read_to_string(env.root().join("datasets/scores/v1.yaml")).unwrap())
            .unwrap();
    assert!(
        manifest
            .files
            .iter()
            .any(|f| f.path == "items/1abc/score.json")
    );
    assert_eq!(verify(&env.root(), Some("scores@v1")).unwrap().problems, 0);
}

#[test] // rq-55bef4eb
fn full_check_of_a_sealed_project_has_no_pipeline_or_data_errors() {
    let env = Env::new();
    env.prepared();
    let ds = tophat::check::run_check(&env.root(), &[]).unwrap();
    let relevant: Vec<_> = ds
        .iter()
        .filter(|d| d.rule.starts_with("pipeline.") || d.rule.starts_with("data."))
        .filter(|d| d.rule != "data.newer-version")
        .collect();
    assert_eq!(relevant, Vec::<&Diagnostic>::new());
}
