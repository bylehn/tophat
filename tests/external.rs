//! Tests for external (cluster) stages (rqm/pipeline/external.md).
//!
//! The job script runs locally under `sh` with a `PATH` holding only POSIX tools, in a directory
//! standing in for the cluster. The end-to-end scenario needs `TOPHAT_TEST_SNAKEMAKE` (see
//! tests/snakemake.rs).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use tophat::check::{Diagnostic, Project, Severity};
use tophat::pipeline::{
    ingest, load_pipeline, parse_receipt, pipeline_rules, seal, stage_key_text, stub, waiting_jobs,
    write_job_folder, write_job_folder_with, ExternalError, KeyContext, ProvenanceRecord,
    SealRequest,
};

// --- Fixtures -------------------------------------------------------------------------------

const MD_RUN: &str = "sh -c 'echo energy > {out}/md.edr; head -c 1000 /dev/zero > {out}/md.trr'";

/// The `md` stage, with `{run}` and `{extra}` (more stage lines) to fill in.
const BASE: &str = "stages:
  - name: prepare
    run: sh scripts/prepare.sh {out}
    code: [scripts/prepare.sh]
    validate: test -s {out}/system.gro
  - name: md
    run: {run}
    inputs: {system: stage:prepare}
    threads: 4
{extra}    external:
      setup:
        - export MD_READY=1
      slurm:
        - --time=24:00:00
      keep: [\"*.trr\"]
  - name: analyse
    run: sh scripts/analyse.sh {in.md} {out}
    code: [scripts/analyse.sh]
    inputs: {md: stage:md}
    result: result.txt
";

const LOCK: &str = "version: 6\nenvironments:\n  default:\n    packages:\n      linux-64:\n      - conda: https://x/python.conda\n";

const FAKE_PIXI: &str = "#!/bin/sh\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";

/// Standard utilities the job script and the test commands may use on the stand-in cluster.
const POSIX_TOOLS: [&str; 15] = [
    "sh",
    "sha256sum",
    "shasum",
    "wc",
    "date",
    "find",
    "grep",
    "mkdir",
    "mv",
    "dirname",
    "uname",
    "ls",
    "head",
    "cat",
    "rm",
];

struct Env {
    base: PathBuf,
}

impl Env {
    /// A committed project with preregistered study `x-vs-docking` whose `prepare` is sealed.
    fn new() -> Self {
        Env::with_md(MD_RUN, "")
    }

    fn with_md(run: &str, extra: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-ext-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        let env = Env { base };
        for dir in ["project/.tophat", "results", "cluster", "bin"] {
            fs::create_dir_all(env.base.join(dir)).unwrap();
        }
        env.write(".tophat/roots.yaml", "roots:\n  results: {}\n");
        env.write(
            ".tophat/roots.local.yaml",
            &format!(
                "roots:\n  results: {}\n",
                env.base.join("results").display()
            ),
        );
        env.write("pixi.lock", LOCK);
        env.write("pixi.toml", "[workspace]\nname = \"p\"\n");
        env.write(
            ".gitignore",
            ".tophat/roots.local.yaml\n.tophat/slurm.local.yaml\n.tophat/cache/\n.tophat/generated/\n.snakemake/\n",
        );
        env.write(
            "studies/x-vs-docking/STUDY.md",
            "---\nstudy: x-vs-docking\nstatus: preregistered\nvisibility: public\nvariants: [base]\n---\n\n## Result\n\n",
        );
        env.set_md(run, extra);
        env.write(
            "studies/x-vs-docking/scripts/prepare.sh",
            "echo 'system 1' > \"$1/system.gro\"\n",
        );
        env.write(
            "studies/x-vs-docking/scripts/analyse.sh",
            "echo \"energy=$(cat \"$1/md.edr\")\" > \"$2/result.txt\"\n",
        );
        env.write("studies/x-vs-docking/Snakefile", &stub("x-vs-docking"));
        env.git(&["init", "-q"]);
        env.commit();
        let prepare = env.out_dir("prepare");
        fs::create_dir_all(&prepare).unwrap();
        fs::write(prepare.join("system.gro"), "system 1\n").unwrap();
        seal(&env.root(), &env.request("prepare")).unwrap();
        env
    }

    fn set_md(&self, run: &str, extra: &str) {
        self.write(
            "studies/x-vs-docking/config/base.yaml",
            &BASE.replace("{run}", run).replace("{extra}", extra),
        );
    }

    fn root(&self) -> PathBuf {
        self.base.join("project")
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn edit_config(&self, from: &str, to: &str) {
        let rel = "studies/x-vs-docking/config/base.yaml";
        let text = fs::read_to_string(self.root().join(rel)).unwrap();
        assert!(text.contains(from), "config lacks {from:?}");
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

    /// The key of a stage, computed only up to that stage.
    fn key(&self, stage: &str) -> String {
        let config = load_pipeline(&self.root(), "x-vs-docking", "base").unwrap();
        tophat::pipeline::compute_keys_until(&config, &KeyContext::new(&self.root()), Some(stage))
            .unwrap()
            .keys[stage]
            .clone()
    }

    fn key_text(&self, stage: &str) -> String {
        let config = load_pipeline(&self.root(), "x-vs-docking", "base").unwrap();
        stage_key_text(&config, stage, &KeyContext::new(&self.root())).unwrap()
    }

    fn out_dir(&self, stage: &str) -> PathBuf {
        self.base
            .join("results/x-vs-docking")
            .join(stage)
            .join(self.key(stage))
    }

    /// The job folder: `job/` in the stage's output directory.
    fn folder(&self) -> PathBuf {
        self.out_dir("md").join("job")
    }

    fn request(&self, stage: &str) -> SealRequest {
        SealRequest {
            study: "x-vs-docking".into(),
            stage: stage.into(),
            key: self.key(stage),
            variant: Some("base".into()),
            ..SealRequest::default()
        }
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        pipeline_rules(&Project::new(self.root()))
    }

    fn tophat(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_tophat"))
            .args(args)
            .current_dir(self.root())
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    /// `tophat stage external x-vs-docking md --key <key>`.
    fn stage_external(&self) -> (i32, String, String) {
        let key = self.key("md");
        self.tophat(&["stage", "external", "x-vs-docking", "md", "--key", &key])
    }

    /// The stand-in cluster's copy of the job folder.
    /// On the cluster the folder is named after the study, stage and key.
    fn remote(&self) -> PathBuf {
        self.base
            .join("cluster")
            .join(format!("x-vs-docking.md.{}", self.key("md")))
    }

    /// Copies the job folder to the stand-in cluster (step 1).
    fn copy_to_cluster(&self) {
        let _ = fs::remove_dir_all(self.remote());
        copy_dir(&self.folder(), &self.remote(), &|_| false);
    }

    /// A directory of links to the POSIX tools only.
    fn posix_path(&self) -> PathBuf {
        let dir = self.base.join("posix-bin");
        if !dir.exists() {
            fs::create_dir_all(&dir).unwrap();
            for tool in POSIX_TOOLS {
                for prefix in ["/usr/bin", "/bin"] {
                    let source = Path::new(prefix).join(tool);
                    if source.exists() {
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&source, dir.join(tool)).unwrap();
                        break;
                    }
                }
            }
        }
        dir
    }

    /// Runs `sh job.sh` on the stand-in cluster with only POSIX tools on `PATH` (step 2).
    fn run_job(&self) -> i32 {
        let status = Command::new(self.posix_path().join("sh"))
            .arg("job.sh")
            .env_clear()
            .env("PATH", self.posix_path())
            .current_dir(self.remote())
            .status()
            .unwrap();
        status.code().unwrap()
    }

    /// Copies the folder back, leaving `out/*.trr` on the cluster unless `with_kept` (step 3).
    fn copy_back(&self, with_kept: bool) {
        copy_dir(&self.remote(), &self.folder(), &|rel: &str| {
            !with_kept && rel.starts_with("out/") && rel.ends_with(".trr")
        });
    }

    /// Writes the job folder, runs it on the stand-in cluster, and copies it back.
    fn round_trip(&self, edit_remote: &dyn Fn(&Path)) {
        let (code, _, err) = self.stage_external();
        assert_eq!(code, 3, "{err}");
        self.copy_to_cluster();
        edit_remote(&self.remote());
        assert_eq!(self.run_job(), 0);
        self.copy_back(false);
    }

    fn receipt(&self) -> String {
        fs::read_to_string(self.folder().join("receipt.txt")).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Copies a directory recursively (like `rsync -a`), skipping files for which `exclude` holds
/// (given the `/`-separated path relative to `from`).
fn copy_dir(from: &Path, to: &Path, exclude: &dyn Fn(&str) -> bool) {
    fn walk(from: &Path, to: &Path, prefix: &str, exclude: &dyn Fn(&str) -> bool) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &to.join(&name), &rel, exclude);
            } else if !exclude(&rel) {
                fs::copy(entry.path(), to.join(&name)).unwrap();
            }
        }
    }
    walk(from, to, "", exclude);
}

fn errors<'a>(ds: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    ds.iter()
        .filter(|d| d.rule == rule && d.severity == Severity::Error)
        .collect()
}

