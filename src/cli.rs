//! Command-line interface of the `tophat` binary.
//!
//! [`run`] takes its environment (arguments, working directory, rule families, terminal streams)
//! as parameters so every command can be exercised in tests.

use crate::check::{find_project_root, resolve_scope, run_families, CheckError, RuleFamily};
use crate::data::{self, DataError, Origin, RegisterOptions};
use crate::pipeline;
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
  study summarize <ID>                           Write SUMMARY.md from sealed result stages
  question stamp                                 Assign IDs to new questions in PROJECT.md
  data add <NAME> --root <R> --path <P> --source <TEXT> [--date D] [--license L] [--visibility V]
                                                 Register raw data as the next dataset version
  data declare <NAME> --root <R> --path <P> --statement <TEXT> [--visibility V]
                                                 Register data of unknown history
  data publish <STUDY> --stage <S> [--variant V] --as <NAME>
                                                 Publish a sealed stage output as a dataset
  data verify [<NAME>[@v<N>]]                    Re-hash datasets and compare to manifests
  pipeline keys <STUDY> [--variant V] [--json]   Show stage keys and sealed state
  pipeline rules <STUDY> --output <FILE>         Generate the study's Snakemake rules
  pipeline stub <STUDY>                          Write the study's Snakefile stub
  stage qc <STUDY> <STAGE> [--variant V]         Record a QC verdict (interactive, by a human)
  stage qc-status <STUDY> <STAGE> --key <K>      Exit 0 if the verdict is pass
  stage seal <STUDY> <STAGE> --key <K> [--variant V] [--item I] [--trial]
                                                 Record provenance for a finished stage
  data retire <NAME>@v<N> --reason <TEXT>        Mark a dataset version as defective
  data lineage <NAME>@v<N>|<STUDY>/<STAGE>@<KEY> [--json]
                                                 Show what a dataset or output derives from
  data used-by <NAME>[@v<N>] [--json]            List consumers of a dataset
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
    Summarize(String),
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
        "summarize" => Ok(StudyCommand::Summarize(one_id(
            parse_options(rest, &[])?.0,
        )?)),
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
    if let StudyCommand::Summarize(id) = &command {
        return match pipeline::summarize(&root, id)
            .and_then(|summary| pipeline::write_summary(&root, &summary))
        {
            Ok(written) => {
                let verb = if written { "Wrote" } else { "Unchanged:" };
                let _ = writeln!(io.out, "{verb} studies/{id}/SUMMARY.md");
                EXIT_OK
            }
            Err(e) => {
                let _ = writeln!(io.err, "tophat study summarize: {e}");
                EXIT_FAILED
            }
        };
    }
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
        StudyCommand::Summarize(_) => unreachable!("handled above"),
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            let _ = writeln!(io.err, "tophat study: {e}");
            EXIT_FAILED
        }
    }
}

/// Removes a boolean `--json` flag from the arguments, returning whether it was present.
fn take_json_flag(args: &[String]) -> (bool, Vec<String>) {
    let json = args.iter().any(|a| a == "--json");
    (
        json,
        args.iter().filter(|a| *a != "--json").cloned().collect(),
    )
}

