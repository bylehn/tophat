//! `cargo xtask <command>`: development tooling for the TopHat template (see the library).

use xtask::{adoption, conformance, eval, lint, lock, release_pin, render, render_test, scenarios};

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  render --answers <name|file> --out <dir> [--set <question>=<value>]...
                                              Render a project from the working tree
  lock [--check]                              Regenerate or check the pre-solved pixi locks
  conformance [--tophat <binary>] [--case <name>]...
                                              Run the conformance suite
  adoption [--tophat <binary>] [--fixture <name>]...
                                              Run the adoption fixtures
  eval [--agent claude-code|codex|opencode] [--eval <name>]... [--repeat <n>]
                                              Run the skill evals
  release-pin --version <version> --url <base url> --binary <target>=<path>...
                                              Write the template's pinned tophat release
  render-test [--answers <name>]... [--no-image]
                                              Render every answer set and check the projects
  scenarios                                   Check that every Gherkin scenario in rqm/ is referenced by a test
  lint-skills <project dir> [--tophat <bin>]  Lint the agent skills of a rendered project
";

/// The repository root: the parent of this crate's directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the repository")
        .to_path_buf()
}

/// Parses `cargo xtask render` arguments.
fn render_request(args: &[&str]) -> Result<render::Request, String> {
    let mut answers = None;
    let mut out = None;
    let mut sets = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| {
            args.get(i + 1)
                .copied()
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i] {
            "--answers" => answers = Some(value(i)?.to_string()),
            "--out" => out = Some(PathBuf::from(value(i)?)),
            "--set" => sets.push(render::parse_set(value(i)?).map_err(|e| e.to_string())?),
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 2;
    }
    Ok(render::Request {
        answers: answers.ok_or("--answers is required")?,
        out: out.ok_or("--out is required")?,
        sets,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["render", rest @ ..] => {
            let request = match render_request(rest) {
                Ok(request) => request,
                Err(message) => {
                    eprintln!("cargo xtask render: {message}");
                    return ExitCode::from(2);
                }
            };
            match render::render(&repo_root(), &request) {
                Ok(out) => {
                    println!("{}", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("cargo xtask render: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["lock"] | ["lock", "--check"] => {
            let check = args.get(1).is_some_and(|arg| arg == "--check");
            match lock::lock(&repo_root(), check) {
                Ok(outcomes) => {
                    for outcome in &outcomes {
                        println!(
                            "lock {}: {} ({})",
                            outcome.selection,
                            outcome.status.as_str(),
                            outcome.lock.display()
                        );
                    }
                    if lock::any_stale(&outcomes) {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask lock: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["conformance", rest @ ..] => {
            let mut binary = repo_root().join("target/debug/tophat");
            let mut cases = Vec::new();
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--tophat" if i + 1 < rest.len() => {
                        binary = PathBuf::from(rest[i + 1]);
                        i += 2;
                    }
                    "--case" if i + 1 < rest.len() => {
                        cases.push(rest[i + 1].to_string());
                        i += 2;
                    }
                    other => {
                        eprintln!(
                            "cargo xtask conformance: unknown or incomplete option '{other}'"
                        );
                        return ExitCode::from(2);
                    }
                }
            }
            match conformance::run(&repo_root(), &binary, &cases) {
                Ok(report) => {
                    for line in &report.lines {
                        println!("{line}");
                    }
                    if report.failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask conformance: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["eval", rest @ ..] => {
            let mut options = eval::Options::default();
            let mut binary = repo_root().join("target/debug/tophat");
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--agent" if i + 1 < rest.len() => {
                        options.agent = rest[i + 1].to_string();
                        i += 2;
                    }
                    "--eval" if i + 1 < rest.len() => {
                        options.evals.push(rest[i + 1].to_string());
                        i += 2;
                    }
                    "--repeat" if i + 1 < rest.len() => {
                        options.repeat = rest[i + 1].parse().unwrap_or(0);
                        i += 2;
                    }
                    "--tophat" if i + 1 < rest.len() => {
                        binary = PathBuf::from(rest[i + 1]);
                        i += 2;
                    }
                    other => {
                        eprintln!("cargo xtask eval: unknown or incomplete option '{other}'");
                        return ExitCode::from(2);
                    }
                }
            }
            if options.repeat == 0 {
                eprintln!("cargo xtask eval: --repeat must be a positive integer");
                return ExitCode::from(2);
            }
            match eval::run(&repo_root(), &binary, &options) {
                Ok(report) => {
                    for line in &report.lines {
                        println!("{line}");
                    }
                    if report.failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask eval: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["adoption", rest @ ..] => {
            let mut binary = repo_root().join("target/debug/tophat");
            let mut fixtures = Vec::new();
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--tophat" if i + 1 < rest.len() => {
                        binary = PathBuf::from(rest[i + 1]);
                        i += 2;
                    }
                    "--fixture" if i + 1 < rest.len() => {
                        fixtures.push(rest[i + 1].to_string());
                        i += 2;
                    }
                    other => {
                        eprintln!("cargo xtask adoption: unknown or incomplete option '{other}'");
                        return ExitCode::from(2);
                    }
                }
            }
            match adoption::run(&repo_root(), &binary, &fixtures) {
                Ok(report) => {
                    for line in &report.lines {
                        println!("{line}");
                    }
                    if report.failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask adoption: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["render-test", rest @ ..] => {
            let mut options = render_test::Options::default();
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--answers" if i + 1 < rest.len() => {
                        options.answers.push(rest[i + 1].to_string());
                        i += 2;
                    }
                    "--no-image" => {
                        options.no_image = true;
                        i += 1;
                    }
                    other => {
                        eprintln!(
                            "cargo xtask render-test: unknown or incomplete option '{other}'"
                        );
                        return ExitCode::from(2);
                    }
                }
            }
            match render_test::run(&repo_root(), &options) {
                Ok(report) => {
                    for line in &report.lines {
                        println!("{line}");
                    }
                    if report.failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask render-test: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["release-pin", rest @ ..] => {
            let mut version = None;
            let mut url = None;
            let mut binaries = Vec::new();
            let mut i = 0;
            while i < rest.len() {
                let Some(value) = rest.get(i + 1).copied() else {
                    eprintln!("cargo xtask release-pin: {} needs a value", rest[i]);
                    return ExitCode::from(2);
                };
                match rest[i] {
                    "--version" => version = Some(value.to_string()),
                    "--url" => url = Some(value.to_string()),
                    "--binary" => binaries.push(value.to_string()),
                    other => {
                        eprintln!("cargo xtask release-pin: unknown option '{other}'");
                        return ExitCode::from(2);
                    }
                }
                i += 2;
            }
            let (Some(version), Some(url)) = (version, url) else {
                eprintln!("cargo xtask release-pin: --version and --url are required");
                return ExitCode::from(2);
            };
            let request = release_pin::Request {
                version,
                url,
                binaries,
            };
            match release_pin::release_pin(&repo_root(), &request) {
                Ok(report) => {
                    for line in &report.lines {
                        println!("{line}");
                    }
                    if report.failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask release-pin: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["scenarios"] => match scenarios::check(&repo_root()) {
            Ok(report) => {
                print!("{}", report.render());
                if report.problems.is_empty() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("cargo xtask scenarios: {e}");
                ExitCode::from(2)
            }
        },
        ["lint-skills", project, rest @ ..] => {
            let tophat = match rest {
                [] => repo_root().join("target/debug/tophat"),
                ["--tophat", bin] => PathBuf::from(bin),
                _ => {
                    eprint!("{USAGE}");
                    return ExitCode::from(2);
                }
            };
            let usage = match std::process::Command::new(&tophat).arg("--help").output() {
                Ok(out) if out.status.success() => {
                    String::from_utf8_lossy(&out.stdout).into_owned()
                }
                _ => {
                    eprintln!(
                        "cargo xtask lint-skills: cannot run `{} --help`; build tophat or pass --tophat",
                        tophat.display()
                    );
                    return ExitCode::from(2);
                }
            };
            match lint::lint_skills(Path::new(project), &usage) {
                Ok(findings) => {
                    for f in &findings {
                        println!("{f}");
                    }
                    if findings.is_empty() {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(1)
                    }
                }
                Err(e) => {
                    eprintln!("cargo xtask lint-skills: {e}");
                    ExitCode::from(2)
                }
            }
        }
        ["-h" | "--help" | "help"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