fn edit_file(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{} lacks {from:?}", path.display());
    fs::write(path, text.replacen(from, to, 1)).unwrap();
}

// --- Configuration and keys -------------------------------------------------------------------

#[test] // rq-69a74980 rq-fd86ea5e rq-3898af16
fn an_external_stage_cannot_fan_out() {
    let env = Env::new();
    env.edit_config("    threads: 4\n", "    threads: 4\n    foreach: system\n");
    let ds = env.diagnostics();
    let found = errors(&ds, "pipeline.config-invalid");
    assert!(
        found
            .iter()
            .any(|d| d.message.contains("`md`") && d.message.contains("`foreach`")),
        "{ds:?}"
    );
    let md_line = fs::read_to_string(env.root().join("studies/x-vs-docking/config/base.yaml"))
        .unwrap()
        .lines()
        .position(|l| l.contains("name: md"))
        .unwrap() as u32
        + 1;
    assert!(found.iter().any(|d| d.line == Some(md_line)), "{found:?}");
}

#[test] // rq-51d60cdc rq-fd86ea5e rq-3898af16
fn an_external_stage_cannot_have_a_pixi_environment() {
    let env = Env::new();
    env.edit_config("    threads: 4\n", "    threads: 4\n    env: default\n");
    let ds = env.diagnostics();
    assert!(
        errors(&ds, "pipeline.config-invalid")
            .iter()
            .any(|d| d.message.contains("`env`")),
        "{ds:?}"
    );
}

