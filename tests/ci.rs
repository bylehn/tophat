//! The required CI workflows (rqm/testing.md §CI Workflows).

use yaml_rust2::{Yaml, YamlLoader};

const CI: &str = include_str!("../.github/workflows/CI.yaml");
const EVALS: &str = include_str!("../.github/workflows/evals.yaml");
const RELEASE: &str = include_str!("../.github/workflows/release.yaml");
const RELEASE_CHECK: &str = include_str!("../.github/workflows/release-check.yaml");

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
    assert!(
        branches
            .iter()
            .any(|branch| branch.as_str() == Some("main"))
    );
    let schedule = key(on, "schedule").as_vec().expect("there is a schedule");
    assert!(schedule.iter().any(|entry| {
        entry
            .as_hash()
            .is_some_and(|h| h.contains_key(&Yaml::String("cron".into())))
    }));
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

// --- Release ------------------------------------------------------------------------------------

fn release_job(name: &str) -> yaml_rust2::yaml::Hash {
    let doc = document(RELEASE);
    let jobs = key(&doc, "jobs").as_hash().expect("`jobs` is a mapping");
    key(jobs, name)
        .as_hash()
        .unwrap_or_else(|| panic!("the release workflow has no {name} job"))
        .clone()
}

/// The jobs a job waits for, whether `needs` is one name or a list.
fn needs(job: &yaml_rust2::yaml::Hash) -> Vec<String> {
    match job.get(&Yaml::String("needs".into())) {
        Some(Yaml::String(one)) => vec![one.clone()],
        Some(Yaml::Array(many)) => many
            .iter()
            .filter_map(Yaml::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// The `run` script of the step called `name`.
fn step_script(job: &yaml_rust2::yaml::Hash, name: &str) -> String {
    key(job, "steps")
        .as_vec()
        .expect("`steps` is a list")
        .iter()
        .filter_map(Yaml::as_hash)
        .find(|step| {
            step.get(&Yaml::String("name".into()))
                .and_then(Yaml::as_str)
                == Some(name)
        })
        .and_then(|step| step.get(&Yaml::String("run".into())))
        .and_then(Yaml::as_str)
        .unwrap_or_else(|| panic!("no step {name:?} with a script"))
        .to_string()
}

/// Every `run` script of a job, in order.
fn scripts(job: &yaml_rust2::yaml::Hash) -> String {
    key(job, "steps")
        .as_vec()
        .expect("`steps` is a list")
        .iter()
        .filter_map(Yaml::as_hash)
        .filter_map(|step| step.get(&Yaml::String("run".into())).and_then(Yaml::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test] // rq-e6ed00d3
fn release_workflow_publishes_only_after_the_gates_pass() {
    let doc = document(RELEASE);
    let on = key(&doc, "on")
        .as_hash()
        .expect("the workflow has triggers");
    assert!(on.contains_key(&Yaml::String("workflow_dispatch".into())));
    assert!(
        !on.contains_key(&Yaml::String("push".into())),
        "a pushed tag must not start a release"
    );

    assert!(RELEASE.contains("x86_64-unknown-linux-musl"));
    assert!(RELEASE.contains("aarch64-unknown-linux-musl"));
    assert!(
        RELEASE.contains("qemu-aarch64-static"),
        "the aarch64 binary is never run"
    );

    let gates = scripts(&release_job("gates"));
    for needle in [
        "cargo xtask conformance --tophat",
        "cargo xtask release-pin",
        "cargo xtask render-test",
        "--test e2e",
    ] {
        assert!(gates.contains(needle), "the gates job lacks {needle:?}");
    }
    assert!(RELEASE.contains("TOPHAT_TEST_E2E"));

    assert!(needs(&release_job("build")).contains(&"prepare".into()));
    assert!(needs(&release_job("gates")).contains(&"build".into()));
    assert!(needs(&release_job("publish")).contains(&"gates".into()));
    assert!(needs(&release_job("verify")).contains(&"publish".into()));

    for name in ["prepare", "build", "gates", "verify"] {
        assert!(
            !scripts(&release_job(name)).contains("gh release create"),
            "the {name} job uploads a release"
        );
    }
    let publish = scripts(&release_job("publish"));
    assert!(publish.contains("gh release create"));
    assert!(publish.contains("sha256sum"), "no checksums are published");
    let verify = scripts(&release_job("verify"));
    assert!(
        verify.contains("gh release download") && verify.contains("sha256sum -c"),
        "the draft's assets are not checked"
    );
}

#[test] // rq-e6ed00d3
fn release_workflow_scopes_write_access_to_publishing() {
    let doc = document(RELEASE);
    let permissions = key(&doc, "permissions")
        .as_hash()
        .expect("the workflow has permissions");
    assert_eq!(
        key(permissions, "contents").as_str(),
        Some("read"),
        "every release job gets write access"
    );
    for name in ["prepare", "build", "gates"] {
        let job = release_job(name);
        assert!(
            !job.contains_key(&Yaml::String("permissions".into())),
            "the {name} job overrides the read-only workflow permissions"
        );
    }
    for name in ["publish", "verify"] {
        let job = release_job(name);
        let permissions = key(&job, "permissions").as_hash().unwrap();
        assert_eq!(
            key(permissions, "contents").as_str(),
            Some("write"),
            "the {name} job cannot write the release"
        );
    }
}

#[test] // rq-8327b496
fn release_workflow_runs_only_from_main() {
    let prepare = release_job("prepare");
    let first = key(&prepare, "steps").as_vec().unwrap()[0]
        .as_hash()
        .unwrap()
        .clone();
    assert_eq!(
        key(&first, "name").as_str(),
        Some("Refuse a branch other than main"),
        "the branch is not checked first"
    );
    for name in ["build", "gates", "publish", "verify"] {
        assert!(
            needs(&release_job(name)).contains(&"prepare".into()),
            "the {name} job does not wait for the version check"
        );
    }
    #[cfg(unix)]
    {
        let script = step_script(&prepare, "Refuse a branch other than main");
        let dir = shell::temp_dir("branch");
        let feature = shell::run(&dir, &script, &[("GITHUB_REF", "refs/heads/feature")]);
        assert!(!feature.status.success());
        assert!(shell::stdout(&feature).contains("releases run from main"));
        let main = shell::run(&dir, &script, &[("GITHUB_REF", "refs/heads/main")]);
        assert!(main.status.success(), "{}", shell::stderr(&main));
    }
}

#[test] // rq-920bc7fe
fn release_workflow_refuses_a_version_that_is_already_tagged() {
    let script = step_script(
        &release_job("prepare"),
        "Read the version and refuse one that is already tagged",
    );
    assert!(script.contains("git ls-remote --exit-code --tags origin"));
    #[cfg(unix)]
    {
        let (origin, clone) = shell::repository("tagged", "0.1.0");
        shell::git(&origin, &["tag", "v0.1.0"]);
        let output = clone.join("output");
        let out = shell::run(
            &clone,
            &script,
            &[("GITHUB_OUTPUT", output.to_str().unwrap())],
        );
        assert!(!out.status.success());
        assert!(
            shell::stdout(&out).contains("the tag v0.1.0 already exists"),
            "{}",
            shell::stdout(&out)
        );
        assert!(!output.exists(), "a tagged version was accepted");

        let (_, clone) = shell::repository("untagged", "0.2.0");
        let output = clone.join("output");
        let out = shell::run(
            &clone,
            &script,
            &[("GITHUB_OUTPUT", output.to_str().unwrap())],
        );
        assert!(out.status.success(), "{}", shell::stderr(&out));
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "version=0.2.0\ntag=v0.2.0\n"
        );
    }
}

#[test] // rq-843b5970
fn release_pin_is_committed_on_the_commit_the_release_targets() {
    let publish = release_job("publish");
    let upload = step_script(&publish, "Upload the draft release");
    assert!(upload.contains("--draft"));
    assert!(
        upload.contains("--target \"${{ steps.pin.outputs.commit }}\""),
        "the draft does not target the pin commit"
    );
    assert!(
        !upload.contains("--verify-tag"),
        "the draft requires a tag that does not exist yet"
    );
    let verify = scripts(&release_job("verify"));
    assert!(
        verify.contains("targetCommitish"),
        "the draft's target is never checked"
    );

    #[cfg(unix)]
    {
        let script = step_script(&publish, "Commit the pin on top of the built commit");
        let (origin, clone) = shell::repository("pin", "0.2.0");
        let built = shell::git(&clone, &["rev-parse", "HEAD"]);
        let pin = "version: 0.2.0\nurl: https://example.org/v0.2.0\n";
        std::fs::create_dir_all(clone.join("pin")).unwrap();
        std::fs::write(clone.join("pin/tophat-release.yaml"), pin).unwrap();
        let output = clone.join("output");
        let out = shell::run(
            &clone,
            &script,
            &[
                ("GITHUB_SHA", built.as_str()),
                ("TAG", "v0.2.0"),
                ("GITHUB_OUTPUT", output.to_str().unwrap()),
            ],
        );
        assert!(out.status.success(), "{}", shell::stderr(&out));

        let branch = shell::git(&origin, &["rev-parse", "refs/heads/release/v0.2.0"]);
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            format!("commit={branch}\n")
        );
        assert_eq!(
            shell::git(&origin, &["rev-parse", &format!("{branch}^")]),
            built
        );
        assert_eq!(
            shell::git(
                &origin,
                &["diff", "--name-only", &format!("{branch}^"), &branch]
            ),
            "template/.tophat/managed/tophat-release.yaml"
        );
        assert_eq!(
            shell::git(
                &origin,
                &[
                    "show",
                    &format!("{branch}:template/.tophat/managed/tophat-release.yaml")
                ]
            ),
            pin.trim_end()
        );
        assert_eq!(
            shell::git(&origin, &["tag", "--list"]),
            "",
            "a tag was created"
        );

        // A rerun for the same version replaces the branch.
        let out = shell::run(
            &clone,
            &format!("git reset -q --hard \"$GITHUB_SHA\"\n{script}"),
            &[
                ("GITHUB_SHA", built.as_str()),
                ("TAG", "v0.2.0"),
                ("GITHUB_OUTPUT", output.to_str().unwrap()),
            ],
        );
        assert!(out.status.success(), "{}", shell::stderr(&out));
    }
}

#[test] // rq-d81f1b41
fn a_release_never_replaces_a_published_release() {
    let script = step_script(&release_job("publish"), "Upload the draft release");
    assert!(
        script.contains("isDraft"),
        "the upload does not check for a published release"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A fake `gh` that answers `release view` with $IS_DRAFT and logs every call.
        let dir = shell::temp_dir("published");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        std::fs::write(
            &gh,
            "#!/bin/sh\necho \"$*\" >> \"$GH_LOG\"\n\
             if [ \"$1 $2\" = \"release view\" ]; then echo \"$IS_DRAFT\"; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let script = script.replace("${{ steps.pin.outputs.commit }}", "0123abc");

        let published = dir.join("published.log");
        let out = shell::run(
            &dir,
            &script,
            &[
                ("PATH", path.as_str()),
                ("GH_LOG", published.to_str().unwrap()),
                ("IS_DRAFT", "false"),
                ("TAG", "v0.2.0"),
                ("VERSION", "0.2.0"),
                ("GITHUB_REPOSITORY", "o/r"),
            ],
        );
        assert!(!out.status.success(), "a published release was replaced");
        assert!(shell::stdout(&out).contains("already published"));
        let calls = std::fs::read_to_string(&published).unwrap();
        assert!(
            !calls.contains("release delete") && !calls.contains("release create"),
            "{calls}"
        );

        let draft = dir.join("draft.log");
        let out = shell::run(
            &dir,
            &script,
            &[
                ("PATH", path.as_str()),
                ("GH_LOG", draft.to_str().unwrap()),
                ("IS_DRAFT", "true"),
                ("TAG", "v0.2.0"),
                ("VERSION", "0.2.0"),
                ("GITHUB_REPOSITORY", "o/r"),
            ],
        );
        assert!(out.status.success(), "{}", shell::stderr(&out));
        let calls = std::fs::read_to_string(&draft).unwrap();
        let delete = calls
            .find("release delete v0.2.0")
            .expect("the old draft is not replaced");
        let create = calls
            .find("release create v0.2.0")
            .expect("no draft is uploaded");
        assert!(delete < create, "{calls}");
        assert!(
            calls.contains("--draft") && calls.contains("--target 0123abc"),
            "{calls}"
        );
    }
}

#[test] // rq-3057858e
fn only_the_maintainer_makes_a_release_public() {
    for forbidden in ["--draft=false", "gh release edit", "git tag", "refs/tags/v"] {
        assert!(
            !RELEASE.contains(forbidden),
            "the release workflow publishes or tags the release ({forbidden:?})"
        );
    }
    for line in RELEASE.lines().filter(|line| line.contains("git push")) {
        assert!(
            line.contains("refs/heads/release/"),
            "the release workflow pushes something other than its release branch: {line}"
        );
    }
    let verify = scripts(&release_job("verify"));
    assert!(
        verify.contains("isDraft") && verify.contains("refs/tags/$TAG"),
        "the verify job does not check that the release is still an untagged draft"
    );
}

#[test] // rq-9718d88b
fn a_published_release_is_checked_at_its_public_url() {
    let doc = document(RELEASE_CHECK);
    let on = key(&doc, "on")
        .as_hash()
        .expect("the workflow has triggers");
    let release = key(on, "release").as_hash().expect("it runs on releases");
    let types = key(release, "types").as_vec().unwrap();
    assert_eq!(types, &vec![Yaml::String("published".into())]);
    assert!(
        !on.contains_key(&Yaml::String("pull_request".into())),
        "the release check must never be a pull-request check"
    );
    let permissions = key(&doc, "permissions").as_hash().unwrap();
    assert_eq!(key(permissions, "contents").as_str(), Some("read"));

    assert!(RELEASE_CHECK.contains("ref: ${{ github.event.release.tag_name }}"));
    assert!(RELEASE_CHECK.contains("template/.tophat/managed/tophat-release.yaml"));
    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        assert!(RELEASE_CHECK.contains(target));
    }
    assert!(RELEASE_CHECK.contains("curl -fsSL"));
    assert!(RELEASE_CHECK.contains("sha256sum -c"));
    for credential in ["GH_TOKEN", "github.token", "secrets.", "Authorization"] {
        assert!(
            !RELEASE_CHECK.contains(credential),
            "the public download uses credentials ({credential})"
        );
    }
}

/// Runs workflow scripts the way GitHub's `bash` shell does, against throwaway git repositories.
#[cfg(unix)]
mod shell {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub fn temp_dir(label: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tophat-ci-{}-{}-{}",
            label,
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    pub fn run(dir: &Path, script: &str, env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new("bash");
        cmd.args(["--noprofile", "--norc", "-eo", "pipefail", "-c", script])
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for (name, value) in env {
            cmd.env(name, value);
        }
        cmd.output().unwrap()
    }

    pub fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.org"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        stdout(&out).trim_end().to_string()
    }

    /// A bare `origin` holding one commit with a Cargo.toml of `version` and an unreleased pin, and
    /// a clone of it.
    pub fn repository(label: &str, version: &str) -> (PathBuf, PathBuf) {
        let root = temp_dir(label);
        let origin = root.join("origin.git");
        let clone = root.join("clone");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        git(&root, &["clone", "-q", origin.to_str().unwrap(), "clone"]);
        std::fs::write(
            clone.join("Cargo.toml"),
            format!("[package]\nname = \"tophat\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
        let pin = clone.join("template/.tophat/managed/tophat-release.yaml");
        std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
        std::fs::write(&pin, "version: unreleased\n").unwrap();
        git(&clone, &["add", "-A"]);
        git(&clone, &["commit", "-qm", "built"]);
        git(&clone, &["push", "-q", "origin", "HEAD:main"]);
        (origin, clone)
    }

    pub fn stdout(out: &Output) -> String {
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn stderr(out: &Output) -> String {
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
}
