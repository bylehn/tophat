//! `cargo xtask <command>`: development tooling for the TopHat template (see the library).

use xtask::{lint, scenarios};

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
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