#[test] // rq-75cc7ed6 rq-fd86ea5e rq-3898af16
fn an_external_stage_cannot_validate() {
    let env = Env::new();
    env.edit_config(
        "    threads: 4\n",
        "    threads: 4\n    validate: sh scripts/check.sh {out}\n",
    );
    let ds = env.diagnostics();
    let found: Vec<_> = errors(&ds, "pipeline.config-invalid")
        .into_iter()
        .filter(|d| d.message.contains("`validate`") && d.message.contains("`md`"))
        .collect();
    assert_eq!(found.len(), 1, "{ds:?}");
}

#[test] // rq-d8643ac5 rq-fd86ea5e rq-3898af16
fn connection_details_are_not_configuration() {
    let env = Env::new();
    env.edit_config(
        "    external:\n",
        "    external: {remote: archer}\n    external_old:\n",
    );
    // Replace the whole mapping: drop the old one's lines.
    let text =
        fs::read_to_string(env.root().join("studies/x-vs-docking/config/base.yaml")).unwrap();
    let start = text.find("    external_old:\n").unwrap();
    let end = text.find("  - name: analyse").unwrap();
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &format!("{}{}", &text[..start], &text[end..]),
    );
    let ds = env.diagnostics();
    assert!(
        errors(&ds, "pipeline.config-invalid")
            .iter()
            .any(|d| d.message.contains("`remote`")),
        "{ds:?}"
    );
}

#[test] // rq-fd86ea5e rq-3898af16
fn unavailable_placeholders_and_bad_external_values_are_reported() {
    for (from, to, expected) in [
        ("{out}/md.edr", "{item}/md.edr", "`{item}`"),
        (
            "    external:\n",
            "    external: [x]\n    old:\n",
            "must be a mapping",
        ),
        ("keep: [\"*.trr\"]", "keep: [\"../x\"]", "`keep` pattern"),
    ] {
        let env = Env::new();
        env.edit_config(from, to);
        let ds = env.diagnostics();
        assert!(
            errors(&ds, "pipeline.config-invalid")
                .iter()
                .any(|d| d.message.contains(expected)),
            "{to}: {ds:?}"
        );
    }
}

#[test] // rq-fd86ea5e
fn an_empty_external_mapping_is_valid() {
    let env = Env::new();
    let text =
        fs::read_to_string(env.root().join("studies/x-vs-docking/config/base.yaml")).unwrap();
    let start = text.find("    external:\n").unwrap();
    let end = text.find("  - name: analyse").unwrap();
    env.write(
        "studies/x-vs-docking/config/base.yaml",
        &format!("{}    external: {{}}\n{}", &text[..start], &text[end..]),
    );
    assert!(errors(&env.diagnostics(), "pipeline.config-invalid").is_empty());
    assert!(env
        .key_text("md")
        .contains("\nexternal {\"keep\":[],\"setup\":[]}\n"));
}

