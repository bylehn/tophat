//! Tests for the agent skills, their adapters and permissions, and the worked example study
//! (rqm/template/skills.md). The template is rendered with the real Copier; these tests need
//! `copier` on `PATH` (see tests/common/mod.rs).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

use xtask::lint::{lint_skills, Finding};

const SKILLS: [&str; 8] = [
    "th-adopt",
    "th-data",
    "th-next",
    "th-question",
    "th-quiz",
    "th-run",
    "th-study",
    "th-tool",
];

const EXAMPLE: &str = ".tophat/managed/examples/median-vs-mean";

struct Project {
    base: PathBuf,
}

impl Project {
    /// The template rendered with default answers, plus `answers`.
    fn render(answers: &[(&str, &str)]) -> Project {
        static N: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-skills-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let project = Project { base };
        let out = copier_copy(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "HEAD",
            &project.root(),
            answers,
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        project
    }

    fn root(&self) -> PathBuf {
        self.base.join("p")
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn lint(&self) -> Vec<Finding> {
        let usage = Command::new(env!("CARGO_BIN_EXE_tophat"))
            .arg("--help")
            .output()
            .unwrap();
        lint_skills(&self.root(), &String::from_utf8_lossy(&usage.stdout)).unwrap()
    }

    fn tophat(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tophat"))
            .args(args)
            .current_dir(self.root())
            .env_remove("TOPHAT_ROOT_RESULTS")
            .output()
            .unwrap()
    }

    fn git(&self, args: &[&str]) {
        git_in(&self.root(), args);
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn copier_copy(src: &Path, vcs_ref: &str, dst: &Path, answers: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("copier");
    cmd.args(["copy", "--defaults", "--quiet", "--vcs-ref", vcs_ref]);
    let background = [
        ("project_name", "Method X"),
        ("project_description", "Does method X work?"),
        ("author_name", "A. Scientist"),
        ("author_email", "a@example.org"),
    ];
    for (k, v) in background
        .iter()
        .filter(|(k, _)| !answers.iter().any(|(a, _)| a == k))
        .chain(answers)
    {
        cmd.arg("-d").arg(format!("{k}={v}"));
    }
    cmd.arg(src).arg(dst).output().unwrap()
}

fn git_in(dir: &Path, args: &[&str]) {
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
}

fn rules(findings: &[Finding], rule: &str) -> Vec<String> {
    findings
        .iter()
        .filter(|f| f.rule == rule)
        .map(Finding::to_string)
        .collect()
}

/// The front matter field `key` of a skill file.
fn field(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return None;
    }
    lines
        .take_while(|l| *l != "---")
        .find_map(|l| l.strip_prefix(&format!("{key}: ")).map(str::to_string))
}

/// Lines after the front matter.
fn body(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines.iter().skip(1).position(|l| *l == "---").unwrap() + 2;
    lines[end..].to_vec()
}

fn settings(p: &Project) -> serde_json::Value {
    serde_json::from_str(&p.read(".claude/settings.json")).unwrap()
}

fn entries(settings: &serde_json::Value, list: &str) -> Vec<String> {
    settings["permissions"][list]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap().to_string())
        .collect()
}

/// Whether a `Bash(<prefix>:*)` entry matches a command, reading it as a command prefix.
fn entry_matches(entry: &str, command: &str) -> bool {
    entry
        .strip_prefix("Bash(")
        .and_then(|e| e.strip_suffix(":*)"))
        .is_some_and(|prefix| command.starts_with(prefix))
}

// --- Skills and adapters ------------------------------------------------------------------------

#[test] // rq-42fba780 rq-25d62c0c rq-b0419763
fn every_skill_has_a_canonical_text_and_a_local_extension() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let mut found: Vec<String> = fs::read_dir(p.root().join(".tophat/managed/skills"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(found, SKILLS);
    for skill in SKILLS {
        let text = p.read(&format!(".tophat/managed/skills/{skill}/SKILL.md"));
        assert_eq!(field(&text, "name").as_deref(), Some(skill));
        assert!(field(&text, "description").is_some_and(|d| d.len() > 40));
        let local = p.read(&format!(".tophat/user/skills/{skill}/local.md"));
        assert!(local.contains("copier update` never changes it"), "{skill}");
    }
}

#[test] // rq-2b638c5b rq-81ec264f
fn adapters_are_written_for_every_selected_agent() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    for skill in SKILLS {
        assert!(p
            .root()
            .join(format!(".claude/skills/{skill}/SKILL.md"))
            .is_file());
    }
    let findings = p.lint();
    assert!(
        rules(&findings, "skill.adapter-missing").is_empty(),
        "{findings:?}"
    );
}

#[test] // rq-ec2d3828 rq-81ec264f
fn adapters_only_point_at_the_canonical_skill() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    for skill in SKILLS {
        let adapter = p.read(&format!(".claude/skills/{skill}/SKILL.md"));
        let canonical = p.read(&format!(".tophat/managed/skills/{skill}/SKILL.md"));
        assert!(
            adapter.contains(&format!(
                "Read `.tophat/managed/skills/{skill}/SKILL.md` completely"
            )),
            "{skill}"
        );
        assert!(body(&adapter).len() <= 15, "{skill}");
        assert_eq!(
            field(&adapter, "description"),
            field(&canonical, "description")
        );
    }
    assert!(rules(&p.lint(), "skill.adapter-content").is_empty());
}

#[test] // rq-7a0bc07d rq-25d62c0c
fn canonical_skills_are_agent_neutral() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let findings = p.lint();
    assert!(
        rules(&findings, "skill.agent-specific").is_empty(),
        "{findings:?}"
    );
}

