//! The required CI workflows (rqm/testing.md §CI Workflows).

use yaml_rust2::{Yaml, YamlLoader};

const CI: &str = include_str!("../.github/workflows/CI.yaml");
const EVALS: &str = include_str!("../.github/workflows/evals.yaml");

fn document(text: &str) -> yaml_rust2::yaml::Hash {
    let docs = YamlLoader::load_from_str(text).expect("a workflow is valid YAML");
    docs.first()
        .and_then(Yaml::as_hash)
        .expect("a workflow is a mapping")
        .clone()
}

fn key<'a>(hash: &'a yaml_rust2::yaml::Hash, name: &str) -> &'a Yaml {
    hash.get(&Yaml::String(name.to_string()))
        .unwrap_or_else(|| panic!("the workflow has no `{name}`"))
}

#[test] // rq-068b6895
fn ci_workflow_has_the_nine_required_jobs() {
    let doc = document(CI);
    let jobs = key(&doc, "jobs").as_hash().expect("`jobs` is a mapping");
    let mut names: Vec<&str> = jobs.keys().filter_map(Yaml::as_str).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "adoption",
            "cli",
            "conformance",
            "e2e",
            "image",
            "render",
            "snakemake",
            "template",
            "traceability",
        ]
    );
}

#[test] // rq-068b6895
fn ci_workflow_runs_on_pull_requests_main_and_weekly() {
    let doc = document(CI);
    let on = key(&doc, "on")
        .as_hash()
        .expect("the workflow has triggers");
    assert!(on.contains_key(&Yaml::String("pull_request".into())));
    let push = key(on, "push").as_hash().expect("push is a mapping");
    let branches = key(push, "branches")
        .as_vec()
        .expect("push filters branches");
    assert!(branches
        .iter()
        .any(|branch| branch.as_str() == Some("main")));
    let schedule = key(on, "schedule").as_vec().expect("there is a schedule");
    assert!(schedule.iter().any(|entry| entry
        .as_hash()
        .is_some_and(|h| h.contains_key(&Yaml::String("cron".into())))));
}

#[test] // rq-068b6895
fn ci_workflow_runs_every_gate() {
    let doc = document(CI);
    let jobs = key(&doc, "jobs").as_hash().unwrap();
    let cli = key(jobs, "cli").as_hash().unwrap();
    let matrix = key(cli, "strategy").as_hash().unwrap();
    let matrix = key(matrix, "matrix").as_hash().unwrap();
    let os = key(matrix, "os").as_vec().unwrap();
    let os: Vec<&str> = os.iter().filter_map(Yaml::as_str).collect();
    assert_eq!(os, ["ubuntu-latest", "macos-latest", "windows-latest"]);

    for needle in [
        "cargo fmt --all --check",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "cargo test --workspace",
        "cargo llvm-cov",
        "TOPHAT_REQUIRE_SNAKEMAKE",
        "TOPHAT_REQUIRE_COPIER",
        "TOPHAT_TEST_PODMAN",
        "cargo xtask scenarios",
        "cargo xtask lock --check",
        "cargo xtask render-test",
        "cargo xtask conformance",
        "TOPHAT_TEST_E2E",
        "cargo xtask adoption",
    ] {
        assert!(CI.contains(needle), "the CI workflow lacks {needle:?}");
    }
}

#[test] // rq-068b6895
fn evals_workflow_is_weekly_manual_and_never_a_pr_check() {
    let doc = document(EVALS);
    let on = key(&doc, "on")
        .as_hash()
        .expect("the workflow has triggers");
    assert!(on.contains_key(&Yaml::String("schedule".into())));
    assert!(on.contains_key(&Yaml::String("workflow_dispatch".into())));
    assert!(
        !on.contains_key(&Yaml::String("pull_request".into())),
        "evals must never be a pull-request check"
    );
    assert!(EVALS.contains("cargo xtask eval"), "the evals do not run");
    assert!(
        EVALS.contains("${{ secrets."),
        "the evals do not read credentials from secrets"
    );
}