#[test] // rq-254e43a2 rq-23d21623
fn the_key_covers_setup_and_keep_not_resources() {
    let env = Env::new();
    let before = env.key("md");
    env.edit_config("--time=24:00:00", "--time=48:00:00");
    assert_eq!(env.key("md"), before);
    env.write(".tophat/slurm.local.yaml", "sbatch:\n  - --account=e123\n");
    assert_eq!(env.key("md"), before);
    env.edit_config("export MD_READY=1", "export MD_READY=2");
    assert_ne!(env.key("md"), before);
    let text = env.key_text("md");
    let lines: Vec<&str> = text.lines().collect();
    let env_line = lines.iter().position(|l| *l == "env external").unwrap();
    assert_eq!(
        lines[env_line + 1],
        "external {\"keep\":[\"*.trr\"],\"setup\":[\"export MD_READY=2\"]}"
    );
}

#[test] // rq-deeead81 rq-23d21623
fn local_stages_keep_their_keys() {
    let env = Env::new();
    let code = tophat::data::hash_file(&env.root().join("studies/x-vs-docking/scripts/prepare.sh"))
        .unwrap()
        .1;
    let ctx = KeyContext::new(&env.root());
    let expected = format!(
        "tophat-stage-key/1\nrun sh scripts/prepare.sh {{out}}\nvalidate test -s {{out}}/system.gro\nenv {}\nthreads 1\nforeach -\n\
         result -\ncode scripts/prepare.sh {code}\nparams {{}}\n",
        ctx.environment_hash("default").unwrap()
    );
    assert_eq!(env.key_text("prepare"), expected);
    assert!(!env.key_text("analyse").contains("\nexternal "));
}

// --- Job folder and steps ---------------------------------------------------------------------

#[test] // rq-843afa31 rq-340771e5 rq-530610d9 rq-3de08c05
fn reaching_an_external_stage_writes_the_job_folder_and_waits() {
    let env = Env::new();
    let (code, out, err) = env.stage_external();
    assert_eq!(code, 3, "{err}");
    let folder = env.folder();
    for name in [
        "README.md",
        "job.sh",
        "stage.json",
        "inputs.sha256",
        "params.json",
    ] {
        assert!(folder.join(name).is_file(), "{name}");
    }
    assert_eq!(fs::read_dir(folder.join("out")).unwrap().count(), 0);
    assert_eq!(
        fs::read_to_string(folder.join("inputs/system/system.gro")).unwrap(),
        "system 1\n"
    );
    let sums = fs::read_to_string(folder.join("inputs.sha256")).unwrap();
    assert_eq!(
        sums,
        format!(
            "{}  inputs/system/system.gro\n",
            tophat::data::hash_file(&folder.join("inputs/system/system.gro"))
                .unwrap()
                .1
        )
    );
    let stage: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(folder.join("stage.json")).unwrap()).unwrap();
    assert_eq!(stage["key"], env.key("md"));
    assert_eq!(stage["keep"], serde_json::json!(["*.trr"]));
    assert!(out.contains(&folder.display().to_string()), "{out}");
    assert!(out.contains("sbatch job.sh"), "{out}");
}

#[test] // rq-3de08c05 rq-340771e5
fn the_job_folder_function_returns_its_path_and_is_idempotent() {
    let env = Env::new();
    let path = write_job_folder(&env.root(), &env.request("md")).unwrap();
    assert_eq!(path, env.folder());
    assert_eq!(
        write_job_folder(&env.root(), &env.request("md")).unwrap(),
        path
    );
    // No temporary directories are left behind.
    let leftovers: Vec<_> = fs::read_dir(env.out_dir("md"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty());
    // A stale key is refused.
    let mut stale = env.request("md");
    stale.key = "sk-0000000000000000".into();
    assert!(matches!(
        write_job_folder(&env.root(), &stale),
        Err(ExternalError::Pipeline(_))
    ));
}

#[test] // rq-dc6ab3b5 rq-9f1e97a5
fn the_steps_contain_no_connection_details() {
    let env = Env::new();
    let (_, out, _) = env.stage_external();
    let readme = fs::read_to_string(env.folder().join("README.md")).unwrap();
    for text in [&out, &readme] {
        assert!(text.contains("<you>@<cluster>:<directory>"), "{text}");
    }
    let (code, instructions, err) = env.tophat(&["stage", "instructions", "x-vs-docking", "md"]);
    assert_eq!(code, 0, "{err}");
    assert!(instructions.contains("<you>@<cluster>:<directory>"));
    assert!(instructions.contains(&env.folder().display().to_string()));
    // Nothing was written into the project: no host names or remote paths can be there.
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(env.root())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "");
}

