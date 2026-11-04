//! Tests for data roots, datasets and lineage (rqm/data.md).
//!
//! The `tophat data publish` scenarios need stage keys and are tested with
//! rqm/pipeline/provenance.md.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use chrono::NaiveDate;
use tophat::check::{Diagnostic, Project, Severity};
use tophat::data::{
    DataError, DatasetKind, FileEntry, Manifest, Origin, ProducedBy, RegisterOptions, add,
    content_hash, data_rules_with, declare, hash_file, lineage, load_roots_with, parse_manifest,
    retire, used_by, verify,
};
use tophat::study::{StudyError, report};

// --- Fixtures -------------------------------------------------------------------------------

/// A project with roots `raw`, `results` and `archer` declared; `raw` and `results` configured
/// at directories outside the project.
struct Env {
    base: PathBuf,
}

impl Env {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "data-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        let env = Env { base };
        for dir in ["project/.tophat", "raw", "results", "archer"] {
            fs::create_dir_all(env.base.join(dir)).unwrap();
        }
        env.write(
            ".tophat/roots.yaml",
            "roots:\n  raw:\n    description: Instrument data\n  results:\n    description: Outputs\n  \
             archer:\n    description: Cluster\n",
        );
        env.configure(&["raw", "results"]);
        env.write(
            "PROJECT.md",
            "# P\n\n## q-1a2b3c4d · Does X work?\nstatus: open\n",
        );
        env
    }

    fn root(&self) -> PathBuf {
        self.base.join("project")
    }

    fn data_dir(&self, root: &str) -> PathBuf {
        self.base.join(root)
    }

    /// Writes `.tophat/roots.local.yaml` configuring the given roots.
    fn configure(&self, roots: &[&str]) {
        let mut text = String::from("roots:\n");
        for r in roots {
            text.push_str(&format!("  {r}: {}\n", self.data_dir(r).display()));
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

    /// Writes a data file below a root.
    fn data(&self, root: &str, rel: &str, contents: &str) -> PathBuf {
        let path = self.data_dir(root).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        data_rules_with(&Project::new(self.root()), &|_| None)
    }

    /// Registers `<root>/<path>` (which must contain files) as a raw dataset.
    fn add(&self, name: &str, root: &str, path: &str) -> Result<String, DataError> {
        add(&self.root(), &raw_options(name, root, path)).map(|r| r.to_string())
    }

    /// Adds `benchmark-set@v1` with two files.
    fn benchmark(&self) {
        self.data("raw", "benchmark/2024-05/1abc.pdb", "ATOM 1\n");
        self.data("raw", "benchmark/2024-05/sub/2xyz.pdb", "ATOM 22\n");
        self.add("benchmark-set", "raw", "benchmark/2024-05")
            .unwrap();
    }

    /// Writes a minimal study with the given status, visibility, dataset inputs and amendments.
    fn study(&self, id: &str, status: &str, visibility: &str, inputs: &[&str], amendment: &str) {
        self.write(
            &format!("studies/{id}/STUDY.md"),
            &format!(
                "---\nstudy: {id}\ntitle: \"{id}\"\nquestion: q-1a2b3c4d\nstatus: {status}\n\
                 visibility: {visibility}\nvariants: [base]\n---\n\n## Amendments\n\n{amendment}\n\n## Result\n\n"
            ),
        );
        let mut config = String::from("stages:\n  - name: prepare\n    run: echo\n    inputs:\n");
        for (i, input) in inputs.iter().enumerate() {
            config.push_str(&format!("      in{i}: dataset:{input}\n"));
        }
        if inputs.is_empty() {
            config = "stages: []\n".into();
        }
        self.write(&format!("studies/{id}/config/base.yaml"), &config);
    }

    /// Writes a `provenance.json` for `<study>/<stage>@<key>` in the results root, with the given
    /// inputs, and returns the output directory.
    fn provenance(
        &self,
        study: &str,
        stage: &str,
        key: &str,
        inputs: serde_json::Value,
    ) -> PathBuf {
        let dir = self.data_dir("results").join(study).join(stage).join(key);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("out.txt"), format!("{study}/{stage}\n")).unwrap();
        let record = serde_json::json!({
            "format_version": 1, "study": study, "stage": stage, "key": key, "inputs": inputs,
        });
        fs::write(dir.join("provenance.json"), record.to_string()).unwrap();
        dir
    }

    /// Writes a published manifest for the output of `<study>/<stage>@<key>`.
    fn publish_manifest(&self, name: &str, study: &str, stage: &str, key: &str, visibility: &str) {
        let dir = self.data_dir("results").join(study).join(stage).join(key);
        let (size, sha256) = hash_file(&dir.join("out.txt")).unwrap();
        let files = vec![FileEntry {
            path: "out.txt".into(),
            size,
            sha256: sha256.clone(),
        }];
        let manifest = Manifest {
            name: name.into(),
            version: 1,
            kind: DatasetKind::Published,
            visibility: visibility.into(),
            root: "results".into(),
            path: format!("{study}/{stage}/{key}"),
            created: "2026-10-20T10:00:00Z".into(),
            origin: Origin::Published,
            produced_by: Some(ProducedBy {
                study: study.into(),
                stage: stage.into(),
                key: key.into(),
            }),
            content_hash: content_hash(&[("out.txt".into(), sha256)]),
            files,
            retired: None,
        };
        self.write(&format!("datasets/{name}/v1.yaml"), &manifest.render());
    }

    fn tophat(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_tophat"))
            .args(args)
            .current_dir(self.root())
            .envs(env.iter().copied())
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn raw_options(name: &str, root: &str, path: &str) -> RegisterOptions {
    RegisterOptions {
        name: name.into(),
        root: root.into(),
        path: path.into(),
        visibility: "public".into(),
        origin: Origin::Raw {
            source: "PDBbind".into(),
            date: None,
            license: None,
        },
    }
}

fn with_rule<'a>(ds: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    ds.iter().filter(|d| d.rule == rule).collect()
}

fn data_problems(ds: &[Diagnostic]) -> Vec<&Diagnostic> {
    ds.iter()
        .filter(|d| d.severity != Severity::Legacy)
        .collect()
}

const KEY_A: &str = "sk-00000000000000aa";
const KEY_B: &str = "sk-00000000000000bb";

// --- Roots ------------------------------------------------------------------------------------

#[test] // rq-49765877 rq-7ec07143
fn environment_variable_overrides_the_local_roots_file() {
    let env = Env::new();
    let elsewhere = env.base.join("elsewhere");
    fs::create_dir_all(elsewhere.join("set")).unwrap();
    fs::write(elsewhere.join("set/a.txt"), "a").unwrap();
    let roots = load_roots_with(&env.root(), &|v| {
        (v == "TOPHAT_ROOT_RAW").then(|| "/data/elsewhere".to_string())
    })
    .unwrap();
    assert_eq!(roots.location("raw"), Some(Path::new("/data/elsewhere")));

    let (code, stdout, stderr) = env.tophat(
        &[
            "data", "add", "env-set", "--root", "raw", "--path", "set", "--source", "x",
        ],
        &[("TOPHAT_ROOT_RAW", elsewhere.to_str().unwrap())],
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "env-set@v1\n");
}

#[test] // rq-bc2ad0ff rq-e4355ed1
fn missing_roots_file_declares_only_results() {
    let env = Env::new();
    fs::remove_file(env.root().join(".tophat/roots.yaml")).unwrap();
    fs::remove_file(env.root().join(".tophat/roots.local.yaml")).unwrap();
    let roots = load_roots_with(&env.root(), &|_| None).unwrap();
    assert_eq!(roots.declared.keys().collect::<Vec<_>>(), ["results"]);
}

#[test] // rq-b6cac057 rq-7ec07143
fn relative_local_location_is_rejected() {
    let env = Env::new();
    env.write(".tophat/roots.local.yaml", "roots:\n  raw: data/raw\n");
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.roots-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, ".tophat/roots.local.yaml");
}

