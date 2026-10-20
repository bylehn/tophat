//! Command-line interface of the `tophat` binary.
//!
//! [`run`] takes its environment (arguments, working directory, rule families, terminal streams)
//! as parameters so every command can be exercised in tests.

use crate::check::{find_project_root, resolve_scope, run_families, CheckError, RuleFamily};
use crate::question::{self, QuestionError};
use crate::report::{render_human, render_json};
use crate::study::{self, Console, StudyError};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Exit code: the command ran and found nothing that fails.
pub const EXIT_OK: i32 = 0;
/// Exit code: the check found a failing diagnostic, or a command was refused.
pub const EXIT_FAILED: i32 = 1;
/// Exit code: the command could not run (usage error, not in a project, internal error).
pub const EXIT_UNABLE: i32 = 2;

const USAGE: &str = "\
Usage: tophat <command> [options]

Commands:
  check [--json] [--strict] [PATH...]            Check project integrity
  study new <ID> --question <Q-ID> [--visibility public|private]
                                                 Create a draft study
  study preregister <ID>                         Pre-register a draft (interactive, by a human)
  study report <ID>                              Copy SUMMARY.md results into STUDY.md
  study supersede <OLD-ID> --by <NEW-ID>         Mark a study as replaced by another
  question stamp                                 Assign IDs to new questions in PROJECT.md
  question tree [--json]                         Show the question tree with its studies

Options:
  -h, --help       Print help
  -V, --version    Print version
";

/// The terminal streams and their properties.
pub struct Streams<'a> {
    pub input: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// Whether standard input and standard output are both interactive terminals.
    pub interactive: bool,
}

/// Parsed arguments of `tophat check`.
#[derive(Debug, Default, PartialEq, Eq)]
struct CheckArgs {
    json: bool,
    strict: bool,
    paths: Vec<PathBuf>,
}

fn parse_check_args(args: &[String]) -> Result<CheckArgs, String> {
    let mut parsed = CheckArgs::default();
    let mut only_paths = false;
    for arg in args {
        match arg.as_str() {
            _ if only_paths => parsed.paths.push(PathBuf::from(arg)),
            "--" => only_paths = true,
            "--json" => parsed.json = true,
            "--strict" => parsed.strict = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option `{flag}` for `tophat check`"))
            }
            path => parsed.paths.push(PathBuf::from(path)),
        }
    }
    Ok(parsed)
}

// rq-1bce095c rq-35e01b00
fn run_check_command(
    args: &[String],
    cwd: &Path,
    families: &[RuleFamily],
    io: &mut Streams<'_>,
) -> i32 {
    let fail = |err: &mut dyn Write, message: &str| {
        let _ = writeln!(err, "tophat check: {message}");
        EXIT_UNABLE
    };
    let parsed = match parse_check_args(args) {
        Ok(parsed) => parsed,
        Err(message) => return fail(io.err, &message),
    };
    let root = match find_project_root(cwd) {
        Ok(root) => root,
        Err(e) => return fail(io.err, &e.to_string()),
    };
    let scopes = match parsed
        .paths
        .iter()
        .map(|p| resolve_scope(&root, cwd, p))
        .collect::<Result<Vec<_>, CheckError>>()
    {
        Ok(scopes) => scopes,
        Err(e) => return fail(io.err, &e.to_string()),
    };

    let diagnostics = run_families(&root, &scopes, families);
    let rendered = if parsed.json {
        render_json(&diagnostics)
    } else {
        render_human(&diagnostics)
    };
    if io.out.write_all(rendered.as_bytes()).is_err() {
        return fail(io.err, "cannot write output");
    }

    if diagnostics.iter().any(|d| d.severity.fails(parsed.strict)) {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

/// Positional arguments and `(name, value)` options.
type ParsedOptions = (Vec<String>, Vec<(String, String)>);

/// Splits arguments into positionals and `--name value` options, rejecting unknown options.
fn parse_options(args: &[String], known: &[&str]) -> Result<ParsedOptions, String> {
    let mut positionals = Vec::new();
    let mut options = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(name) = arg.strip_prefix("--") {
            if !known.contains(&name) {
                return Err(format!("unknown option `{arg}`"));
            }
            let value = iter
                .next()
                .ok_or_else(|| format!("option `{arg}` needs a value"))?;
            options.push((name.to_string(), value.clone()));
        } else {
            positionals.push(arg.clone());
        }
    }
    Ok((positionals, options))
}

fn option<'a>(options: &'a [(String, String)], name: &str) -> Option<&'a str> {
    options
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// A validated `tophat study` invocation.
enum StudyCommand {
    New {
        id: String,
        question: String,
        private: bool,
    },
    Preregister(String),
    Report(String),
    Supersede {
        old: String,
        new: String,
    },
}