#[test] // rq-3132a729 rq-9f1e97a5
fn return_steps_leave_kept_files_on_the_cluster() {
    let env = Env::new();
    let (_, out, _) = env.stage_external();
    let back = out
        .lines()
        .skip_while(|l| !l.starts_with("# 3."))
        .nth(1)
        .unwrap();
    assert!(back.starts_with("rsync -a "), "{back}");
    assert!(back.contains("--exclude='out/*.trr'"), "{back}");
    // Locally the folder is job/; on the cluster it is named after the study, stage and key.
    let remote = format!(
        "<you>@<cluster>:<directory>/x-vs-docking.md.{}/",
        env.key("md")
    );
    let local = format!("{}/", env.folder().display());
    let returned = out
        .lines()
        .skip_while(|l| !l.starts_with("# 3."))
        .nth(2)
        .unwrap();
    assert_eq!(returned.trim(), format!("{remote} {local}"), "{out}");
    let sent = out
        .lines()
        .skip_while(|l| !l.starts_with("# 1."))
        .nth(1)
        .unwrap();
    assert_eq!(sent, format!("rsync -a {local} {remote}"), "{out}");
}

#[test] // rq-9eddd05a
fn instructions_are_read_only_and_refuse_local_stages() {
    let env = Env::new();
    let (code, out, _) = env.tophat(&["stage", "instructions", "x-vs-docking", "md"]);
    assert_eq!(code, 0);
    assert!(out.contains("has not been written yet"), "{out}");
    assert!(!env.out_dir("md").exists());
    let (code, _, err) = env.tophat(&["stage", "instructions", "x-vs-docking", "prepare"]);
    assert_eq!(code, 1);
    assert!(err.contains("not an external stage"), "{err}");
}

#[test] // rq-96163724 rq-340771e5
fn an_existing_job_folder_is_never_overwritten() {
    let env = Env::new();
    env.stage_external();
    let job = env.folder().join("job.sh");
    edit_file(&job, "#SBATCH --time=24:00:00", "#SBATCH --time=30:00:00");
    let (code, _, _) = env.stage_external();
    assert_eq!(code, 3);
    assert!(fs::read_to_string(&job)
        .unwrap()
        .contains("#SBATCH --time=30:00:00\n"));
}

#[test] // rq-172dd12a rq-964a4533 rq-13602cac
fn slurm_defaults_and_stage_options_are_merged() {
    let env = Env::new();
    env.write(
        ".tophat/slurm.local.yaml",
        "sbatch:\n  - --account=e123\n  - --time=01:00:00\n",
    );
    env.stage_external();
    let job = fs::read_to_string(env.folder().join("job.sh")).unwrap();
    let sbatch: Vec<&str> = job.lines().filter(|l| l.starts_with("#SBATCH")).collect();
    assert_eq!(
        sbatch,
        vec![
            "#SBATCH --account=e123",
            "#SBATCH --time=24:00:00",
            "#SBATCH --cpus-per-task=4",
            "#SBATCH --job-name=md",
        ]
    );
    assert!(job.starts_with("#!/bin/sh\n#SBATCH "));
}

#[test] // rq-3f18bffc rq-340771e5
fn large_inputs_are_copied_with_a_warning() {
    // Copying 10 GB is too slow for a test; the threshold logic is checked directly, and the copy
    // path is the same for inputs of any size.
    let w =
        tophat::pipeline::large_inputs_warning(tophat::pipeline::LARGE_INPUTS_BYTES + 1).unwrap();
    assert!(w.contains("10.0 GB"), "{w}");
    let env = Env::new();
    let mut warnings = Vec::new();
    write_job_folder_with(&env.root(), &env.request("md"), &mut |w| {
        warnings.push(w.to_string())
    })
    .unwrap();
    assert!(warnings.is_empty());
    assert!(env.folder().join("inputs/system/system.gro").is_file());
}

#[test] // rq-c8a251ef rq-fd86ea5e
fn code_files_travel_with_the_job_folder() {
    let env = Env::with_md(
        "sh scripts/run-md.sh {out}",
        "    code: [scripts/run-md.sh]\n",
    );
    env.write(
        "studies/x-vs-docking/scripts/run-md.sh",
        "echo energy > \"$1/md.edr\"\n",
    );
    env.stage_external();
    assert_eq!(
        fs::read_to_string(env.folder().join("scripts/run-md.sh")).unwrap(),
        "echo energy > \"$1/md.edr\"\n"
    );
    // And the script runs from the job folder on the cluster.
    env.copy_to_cluster();
    assert_eq!(env.run_job(), 0);
    assert!(env.remote().join("out/md.edr").is_file());
}

// --- Job script and receipt -------------------------------------------------------------------

