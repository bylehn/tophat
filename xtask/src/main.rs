//! `cargo xtask <command>`: development tooling for the TopHat template (see the library).

use xtask::{lint, render, scenarios};

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  render --answers <name|file> --out <dir> [--set <question>=<value>]...
                                              Render a project from the working tree
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