fn parse_study_args(args: &[String]) -> Result<StudyCommand, String> {
    let (sub, rest) = args
        .split_first()
        .ok_or("missing study subcommand (new, preregister, report, supersede)")?;
    let one_id = |positionals: Vec<String>| match positionals.as_slice() {
        [id] => Ok(id.clone()),
        _ => Err(format!("`tophat study {sub}` takes exactly one study ID")),
    };
    match sub.as_str() {
        "new" => {
            let (positionals, options) = parse_options(rest, &["question", "visibility"])?;
            let id = one_id(positionals)?;
            let question = option(&options, "question")
                .ok_or("`tophat study new` requires --question <question-id>")?
                .to_string();
            let private = match option(&options, "visibility").unwrap_or("public") {
                "public" => false,
                "private" => true,
                other => {
                    return Err(format!(
                        "--visibility must be public or private, not `{other}`"
                    ))
                }
            };
            Ok(StudyCommand::New {
                id,
                question,
                private,
            })
        }
        "preregister" => Ok(StudyCommand::Preregister(one_id(
            parse_options(rest, &[])?.0,
        )?)),
        "report" => Ok(StudyCommand::Report(one_id(parse_options(rest, &[])?.0)?)),
        "supersede" => {
            let (positionals, options) = parse_options(rest, &["by"])?;
            let old = one_id(positionals)?;
            let new = option(&options, "by")
                .ok_or("`tophat study supersede` requires --by <new-study-id>")?
                .to_string();
            Ok(StudyCommand::Supersede { old, new })
        }
        other => Err(format!("unknown study subcommand `{other}`")),
    }
}

fn run_study_command(args: &[String], cwd: &Path, io: &mut Streams<'_>) -> i32 {
    let command = match parse_study_args(args) {
        Ok(command) => command,
        Err(message) => {
            let _ = writeln!(io.err, "tophat study: {message}\n\n{USAGE}");
            return EXIT_UNABLE;
        }
    };
    let root = match find_project_root(cwd) {
        Ok(root) => root,
        Err(e) => {
            let _ = writeln!(io.err, "tophat study: {e}");
            return EXIT_UNABLE;
        }
    };
    let result: Result<(), StudyError> = match &command {
        StudyCommand::New {
            id,
            question,
            private,
        } => study::new_study(&root, id, question, *private).map(|dir| {
            let rel = dir.strip_prefix(&root).unwrap_or(&dir);
            let _ = writeln!(io.out, "Created draft study at {}", rel.display());
        }),
        StudyCommand::Preregister(id) => {
            let today = chrono::Local::now().date_naive();
            let mut console = Console {
                interactive: io.interactive,
                input: &mut *io.input,
                out: &mut *io.out,
            };
            study::preregister(&root, id, today, &mut console)
        }
        StudyCommand::Report(id) => study::report(&root, id).map(|()| {
            let _ = writeln!(io.out, "Copied SUMMARY.md results into study `{id}`.");
        }),
        StudyCommand::Supersede { old, new } => study::supersede(&root, old, new).map(|()| {
            let _ = writeln!(io.out, "Study `{old}` is now superseded by `{new}`.");
        }),
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            let _ = writeln!(io.err, "tophat study: {e}");
            EXIT_FAILED
        }
    }
}