#[test] // rq-52e5c665 rq-13602cac rq-52dc4a39
fn the_job_script_writes_a_receipt() {
    let env = Env::new();
    env.stage_external();
    env.copy_to_cluster();
    assert_eq!(env.run_job(), 0);
    let receipt = fs::read_to_string(env.remote().join("receipt.txt")).unwrap();
    let parsed = parse_receipt(&receipt).unwrap();
    assert!(
        receipt.contains(&format!("\nkey {}\n", env.key("md"))),
        "{receipt}"
    );
    assert!(receipt.lines().any(|l| l.starts_with("script ")));
    assert!(receipt.contains("\nexit 0\n"), "{receipt}");
    assert_eq!(parsed.job, "-");
    let outputs: Vec<&str> = parsed.outputs.iter().map(|o| o.path.as_str()).collect();
    let kept: Vec<&str> = parsed.kept.iter().map(|o| o.path.as_str()).collect();
    assert_eq!(outputs, vec!["md.edr"]);
    assert_eq!(kept, vec!["md.trr"]);
    assert_eq!(parsed.kept[0].size, 1000);
    assert_eq!(parsed.outputs[0].size, 7);
    // The script hash is that of job.sh without its #SBATCH lines.
    let job = fs::read_to_string(env.remote().join("job.sh")).unwrap();
    assert_eq!(parsed.script, tophat::pipeline::script_sha256(&job));
}

#[test] // rq-bfa261cf rq-13602cac
fn the_job_refuses_changed_inputs() {
    let env = Env::new();
    env.stage_external();
    env.copy_to_cluster();
    fs::write(env.remote().join("inputs/system/system.gro"), "system 2\n").unwrap();
    assert_ne!(env.run_job(), 0);
    let receipt = fs::read_to_string(env.remote().join("receipt.txt")).unwrap();
    assert!(receipt.contains("\nexit input-mismatch\n"), "{receipt}");
    assert_eq!(fs::read_dir(env.remote().join("out")).unwrap().count(), 0);
}

#[test] // rq-bfa261cf rq-13602cac
fn the_job_refuses_changed_code_and_extra_inputs() {
    let env = Env::new();
    env.stage_external();
    env.copy_to_cluster();
    fs::write(env.remote().join("inputs/system/extra.gro"), "x\n").unwrap();
    assert_ne!(env.run_job(), 0);
    assert!(fs::read_to_string(env.remote().join("receipt.txt"))
        .unwrap()
        .contains("\nexit input-mismatch\n"));
    env.copy_to_cluster();
    fs::write(env.remote().join("params.json"), "{\"x\": 1}\n").unwrap();
    assert_ne!(env.run_job(), 0);
    assert!(fs::read_to_string(env.remote().join("receipt.txt"))
        .unwrap()
        .contains("\nexit input-mismatch\n"));
}

#[test] // rq-1b912496 rq-13602cac
fn the_receipt_is_written_when_the_command_fails() {
    let env = Env::with_md("sh -c 'echo partial > {out}/md.log; exit 7'", "");
    env.stage_external();
    env.copy_to_cluster();
    assert_eq!(env.run_job(), 7);
    let receipt = fs::read_to_string(env.remote().join("receipt.txt")).unwrap();
    assert!(receipt.contains("\nexit 7\n"), "{receipt}");
    assert!(parse_receipt(&receipt).unwrap().outputs[0].path == "md.log");
}

#[test] // rq-13602cac
fn a_job_that_already_ran_is_not_run_again() {
    let env = Env::new();
    env.stage_external();
    env.copy_to_cluster();
    assert_eq!(env.run_job(), 0);
    let receipt = fs::read_to_string(env.remote().join("receipt.txt")).unwrap();
    assert_eq!(env.run_job(), 2);
    assert_eq!(
        fs::read_to_string(env.remote().join("receipt.txt")).unwrap(),
        receipt
    );
}

#[test] // rq-82d88391 rq-d39d6009
fn receipts_are_parsed_strictly() {
    let hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let text = format!(
        "tophat-receipt 1\nstudy x-vs-docking\nstage md\nkey sk-3f9a1c2b4d5e6f70\nscript {hash}\n\
         host nid001234\njob 8812345\nstart 2026-10-21T08:00:03Z\nend 2026-10-21T19:42:51Z\nexit 0\n\
         output 9f86d081…  48211  md.edr\n"
    );
    let e = parse_receipt(&text).unwrap_err();
    assert!(
        e.contains("line 11") && e.contains("output 9f86d081…"),
        "{e}"
    );
}

// --- Ingest -----------------------------------------------------------------------------------

