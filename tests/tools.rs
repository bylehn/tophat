//! Tests for tool stages (rqm/pipeline/tools.md), against a local git repository standing in for a
//! tool's remote.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use tophat::check::{Diagnostic, Project, Severity};
use tophat::pipeline::{
    begin, compute_keys, load_pipeline, pipeline_rules, seal, stage_key_text, KeyContext,
    ProvenanceRecord, SealRequest,
};
use tophat::tools::{sync, ToolPin};

const LOCK: &str = "version: 6\nenvironments:\n  default:\n    packages:\n      linux-64:\n      - conda: https://x/{pkg}.conda\n";

fn git(dir: &Path, args: &[&str]) -> String {
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
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

struct Env {
    base: PathBuf,
    /// Commits A, B (both with their own environment) and C (without one).
    a: String,
    b: String,
    c: String,
}

impl Env {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-tools-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("method-x");
        fs::create_dir_all(repo.join("bin")).unwrap();
        fs::create_dir_all(repo.join("data")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(
            repo.join("bin/x-score.sh"),
            "wc -c < \"$1\" > \"$2/score.txt\"\n",
        )
        .unwrap();
        fs::write(repo.join("data/ref.txt"), "reference\n").unwrap();
        fs::write(repo.join("pixi.toml"), "[workspace]\nname = \"method-x\"\n").unwrap();
        fs::write(repo.join("pixi.lock"), LOCK.replace("{pkg}", "rdkit-2024")).unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "A"]);
        let a = git(&repo, &["rev-parse", "HEAD"]);
        fs::write(repo.join("pixi.lock"), LOCK.replace("{pkg}", "rdkit-2025")).unwrap();
        git(&repo, &["commit", "-q", "-am", "B"]);
        let b = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["rm", "-q", "pixi.toml", "pixi.lock"]);
        git(&repo, &["commit", "-q", "-m", "C"]);
        let c = git(&repo, &["rev-parse", "HEAD"]);

        let env = Env { base, a, b, c };
        fs::create_dir_all(env.root().join(".tophat")).unwrap();
        fs::create_dir_all(env.base.join("results")).unwrap();
        env.write(".tophat/roots.yaml", "roots:\n  results: {}\n");
        env.write(
            ".tophat/roots.local.yaml",
            &format!(
                "roots:\n  results: {}\n",
                env.base.join("results").display()
            ),
        );
        env.write(
            ".gitignore",
            ".tophat/roots.local.yaml\n.tophat/cache/\n.tophat/tools/\n",
        );
        env.write("pixi.lock", &LOCK.replace("{pkg}", "python-3.12"));
        env.write("pixi.toml", "[workspace]\nname = \"p\"\n");
        env.write(
            ".tophat/tools.yaml",
            &format!("tools:\n  method-x:\n    url: {}\n", repo.display()),
        );
        env.write(
            "studies/x-vs-docking/STUDY.md",
            "---\nstudy: x-vs-docking\nvisibility: public\nvariants: [base]\n---\n",
        );
        env.write(
            "studies/x-vs-docking/Snakefile",
            &tophat::pipeline::stub("x-vs-docking"),
        );
        env.set_pin(&env.a[..7]);
        git(&env.root(), &["init", "-q"]);
        env.commit();
        env
    }

    fn set_pin(&self, pin: &str) {
        self.write(
            "studies/x-vs-docking/config/base.yaml",
            &format!(
                "stages:\n  - name: prepare\n    run: echo x > {{out}}/a.pdb\n    qc: skip\n    qc_reason: trivial\n  \
                 - name: score\n    tool: method-x@{pin}\n    run: sh {{tool}}/bin/x-score.sh {{in.structures}}/a.pdb {{out}}\n    \
                 inputs: {{structures: stage:prepare}}\n"
            ),
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

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root().join(rel)).unwrap()
    }

    fn commit(&self) {
        git(&self.root(), &["add", "-A"]);
        git(
            &self.root(),
            &["commit", "-q", "--allow-empty", "-m", "fixture"],
        );
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

    fn keys(&self, variant: &str) -> BTreeMap<String, String> {
        let config = load_pipeline(&self.root(), "x-vs-docking", variant).unwrap();
        compute_keys(&config, &KeyContext::new(&self.root()))
            .unwrap()
            .keys
    }

    /// The key of one stage, computed only up to that stage.
    fn key_of(&self, stage: &str) -> String {
        let config = load_pipeline(&self.root(), "x-vs-docking", "base").unwrap();
        tophat::pipeline::compute_keys_until(&config, &KeyContext::new(&self.root()), Some(stage))
            .unwrap()
            .keys[stage]
            .clone()
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        pipeline_rules(&Project::new(self.root()))
    }

    fn cache(&self, commit: &str) -> PathBuf {
        self.root().join(".tophat/tools/method-x").join(commit)
    }

    fn request(&self, stage: &str) -> SealRequest {
        SealRequest {
            study: "x-vs-docking".into(),
            stage: stage.into(),
            key: self.key_of(stage),
            ..SealRequest::default()
        }
    }

    /// Runs and seals `prepare`.
    fn prepare(&self) {
        let dir = self
            .base
            .join("results/x-vs-docking/prepare")
            .join(self.key_of("prepare"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.pdb"), "ATOM\n").unwrap();
        seal(&self.root(), &self.request("prepare")).unwrap();
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

// --- Sync and pinning -------------------------------------------------------------------------

#[test] // rq-08c87cc4 rq-052f542d rq-097c7d03 rq-ab9a813f rq-c5b9e99a
fn sync_resolves_a_prefix_and_caches_the_checkout() {
    let env = Env::new();
    let (code, out, err) = env.tophat(&["tools", "sync"]);
    assert_eq!(code, 0, "{err}");
    assert!(env.read(".tophat/tools.lock").contains(&format!(
        "\"{}\": \"{}\"",
        &env.a[..7],
        env.a
    )));
    assert_eq!(git(&env.cache(&env.a), &["rev-parse", "HEAD"]), env.a);
    assert_eq!(
        out.trim(),
        format!("method-x@{} -> {} (fetched)", &env.a[..7], env.a)
    );
}

#[test] // rq-e2b68626 rq-052f542d
fn sync_is_idempotent() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    let lock = env.read(".tophat/tools.lock");
    let report = sync(&env.root()).unwrap();
    assert_eq!(env.read(".tophat/tools.lock"), lock);
    assert!(report.lines[0].ends_with("(cached)"), "{:?}", report.lines);
}

#[test] // rq-bd5b27af rq-ab9a813f
fn a_full_pin_needs_no_lock_entry() {
    let env = Env::new();
    env.set_pin(&env.a);
    sync(&env.root()).unwrap();
    assert!(!env.root().join(".tophat/tools.lock").exists());
    assert!(env.cache(&env.a).is_dir());
}

#[test] // rq-f1e21338 rq-cfae36f4
fn variants_pin_different_versions_side_by_side() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/config/new-x.yaml",
        &format!(
            "stages: [{{name: score, tool: \"method-x@{}\"}}]\n",
            &env.b[..7]
        ),
    );
    let report = sync(&env.root()).unwrap();
    assert!(report.ok, "{:?}", report.lines);
    assert!(env.cache(&env.a).is_dir() && env.cache(&env.b).is_dir());
    assert_ne!(env.keys("base")["score"], env.keys("new-x")["score"]);
}

#[test] // rq-77e1e92c rq-052f542d
fn an_unknown_commit_fails_sync_for_that_pin_only() {
    let env = Env::new();
    env.write(
        "studies/x-vs-docking/config/broken.yaml",
        "stages: [{name: score, tool: \"method-x@deadbee\"}]\n",
    );
    let report = sync(&env.root()).unwrap();
    assert!(!report.ok);
    let broken: Vec<_> = report
        .lines
        .iter()
        .filter(|l| l.contains("deadbee"))
        .collect();
    assert_eq!(broken.len(), 1);
    assert!(broken[0].contains("(failed:"), "{broken:?}");
    assert!(env.cache(&env.a).is_dir());
    assert_eq!(env.tophat(&["tools", "sync"]).0, 1);
}

#[test] // rq-bf589d85 rq-85215682
fn branch_names_are_not_pins() {
    let env = Env::new();
    env.set_pin("main");
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.tool-pin-invalid").len(),
        1
    );
}