#[test] // rq-b4146a69 rq-531eebf2
fn list_the_roots_and_their_locations() {
    let env = Env::new();
    let (code, out, err) = env.tophat(
        &["data", "roots"],
        &[("TOPHAT_ROOT_RAW", "/data/elsewhere")],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        format!(
            "archer\t-\nraw\t/data/elsewhere\nresults\t{}\n",
            env.data_dir("results").display()
        )
    );
}

#[test] // rq-f3dab8f0 rq-531eebf2
fn listing_invalid_roots_fails() {
    let env = Env::new();
    env.write(".tophat/roots.local.yaml", "roots:\n  raw: data/raw\n");
    let (code, out, err) = env.tophat(&["data", "roots"], &[]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains(".tophat/roots.local.yaml"), "{err}");
}

// --- Registering raw and declared data ---------------------------------------------------------

#[test] // rq-88716093 rq-af5ed3df rq-011facdf
fn add_a_raw_dataset() {
    let env = Env::new();
    env.data("raw", "benchmark/2024-05/1abc.pdb", "ATOM 1\n");
    env.data("raw", "benchmark/2024-05/sub/2xyz.pdb", "ATOM 22\n");
    let (code, stdout, stderr) = env.tophat(
        &[
            "data",
            "add",
            "benchmark-set",
            "--root",
            "raw",
            "--path",
            "benchmark/2024-05",
            "--source",
            "PDBbind",
        ],
        &[],
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "benchmark-set@v1\n");
    let m = parse_manifest(&env.read("datasets/benchmark-set/v1.yaml")).unwrap();
    assert_eq!(m.kind, DatasetKind::Raw);
    assert_eq!(m.visibility, "public");
    let paths: Vec<_> = m.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["1abc.pdb", "sub/2xyz.pdb"]);
    let (size, sha) = hash_file(&env.data_dir("raw").join("benchmark/2024-05/1abc.pdb")).unwrap();
    assert_eq!(
        (m.files[0].size, m.files[0].sha256.as_str()),
        (size, sha.as_str())
    );
    let cache = env.read(".tophat/cache/hashes.json");
    assert!(cache.contains("raw:benchmark/2024-05/1abc.pdb"));
    assert!(cache.contains("raw:benchmark/2024-05/sub/2xyz.pdb"));
}