#[test] // rq-9b7fb838 rq-4a032ea7
fn agent_specific_wording_is_caught() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let rel = ".tophat/managed/skills/th-study/SKILL.md";
    let text = p.read(rel).replace(
        "Then continue with `th-run`.",
        "Then continue with `/th-run`.",
    );
    p.write(rel, &text);
    let found = rules(&p.lint(), "skill.agent-specific");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("th-study/SKILL.md"), "{found:?}");
}

#[test] // rq-07fe36ca rq-4a032ea7
fn skills_mention_only_real_cli_commands() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let findings = p.lint();
    assert!(
        rules(&findings, "skill.cli-unknown").is_empty(),
        "{findings:?}"
    );
    // The whole lint is clean.
    assert!(findings.is_empty(), "{findings:?}");
}

#[test] // rq-7e3bca84 rq-4a032ea7
fn an_unknown_cli_command_is_caught() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let rel = ".tophat/managed/skills/th-data/SKILL.md";
    let text = p
        .read(rel)
        .replace("`tophat data verify", "`tophat data copy");
    p.write(rel, &text);
    let found = rules(&p.lint(), "skill.cli-unknown");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("tophat data copy"), "{found:?}");
}

#[test] // rq-cf3ebd09 rq-b0419763
fn every_skill_defers_to_its_local_extension() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    for skill in SKILLS {
        let text = p.read(&format!(".tophat/managed/skills/{skill}/SKILL.md"));
        let last = text.rsplit("\n## ").next().unwrap();
        assert!(last.starts_with("Project Extensions\n"), "{skill}");
        assert!(
            last.contains(&format!(".tophat/user/skills/{skill}/local.md")),
            "{skill}"
        );
    }
    assert!(rules(&p.lint(), "skill.local-missing").is_empty());
}