#[test] // rq-5aaa8478 rq-85215682
fn undeclared_tool() {
    let env = Env::new();
    let text = env
        .read("studies/x-vs-docking/config/base.yaml")
        .replace("method-x@", "other-tool@");
    env.write("studies/x-vs-docking/config/base.yaml", &text);
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.tool-unknown").len(),
        1
    );
}

#[test] // rq-648eee87 rq-85215682
fn prefix_without_a_lock_entry() {
    let env = Env::new();
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.tool-unlocked");
    assert_eq!(found.len(), 1);
    let config = env.read("studies/x-vs-docking/config/base.yaml");
    let line = config
        .lines()
        .position(|l| l.contains("name: score"))
        .unwrap() as u32
        + 1;
    assert_eq!(found[0].line, Some(line));
}

#[test] // rq-692c1c23 rq-85215682
fn not_cached_on_this_machine_is_a_warning() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    fs::remove_dir_all(env.root().join(".tophat/tools")).unwrap();
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.tool-not-cached");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    assert!(ds.iter().all(|d| d.severity != Severity::Error), "{ds:#?}");
}

#[test] // rq-df6d227b rq-c5b9e99a
fn a_modified_checkout_is_refused() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    env.prepare();
    let request = env.request("score");
    fs::write(env.cache(&env.a).join("bin/x-score.sh"), "echo hacked\n").unwrap();
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.tool-not-pristine").len(),
        1
    );
    let err = begin(&env.root(), &request).unwrap_err();
    assert!(err.to_string().contains("local changes"), "{err}");
}