#[test] // rq-622fce95 rq-af5ed3df
fn adding_again_creates_the_next_version() {
    let env = Env::new();
    env.benchmark();
    let v1 = env.read("datasets/benchmark-set/v1.yaml");
    env.data("raw", "benchmark/2024-06/3pqr.pdb", "ATOM 3\n");
    assert_eq!(
        env.add("benchmark-set", "raw", "benchmark/2024-06")
            .unwrap(),
        "benchmark-set@v2"
    );
    assert_eq!(env.read("datasets/benchmark-set/v1.yaml"), v1);
    assert_eq!(
        parse_manifest(&env.read("datasets/benchmark-set/v2.yaml"))
            .unwrap()
            .version,
        2
    );
}

#[test] // rq-71bac1e4 rq-af5ed3df
fn adding_from_an_unconfigured_root_fails() {
    let env = Env::new();
    env.data("archer", "runs/a.txt", "x");
    assert_eq!(
        env.add("traj-set", "archer", "runs"),
        Err(DataError::UnconfiguredRoot("archer".into()))
    );
    assert!(!env.root().join("datasets").exists());
}

#[test] // rq-872d0d8b rq-af5ed3df
fn adding_a_path_that_escapes_the_root_fails() {
    let env = Env::new();
    assert!(matches!(
        env.add("x-set", "raw", "../outside"),
        Err(DataError::PathOutsideRoot(_))
    ));
    assert!(matches!(
        env.add("x-set", "raw", "/etc"),
        Err(DataError::PathOutsideRoot(_))
    ));
}

#[test] // rq-44c48945 rq-af5ed3df
fn adding_an_empty_directory_fails() {
    let env = Env::new();
    fs::create_dir_all(env.data_dir("raw").join("empty/sub")).unwrap();
    assert!(matches!(
        env.add("x-set", "raw", "empty"),
        Err(DataError::NoFiles(_))
    ));
}