fn run_question_command(args: &[String], cwd: &Path, io: &mut Streams<'_>) -> i32 {
    let usage_error = |io: &mut Streams<'_>, message: &str| {
        let _ = writeln!(io.err, "tophat question: {message}\n\n{USAGE}");
        EXIT_UNABLE
    };
    let (json, valid) = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["stamp"] => (false, true),
        ["tree"] => (false, true),
        ["tree", "--json"] => (true, true),
        _ => (false, false),
    };
    if !valid {
        return usage_error(io, "expected `stamp` or `tree [--json]`");
    }
    let root = match find_project_root(cwd) {
        Ok(root) => root,
        Err(e) => {
            let _ = writeln!(io.err, "tophat question: {e}");
            return EXIT_UNABLE;
        }
    };
    let result: Result<(), QuestionError> = if args[0] == "stamp" {
        question::stamp_outcome(&root).and_then(|outcome| {
            for (id, text) in &outcome.assigned {
                let _ = writeln!(io.out, "{id} {text}");
            }
            if outcome.unresolved.is_empty() {
                Ok(())
            } else {
                Err(QuestionError::Unresolved(outcome.unresolved))
            }
        })
    } else {
        let rendered = if json {
            question::render_tree_json(&root)
        } else {
            question::render_tree(&root)
        };
        rendered.map(|text| {
            let _ = io.out.write_all(text.as_bytes());
        })
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            let _ = writeln!(io.err, "tophat question: {e}");
            EXIT_FAILED
        }
    }
}

/// Runs the `tophat` command with `args` (excluding the program name) as if started in `cwd`,
/// evaluating `families` for `check`, and returns the process exit code.
pub fn run(args: &[String], cwd: &Path, families: &[RuleFamily], io: &mut Streams<'_>) -> i32 {
    match args.first().map(String::as_str) {
        Some("check") => run_check_command(&args[1..], cwd, families, io),
        Some("study") => run_study_command(&args[1..], cwd, io),
        Some("question") => run_question_command(&args[1..], cwd, io),
        Some("-h" | "--help" | "help") => {
            let _ = io.out.write_all(USAGE.as_bytes());
            EXIT_OK
        }
        Some("-V" | "--version") => {
            let _ = writeln!(io.out, "tophat {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Some(other) => {
            let _ = writeln!(io.err, "tophat: unknown command `{other}`\n\n{USAGE}");
            EXIT_UNABLE
        }
        None => {
            let _ = write!(io.err, "{USAGE}");
            EXIT_UNABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags_and_paths() {
        let parsed = parse_check_args(&args(&["--json", "studies/a", "--strict", "--", "--x"]));
        assert_eq!(
            parsed,
            Ok(CheckArgs {
                json: true,
                strict: true,
                paths: vec![PathBuf::from("studies/a"), PathBuf::from("--x")],
            })
        );
    }

    #[test]
    fn rejects_unknown_option() {
        assert!(parse_check_args(&args(&["--jsn"])).is_err());
    }

    #[test]
    fn parses_study_subcommands() {
        assert!(matches!(
            parse_study_args(&args(&[
                "new",
                "abc",
                "--question",
                "q-1a2b3c4d",
                "--visibility",
                "private"
            ])),
            Ok(StudyCommand::New { private: true, .. })
        ));
        assert!(matches!(
            parse_study_args(&args(&["supersede", "abc", "--by", "def"])),
            Ok(StudyCommand::Supersede { .. })
        ));
        assert!(parse_study_args(&args(&["new", "abc"])).is_err());
        assert!(parse_study_args(&args(&[
            "new",
            "abc",
            "--visibility",
            "secret",
            "--question",
            "q"
        ]))
        .is_err());
        assert!(parse_study_args(&args(&["report"])).is_err());
        assert!(parse_study_args(&args(&["report", "a", "b"])).is_err());
        assert!(parse_study_args(&args(&["frobnicate", "a"])).is_err());
    }
}