// --- Running ----------------------------------------------------------------------------------

#[test] // rq-7f815511 rq-c5b9e99a
fn generation_refuses_an_uncached_tool() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    fs::remove_dir_all(env.root().join(".tophat/tools")).unwrap();
    let (code, _, err) =
        env.tophat(&["pipeline", "rules", "x-vs-docking", "--output", "rules.smk"]);
    assert_ne!(code, 0);
    assert!(err.contains("tophat tools sync"), "{err}");
    assert!(!env.root().join("rules.smk").exists());
}

#[test] // rq-175f7439 rq-f9ae0061 rq-a96b0574
fn the_tool_runs_in_its_own_environment() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    env.prepare();
    let job = begin(&env.root(), &env.request("score")).unwrap();
    assert_eq!(job.manifest, Some(env.cache(&env.a).join("pixi.toml")));
    assert!(
        job.run
            .contains(&format!("'{}'/bin/x-score.sh", env.cache(&env.a).display())),
        "{}",
        job.run
    );
}

#[test] // rq-80cee75b rq-a96b0574
fn project_supplied_environment_for_a_tool_without_one() {
    let env = Env::new();
    env.set_pin(&env.c[..7]);
    env.write("envs/method-x/pixi.toml", "[workspace]\nname = \"x\"\n");
    env.write("envs/method-x/pixi.lock", &LOCK.replace("{pkg}", "numpy-1"));
    sync(&env.root()).unwrap();
    let before = env.keys("base")["score"].clone();
    env.write("envs/method-x/pixi.lock", &LOCK.replace("{pkg}", "numpy-2"));
    assert_ne!(env.keys("base")["score"], before);
    env.prepare();
    let job = begin(&env.root(), &env.request("score")).unwrap();
    assert_eq!(
        job.manifest,
        Some(env.root().join("envs/method-x/pixi.toml"))
    );
}

#[test] // rq-d7d4093c rq-a96b0574
fn a_tool_without_any_environment() {
    let env = Env::new();
    env.set_pin(&env.c[..7]);
    sync(&env.root()).unwrap();
    assert_eq!(
        with_rule(&env.diagnostics(), "pipeline.env-missing").len(),
        1
    );
}