#[cfg(unix)]
#[test] // rq-701626f2 rq-af5ed3df
fn adding_a_directory_with_a_symbolic_link_fails() {
    use std::os::unix::fs::symlink;
    let env = Env::new();
    let target = env.data("raw", "elsewhere.txt", "x");
    env.data("raw", "linked/a.txt", "a");
    symlink(&target, env.data_dir("raw").join("linked/link.txt")).unwrap();
    assert!(matches!(
        env.add("x-set", "raw", "linked"),
        Err(DataError::SymbolicLink(_))
    ));
    assert!(!env.root().join("datasets").exists());
}

#[test] // rq-ce1e3132 rq-d89c8a6b
fn declare_data_of_unknown_history() {
    let env = Env::new();
    env.data("raw", "old-processed/a.csv", "1,2\n");
    let reference = declare(
        &env.root(),
        &RegisterOptions {
            name: "old-set".into(),
            root: "raw".into(),
            path: "old-processed".into(),
            visibility: "public".into(),
            origin: Origin::Declared {
                statement: "Processed in 2023 by unknown scripts".into(),
            },
        },
    )
    .unwrap();
    assert_eq!(reference.to_string(), "old-set@v1");
    let m = parse_manifest(&env.read("datasets/old-set/v1.yaml")).unwrap();
    assert_eq!(m.kind, DatasetKind::Declared);
    assert_eq!(
        m.origin,
        Origin::Declared {
            statement: "Processed in 2023 by unknown scripts".into()
        }
    );
}

// --- Manifests in the check -------------------------------------------------------------------

#[test] // rq-79aca0da rq-b2eccf77
fn valid_manifests_pass() {
    let env = Env::new();
    env.benchmark();
    assert_eq!(env.diagnostics(), vec![]);
}

#[test] // rq-1690899a rq-011facdf
fn hand_edited_content_hash_is_detected() {
    let env = Env::new();
    env.benchmark();
    let text = env.read("datasets/benchmark-set/v1.yaml");
    env.write(
        "datasets/benchmark-set/v1.yaml",
        &text.replacen("content_hash: \"sha256:", "content_hash: \"sha256:f", 1),
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.manifest-invalid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "datasets/benchmark-set/v1.yaml");
}