#[test] // rq-ce21a33e rq-b0419763
fn local_extensions_survive_updates() {
    if !common::copier() {
        return;
    }
    let base = std::env::temp_dir().join(format!("tophat-skills-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    // A template repository at v1, and a newer v2 whose local.md and skill text differ.
    let tpl = base.join("tpl");
    fs::create_dir_all(&tpl).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::copy(repo.join("copier.yml"), tpl.join("copier.yml")).unwrap();
    copy_dir(&repo.join("template"), &tpl.join("template"));
    git_in(&tpl, &["init", "-q"]);
    git_in(&tpl, &["add", "-A"]);
    git_in(&tpl, &["commit", "-qm", "v1"]);
    git_in(&tpl, &["tag", "v1"]);
    let p = base.join("p");
    let out = copier_copy(&tpl, "v1", &p, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git_in(&p, &["init", "-q"]);
    let local = p.join(".tophat/user/skills/th-study/local.md");
    fs::write(&local, "- Our own rule.\n").unwrap();
    git_in(&p, &["add", "-A"]);
    git_in(&p, &["commit", "-qm", "project"]);
    for (f, add) in [
        (
            "template/.tophat/user/skills/th-study/local.md",
            "\nNewer text.\n",
        ),
        (
            "template/.tophat/managed/skills/th-study/SKILL.md",
            "\nNewer skill text.\n",
        ),
    ] {
        let text = fs::read_to_string(tpl.join(f)).unwrap();
        fs::write(tpl.join(f), format!("{text}{add}")).unwrap();
    }
    git_in(&tpl, &["commit", "-qam", "v2"]);
    git_in(&tpl, &["tag", "v2"]);
    let out = Command::new("copier")
        .args([
            "update",
            "--defaults",
            "--quiet",
            "--vcs-ref",
            "v2",
            "--conflict",
            "inline",
        ])
        .current_dir(&p)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(&local).unwrap(), "- Our own rule.\n");
    assert!(
        fs::read_to_string(p.join(".tophat/managed/skills/th-study/SKILL.md"))
            .unwrap()
            .contains("Newer skill text.")
    );
    let _ = fs::remove_dir_all(&base);
}

#[test] // rq-165e9913 rq-9139a849 rq-b0419763
fn the_quiz_policy_file_shows_every_policy() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let local = p.read(".tophat/user/skills/th-quiz/local.md");
    let (before, rest) = local.split_once("<!--").unwrap();
    let (comment, after) = rest.split_once("-->").unwrap();
    for policy in ["offer", "always", "never"] {
        assert!(
            comment.contains(&format!("- Quiz policy: {policy}\n")),
            "{policy}"
        );
    }
    assert!(
        !format!("{before}{after}").contains("- Quiz policy:"),
        "{local}"
    );
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

// --- Agent instructions and permissions ---------------------------------------------------------

#[test] // rq-01a6c860 rq-e5bfbedd rq-0694746a
fn agent_entry_points_read_the_managed_rules() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    assert!(p
        .read("AGENTS.md")
        .starts_with("Read `.tophat/managed/AGENTS.md` first"));
    let claude = p.read("CLAUDE.md");
    assert!(claude.contains("@.tophat/managed/AGENTS.md") && claude.contains("@AGENTS.md"));
    let rules = p.read(".tophat/managed/AGENTS.md");
    let hard = rules
        .split("## Hard rules")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let numbered: Vec<&str> = hard
        .lines()
        .filter(|l| {
            l.split_once(". ")
                .is_some_and(|(n, _)| n.parse::<u32>().is_ok())
        })
        .collect();
    assert_eq!(numbered.len(), 9, "{numbered:?}");
    for skill in SKILLS {
        assert!(rules.contains(&format!("- `{skill}`:")), "{skill}");
    }
}

#[test] // rq-ed65c3a5 rq-c092fd66
fn remote_access_and_human_only_commands_are_denied() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let mut deny = entries(&settings(&p), "deny");
    deny.sort();
    let mut expected: Vec<String> = [
        "ssh",
        "scp",
        "sftp",
        "rsync",
        "sbatch",
        "srun",
        "salloc",
        "git commit",
        "git push",
        "tophat study preregister",
        "tophat stage qc",
        "tophat tools sync",
    ]
    .iter()
    .map(|c| format!("Bash({c}:*)"))
    .collect();
    expected.sort();
    assert_eq!(deny, expected);
    // The container check runs before every prompt.
    assert_eq!(
        settings(&p)["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        ".tophat/managed/hooks/check-container.sh"
    );
}

#[test] // rq-9d45bd87 rq-c092fd66
fn read_only_tophat_commands_need_no_approval() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let mut allow = entries(&settings(&p), "allow");
    allow.sort();
    let mut expected: Vec<String> = [
        "tophat status",
        "tophat check",
        "tophat question tree",
        "tophat pipeline keys",
        "tophat tools list",
        "tophat data lineage",
        "tophat data used-by",
        "tophat stage instructions",
    ]
    .iter()
    .map(|c| format!("Bash({c}:*)"))
    .collect();
    expected.sort();
    assert_eq!(allow, expected);
}

#[test] // rq-2de196ad rq-c092fd66
fn the_human_only_commands_are_denied_with_their_arguments() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let deny = entries(&settings(&p), "deny");
    for command in [
        "tophat stage qc x-vs-docking score --variant base",
        "tophat study preregister x-vs-docking",
        "tophat tools sync",
        "git commit -m x",
    ] {
        assert!(deny.iter().any(|e| entry_matches(e, command)), "{command}");
    }
    for command in ["tophat status --json", "tophat check studies/x-vs-docking"] {
        assert!(!deny.iter().any(|e| entry_matches(e, command)), "{command}");
    }
}

// --- Worked example -----------------------------------------------------------------------------

#[test] // rq-78cc426d rq-428f178b
fn the_example_is_not_part_of_a_new_projects_studies() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    assert!(p.root().join(EXAMPLE).join("STUDY.md").is_file());
    assert!(!p.root().join("studies").exists());
    let out = p.tophat(&["check", "--strict", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["diagnostics"], serde_json::json!([]), "{report}");
}

