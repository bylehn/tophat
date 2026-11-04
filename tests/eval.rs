//! Tests for the skill-eval harness (rqm/testing.md).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use xtask::eval::{self, Options};

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn temp_dir(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "tophat-eval-test-{}-{}-{}",
        label,
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn write_eval(dir: &Path, name: &str, scenario: &str, replies: &str, checks: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{name}.yaml")),
        format!(
            "scenario: {scenario}\nanswers: minimal-claude\nprompt: \"Do the task.\"\nreplies:\n{replies}max_turns: 10\nchecks:\n{checks}"
        ),
    )
    .unwrap();
}

fn run(evals: &Path, program: &Path, name: &str, repeat: usize) -> eval::Report {
    eval::run_dir(
        &repo_root(),
        evals,
        Path::new(env!("CARGO_BIN_EXE_tophat")),
        &Options {
            agent: "claude-code".into(),
            evals: vec![name.to_string()],
            repeat,
        },
        Some(program),
    )
    .unwrap()
}

const DONE: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}"#;

#[test] // rq-cd345094
fn an_unmatched_agent_question_fails_the_eval() {
    if !common::copier() {
        return;
    }
    let temp = temp_dir("unmatched");
    let evals = temp.join("evals");
    write_eval(
        &evals,
        "unmatched",
        "rq-cd345094",
        "  - when: \"(?i)something else\"\n    say: \"Never matched.\"\n",
        "",
    );
    let agent = temp.join("agent");
    write_script(
        &agent,
        r#"printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"What is the endpoint?"}]}}'"#,
    );
    let report = run(&evals, &agent, "unmatched", 1);
    let text = report.lines.join("\n");
    assert!(report.failed, "{text}");
    assert!(
        text.contains("unanswered question: What is the endpoint?"),
        "{text}"
    );
}

#[test] // rq-6fcb1f55
fn an_eval_passes_only_if_every_repetition_passes() {
    if !common::copier() {
        return;
    }
    let temp = temp_dir("repeat");
    let evals = temp.join("evals");
    write_eval(&evals, "repeat", "rq-6fcb1f55", "", "");
    let agent = temp.join("agent");
    write_script(
        &agent,
        &format!("if [ \"$TOPHAT_EVAL_REPEAT\" = 1 ]; then exit 1; fi\nprintf '%s\\n' '{DONE}'"),
    );
    let report = run(&evals, &agent, "repeat", 3);
    let text = report.lines.join("\n");
    assert!(report.failed, "{text}");
    assert!(text.contains("2/3"), "{text}");
}

#[test] // rq-fd2bf270
fn forbidden_commands_are_detected_from_the_transcript() {
    if !common::copier() {
        return;
    }
    let temp = temp_dir("forbidden");
    let evals = temp.join("evals");
    write_eval(
        &evals,
        "forbidden",
        "rq-fd2bf270",
        "",
        "  - not_run: [\"git commit\"]\n",
    );
    let agent = temp.join("agent");
    write_script(
        &agent,
        r#"printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"Done."},{"type":"tool_use","name":"Bash","input":{"command":"git commit -m x"}}]}}'"#,
    );
    let report = run(&evals, &agent, "forbidden", 1);
    let text = report.lines.join("\n");
    assert!(report.failed, "{text}");
    assert!(text.contains("forbidden command `git commit`"), "{text}");
}

/// The scenario IDs in the behaviour feature of `rqm/template/skills.md`.
fn behaviour_scenarios() -> Vec<String> {
    let text = fs::read_to_string(repo_root().join("rqm/template/skills.md")).unwrap();
    let mut ids = Vec::new();
    let mut in_feature = false;
    for line in text.lines() {
        if line.contains("Feature: Agent skills (behaviour evals") {
            in_feature = true;
            continue;
        }
        if in_feature {
            if line.trim() == "```" {
                break;
            }
            if let Some(id) = line.trim().strip_prefix("@rq-") {
                ids.push(format!("rq-{id}"));
            }
        }
    }
    ids
}

#[test] // rq-7238187f
fn every_behaviour_scenario_has_an_eval() {
    let scenarios = behaviour_scenarios();
    assert!(!scenarios.is_empty());
    let evals = repo_root().join("evals");
    let mut named = Vec::new();
    for entry in fs::read_dir(&evals).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "yaml") {
            let text = fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                if let Some(id) = line.trim().strip_prefix("scenario: ") {
                    named.push(id.to_string());
                }
            }
        }
    }
    let missing: Vec<&String> = scenarios.iter().filter(|id| !named.contains(id)).collect();
    assert!(missing.is_empty(), "missing evals for {missing:?}");
}