#[test] // rq-afe4a64f rq-a1948f0a rq-0ab3c68d rq-530610d9
fn ingest_a_returned_job_folder() {
    let env = Env::new();
    env.round_trip(&|_| {});
    let (code, out, err) = env.stage_external();
    assert_eq!(code, 0, "{out}{err}");
    let out_dir = env.out_dir("md");
    assert_eq!(
        fs::read_to_string(out_dir.join("md.edr")).unwrap(),
        "energy\n"
    );
    assert!(!env.folder().join("out/md.edr").exists());
    assert!(env.folder().join("receipt.txt").is_file());
    let record = ProvenanceRecord::read(&out_dir).unwrap();
    let outputs: Vec<&str> = record.outputs.iter().map(|o| o.path.as_str()).collect();
    assert_eq!(outputs, vec!["md.edr"]);
    let external = &record.json["external"];
    for field in [
        "host",
        "job",
        "start",
        "end",
        "setup",
        "script_sha256",
        "receipt_sha256",
        "modules",
    ] {
        assert!(!external[field].is_null(), "{field}: {external}");
    }
    assert_eq!(external["setup"], serde_json::json!(["export MD_READY=1"]));
    assert_eq!(external["kept"][0]["path"], "md.trr");
    assert_eq!(external["kept"][0]["size"], 1000);
    assert!(record.json["env"].is_null());
    assert_eq!(
        record.run,
        "sh -c 'echo energy > 'out'/md.edr; head -c 1000 /dev/zero > 'out'/md.trr'"
    );
    assert!(record.clean, "{:?}", record.unclean_files);
}

#[test] // rq-add5fac2 rq-a1948f0a
fn edited_sbatch_lines_are_accepted() {
    let env = Env::new();
    env.round_trip(&|remote| {
        edit_file(
            &remote.join("job.sh"),
            "#SBATCH --time=24:00:00",
            "#SBATCH --time=02:00:00",
        )
    });
    ingest(&env.root(), &env.request("md")).unwrap();
    assert!(ProvenanceRecord::read(&env.out_dir("md")).is_some());
}

#[test] // rq-3727b411 rq-a1948f0a
fn any_other_edit_to_the_job_script_is_refused() {
    let env = Env::new();
    env.round_trip(&|remote| edit_file(&remote.join("job.sh"), "echo energy", "echo ENERGY"));
    assert_eq!(
        ingest(&env.root(), &env.request("md")),
        Err(ExternalError::ScriptModified)
    );
    assert!(!env.out_dir("md").join("provenance.json").exists());
}

#[test] // rq-1d95d762 rq-a1948f0a
fn a_receipt_for_another_key_is_refused() {
    let env = Env::new();
    env.round_trip(&|_| {});
    let key = env.key("md");
    fs::write(
        env.folder().join("receipt.txt"),
        env.receipt()
            .replace(&format!("key {key}"), "key sk-0123456789abcdef"),
    )
    .unwrap();
    assert!(matches!(
        ingest(&env.root(), &env.request("md")),
        Err(ExternalError::KeyMismatch { expected, actual })
            if expected == key && actual == "sk-0123456789abcdef"
    ));
}

#[test] // rq-c0f8402d rq-a1948f0a
fn a_failed_job_is_refused() {
    let env = Env::new();
    env.round_trip(&|_| {});
    fs::write(
        env.folder().join("receipt.txt"),
        env.receipt().replace("\nexit 0\n", "\nexit 7\n"),
    )
    .unwrap();
    let e = ingest(&env.root(), &env.request("md")).unwrap_err();
    assert_eq!(e, ExternalError::JobFailed { exit: "7".into() });
    assert!(e.to_string().contains("`7`"));
    let (code, _, err) = env.stage_external();
    assert_eq!(code, 1);
    assert!(err.contains("`7`"), "{err}");
}

#[test] // rq-d571140f rq-a1948f0a
fn a_changed_returned_file_is_refused() {
    let env = Env::new();
    env.round_trip(&|_| {});
    fs::write(env.folder().join("out/md.edr"), "energy?\n").unwrap();
    match ingest(&env.root(), &env.request("md")) {
        Err(ExternalError::OutputMismatch(problems)) => {
            assert!(
                problems.iter().any(|p| p.contains("md.edr")),
                "{problems:?}"
            )
        }
        other => panic!("{other:?}"),
    }
    assert!(!env.out_dir("md").join("provenance.json").exists());
}

#[test] // rq-42c672ac rq-a1948f0a
fn an_unexpected_returned_file_is_refused() {
    let env = Env::new();
    env.round_trip(&|_| {});
    fs::write(env.folder().join("out/extra.dat"), "x\n").unwrap();
    match ingest(&env.root(), &env.request("md")) {
        Err(ExternalError::OutputMismatch(problems)) => {
            assert!(
                problems.iter().any(|p| p.contains("extra.dat")),
                "{problems:?}"
            )
        }
        other => panic!("{other:?}"),
    }
}

