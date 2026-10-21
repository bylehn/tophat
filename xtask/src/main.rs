//! Development tooling for the TopHat template, run as `cargo xtask <command>`. Specified in
//! `rqm/tooling.md`; never published or shipped to generated projects.

mod scenarios;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  scenarios    Check that every Gherkin scenario in rqm/ is referenced by a test
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