#[test] // rq-007499c3 rq-5b6f6e50
fn manifest_file_name_disagrees_with_its_contents() {
    let env = Env::new();
    env.benchmark();
    let text = env.read("datasets/benchmark-set/v1.yaml");
    env.write(
        "datasets/benchmark-set/v2.yaml",
        &text.replacen("version: 1", "version: 3", 1),
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.version-mismatch");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "datasets/benchmark-set/v2.yaml");
}

#[test] // rq-2ba558eb rq-5b6f6e50
fn version_gap() {
    let env = Env::new();
    env.benchmark();
    let text = env.read("datasets/benchmark-set/v1.yaml");
    env.write(
        "datasets/benchmark-set/v3.yaml",
        &text.replacen("version: 1", "version: 3", 1),
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.version-gap");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "datasets/benchmark-set");
}

#[test] // rq-95cf00a8 rq-84e20fdc
fn missing_data_file() {
    let env = Env::new();
    env.benchmark();
    fs::remove_file(env.data_dir("raw").join("benchmark/2024-05/1abc.pdb")).unwrap();
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.file-missing");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("1abc.pdb"));
}

#[test] // rq-2e2aead6 rq-84e20fdc
fn changed_file_size() {
    let env = Env::new();
    env.benchmark();
    let path = env.data_dir("raw").join("benchmark/2024-05/1abc.pdb");
    fs::write(&path, "ATOM 1\nATOM 2\n").unwrap();
    assert_eq!(with_rule(&env.diagnostics(), "data.file-changed").len(), 1);
}

#[test] // rq-7d9f7e52 rq-84e20fdc rq-731766dd
fn same_size_modification_is_flagged_by_the_cache() {
    let env = Env::new();
    env.benchmark();
    let path = env.data_dir("raw").join("benchmark/2024-05/1abc.pdb");
    fs::write(&path, "ATOM 9\n").unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.unverified");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    let report = verify(&env.root(), Some("benchmark-set@v1")).unwrap();
    assert_eq!(report.problems, 1);
    assert!(report.lines[0].contains("1abc.pdb"));
    let (code, _, _) = env.tophat(&["data", "verify", "benchmark-set@v1"], &[]);
    assert_eq!(code, 1);
}

/// Adds `traj-set@v1` and `@v2` in root `archer`, then unconfigures `archer`.
fn archer_datasets(env: &Env) {
    env.configure(&["raw", "results", "archer"]);
    env.data("archer", "runs/1/a.dcd", "frames");
    env.data("archer", "runs/2/a.dcd", "frames2");
    env.add("traj-set", "archer", "runs/1").unwrap();
    env.add("traj-set", "archer", "runs/2").unwrap();
    env.configure(&["raw", "results"]);
}

#[test] // rq-b418ac06 rq-84e20fdc
fn unconfigured_root_is_a_warning_not_an_error() {
    let env = Env::new();
    archer_datasets(&env);
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.root-unconfigured");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    assert!(found[0].message.contains("archer"));
    assert!(with_rule(&ds, "data.file-missing").is_empty());
}

#[test] // rq-2446b713 rq-731766dd
fn fresh_clone_without_a_hash_cache() {
    let env = Env::new();
    env.benchmark();
    fs::remove_file(env.root().join(".tophat/cache/hashes.json")).unwrap();
    let path = env.data_dir("raw").join("benchmark/2024-05/1abc.pdb");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert!(with_rule(&env.diagnostics(), "data.unverified").is_empty());
}

// --- Verification -----------------------------------------------------------------------------

#[test] // rq-43c471b8 rq-33c3cc6a rq-597bb615
fn verify_a_matching_dataset() {
    let env = Env::new();
    env.benchmark();
    fs::remove_file(env.root().join(".tophat/cache/hashes.json")).unwrap();
    let report = verify(&env.root(), Some("benchmark-set@v1")).unwrap();
    assert_eq!((report.verified, report.problems), (1, 0));
    assert!(env.read(".tophat/cache/hashes.json").contains("1abc.pdb"));
    let (code, stdout, _) = env.tophat(&["data", "verify", "benchmark-set@v1"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("1 dataset version(s) verified, 0 problem(s), 0 skipped"));
}

#[test] // rq-973f9875 rq-33c3cc6a
fn verify_reports_extra_files() {
    let env = Env::new();
    env.benchmark();
    env.data("raw", "benchmark/2024-05/new.pdb", "ATOM 4\n");
    let report = verify(&env.root(), Some("benchmark-set@v1")).unwrap();
    assert_eq!(report.problems, 1);
    assert!(report.lines[0].starts_with("EXTRA") && report.lines[0].contains("new.pdb"));
    assert_eq!(env.tophat(&["data", "verify"], &[]).0, 1);
}

#[test] // rq-b6d6ddcc rq-33c3cc6a
fn verify_skips_unconfigured_roots() {
    let env = Env::new();
    archer_datasets(&env);
    let report = verify(&env.root(), None).unwrap();
    assert_eq!(report.skipped, 2);
    assert!(
        report
            .lines
            .iter()
            .any(|l| l.starts_with("SKIPPED") && l.contains("traj-set@v1"))
    );
}

#[test] // rq-33c3cc6a
fn verify_unknown_dataset_fails() {
    let env = Env::new();
    assert!(matches!(
        verify(&env.root(), Some("nope-set")),
        Err(DataError::DatasetNotFound(_))
    ));
}

// --- Retiring and references ------------------------------------------------------------------

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()
}

#[test] // rq-98b54f50 rq-a7709a28
fn retire_a_dataset_version() {
    let env = Env::new();
    env.benchmark();
    let before = env.read("datasets/benchmark-set/v1.yaml");
    retire(
        &env.root(),
        "benchmark-set@v1",
        "Wrong protonation",
        today(),
    )
    .unwrap();
    let after = env.read("datasets/benchmark-set/v1.yaml");
    assert_eq!(
        after,
        format!("{before}retired:\n  date: \"2026-11-02\"\n  reason: \"Wrong protonation\"\n")
    );
    let m = parse_manifest(&after).unwrap();
    assert_eq!(m.retired.unwrap().reason, "Wrong protonation");
}

#[test] // rq-c3395d20 rq-a7709a28
fn retiring_twice_fails() {
    let env = Env::new();
    env.benchmark();
    retire(&env.root(), "benchmark-set@v1", "x", today()).unwrap();
    assert_eq!(
        retire(&env.root(), "benchmark-set@v1", "again", today()),
        Err(DataError::AlreadyRetired("benchmark-set@v1".into()))
    );
}

#[test] // rq-c25e83b5 rq-f1d5b29a
fn referencing_a_retired_version_is_an_error() {
    let env = Env::new();
    env.benchmark();
    retire(&env.root(), "benchmark-set@v1", "x", today()).unwrap();
    env.study("x-vs-docking", "draft", "public", &["benchmark-set@v1"], "");
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.reference-retired");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "studies/x-vs-docking/config/base.yaml");
    assert_eq!(found[0].line, Some(5));
}

#[test] // rq-dbcd9057 rq-f1d5b29a
fn referencing_an_unknown_version_is_an_error() {
    let env = Env::new();
    env.benchmark();
    env.study("x-vs-docking", "draft", "public", &["benchmark-set@v9"], "");
    assert_eq!(
        with_rule(&env.diagnostics(), "data.reference-unknown").len(),
        1
    );
}

/// Adds versions 1 and 2 of `benchmark-set`.
fn two_versions(env: &Env) {
    env.benchmark();
    env.data("raw", "benchmark/2024-06/3pqr.pdb", "ATOM 3\n");
    env.add("benchmark-set", "raw", "benchmark/2024-06")
        .unwrap();
}

#[test] // rq-0a389452 rq-f1d5b29a
fn newer_version_available_for_a_draft() {
    let env = Env::new();
    two_versions(&env);
    env.study("x-vs-docking", "draft", "public", &["benchmark-set@v1"], "");
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.newer-version");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test] // rq-f19caed4 rq-f1d5b29a
fn reported_study_pinned_to_an_older_version_is_not_warned() {
    let env = Env::new();
    two_versions(&env);
    env.study(
        "x-vs-docking",
        "reported",
        "public",
        &["benchmark-set@v1"],
        "",
    );
    assert!(with_rule(&env.diagnostics(), "data.newer-version").is_empty());
}

// --- Maturity and visibility ------------------------------------------------------------------

/// `clean-set@v1` published by study `clean-structures` with the given status.
fn published(env: &Env, producer_status: &str, visibility: &str) {
    env.study("clean-structures", producer_status, visibility, &[], "");
    env.provenance("clean-structures", "clean", KEY_A, serde_json::json!([]));
    env.publish_manifest("clean-set", "clean-structures", "clean", KEY_A, visibility);
}

#[test] // rq-fed9f56b rq-5512a963
fn consuming_unreported_work_while_still_in_draft_warns() {
    let env = Env::new();
    published(&env, "preregistered", "public");
    env.study("x-vs-docking", "draft", "public", &["clean-set@v1"], "");
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.producer-unreported");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test] // rq-228b4cbc rq-5512a963
fn a_reported_study_cannot_rest_on_unreported_work() {
    let env = Env::new();
    published(&env, "preregistered", "public");
    env.study("x-vs-docking", "reported", "public", &["clean-set@v1"], "");
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.producer-unreported");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Error);
}

#[test] // rq-5512a963
fn a_reported_producer_is_fine() {
    let env = Env::new();
    published(&env, "reported", "public");
    env.study("x-vs-docking", "reported", "public", &["clean-set@v1"], "");
    assert!(with_rule(&env.diagnostics(), "data.producer-unreported").is_empty());
}

#[test] // rq-0ca6fca2 rq-5512a963
fn an_amendment_accepts_the_unreported_dependency() {
    let env = Env::new();
    published(&env, "preregistered", "public");
    env.study(
        "x-vs-docking",
        "reported",
        "public",
        &["clean-set@v1"],
        "- 2026-10-01 — results seen: no — accept clean-set@v1 before its study is reported",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "data.producer-unreported");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test] // rq-d682f5a4 rq-5512a963
fn report_refuses_while_depending_on_unreported_work() {
    let env = Env::new();
    published(&env, "preregistered", "public");
    env.study(
        "x-vs-docking",
        "preregistered",
        "public",
        &["clean-set@v1"],
        "",
    );
    env.write(
        "studies/x-vs-docking/SUMMARY.md",
        "---\nstudy: x-vs-docking\ncommit: \"0123456789abcdef0123456789abcdef01234567\"\ndirty: false\n\
         generated: \"2026-10-01T12:00:00Z\"\ntophat_version: \"0.1.0\"\n---\n\n## Results\n\n- base/score: n=1\n",
    );
    let before = env.read("studies/x-vs-docking/STUDY.md");
    match report(&env.root(), "x-vs-docking") {
        Err(StudyError::Summary(message)) => assert!(message.contains("clean-set@v1"), "{message}"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(env.read("studies/x-vs-docking/STUDY.md"), before);
}

#[test] // rq-5dee52f5 rq-5512a963
fn public_study_consuming_private_data() {
    let env = Env::new();
    env.data("raw", "client/a.pdb", "x");
    add(
        &env.root(),
        &RegisterOptions {
            visibility: "private".into(),
            ..raw_options("client-set", "raw", "client")
        },
    )
    .unwrap();
    env.study("x-vs-docking", "draft", "public", &["client-set@v1"], "");
    assert_eq!(
        with_rule(&env.diagnostics(), "data.visibility-leak").len(),
        1
    );
    env.study("x-vs-docking", "draft", "private", &["client-set@v1"], "");
    assert!(with_rule(&env.diagnostics(), "data.visibility-leak").is_empty());
}

#[test] // rq-e4c5a373 rq-4c85d6f4
fn consuming_declared_data_is_flagged_as_legacy() {
    let env = Env::new();
    env.data("raw", "old/a.csv", "1\n");
    declare(
        &env.root(),
        &RegisterOptions {
            origin: Origin::Declared {
                statement: "unknown".into(),
            },
            ..raw_options("old-set", "raw", "old")
        },
    )
    .unwrap();
    env.study("x-vs-docking", "draft", "public", &["old-set@v1"], "");
    let ds = env.diagnostics();
    assert_eq!(with_rule(&ds, "data.declared-input").len(), 1);
    assert_eq!(
        with_rule(&ds, "data.declared-input")[0].severity,
        Severity::Legacy
    );
    assert_eq!(data_problems(&ds), Vec::<&Diagnostic>::new());
}

// --- Lineage ----------------------------------------------------------------------------------

#[test] // rq-7d7e180b rq-1a97b91b
fn lineage_of_a_published_dataset_reaches_raw_data() {
    let env = Env::new();
    env.benchmark();
    env.provenance(
        "clean-structures",
        "prepare",
        KEY_A,
        serde_json::json!([{"name": "raw", "kind": "dataset", "ref": "benchmark-set@v1", "content_hash": "sha256:x"}]),
    );
    env.provenance(
        "clean-structures",
        "clean",
        KEY_B,
        serde_json::json!([{"name": "prepared", "kind": "stage", "stage": "prepare", "key": KEY_A}]),
    );
    env.study("clean-structures", "reported", "public", &[], "");
    env.publish_manifest("clean-set", "clean-structures", "clean", KEY_B, "public");

    let tree = lineage(&env.root(), "clean-set@v1").unwrap();
    assert_eq!(
        tree.render(),
        format!(
            "clean-set@v1 (published)\n  clean-structures/clean@{KEY_B} (stage)\n    \
             clean-structures/prepare@{KEY_A} (stage)\n      benchmark-set@v1 (raw)\n"
        )
    );
    let (code, stdout, _) = env.tophat(&["data", "lineage", "clean-set@v1", "--json"], &[]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        json["inputs"][0]["inputs"][0]["inputs"][0]["label"],
        "benchmark-set@v1"
    );
}

#[test] // rq-1eaa34af rq-1a97b91b
fn lineage_stops_at_declared_data() {
    let env = Env::new();
    env.data("raw", "old/a.csv", "1\n");
    declare(
        &env.root(),
        &RegisterOptions {
            origin: Origin::Declared {
                statement: "unknown".into(),
            },
            ..raw_options("old-set", "raw", "old")
        },
    )
    .unwrap();
    env.provenance(
        "x-vs-docking",
        "prepare",
        KEY_A,
        serde_json::json!([{"name": "raw", "kind": "dataset", "ref": "old-set@v1", "content_hash": "sha256:x"}]),
    );
    let tree = lineage(&env.root(), &format!("x-vs-docking/prepare@{KEY_A}")).unwrap();
    assert_eq!(tree.inputs[0].label, "old-set@v1");
    assert_eq!(
        tree.inputs[0].mark.as_deref(),
        Some("[declared, not verified]")
    );
    assert!(tree.inputs[0].inputs.is_empty());
}

#[test] // rq-6c036885 rq-1a97b91b
fn lineage_through_an_unconfigured_root() {
    let env = Env::new();
    env.provenance("clean-structures", "clean", KEY_A, serde_json::json!([]));
    env.publish_manifest("clean-set", "clean-structures", "clean", KEY_A, "public");
    env.configure(&["raw"]);
    let tree = lineage(&env.root(), "clean-set@v1").unwrap();
    assert_eq!(tree.inputs[0].mark.as_deref(), Some("[unavailable]"));
    let (code, stdout, _) = env.tophat(&["data", "lineage", "clean-set@v1"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("[unavailable]"));
}

#[test] // rq-8ccc06c7 rq-23b73104
fn used_by_lists_consumers() {
    let env = Env::new();
    env.benchmark();
    env.study("x-vs-docking", "draft", "public", &["benchmark-set@v1"], "");
    env.study("x-energy", "draft", "public", &["benchmark-set@v1"], "");
    env.provenance(
        "clean-structures",
        "clean",
        KEY_A,
        serde_json::json!([{"name": "raw", "kind": "dataset", "ref": "benchmark-set@v1", "content_hash": "sha256:x"}]),
    );
    env.publish_manifest("clean-set", "clean-structures", "clean", KEY_A, "public");
    let used = used_by(&env.root(), "benchmark-set@v1").unwrap();
    assert_eq!(
        used.stages,
        vec![
            (
                "x-energy".into(),
                "base".into(),
                "prepare".into(),
                "benchmark-set@v1".into()
            ),
            (
                "x-vs-docking".into(),
                "base".into(),
                "prepare".into(),
                "benchmark-set@v1".into()
            ),
        ]
    );
    assert_eq!(
        used.datasets,
        vec![("clean-set@v1".into(), "benchmark-set@v1".into())]
    );
    let (code, stdout, _) = env.tophat(&["data", "used-by", "benchmark-set"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("study x-vs-docking variant base stage prepare uses benchmark-set@v1"));
    assert!(stdout.contains("dataset clean-set@v1 was derived from benchmark-set@v1"));
}

#[test] // rq-23b73104
fn used_by_follows_variant_overrides() {
    let env = Env::new();
    two_versions(&env);
    env.study("x-vs-docking", "draft", "public", &["benchmark-set@v1"], "");
    env.write(
        "studies/x-vs-docking/config/v2.yaml",
        "stages:\n  - name: prepare\n    inputs:\n      in0: dataset:benchmark-set@v2\n",
    );
    let used = used_by(&env.root(), "benchmark-set").unwrap();
    assert_eq!(
        used.stages,
        vec![
            (
                "x-vs-docking".into(),
                "base".into(),
                "prepare".into(),
                "benchmark-set@v1".into()
            ),
            (
                "x-vs-docking".into(),
                "v2".into(),
                "prepare".into(),
                "benchmark-set@v2".into()
            ),
        ]
    );
}