#[test] // rq-f8fa9801 rq-f9ae0061
fn the_tool_placeholder_is_only_for_tool_stages() {
    let env = Env::new();
    let text = env.read("studies/x-vs-docking/config/base.yaml").replace(
        "run: echo x > {out}/a.pdb",
        "run: cp {tool}/data/ref.txt {out}/a.pdb",
    );
    env.write("studies/x-vs-docking/config/base.yaml", &text);
    let ds = env.diagnostics();
    assert!(with_rule(&ds, "pipeline.config-invalid")
        .iter()
        .any(|d| d.message.contains("{tool}")));
}

// --- Keys and provenance ----------------------------------------------------------------------

#[test] // rq-71e65d00 rq-37a6119e
fn the_key_records_the_resolved_commit() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    let config = load_pipeline(&env.root(), "x-vs-docking", "base").unwrap();
    let ctx = KeyContext::new(&env.root());
    let text = stage_key_text(&config, "score", &ctx).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[1].starts_with("run "));
    assert_eq!(lines[2], format!("tool method-x@{}", env.a));
    assert!(!stage_key_text(&config, "prepare", &ctx)
        .unwrap()
        .contains("\ntool "));
}

#[test] // rq-3b6d26cd rq-37a6119e
fn existing_keys_are_unchanged_by_this_feature() {
    let env = Env::new();
    fs::remove_file(env.root().join(".tophat/tools.yaml")).unwrap();
    let config = load_pipeline(&env.root(), "x-vs-docking", "base").unwrap();
    let before = stage_key_text(&config, "prepare", &KeyContext::new(&env.root())).unwrap();
    env.write(
        ".tophat/tools.yaml",
        "tools:\n  other-tool:\n    url: /nowhere\n",
    );
    let after = stage_key_text(&config, "prepare", &KeyContext::new(&env.root())).unwrap();
    assert_eq!(before, after);
}

#[test] // rq-27bb0a4e rq-37a6119e
fn changing_the_pinned_commit_re_runs_the_stage_and_downstream() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    let before = env.keys("base");
    env.set_pin(&env.b[..7]);
    sync(&env.root()).unwrap();
    let after = env.keys("base");
    assert_ne!(before["score"], after["score"]);
    assert_eq!(before["prepare"], after["prepare"]);
}

#[test] // rq-a06c8348 rq-37a6119e
fn provenance_records_the_tool() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    env.commit();
    env.prepare();
    let dir = env
        .base
        .join("results/x-vs-docking/score")
        .join(&env.keys("base")["score"]);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("score.txt"), "5\n").unwrap();
    seal(&env.root(), &env.request("score")).unwrap();
    let record = ProvenanceRecord::read(&dir).unwrap();
    assert_eq!(record.json["tool"]["name"], "method-x");
    assert_eq!(record.json["tool"]["commit"], env.a.as_str());
    assert_eq!(
        record.json["tool"]["url"],
        env.base.join("method-x").display().to_string()
    );
    assert!(record.clean, "{:?}", record.unclean_files);
}

#[test] // rq-59a106f1 rq-37a6119e
fn an_uncommitted_lock_makes_the_record_unclean() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    env.prepare();
    let dir = env
        .base
        .join("results/x-vs-docking/score")
        .join(&env.keys("base")["score"]);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("score.txt"), "5\n").unwrap();
    seal(&env.root(), &env.request("score")).unwrap();
    let record = ProvenanceRecord::read(&dir).unwrap();
    assert!(!record.clean);
    assert!(
        record
            .unclean_files
            .contains(&".tophat/tools.lock".to_string()),
        "{:?}",
        record.unclean_files
    );
}

// --- Listing ----------------------------------------------------------------------------------