fn run_data_command(args: &[String], cwd: &Path, io: &mut Streams<'_>) -> i32 {
    let usage = |io: &mut Streams<'_>, message: &str| {
        let _ = writeln!(io.err, "tophat data: {message}\n\n{USAGE}");
        EXIT_UNABLE
    };
    let Some((sub, rest)) = args.split_first() else {
        return usage(io, "missing data subcommand");
    };
    let (json, rest) = take_json_flag(rest);
    let known: &[&str] = match sub.as_str() {
        "add" => &["root", "path", "source", "date", "license", "visibility"],
        "declare" => &["root", "path", "statement", "visibility"],
        "retire" => &["reason"],
        "verify" | "lineage" | "used-by" => &[],
        "publish" => &["stage", "variant", "as"],
        other => return usage(io, &format!("unknown data subcommand `{other}`")),
    };
    let (positionals, options) = match parse_options(&rest, known) {
        Ok(parsed) => parsed,
        Err(message) => return usage(io, &message),
    };
    let opt = |name: &str| option(&options, name).map(str::to_string);
    let one = |positionals: &[String]| match positionals {
        [x] => Ok(x.clone()),
        _ => Err(format!("`tophat data {sub}` takes exactly one argument")),
    };
    let root = match find_project_root(cwd) {
        Ok(root) => root,
        Err(e) => {
            let _ = writeln!(io.err, "tophat data: {e}");
            return EXIT_UNABLE;
        }
    };

    let result: Result<i32, DataError> = match sub.as_str() {
        "add" | "declare" => {
            let name = match one(&positionals) {
                Ok(n) => n,
                Err(m) => return usage(io, &m),
            };
            let (Some(data_root), Some(path)) = (opt("root"), opt("path")) else {
                return usage(io, "--root and --path are required");
            };
            let origin = if sub == "add" {
                let Some(source) = opt("source") else {
                    return usage(io, "--source is required");
                };
                Origin::Raw {
                    source,
                    date: opt("date"),
                    license: opt("license"),
                }
            } else {
                let Some(statement) = opt("statement") else {
                    return usage(io, "--statement is required");
                };
                Origin::Declared { statement }
            };
            let options = RegisterOptions {
                name,
                root: data_root,
                path,
                visibility: opt("visibility").unwrap_or_else(|| "public".into()),
                origin,
            };
            let registered = if sub == "add" {
                data::add(&root, &options)
            } else {
                data::declare(&root, &options)
            };
            registered.map(|r| {
                let _ = writeln!(io.out, "{r}");
                EXIT_OK
            })
        }
        "verify" => {
            let target = match positionals.as_slice() {
                [] => None,
                [t] => Some(t.as_str()),
                _ => return usage(io, "`tophat data verify` takes at most one dataset"),
            };
            data::verify(&root, target).map(|report| {
                for line in &report.lines {
                    let _ = writeln!(io.out, "{line}");
                }
                let _ = writeln!(io.out, "{}", report.summary());
                if report.problems == 0 {
                    EXIT_OK
                } else {
                    EXIT_FAILED
                }
            })
        }
        "publish" => {
            let study = match one(&positionals) {
                Ok(s) => s,
                Err(m) => return usage(io, &m),
            };
            let (Some(stage), Some(name)) = (opt("stage"), opt("as")) else {
                return usage(io, "--stage and --as are required");
            };
            data::publish(&root, &study, &stage, opt("variant").as_deref(), &name).map(|r| {
                let _ = writeln!(io.out, "{r}");
                EXIT_OK
            })
        }
        "retire" => {
            let reference = match one(&positionals) {
                Ok(r) => r,
                Err(m) => return usage(io, &m),
            };
            let Some(reason) = opt("reason") else {
                return usage(io, "--reason is required");
            };
            let today = chrono::Local::now().date_naive();
            data::retire(&root, &reference, &reason, today).map(|()| {
                let _ = writeln!(io.out, "Retired {reference}.");
                EXIT_OK
            })
        }
        "lineage" => {
            let target = match one(&positionals) {
                Ok(t) => t,
                Err(m) => return usage(io, &m),
            };
            data::lineage(&root, &target).map(|node| {
                let text = if json {
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&node.to_json()).expect("serializes")
                    )
                } else {
                    node.render()
                };
                let _ = io.out.write_all(text.as_bytes());
                EXIT_OK
            })
        }
        _ => {
            let query = match one(&positionals) {
                Ok(q) => q,
                Err(m) => return usage(io, &m),
            };
            data::used_by(&root, &query).map(|used| {
                if json {
                    let _ = writeln!(
                        io.out,
                        "{}",
                        serde_json::to_string_pretty(&used.to_json()).expect("serializes")
                    );
                } else {
                    let _ = io.out.write_all(used.render().as_bytes());
                }
                EXIT_OK
            })
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(io.err, "tophat data {sub}: {e}");
            EXIT_FAILED
        }
    }
}