#[test] // rq-4865c985 rq-a1948f0a
fn kept_files_copied_back_anyway_are_accepted() {
    let env = Env::new();
    env.round_trip(&|_| {});
    env.copy_back(true);
    assert!(env.folder().join("out/md.trr").is_file());
    ingest(&env.root(), &env.request("md")).unwrap();
    let record = ProvenanceRecord::read(&env.out_dir("md")).unwrap();
    assert!(record.outputs.iter().all(|o| o.path != "md.trr"));
}

#[test] // rq-a1948f0a
fn an_interrupted_ingest_can_be_repeated() {
    let env = Env::new();
    env.round_trip(&|_| {});
    // As if an earlier ingest moved the output and stopped before sealing.
    fs::rename(
        env.folder().join("out/md.edr"),
        env.out_dir("md").join("md.edr"),
    )
    .unwrap();
    ingest(&env.root(), &env.request("md")).unwrap();
    assert!(ProvenanceRecord::read(&env.out_dir("md")).is_some());
}

#[test] // rq-a1948f0a rq-530610d9
fn external_stages_are_sealed_only_by_ingest() {
    let env = Env::new();
    let (code, _, err) = env.tophat(&[
        "stage",
        "seal",
        "x-vs-docking",
        "md",
        "--key",
        &env.key("md"),
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("tophat stage external"), "{err}");
}

#[test] // rq-9f1e97a5
fn a_written_job_folder_is_waiting_until_it_returns() {
    let env = Env::new();
    assert!(waiting_jobs(&env.root(), "x-vs-docking", "base").is_empty());
    env.stage_external();
    assert_eq!(
        waiting_jobs(&env.root(), "x-vs-docking", "base"),
        vec!["md"]
    );
    env.copy_to_cluster();
    env.run_job();
    env.copy_back(false);
    assert!(waiting_jobs(&env.root(), "x-vs-docking", "base").is_empty());
}

// --- End to end -------------------------------------------------------------------------------

#[test] // rq-5cf6fbfe rq-4acafba4
fn the_pipeline_stops_at_an_external_stage_and_resumes_after_the_return() {
    let env = Env::new();
    let rules = tophat::pipeline::generate_rules(&env.root(), "x-vs-docking").unwrap();
    assert!(rules.contains("external=True"), "{rules}");
    let Some(snakemake) = common::snakemake() else {
        return;
    };
    let bin = env.base.join("bin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(bin.join("pixi"), FAKE_PIXI).unwrap();
        fs::set_permissions(bin.join("pixi"), fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tophat"), bin.join("tophat")).unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let snakemake_run = || {
        let out = Command::new(&snakemake)
            .args(["-s", "studies/x-vs-docking/Snakefile", "--cores", "2"])
            .env("PATH", &path)
            .current_dir(env.root())
            .output()
            .unwrap();
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    let (ok, log) = snakemake_run();
    assert!(!ok, "{log}");
    assert!(log.contains("sbatch job.sh"), "{log}");
    assert_eq!(
        waiting_jobs(&env.root(), "x-vs-docking", "base"),
        vec!["md"]
    );
    env.copy_to_cluster();
    assert_eq!(env.run_job(), 0);
    env.copy_back(false);
    let key = env.key("md");
    env.write(
        &format!("studies/x-vs-docking/qc/md/{key}.md"),
        &format!("---\nstage: md\nkey: {key}\nverdict: pass\ndate: \"2026-10-21\"\n---\n\nok\n"),
    );
    let (ok, log) = snakemake_run();
    assert!(ok, "{log}");
    assert!(ProvenanceRecord::read(&env.out_dir("md")).is_some());
    let analyse = ProvenanceRecord::read(&env.out_dir("analyse")).unwrap();
    assert_eq!(analyse.key, env.key("analyse"));
}

// --- SLURM defaults ---------------------------------------------------------------------------

#[test] // rq-f9c4d1f4 rq-3898af16 rq-964a4533
fn invalid_slurm_defaults() {
    let env = Env::new();
    env.write(".tophat/slurm.local.yaml", "sbatch: account=e123\n");
    let ds = env.diagnostics();
    let found = errors(&ds, "pipeline.slurm-defaults-invalid");
    assert_eq!(found.len(), 1, "{ds:?}");
    assert_eq!(found[0].path, ".tophat/slurm.local.yaml");
    env.write(".tophat/slurm.local.yaml", "sbatch:\n  - account=e123\n");
    assert_eq!(
        errors(&env.diagnostics(), "pipeline.slurm-defaults-invalid").len(),
        1
    );
    env.write(".tophat/slurm.local.yaml", "sbatch:\n  - --account=e123\n");
    assert!(errors(&env.diagnostics(), "pipeline.slurm-defaults-invalid").is_empty());
}
