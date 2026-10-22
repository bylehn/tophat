//! The end-to-end test (rqm/testing.md): the worked example study, run in a rendered project with the
//! project's own pixi environment, the way a human and an agent would, from pre-registration to
//! report, then a tool pin change and failing returns of an external job.
//!
//! It runs only with `TOPHAT_TEST_E2E=1`, and needs `copier`, `pixi`, `git` and `script` on `PATH`
//! and network access for the packages.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const STUDY: &str = "median-vs-mean";
/// Utilities a cluster offers: the job script must need nothing else.
const POSIX_TOOLS: [&str; 15] = [
    "sh",
    "sha256sum",
    "wc",
    "date",
    "find",
    "grep",
    "mkdir",
    "mv",
    "dirname",
    "uname",
    "ls",
    "awk",
    "cat",
    "head",
    "rm",
];

fn enabled() -> bool {
    let on = std::env::var_os("TOPHAT_TEST_E2E").is_some_and(|v| v == "1");
    if !on {
        eprintln!(
            "skipped: set TOPHAT_TEST_E2E=1 (with copier, pixi, git and script) to run this test"
        );
    }
    on
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Finds a program in `/usr/bin` or `/bin` (not through shell aliases).
fn system_program(name: &str) -> PathBuf {
    ["/usr/bin", "/bin"]
        .iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{name} is not installed"))
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Env {
    base: PathBuf,
    path: String,
    /// Snakemake jobs per rule, across the runs since the last reset.
    jobs: BTreeMap<String, u32>,
}

impl Env {
    fn new() -> Env {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("e2e");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("bin")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tophat"), base.join("bin/tophat")).unwrap();
        let posix = base.join("posix");
        fs::create_dir_all(&posix).unwrap();
        for tool in POSIX_TOOLS {
            #[cfg(unix)]
            std::os::unix::fs::symlink(system_program(tool), posix.join(tool)).unwrap();
        }
        let path = format!(
            "{}:{}",
            base.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Env {
            base,
            path,
            jobs: BTreeMap::new(),
        }
    }

    fn root(&self) -> PathBuf {
        self.base.join("project")
    }

    fn study(&self, rel: &str) -> PathBuf {
        self.root().join("studies").join(STUDY).join(rel)
    }

    fn command(&self, program: &str, dir: &Path) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(dir)
            .env("PATH", &self.path)
            .env_remove("TOPHAT_ROOT_RESULTS");
        cmd
    }

    fn tophat(&self, args: &[&str]) -> Output {
        self.command("tophat", &self.root())
            .args(args)
            .output()
            .unwrap()
    }

    fn git(&self, dir: &Path, args: &[&str]) {
        let out = self
            .command("git", dir)
            .args([
                "-c",
                "user.name=E2E",
                "-c",
                "user.email=e2e@example.org",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", text(&out));
    }

    fn commit(&self, message: &str) {
        self.git(&self.root(), &["add", "-A"]);
        self.git(&self.root(), &["commit", "-qm", message]);
    }

    /// Runs `command` in a pseudo-terminal, typing `input`, as a human at a terminal would.
    fn human(&self, command: &str, input: &str) -> String {
        let mut child = self
            .command("script", &self.root())
            .args(["-qec", command, "/dev/null"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{command}: {}", text(&out));
        text(&out)
    }

    /// `tophat run`, recording which Snakemake jobs ran.
    fn run(&mut self) -> Output {
        let out = self.tophat(&["run", STUDY, "--cores", "4"]);
        for (rule, n) in job_stats(&text(&out)) {
            *self.jobs.entry(rule).or_default() += n;
        }
        out
    }

    /// The study's next action from `tophat status --json`: (row, command).
    fn next(&self) -> (u64, String) {
        let out = self.tophat(&["status", "--json", STUDY]);
        let status: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let next = &status["studies"][0]["next"];
        (
            next["row"].as_u64().unwrap(),
            next["command"].as_str().unwrap_or_default().to_string(),
        )
    }

    /// The key and output directory of a stage.
    fn stage(&self, stage: &str, variant: &str) -> (String, PathBuf) {
        let out = self.tophat(&["pipeline", "keys", STUDY, "--variant", variant, "--json"]);
        let keys: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let entry = keys
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["stage"] == stage)
            .unwrap();
        (
            entry["key"].as_str().unwrap().to_string(),
            PathBuf::from(entry["output"].as_str().unwrap()),
        )
    }

    /// Runs a job folder on the stand-in cluster with only POSIX tools on PATH, as
    /// `<cluster>/<name>`, and returns that directory. `edit` changes the copy before it runs.
    fn run_on_cluster(&self, folder: &Path, name: &str, edit: &dyn Fn(&Path)) -> PathBuf {
        let remote = self.base.join("cluster").join(name);
        let _ = fs::remove_dir_all(&remote);
        copy_dir(folder, &remote);
        edit(&remote);
        let out = Command::new(self.base.join("posix/sh"))
            .arg("job.sh")
            .current_dir(&remote)
            .env_clear()
            .env("PATH", self.base.join("posix"))
            .output()
            .unwrap();
        assert!(out.status.success(), "job.sh in {name}: {}", text(&out));
        remote
    }

    /// The human's part of an external stage: the steps of `tophat stage instructions`, with a
    /// local directory standing in for the cluster.
    fn external_job(&self, stage: &str, variant: &str) {
        let (key, out_dir) = self.stage(stage, variant);
        let folder = out_dir.join("job");
        let steps =
            text(&self.tophat(&["stage", "instructions", STUDY, stage, "--variant", variant]));
        assert!(steps.contains("<you>@<cluster>:<directory>"), "{steps}");
        let remote = self.run_on_cluster(&folder, &format!("{STUDY}.{stage}.{key}"), &|_| {});
        copy_dir(&remote, &folder);
    }

    /// Follows the next actions until the study is ready to report, or, once reported, until the
    /// pipeline is current again.
    fn run_to_report(&mut self) {
        for _ in 0..20 {
            let (row, command) = self.next();
            let words: Vec<&str> = command.split_whitespace().collect();
            match row {
                8 => self.external_job(words[4], words[6]),
                9 => {
                    self.human(&command, "pass\nlooks as expected\n");
                }
                11 => {
                    self.run();
                }
                // Report (a study not reported yet), or answer the question (a reported one).
                13 | 14 => return,
                other => panic!("unexpected next action, row {other}: {command}"),
            }
        }
        panic!("the study did not reach its report");
    }
}

/// Snakemake's "Job stats" counts per rule (target rules excluded).
fn job_stats(log: &str) -> BTreeMap<String, u32> {
    let mut jobs = BTreeMap::new();
    let mut in_stats = false;
    for line in log.lines() {
        if line.starts_with("Job stats:") {
            in_stats = true;
            continue;
        }
        if !in_stats {
            continue;
        }
        match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            [rule, count] if count.parse::<u32>().is_ok() => {
                if !["total", "tophat_default", "summary"].contains(rule)
                    && !rule.starts_with("variant_")
                {
                    *jobs.entry(rule.to_string()).or_default() += count.parse::<u32>().unwrap();
                }
                if *rule == "summary" {
                    *jobs.entry("summary".into()).or_default() += count.parse::<u32>().unwrap();
                }
            }
            [] => in_stats = false,
            _ => {}
        }
    }
    jobs
}

/// The stage a Snakemake rule belongs to: `score_<hex>_item` → `score`.
fn rule_stage(rule: &str) -> &str {
    rule.split('_').next().unwrap_or(rule)
}

#[test] // rq-2da2ae3f rq-9b40e3a8 rq-6ffea465 rq-aea3767f
fn the_example_study_runs_end_to_end() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();

    // 1. A project with Julia, a stamped question, and the example copied into studies/.
    let out = env
        .command("copier", &env.base)
        .args(["copy", "--defaults", "--quiet", "--vcs-ref", "HEAD"])
        .args([
            "-d",
            "project_name=Filters",
            "-d",
            "project_description=Filter study",
        ])
        .args([
            "-d",
            "author_name=E2E",
            "-d",
            "author_email=e2e@example.org",
        ])
        .args(["-d", "languages=[\"julia\"]"])
        .arg(env!("CARGO_MANIFEST_DIR"))
        .arg(env.root())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let project_md = fs::read_to_string(env.root().join("PROJECT.md")).unwrap();
    fs::write(
        env.root().join("PROJECT.md"),
        format!("{project_md}\n## Does a median filter recover noisy step signals better than a mean filter?\nstatus: open\n"),
    )
    .unwrap();
    let out = env.tophat(&["question", "stamp"]);
    let question = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    copy_dir(
        &env.root().join(".tophat/managed/examples").join(STUDY),
        &env.root().join("studies").join(STUDY),
    );
    let study_md = fs::read_to_string(env.study("STUDY.md")).unwrap();
    fs::write(
        env.study("STUDY.md"),
        study_md.replace("question: q-00000000", &format!("question: {question}")),
    )
    .unwrap();

    // 2. A fixture tool providing the Julia scoring script, at two commits, with a
    //    project-supplied environment; `score` becomes a tool stage pinned to the first.
    let tool = env.base.join("scorer");
    fs::create_dir_all(&tool).unwrap();
    fs::copy(env.study("scripts/score.jl"), tool.join("score.jl")).unwrap();
    env.git(&tool, &["init", "-q"]);
    env.git(&tool, &["add", "-A"]);
    env.git(&tool, &["commit", "-qm", "first"]);
    let first = String::from_utf8_lossy(
        &env.command("git", &tool)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    let script = fs::read_to_string(tool.join("score.jl")).unwrap();
    fs::write(
        tool.join("score.jl"),
        format!("{script}# Second version.\n"),
    )
    .unwrap();
    env.git(&tool, &["commit", "-qam", "second"]);
    let second = String::from_utf8_lossy(
        &env.command("git", &tool)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    fs::write(
        env.root().join(".tophat/tools.yaml"),
        format!("tools:\n  scorer:\n    url: {}\n", tool.display()),
    )
    .unwrap();
    fs::create_dir_all(env.root().join("envs/scorer")).unwrap();
    for f in ["pixi.toml", "pixi.lock"] {
        fs::copy(env.root().join(f), env.root().join("envs/scorer").join(f)).unwrap();
    }
    let config_path = env.study("config/base.yaml");
    let config = fs::read_to_string(&config_path).unwrap();
    let local = "    run: julia scripts/score.jl {item} {out}\n    code: [scripts/score.jl]\n";
    assert!(config.contains(local), "{config}");
    let pin = |commit: &str| {
        format!(
            "    tool: scorer@{}\n    run: julia {{tool}}/score.jl {{item}} {{out}}\n",
            &commit[..7]
        )
    };
    fs::write(&config_path, config.replace(local, &pin(&first))).unwrap();
    let out = env.tophat(&["tools", "sync"]);
    assert!(out.status.success(), "{}", text(&out));

    // 3. Commit, then pre-register through a pseudo-terminal.
    env.git(&env.root(), &["init", "-q"]);
    env.commit("Plan and pipeline");
    let said = env.human(
        &format!("tophat study preregister {STUDY}"),
        &format!("{STUDY}\n"),
    );
    assert!(said.contains("is pre-registered"), "{said}");
    env.commit("Pre-register");

    // 4 and 5. Run, run the external jobs, give the verdicts, until the report.
    env.run_to_report();
    let summary = fs::read_to_string(env.study("SUMMARY.md")).unwrap();
    assert!(summary.contains("dirty: false"), "{summary}");
    let out = env.tophat(&["study", "report", STUDY]);
    assert!(out.status.success(), "{}", text(&out));
    let out = env.tophat(&["check", "--strict", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["summary"]["errors"], 0, "{report}");
    let (row, _) = env.next();
    assert_eq!(row, 14, "the next action is to answer the question");

    // 6. Outputs, provenance, summary and lineage.
    let study_md = fs::read_to_string(env.study("STUDY.md")).unwrap();
    for variant in ["base", "impulse"] {
        assert!(
            study_md.contains(&format!("- {variant}/analyse: n=30 ")),
            "{study_md}"
        );
        let (key, _) = env.stage("analyse", variant);
        assert!(
            summary.contains(&format!("\"{variant}/analyse\": \"{key}\"")),
            "{summary}"
        );
        let (_, filter_dir) = env.stage("filter", variant);
        let record: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(filter_dir.join("provenance.json")).unwrap())
                .unwrap();
        assert!(record["external"]["host"].is_string(), "{record}");
        assert_eq!(record["outputs"].as_array().unwrap().len(), 30);
        assert!(filter_dir.join("job/receipt.txt").is_file());
    }
    let (analyse_key, _) = env.stage("analyse", "base");
    let lineage =
        text(&env.tophat(&["data", "lineage", &format!("{STUDY}/analyse@{analyse_key}")]));
    assert!(lineage.contains("generate"), "{lineage}");

    // 7. A new tool commit re-runs exactly score, analyse and the summary.
    let before: BTreeMap<String, String> = ["generate", "filter", "score", "analyse"]
        .iter()
        .map(|s| (s.to_string(), env.stage(s, "base").0))
        .collect();
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(&config_path, config.replace(&pin(&first), &pin(&second))).unwrap();
    let out = env.tophat(&["tools", "sync"]);
    assert!(out.status.success(), "{}", text(&out));
    env.commit("Score with the second tool version");
    env.jobs.clear();
    env.run_to_report();
    let stages: std::collections::BTreeSet<&str> = env.jobs.keys().map(|r| rule_stage(r)).collect();
    assert_eq!(
        stages.into_iter().collect::<Vec<_>>(),
        ["analyse", "score", "summary"],
        "{:?}",
        env.jobs
    );
    assert_eq!(env.stage("generate", "base").0, before["generate"]);
    assert_eq!(env.stage("filter", "base").0, before["filter"]);
    assert_ne!(env.stage("score", "base").0, before["score"]);

    // 8. Failing returns of the external job: each is refused and writes no record.
    let config = fs::read_to_string(&config_path).unwrap();
    let external = "    external:\n";
    let with_setup = |line: &str| {
        config.replace(
            external,
            &format!("{external}      setup:\n        - {line}\n"),
        )
    };
    let ingest = |env: &Env| {
        let (key, _) = env.stage("filter", "base");
        env.tophat(&[
            "stage",
            "external",
            STUDY,
            "filter",
            "--key",
            &key,
            "--variant",
            "base",
        ])
    };
    fs::write(&config_path, with_setup("export E2E_SETUP=1")).unwrap();
    let (key_a, dir_a) = env.stage("filter", "base");
    assert_eq!(
        ingest(&env).status.code(),
        Some(3),
        "the job folder is written"
    );
    let returned_a = env.run_on_cluster(
        &dir_a.join("job"),
        &format!("{STUDY}.filter.{key_a}"),
        &|_| {},
    );

    // An output altered after the run.
    copy_dir(&returned_a, &dir_a.join("job"));
    fs::write(dir_a.join("job/out/sig-001.tsv"), "clean\tmean\tmedian\n").unwrap();
    let out = ingest(&env);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains("do not match the receipt"),
        "{}",
        text(&out)
    );
    assert!(!dir_a.join("provenance.json").exists());

    // A job script whose command line was edited before it ran.
    fs::remove_dir_all(dir_a.join("job")).unwrap();
    assert_eq!(
        ingest(&env).status.code(),
        Some(3),
        "the job folder is written again"
    );
    let edited = env.run_on_cluster(&dir_a.join("job"), "edited", &|remote| {
        let job = fs::read_to_string(remote.join("job.sh")).unwrap();
        assert!(job.contains("scripts/filter.sh"));
        fs::write(
            remote.join("job.sh"),
            job.replacen("scripts/filter.sh", "./scripts/filter.sh", 1),
        )
        .unwrap();
    });
    copy_dir(&edited, &dir_a.join("job"));
    let out = ingest(&env);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains("differs from the one tophat generates"),
        "{}",
        text(&out)
    );
    assert!(!dir_a.join("provenance.json").exists());

    // A folder returned after a configuration change: its receipt is for the old key.
    fs::write(&config_path, with_setup("export E2E_SETUP=2")).unwrap();
    let (key_b, dir_b) = env.stage("filter", "base");
    assert_ne!(key_b, key_a);
    assert_eq!(
        ingest(&env).status.code(),
        Some(3),
        "a new job folder is written"
    );
    copy_dir(&returned_a, &dir_b.join("job"));
    let out = ingest(&env);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains(&format!("is for key {key_a}")),
        "{}",
        text(&out)
    );
    assert!(!dir_b.join("provenance.json").exists());
}