/// Copies the example into `studies/` with a stamped question; returns the question ID.
fn adopt_example(p: &Project) -> String {
    p.git(&["init", "-q"]);
    let project_md = p.read("PROJECT.md");
    p.write(
        "PROJECT.md",
        &format!(
            "{project_md}\n## Does a median filter recover step signals better?\nstatus: open\n"
        ),
    );
    let out = p.tophat(&["question", "stamp"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    copy_dir(
        &p.root().join(EXAMPLE),
        &p.root().join("studies/median-vs-mean"),
    );
    let study = p
        .read("studies/median-vs-mean/STUDY.md")
        .replace("question: q-00000000", &format!("question: {id}"));
    p.write("studies/median-vs-mean/STUDY.md", &study);
    id
}

#[test] // rq-a4160e43 rq-428f178b
fn the_copied_example_passes_the_check() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    adopt_example(&p);
    let out = p.tophat(&["check", "--json", "studies/median-vs-mean"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["summary"]["errors"], 0, "{report}");
    let tests = p.read("studies/median-vs-mean/tests/test_scenarios.py");
    let study = p.read("studies/median-vs-mean/STUDY.md");
    for n in 1..=3 {
        assert!(study.contains(&format!("\n- S{n}: ")), "S{n}");
        assert!(tests.contains(&format!("th:S{n}")), "S{n}");
    }
    assert!(!study.contains("- S4:"));
    let parsed = tophat::study::parse_study_text("median-vs-mean", "STUDY.md", &study).unwrap();
    let numbers: Vec<Option<u32>> = parsed.scenarios.iter().map(|s| s.number).collect();
    assert_eq!(numbers, [Some(1), Some(2), Some(3)]);
}

#[test] // rq-38e5cf5f rq-428f178b
fn the_example_validates_what_code_can_check_and_reviews_what_needs_judgment() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let config = p.read(&format!("{EXAMPLE}/config/base.yaml"));
    let block = |stage: &str| {
        config
            .split("  - name: ")
            .find(|s| s.starts_with(&format!("{stage}\n")))
            .unwrap()
            .to_string()
    };
    for stage in ["generate", "score"] {
        assert!(
            block(stage).contains("\n    validate: "),
            "{stage}:\n{}",
            block(stage)
        );
    }
    assert!(
        block("filter").contains("\n    review: "),
        "{}",
        block("filter")
    );
    adopt_example(&p);
    let out = p.tophat(&["check", "--json", "studies/median-vs-mean"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rules: Vec<&str> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["rule"].as_str())
        .collect();
    assert!(!rules.contains(&"pipeline.validate-missing"), "{report}");
}

#[test] // rq-60ea5f24 rq-428f178b
fn the_examples_external_stage_needs_only_posix_tools() {
    if !common::copier() {
        return;
    }
    let p = Project::render(&[]);
    let config = p.read(&format!("{EXAMPLE}/config/base.yaml"));
    let filter = config
        .split("  - name: ")
        .find(|s| s.starts_with("filter\n"))
        .unwrap();
    assert!(filter.contains("\n    external:\n"), "{filter}");
    assert!(filter.contains("run: sh scripts/filter.sh "), "{filter}");

    // Run it with only sh, awk and mkdir on PATH, on a signal from generate.py.
    let tmp = p.base.join("posix");
    let bin = tmp.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let system = std::env::var_os("PATH").unwrap_or_default();
    for tool in ["sh", "awk", "mkdir"] {
        let found = std::env::split_paths(&system)
            .map(|d| d.join(tool))
            .find(|p| p.is_file())
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(found, bin.join(tool)).unwrap();
    }
    let signals = tmp.join("signals");
    fs::create_dir_all(&signals).unwrap();
    fs::write(
        signals.join("sig-001.tsv"),
        "clean\tnoisy\n0\t0.5\n0\t-0.2\n1\t1.4\n1\t0.9\n1\t3.0\n",
    )
    .unwrap();
    let out = Command::new(bin.join("sh"))
        .arg(p.root().join(EXAMPLE).join("scripts/filter.sh"))
        .arg(&signals)
        .arg(tmp.join("out"))
        .arg("3")
        .env_clear()
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows = fs::read_to_string(tmp.join("out/sig-001.tsv")).unwrap();
    let rows: Vec<&str> = rows.lines().collect();
    assert_eq!(rows[0], "clean\tmean\tmedian");
    assert_eq!(rows.len(), 6);
    // The middle sample: window (−0.2, 1.4, 0.9) has mean 0.7 and median 0.9.
    assert_eq!(rows[3], "1\t0.700000\t0.900000");
}

#[test] // rq-db5eda37 rq-428f178b
fn the_example_uses_julia_when_selected() {
    if !common::copier() {
        return;
    }
    let julia = Project::render(&[("languages", "[\"julia\"]")]);
    let config = julia.read(&format!("{EXAMPLE}/config/base.yaml"));
    assert!(
        config.contains("run: julia scripts/score.jl {item} {out}"),
        "{config}"
    );
    assert!(julia
        .root()
        .join(EXAMPLE)
        .join("scripts/score.jl")
        .is_file());
    assert!(!julia.root().join(EXAMPLE).join("scripts/score.py").exists());

    let python = Project::render(&[]);
    let config = python.read(&format!("{EXAMPLE}/config/base.yaml"));
    assert!(
        config.contains("run: python scripts/score.py {item} {out}"),
        "{config}"
    );
    assert!(python
        .root()
        .join(EXAMPLE)
        .join("scripts/score.py")
        .is_file());
    assert!(!python
        .root()
        .join(EXAMPLE)
        .join("scripts/score.jl")
        .exists());
}