/// Removes a boolean flag from the arguments, returning whether it was present.
fn take_flag(args: &[String], flag: &str) -> (bool, Vec<String>) {
    (
        args.iter().any(|a| a == flag),
        args.iter().filter(|a| *a != flag).cloned().collect(),
    )
}

fn run_pipeline_command(args: &[String], cwd: &Path, io: &mut Streams<'_>) -> i32 {
    let usage = |io: &mut Streams<'_>, message: &str| {
        let _ = writeln!(io.err, "tophat pipeline: {message}\n\n{USAGE}");
        EXIT_UNABLE
    };
    let Some((sub, rest)) = args.split_first() else {
        return usage(io, "expected `keys`, `rules` or `stub`");
    };
    let (json, rest) = take_flag(rest, "--json");
    let known: &[&str] = match sub.as_str() {
        "keys" => &["variant"],
        "rules" => &["output"],
        "stub" => &[],
        other => return usage(io, &format!("unknown pipeline subcommand `{other}`")),
    };
    let (positionals, options) = match parse_options(&rest, known) {
        Ok(parsed) => parsed,
        Err(m) => return usage(io, &m),
    };
    let [study] = positionals.as_slice() else {
        return usage(io, &format!("`tophat pipeline {sub}` takes one study ID"));
    };
    let root = match find_project_root(cwd) {
        Ok(root) => root,
        Err(e) => {
            let _ = writeln!(io.err, "tophat pipeline: {e}");
            return EXIT_UNABLE;
        }
    };
    let result: Result<(), pipeline::PipelineError> = match sub.as_str() {
        "keys" => pipeline::key_status(&root, study, option(&options, "variant").unwrap_or("base"))
            .map(|stages| {
                if json {
                    let value: Vec<_> = stages
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "stage": s.stage, "key": s.key,
                                "output": s.output.as_ref().map(|p| p.display().to_string()),
                                "status": s.status,
                            })
                        })
                        .collect();
                    let _ = writeln!(
                        io.out,
                        "{}",
                        serde_json::to_string_pretty(&value).expect("serializes")
                    );
                } else {
                    for s in &stages {
                        let output = s
                            .output
                            .as_ref()
                            .map_or_else(|| "-".to_string(), |p| p.display().to_string());
                        let _ =
                            writeln!(io.out, "{}  {}  {}  {}", s.stage, s.key, s.status, output);
                    }
                }
            }),
        "rules" => {
            let Some(output) = option(&options, "output") else {
                return usage(io, "--output is required");
            };
            let output = cwd.join(output);
            pipeline::generate_rules(&root, study).and_then(|text| {
                // Snakemake re-reads the Snakefile in every job process, so this runs concurrently;
                // an unchanged file is left alone, and writes use a per-process temporary file.
                if std::fs::read_to_string(&output).is_ok_and(|old| old == text) {
                    return Ok(());
                }
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| pipeline::PipelineError::Io(e.to_string()))?;
                }
                crate::fsutil::write_atomic_unique(&output, &text)
                    .map_err(|e| pipeline::PipelineError::Io(e.to_string()))
            })
        }
        _ => {
            let path = root.join(pipeline::snakefile_path(study));
            if !path.parent().is_some_and(Path::is_dir) {
                Err(pipeline::PipelineError::Study(format!(
                    "study `{study}` does not exist"
                )))
            } else {
                crate::fsutil::write_atomic(&path, &pipeline::stub(study))
                    .map_err(|e| pipeline::PipelineError::Io(e.to_string()))
                    .map(|()| {
                        let _ = writeln!(io.out, "Wrote {}", pipeline::snakefile_path(study));
                    })
            }
        }
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            let _ = writeln!(io.err, "tophat pipeline {sub}: {e}");
            EXIT_FAILED
        }
    }
}