#[test] // rq-533b4dc3 rq-4ebf0587
fn list_tools_and_their_use() {
    let env = Env::new();
    sync(&env.root()).unwrap();
    let (code, out, _) = env.tophat(&["tools", "list"]);
    assert_eq!(code, 0);
    assert!(out.contains("method-x"));
    assert!(out.contains(&env.base.join("method-x").display().to_string()));
    assert!(out.contains(&format!("@{}", &env.a[..7])));
    assert!(
        out.contains(&env.a) && out.contains("cached") && out.contains("x-vs-docking/base/score"),
        "{out}"
    );
}

#[test] // rq-d7bdbaae
fn pins_display_as_written() {
    assert_eq!(
        ToolPin::parse("method-x@3f9a1c2").unwrap().to_string(),
        "method-x@3f9a1c2"
    );
}

// --- Public repositories only -----------------------------------------------------------------

#[test] // rq-582d60cc rq-da50d924 rq-85215682
fn locations_that_need_credentials_are_rejected() {
    let env = Env::new();
    for url in [
        "git@github.com:example/method-x.git",
        "ssh://git@github.com/example/method-x.git",
    ] {
        env.write(
            ".tophat/tools.yaml",
            &format!("tools:\n  method-x:\n    url: {url}\n"),
        );
        let ds = env.diagnostics();
        let found = with_rule(&ds, "pipeline.tools-invalid");
        assert_eq!(found.len(), 1, "{url}: {ds:?}");
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[0].path, ".tophat/tools.yaml");
        assert!(
            found[0].message.contains("method-x") && found[0].message.contains("credentials"),
            "{}",
            found[0].message
        );
    }
}

#[test] // rq-d4af23a7 rq-da50d924 rq-85215682
fn urls_with_embedded_credentials_are_rejected() {
    let env = Env::new();
    env.write(
        ".tophat/tools.yaml",
        "tools:\n  method-x:\n    url: https://user:token@github.com/example/method-x.git\n",
    );
    let ds = env.diagnostics();
    let found = with_rule(&ds, "pipeline.tools-invalid");
    assert_eq!(found.len(), 1, "{ds:?}");
    assert!(
        found[0].message.contains("method-x") && found[0].message.contains("credentials"),
        "{}",
        found[0].message
    );
    // The secret is never repeated.
    assert!(
        !found[0].message.contains("token") && !found[0].message.contains("user:"),
        "{}",
        found[0].message
    );
}

#[test] // rq-da50d924
fn public_and_local_locations_are_accepted() {
    use tophat::tools::url_problem;
    for ok in [
        "https://github.com/example/method-x.git",
        "https://gitlab.example.org/group/x.git?ref=1",
        "/srv/git/method-x.git",
        "file:///srv/git/method-x.git",
    ] {
        assert_eq!(url_problem(ok), None, "{ok}");
    }
    for bad in [
        "http://example.org/x.git",
        "git://example.org/x.git",
        "relative/path",
        "host:path/x.git",
        "https://token@github.com/x.git",
    ] {
        assert!(url_problem(bad).is_some(), "{bad}");
    }
}

#[test] // rq-940c97c9 rq-052f542d
fn a_repository_that_needs_credentials_fails_sync_without_prompting() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let env = Env::new();
    // A server that asks every request for credentials.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"tools\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    env.write(
        ".tophat/tools.yaml",
        "tools:\n  method-x:\n    url: https://tools.example.invalid/method-x.git\n",
    );
    // git rewrites the declared public URL to the local server.
    let mut child = Command::new(env!("CARGO_BIN_EXE_tophat"))
        .args(["tools", "sync"])
        .current_dir(env.root())
        .env("GIT_CONFIG_COUNT", "1")
        .env(
            "GIT_CONFIG_KEY_0",
            format!("url.http://127.0.0.1:{port}/.insteadOf"),
        )
        .env("GIT_CONFIG_VALUE_0", "https://tools.example.invalid/")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("tophat tools sync waited for input");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert_eq!(status.code(), Some(1));
    let line = out.lines().next().unwrap_or_default();
    assert!(
        line.starts_with(&format!("method-x@{} -> ? (failed:", &env.a[..7])),
        "{out}"
    );
    assert!(line.contains("not publicly reachable"), "{out}");
}