fn run_stage_command(args: &[String], cwd: &Path, io: &mut Streams<'_>) -> i32 {
    let usage = |io: &mut Streams<'_>, message: &str| {
        let _ = writeln!(io.err, "tophat stage: {message}\n\n{USAGE}");
        EXIT_UNABLE
    };
    let Some((sub, rest)) = args.split_first() else {
        return usage(io, "expected `seal`, `begin`, `qc` or `qc-status`");
    };
    let (trial, rest) = take_flag(rest, "--trial");
    let known: &[&str] = match sub.as_str() {
        "seal" | "begin" => &["key", "variant", "item"],
        "qc" => &["variant"],
        "qc-status" => &["key"],
        other => return usage(io, &format!("unknown stage subcommand `{other}`")),
    };
    let (positionals, options) = match parse_options(&rest, known) {
        Ok(parsed) => parsed,
        Err(m) => return usage(io, &m),
    };
    let [study, stage] = positionals.as_slice() else {
        return usage(
            io,
            &format!("`tophat stage {sub}` takes a study ID and a stage name"),
        );
    };
    let root = match find_project_root(cwd) {
        Ok(root) => root,
        Err(e) => {
            let _ = writeln!(io.err, "tophat stage: {e}");
            return EXIT_UNABLE;
        }
    };
    let fail = |io: &mut Streams<'_>, e: &dyn std::fmt::Display| {
        let _ = writeln!(io.err, "tophat stage {sub}: {e}");
        EXIT_FAILED
    };
    match sub.as_str() {
        "qc" => {
            let today = chrono::Local::now().date_naive();
            let variant = option(&options, "variant").unwrap_or("base").to_string();
            let mut console = Console {
                interactive: io.interactive,
                input: &mut *io.input,
                out: &mut *io.out,
            };
            match pipeline::qc(&root, study, stage, &variant, today, &mut console) {
                Ok(_) => EXIT_OK,
                Err(e) => fail(io, &e),
            }
        }
        "qc-status" => {
            let Some(key) = option(&options, "key") else {
                return usage(io, "--key is required");
            };
            match pipeline::qc_status(&root, study, stage, key) {
                Ok(pipeline::Verdict::Pass) => {
                    let _ = writeln!(io.out, "pass");
                    EXIT_OK
                }
                Ok(pipeline::Verdict::Fail) => {
                    let _ = writeln!(io.out, "fail");
                    EXIT_FAILED
                }
                Err(message) => {
                    let _ = writeln!(io.out, "{message}");
                    EXIT_UNABLE
                }
            }
        }
        _ => {
            let Some(key) = option(&options, "key") else {
                return usage(io, "--key is required");
            };
            let request = pipeline::SealRequest {
                study: study.clone(),
                stage: stage.clone(),
                key: key.to_string(),
                variant: option(&options, "variant").map(str::to_string),
                item: option(&options, "item").map(str::to_string),
                trial,
            };
            if sub == "begin" {
                match pipeline::begin(&root, &request) {
                    Ok(job) => {
                        let value = serde_json::json!({
                            "run": job.run, "env": job.env, "cwd": job.cwd.display().to_string(),
                        });
                        let _ = writeln!(io.out, "{value}");
                        EXIT_OK
                    }
                    Err(e) => fail(io, &e),
                }
            } else {
                match pipeline::seal(&root, &request) {
                    Ok(path) => {
                        let _ = writeln!(io.out, "Sealed {}", path.display());
                        EXIT_OK
                    }
                    Err(e) => fail(io, &e),
                }
            }
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
        Some("data") => run_data_command(&args[1..], cwd, io),
        Some("pipeline") => run_pipeline_command(&args[1..], cwd, io),
        Some("stage") => run_stage_command(&args[1..], cwd, io),
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
