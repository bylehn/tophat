//! External (cluster) stages: the job folder, its job script and receipt, and ingest. Specified in
//! `rqm/pipeline/external.md`.
//!
//! Nothing here connects to a remote machine: the human copies the job folder to the cluster,
//! submits it, and copies it back.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use yaml_rust2::{Yaml, YamlLoader};

use super::config::{ExternalConfig, StageConfig, StageInput};
use super::keys::{render_run, shell_quote, RunPaths};
use super::provenance::{
    write_record, OutputFile, ProvenanceRecord, RecordContent, ResolvedStage, SealRequest,
    RECORD_FILE,
};
use super::PipelineError;
use crate::data::{hash_files, is_safe_relative_path, to_hex};
use crate::study::Status;

/// The receipt the job writes in its folder.
pub const RECEIPT_FILE: &str = "receipt.txt";
/// The user's own `sbatch` options (git-ignored, per machine).
pub const SLURM_DEFAULTS_FILE: &str = ".tophat/slurm.local.yaml";
/// First line of every receipt.
const RECEIPT_FORMAT: &str = "tophat-receipt 1";
/// Copied inputs above this total size (10 GB) get a warning.
pub const LARGE_INPUTS_BYTES: u64 = 10_000_000_000;
/// Names at the top of a job folder that `code` files cannot take.
pub(crate) const RESERVED_NAMES: [&str; 8] = [
    "README.md",
    "job.sh",
    "stage.json",
    "inputs",
    "inputs.sha256",
    "params.json",
    "out",
    RECEIPT_FILE,
];

// rq-57a8f3ae
/// Error of the external-stage commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalError {
    /// The job folder is ready and has not been returned; carries the steps.
    Waiting(String),
    ReceiptInvalid(String),
    /// The receipt is for another key than the stage's current one.
    KeyMismatch {
        expected: String,
        actual: String,
    },
    /// The job script that ran is not the one generated for this key (beyond `#SBATCH` lines).
    ScriptModified,
    JobFailed {
        exit: String,
    },
    /// Returned files that are missing, changed or unexpected.
    OutputMismatch(Vec<String>),
    /// The stage cannot be used (configuration, key, inputs).
    Pipeline(PipelineError),
    Io(String),
}

impl fmt::Display for ExternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalError::Waiting(steps) => write!(f, "{steps}"),
            ExternalError::ReceiptInvalid(m) => write!(f, "{RECEIPT_FILE} is invalid: {m}"),
            ExternalError::KeyMismatch { expected, actual } => write!(
                f,
                "{RECEIPT_FILE} is for key {actual}, but the stage's key is {expected}: it belongs \
                 to another job folder"
            ),
            ExternalError::ScriptModified => write!(
                f,
                "the job script that ran differs from the one tophat generates for this key in more \
                 than its #SBATCH lines (or the stage's inputs were re-sealed after the job folder \
                 was written). Delete the job folder and run the pipeline again for a fresh one"
            ),
            ExternalError::JobFailed { exit } => write!(
                f,
                "the job ended with exit status `{exit}`. To try again, delete {RECEIPT_FILE} and \
                 the contents of out/ in the job folder on the cluster, and submit it again; if the \
                 configuration must change, the next run writes a new job folder"
            ),
            ExternalError::OutputMismatch(problems) => write!(
                f,
                "the returned files do not match the receipt: {}",
                problems.join("; ")
            ),
            ExternalError::Pipeline(e) => write!(f, "{e}"),
            ExternalError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ExternalError {}

impl From<PipelineError> for ExternalError {
    fn from(e: PipelineError) -> Self {
        ExternalError::Pipeline(e)
    }
}

fn io_error(context: &Path) -> impl Fn(std::io::Error) -> ExternalError + '_ {
    move |e| ExternalError::Io(format!("{}: {e}", context.display()))
}

// rq-10f972ce
/// A parsed `receipt.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub study: String,
    pub stage: String,
    pub key: String,
    /// SHA-256 of `job.sh` without its `#SBATCH` lines.
    pub script: String,
    pub host: String,
    /// `$SLURM_JOB_ID`, or `-` outside SLURM.
    pub job: String,
    /// Start and end time, or `-` when the command did not run.
    pub start: String,
    pub end: String,
    /// The command's exit status, or `input-mismatch`.
    pub exit: String,
    pub modules: Vec<String>,
    /// Files to copy back.
    pub outputs: Vec<OutputFile>,
    /// Files matching `keep`, which stay on the cluster.
    pub kept: Vec<OutputFile>,
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `path` can be listed in a job folder file: a safe relative path without control
/// characters.
fn is_listable_path(path: &str) -> bool {
    is_safe_relative_path(path) && !path.chars().any(char::is_control)
}

/// Parses `<sha256>  <size>  <path>`.
fn parse_file_line(value: &str) -> Result<OutputFile, String> {
    let (sha256, rest) = value
        .split_once("  ")
        .ok_or("expected `<sha256>  <size>  <path>`")?;
    let (size, path) = rest
        .split_once("  ")
        .ok_or("expected `<sha256>  <size>  <path>`")?;
    if !is_sha256(sha256) {
        return Err(format!("malformed hash `{sha256}`"));
    }
    let size = size
        .parse::<u64>()
        .map_err(|_| format!("malformed size `{size}`"))?;
    if !is_listable_path(path) {
        return Err(format!("`{path}` is not a relative path inside out/"));
    }
    Ok(OutputFile {
        path: path.to_string(),
        size,
        sha256: sha256.to_string(),
    })
}

// rq-d39d6009
/// Parses and validates a receipt. Errors name the offending line.
pub fn parse_receipt(text: &str) -> Result<Receipt, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, RECEIPT_FORMAT)) => {}
        Some((_, other)) => {
            return Err(format!("line 1 (`{other}`): expected `{RECEIPT_FORMAT}`"));
        }
        None => return Err("the receipt is empty".into()),
    }
    let mut single: BTreeMap<&str, String> = BTreeMap::new();
    let mut modules = Vec::new();
    let mut outputs = Vec::new();
    let mut kept = Vec::new();
    let mut paths = BTreeSet::new();
    for (i, line) in lines {
        let bad = |m: &str| format!("line {} (`{line}`): {m}", i + 1);
        let Some((field, value)) = line.split_once(' ') else {
            return Err(bad("expected `<field> <value>`"));
        };
        match field {
            "study" | "stage" | "key" | "script" | "host" | "job" | "start" | "end" | "exit" => {
                if value.is_empty() || value.trim() != value {
                    return Err(bad("empty or padded value"));
                }
                if single.insert(field, value.to_string()).is_some() {
                    return Err(bad(&format!("`{field}` appears more than once")));
                }
            }
            "module" => {
                if value.trim().is_empty() {
                    return Err(bad("empty module"));
                }
                modules.push(value.to_string());
            }
            "output" | "kept" => {
                let file = parse_file_line(value).map_err(|m| bad(&m))?;
                if !paths.insert(file.path.clone()) {
                    return Err(bad(&format!("`{}` is listed more than once", file.path)));
                }
                if field == "output" {
                    outputs.push(file);
                } else {
                    kept.push(file);
                }
            }
            other => return Err(bad(&format!("unknown field `{other}`"))),
        }
    }
    let mut take = |field: &str| {
        single
            .remove(field)
            .ok_or_else(|| format!("the `{field}` line is missing"))
    };
    let receipt = Receipt {
        study: take("study")?,
        stage: take("stage")?,
        key: take("key")?,
        script: take("script")?,
        host: take("host")?,
        job: take("job")?,
        start: take("start")?,
        end: take("end")?,
        exit: take("exit")?,
        modules,
        outputs,
        kept,
    };
    let key_ok = receipt.key.strip_prefix("sk-").is_some_and(|hex| {
        hex.len() == 16
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    if !key_ok {
        return Err(format!("`key {}` is not a stage key", receipt.key));
    }
    if !is_sha256(&receipt.script) {
        return Err(format!("`script {}` is not a SHA-256 hash", receipt.script));
    }
    let exit_ok = receipt.exit == "input-mismatch"
        || (receipt.exit.len() <= 3 && receipt.exit.bytes().all(|b| b.is_ascii_digit()));
    if !exit_ok {
        return Err(format!(
            "`exit {}` must be a number or `input-mismatch`",
            receipt.exit
        ));
    }
    Ok(receipt)
}

/// Matches a `[...]` class at the start of `pattern` (after the `[`) against `c`. Returns whether
/// it matched and the rest of the pattern, or `None` if the class is not closed.
fn match_class(pattern: &[u8], c: u8) -> Option<(bool, &[u8])> {
    let (negate, body) = match pattern.first() {
        Some(b'!') => (true, &pattern[1..]),
        _ => (false, pattern),
    };
    // A `]` right after the opening bracket is literal.
    let close = body.iter().skip(1).position(|&b| b == b']')? + 1;
    let set = &body[..close];
    let mut found = false;
    let mut i = 0;
    while i < set.len() {
        if i + 2 < set.len() && set[i + 1] == b'-' {
            found |= set[i] <= c && c <= set[i + 2];
            i += 3;
        } else {
            found |= set[i] == c;
            i += 1;
        }
    }
    Some((found != negate, &body[close + 1..]))
}

/// Shell-style glob matching in which `*`, `?` and `[...]` never match `/` (as in
/// `rsync --exclude`).
fn glob_match(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((b'*', rest)) => (0..=path.len())
            .take_while(|&i| i == 0 || path[i - 1] != b'/')
            .any(|i| glob_match(rest, &path[i..])),
        Some((b'?', rest)) => {
            path.first().is_some_and(|&c| c != b'/') && glob_match(rest, &path[1..])
        }
        Some((b'[', rest)) => {
            match (path.first(), match_class(rest, *path.first().unwrap_or(&0))) {
                (Some(&c), Some((matched, after))) => {
                    c != b'/' && matched && glob_match(after, &path[1..])
                }
                (Some(&c), None) => c == b'[' && glob_match(rest, &path[1..]),
                (None, _) => false,
            }
        }
        Some((&c, rest)) => path.first() == Some(&c) && glob_match(rest, &path[1..]),
    }
}

/// Whether a path relative to `out/` matches one of the `keep` patterns.
pub(crate) fn is_kept(keep: &[String], path: &str) -> bool {
    keep.iter()
        .any(|p| glob_match(p.as_bytes(), path.as_bytes()))
}

// rq-964a4533
/// Reads `.tophat/slurm.local.yaml`: the user's `sbatch` options. A missing file means none.
pub fn slurm_defaults(root: &Path) -> Result<Vec<String>, String> {
    let Ok(text) = fs::read_to_string(root.join(SLURM_DEFAULTS_FILE)) else {
        return Ok(Vec::new());
    };
    let doc = YamlLoader::load_from_str(&text)
        .map_err(|e| format!("invalid YAML: {}", e.info()))?
        .into_iter()
        .next()
        .unwrap_or(Yaml::Null);
    let hash = match doc {
        Yaml::Null => return Ok(Vec::new()),
        Yaml::Hash(hash) => hash,
        _ => return Err("the file must be a mapping with `sbatch`".into()),
    };
    let mut options = Vec::new();
    for (key, value) in hash {
        match key.as_str() {
            Some("sbatch") => {}
            _ => return Err(format!("unknown key {key:?}; only `sbatch` is allowed")),
        }
        let items = match value {
            Yaml::Null => Vec::new(),
            Yaml::Array(items) => items,
            _ => return Err("`sbatch` must be a list of options such as `--account=e123`".into()),
        };
        for item in items {
            match item {
                Yaml::String(s) if s.starts_with("--") && !s.contains(['\n', '\r']) => {
                    options.push(s)
                }
                other => {
                    return Err(format!(
                        "`sbatch` entry {other:?} must be an option starting with `--`, such as \
                         `--account=e123`"
                    ))
                }
            }
        }
    }
    Ok(options)
}

/// The name of an `sbatch` option: the text before `=` (or before a space).
fn option_name(option: &str) -> &str {
    option.split(['=', ' ']).next().unwrap_or(option)
}

// rq-964a4533 rq-13602cac
/// The `#SBATCH` options of a job: the defaults, overridden by the stage's options of the same
/// name, then the stage's options, then `--cpus-per-task` and `--job-name` unless already set.
fn sbatch_options(defaults: &[String], stage: &StageConfig, slurm: &[String]) -> Vec<String> {
    let overridden: BTreeSet<&str> = slurm.iter().map(|o| option_name(o)).collect();
    let mut options: Vec<String> = defaults
        .iter()
        .filter(|o| !overridden.contains(option_name(o)))
        .cloned()
        .collect();
    options.extend(slurm.iter().cloned());
    let set = |options: &[String], name: &str| options.iter().any(|o| option_name(o) == name);
    if !set(&options, "--cpus-per-task") {
        options.push(format!("--cpus-per-task={}", stage.threads));
    }
    if !set(&options, "--job-name") {
        options.push(format!("--job-name={}", stage.name));
    }
    options
}

/// One input file copied into the job folder.
struct InputFile {
    /// Path in the job folder: `inputs/<input>/<path>`.
    rel: String,
    source: PathBuf,
    size: u64,
    sha256: String,
}

/// The files of a sealed stage output (and its fan-out items), as recorded.
fn sealed_files(
    dir: &Path,
    key: &str,
    what: &str,
) -> Result<Vec<(String, OutputFile)>, ExternalError> {
    let unsealed =
        || PipelineError::NotSealed(format!("{what} has no sealed output for key {key}"));
    let record = ProvenanceRecord::read(dir)
        .filter(|r| r.key == key)
        .ok_or_else(unsealed)?;
    let mut files: Vec<(String, OutputFile)> = record
        .outputs
        .iter()
        .map(|o| (o.path.clone(), o.clone()))
        .collect();
    for item in record.items.iter().flatten() {
        let item_dir = dir.join("items").join(&item.name);
        let item_record = ProvenanceRecord::read(&item_dir)
            .filter(|r| r.key == key)
            .ok_or_else(unsealed)?;
        files.extend(
            item_record
                .outputs
                .into_iter()
                .map(|o| (format!("items/{}/{}", item.name, o.path), o)),
        );
    }
    Ok(files)
}

/// Every input file of a stage, sorted by its path in the job folder.
fn input_files(
    resolved: &ResolvedStage,
    stage: &StageConfig,
    trial: bool,
) -> Result<Vec<InputFile>, ExternalError> {
    let mut files = Vec::new();
    for (name, input) in &stage.inputs {
        let (dir, listed): (PathBuf, Vec<(String, u64, String)>) = match input {
            StageInput::Stage(upstream) => {
                let dir = resolved.out_dir(upstream, trial)?;
                let listed = sealed_files(
                    &dir,
                    resolved.key(upstream),
                    &format!("input `{name}` (stage `{upstream}`)"),
                )?
                .into_iter()
                .map(|(rel, o)| (rel, o.size, o.sha256))
                .collect();
                (dir, listed)
            }
            StageInput::Dataset(r) => {
                let m = resolved
                    .ctx
                    .manifest(r)
                    .ok_or_else(|| PipelineError::Data(format!("dataset `{r}` does not exist")))?;
                let location = resolved
                    .roots
                    .location(&m.root)
                    .ok_or_else(|| PipelineError::UnconfiguredRoot(m.root.clone()))?;
                let listed = m
                    .files
                    .iter()
                    .map(|f| (f.path.clone(), f.size, f.sha256.clone()))
                    .collect();
                (location.join(&m.path), listed)
            }
        };
        for (path, size, sha256) in listed {
            if !is_listable_path(&path) {
                return Err(ExternalError::Io(format!(
                    "input `{name}`: file name {path:?} cannot be copied into a job folder"
                )));
            }
            files.push(InputFile {
                rel: format!("inputs/{name}/{path}"),
                source: dir.join(&path),
                size,
                sha256,
            });
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}

/// The text of `inputs.sha256`.
fn inputs_sha256(files: &[InputFile]) -> String {
    files
        .iter()
        .map(|f| format!("{}  {}\n", f.sha256, f.rel))
        .collect()
}

fn sha256_hex(text: &str) -> String {
    to_hex(&Sha256::digest(text.as_bytes()))
}

/// The SHA-256 of a job script with every line starting with `#SBATCH` removed, as
/// `grep -v '^#SBATCH' job.sh | sha256sum` computes it.
pub fn script_sha256(text: &str) -> String {
    let mut hasher = Sha256::new();
    for line in text.split_inclusive('\n') {
        if !line.starts_with("#SBATCH") {
            hasher.update(line.as_bytes());
            if !line.ends_with('\n') {
                hasher.update(b"\n");
            }
        }
    }
    to_hex(&hasher.finalize())
}

/// The command with placeholders substituted by paths relative to the job folder.
fn job_run(stage: &StageConfig) -> Result<String, ExternalError> {
    let paths = RunPaths {
        out: PathBuf::from("out"),
        inputs: stage
            .inputs
            .keys()
            .map(|n| (n.clone(), PathBuf::from(format!("inputs/{n}"))))
            .collect(),
        item: None,
        params_file: PathBuf::from("params.json"),
        tool: None,
    };
    render_run(stage, &paths).map_err(ExternalError::Io)
}

/// The text of `params.json`.
fn params_json(stage: &StageConfig) -> String {
    serde_json::to_string_pretty(&Value::Object(stage.params.clone()))
        .expect("JSON values always serialize")
        + "\n"
}

/// The shell function deciding whether a path below `out/` is kept on the cluster. A pattern
/// matches only if `case` matches and the path has as many `/` as the pattern, so wildcards never
/// match `/`.
fn kept_function(keep: &[String]) -> String {
    let mut out = String::from("tophat_kept() {\n");
    for pattern in keep {
        let slashes = pattern.matches('/').count();
        out.push_str(&format!(
            "  case \"$1\" in\n    {pattern})\n      if [ \"$(tophat_slashes \"$1\")\" -eq {slashes} ]; then return 0; fi ;;\n  esac\n"
        ));
    }
    out.push_str("  return 1\n}\n");
    out
}

// rq-d966a5fb rq-13602cac
/// The job script of an external stage. Only its `#SBATCH` lines depend on `sbatch`; the rest is
/// determined by the stage key (with the inputs' recorded hashes), so ingest can regenerate it.
fn job_script(
    resolved: &ResolvedStage,
    stage: &StageConfig,
    inputs: &[InputFile],
    sbatch: &[String],
) -> Result<String, ExternalError> {
    let external = stage.external.as_ref().expect("an external stage");
    let study = &resolved.config.study;
    let key = resolved.key(&stage.name);
    let code = resolved
        .ctx
        .code_files(study, stage)
        .map_err(ExternalError::Io)?;
    let mut checks = vec![
        (
            sha256_hex(&inputs_sha256(inputs)),
            "inputs.sha256".to_string(),
        ),
        (sha256_hex(&params_json(stage)), "params.json".to_string()),
    ];
    checks.extend(code.into_iter().map(|c| (c.sha256, c.path)));

    let mut s = String::from("#!/bin/sh\n");
    for option in sbatch {
        s.push_str(&format!("#SBATCH {option}\n"));
    }
    s.push_str(&format!(
        "#
# External job for stage `{stage_name}` of study `{study}`, key {key}, written by tophat {version}.
# You may edit the #SBATCH lines above. Any other change is detected when the folder returns.
# README.md has the steps.

set -eu
tophat_key='{key}'
tophat_here() {{
  [ -f \"$1/inputs.sha256\" ] && grep -F -q \"$tophat_key\" \"$1/stage.json\" 2>/dev/null
}}
if [ -n \"${{SLURM_SUBMIT_DIR:-}}\" ] && tophat_here \"$SLURM_SUBMIT_DIR\"; then
  cd \"$SLURM_SUBMIT_DIR\"
elif tophat_here \"$(dirname \"$0\")\"; then
  cd \"$(dirname \"$0\")\"
else
  echo \"job.sh: submit this script from its job folder: cd into the folder, then sbatch job.sh\" >&2
  exit 2
fi

if command -v sha256sum >/dev/null 2>&1; then
  tophat_sha256() {{ tophat_h=$(sha256sum); printf '%s\\n' \"${{tophat_h%% *}}\"; }}
elif command -v shasum >/dev/null 2>&1; then
  tophat_sha256() {{ tophat_h=$(shasum -a 256); printf '%s\\n' \"${{tophat_h%% *}}\"; }}
else
  echo \"job.sh: neither sha256sum nor shasum is available\" >&2
  exit 2
fi
tophat_size() {{ printf '%s\\n' \"$(( $(wc -c < \"$1\") ))\"; }}
tophat_now() {{ date -u +%Y-%m-%dT%H:%M:%SZ; }}
tophat_slashes() {{
  tophat_n=0; tophat_r=$1
  while :; do
    case \"$tophat_r\" in
      */*) tophat_r=${{tophat_r#*/}}; tophat_n=$((tophat_n + 1)) ;;
      *) break ;;
    esac
  done
  printf '%s\\n' \"$tophat_n\"
}}
{kept}
tophat_receipt() {{
  {{
    printf '%s\\n' '{RECEIPT_FORMAT}' 'study {study}' 'stage {stage_name}' \"key $tophat_key\"
    printf 'script %s\\n' \"$(grep -v '^#SBATCH' job.sh | tophat_sha256)\"
    printf 'host %s\\n' \"$(uname -n)\"
    printf 'job %s\\n' \"${{SLURM_JOB_ID:--}}\"
    printf 'start %s\\n' \"$tophat_start\"
    printf 'end %s\\n' \"$tophat_end\"
    printf 'exit %s\\n' \"$1\"
    printf '%s\\n' \"$tophat_modules\" | while IFS= read -r tophat_m; do
      case \"$tophat_m\" in
        ''|*:|*' '*) ;;
        *) printf 'module %s\\n' \"$tophat_m\" ;;
      esac
    done
    find out -type f | while IFS= read -r tophat_f; do
      tophat_rel=${{tophat_f#out/}}
      tophat_line=\"$(tophat_sha256 < \"$tophat_f\")  $(tophat_size \"$tophat_f\")  $tophat_rel\"
      if tophat_kept \"$tophat_rel\"; then
        printf 'kept %s\\n' \"$tophat_line\"
      else
        printf 'output %s\\n' \"$tophat_line\"
      fi
    done
  }} > .receipt.txt.tmp
  mv .receipt.txt.tmp {RECEIPT_FILE}
}}

mkdir -p out
if [ -e {RECEIPT_FILE} ]; then
  echo \"job.sh: {RECEIPT_FILE} exists, so this job already ran. To run it again, delete {RECEIPT_FILE} and the contents of out/\" >&2
  exit 2
fi
if [ -n \"$(ls -A out)\" ]; then
  echo \"job.sh: out/ is not empty. Delete its contents before submitting the job\" >&2
  exit 2
fi

# Verify the inputs, parameters and code against the hashes tophat recorded.
tophat_bad=0
tophat_check() {{
  if [ ! -f \"$2\" ] || [ \"$(tophat_sha256 < \"$2\")\" != \"$1\" ]; then
    printf 'job.sh: %s differs from the copy tophat wrote\\n' \"$2\" >&2
    tophat_bad=1
  fi
}}
",
        stage_name = stage.name,
        version = env!("CARGO_PKG_VERSION"),
        kept = kept_function(&external.keep),
    ));
    for (sha256, path) in &checks {
        s.push_str(&format!(
            "tophat_check {} {}\n",
            shell_quote(sha256),
            shell_quote(path)
        ));
    }
    s.push_str(&format!(
        "if [ -f inputs.sha256 ]; then
  while IFS= read -r tophat_l; do
    tophat_check \"${{tophat_l%%  *}}\" \"${{tophat_l#*  }}\"
  done < inputs.sha256
fi
if [ \"$(( $(find inputs -type f 2>/dev/null | wc -l) ))\" -ne {count} ]; then
  echo \"job.sh: inputs/ holds files that tophat did not write\" >&2
  tophat_bad=1
fi
tophat_modules=
if [ \"$tophat_bad\" -ne 0 ]; then
  tophat_start=-
  tophat_end=-
  tophat_receipt input-mismatch
  echo \"job.sh: the job folder's inputs changed; the command was not run\" >&2
  exit 1
fi

# Set up, run the command, and record what happened.
set +eu
tophat_status=0
",
        count = inputs.len(),
    ));
    for line in &external.setup {
        s.push_str(&format!(
            "if [ \"$tophat_status\" -eq 0 ]; then\n{line}\ntophat_status=$?\nfi\n"
        ));
    }
    s.push_str(&format!(
        "if command -v module >/dev/null 2>&1; then
  tophat_modules=$(module -t list 2>&1)
fi
tophat_start=$(tophat_now)
if [ \"$tophat_status\" -eq 0 ]; then
  sh -c {run}
  tophat_status=$?
fi
tophat_end=$(tophat_now)
set -eu
tophat_receipt \"$tophat_status\"
exit \"$tophat_status\"
",
        run = shell_quote(&job_run(stage)?),
    ));
    Ok(s)
}

/// A word for a shell command line: as is when it needs no quoting.
fn shell_word(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+:@=,".contains(&b));
    if plain {
        s.to_string()
    } else {
        shell_quote(s)
    }
}

/// The name of the job folder inside a stage's output directory.
pub const JOB_DIR: &str = "job";

/// The name of a job folder on the cluster.
pub fn job_folder_name(study: &str, stage: &str, key: &str) -> String {
    format!("{study}.{stage}.{key}")
}

// rq-9f1e97a5
/// The human's steps for a job folder: copy it to the cluster, submit, copy it back, continue.
/// The cluster side is written with placeholders only.
fn steps(
    study: &str,
    stage: &str,
    variant: &str,
    key: &str,
    external: &ExternalConfig,
    folder: &Path,
    trial: bool,
) -> String {
    // Locally the folder is `job/`; on the cluster it carries the study, stage and key.
    let name = job_folder_name(study, stage, key);
    let local = format!("{}/", shell_word(&folder.display().to_string()));
    let back = local.clone();
    let copy_back = if external.keep.is_empty() {
        format!(
            "# 3. When the job has finished, copy the folder back\n\
             rsync -a <you>@<cluster>:<directory>/{name}/ {back}\n"
        )
    } else {
        let excludes: Vec<String> = external
            .keep
            .iter()
            .map(|p| format!("--exclude={}", shell_quote(&format!("out/{p}"))))
            .collect();
        format!(
            "# 3. When the job has finished, copy the folder back, leaving the large files on the cluster\n\
             rsync -a {} \\\n  <you>@<cluster>:<directory>/{name}/ {back}\n",
            excludes.join(" ")
        )
    };
    let draft = if trial { " --draft" } else { "" };
    format!(
        "External job for stage `{stage}` of study `{study}` (variant {variant}), key {key}.
Job folder: {folder}

TopHat never connects to the cluster: you copy the folder, submit the job and copy it back. The
commands are examples; <you>@<cluster>:<directory> stands for your login and a directory of your
choice on the cluster.

# 1. Copy this folder to the cluster
rsync -a {local} <you>@<cluster>:<directory>/{name}/

# 2. On the cluster, submit it (you may edit the #SBATCH lines first; change nothing else)
cd <directory>/{name} && sbatch job.sh

{copy_back}
# 4. Continue the pipeline
tophat run {study}{draft}

If the job fails for a reason outside the configuration (a time limit, a node failure), delete
{RECEIPT_FILE} and the contents of out/ in the folder on the cluster and submit it again. If the
configuration must change, the next run writes a new job folder.
",
        folder = folder.display(),
    )
}

/// The `README.md` of a job folder.
fn readme(steps: &str) -> String {
    format!(
        "# External job

This folder is a self-contained job written by TopHat. It holds:

- `job.sh`: the sbatch script. It verifies the inputs, runs the command, and writes `{RECEIPT_FILE}`.
- `stage.json`: the study, stage, variant, key and keep patterns.
- `inputs/`: copies of every input file; `inputs.sha256` lists their hashes.
- `params.json`: the stage's parameters, and the study's code files at their usual paths.
- `out/`: empty; the job writes its outputs here.

## Steps

```sh
{steps}```
"
    )
}

/// The total size of the inputs as a warning, if above [`LARGE_INPUTS_BYTES`].
pub fn large_inputs_warning(total: u64) -> Option<String> {
    (total > LARGE_INPUTS_BYTES).then(|| {
        format!(
            "the inputs copied into the job folder total {:.1} GB ({total} bytes)",
            total as f64 / 1e9
        )
    })
}

/// Loads a stage for an external command: it must be external and `request.key` current.
fn load_external(root: &Path, request: &SealRequest) -> Result<ResolvedStage, ExternalError> {
    let variant = request.variant.as_deref().unwrap_or("base");
    let resolved = ResolvedStage::load_mode(
        root,
        &request.study,
        variant,
        Some(&request.stage),
        request.trial,
    )?;
    let stage = resolved.stage(&request.stage)?;
    if stage.external.is_none() {
        return Err(PipelineError::Config {
            path: stage.file.clone(),
            line: stage.line,
            message: format!("stage `{}` is not an external stage", stage.name),
        }
        .into());
    }
    let actual = resolved.key(&stage.name).to_string();
    if actual != request.key {
        return Err(PipelineError::KeyMismatch {
            expected: request.key.clone(),
            actual,
        }
        .into());
    }
    Ok(resolved)
}

/// The job folder of a stage: `job/` in its output directory.
fn folder_of(resolved: &ResolvedStage, stage: &str, trial: bool) -> Result<PathBuf, ExternalError> {
    Ok(resolved.out_dir(stage, trial)?.join(JOB_DIR))
}

fn steps_of(resolved: &ResolvedStage, stage: &StageConfig, folder: &Path, trial: bool) -> String {
    steps(
        &resolved.config.study,
        &stage.name,
        &resolved.config.variant,
        resolved.key(&stage.name),
        stage.external.as_ref().expect("an external stage"),
        folder,
        trial,
    )
}

/// Writes the contents of a job folder into `dir`.
fn populate(
    root: &Path,
    resolved: &ResolvedStage,
    stage: &StageConfig,
    request: &SealRequest,
    inputs: &[InputFile],
    dir: &Path,
    folder: &Path,
) -> Result<(), ExternalError> {
    let external = stage.external.as_ref().expect("an external stage");
    for sub in ["inputs", "out"] {
        fs::create_dir_all(dir.join(sub)).map_err(io_error(dir))?;
    }
    for f in inputs {
        let target = dir.join(&f.rel);
        fs::create_dir_all(target.parent().expect("inside the folder")).map_err(io_error(dir))?;
        fs::copy(&f.source, &target).map_err(io_error(&f.source))?;
    }
    // The copies must be what was recorded: a changed upstream file is never sent.
    let targets: Vec<PathBuf> = inputs.iter().map(|f| dir.join(&f.rel)).collect();
    for (f, hashed) in inputs.iter().zip(hash_files(&targets)) {
        let (size, sha256) = hashed.map_err(io_error(&f.source))?;
        if size != f.size || sha256 != f.sha256 {
            return Err(ExternalError::Io(format!(
                "input file {} differs from its recorded hash; verify the input before running \
                 the stage",
                f.source.display()
            )));
        }
    }
    let code = resolved
        .ctx
        .code_files(&resolved.config.study, stage)
        .map_err(ExternalError::Io)?;
    for c in &code {
        let target = dir.join(&c.path);
        fs::create_dir_all(target.parent().expect("inside the folder")).map_err(io_error(dir))?;
        let source = root.join(&c.project_path);
        fs::copy(&source, &target).map_err(io_error(&source))?;
    }
    let write = |name: &str, text: &str| fs::write(dir.join(name), text).map_err(io_error(dir));
    write("inputs.sha256", &inputs_sha256(inputs))?;
    write("params.json", &params_json(stage))?;
    let defaults = slurm_defaults(root)
        .map_err(|m| ExternalError::Io(format!("{SLURM_DEFAULTS_FILE}: {m}")))?;
    let sbatch = sbatch_options(&defaults, stage, &external.slurm);
    write("job.sh", &job_script(resolved, stage, inputs, &sbatch)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.join("job.sh"), fs::Permissions::from_mode(0o755))
            .map_err(io_error(dir))?;
    }
    let stage_json = json!({
        "study": resolved.config.study,
        "stage": stage.name,
        "variant": resolved.config.variant,
        "key": resolved.key(&stage.name),
        "keep": external.keep,
    });
    write(
        "stage.json",
        &(serde_json::to_string_pretty(&stage_json).expect("serializes") + "\n"),
    )?;
    write(
        "README.md",
        &readme(&steps_of(resolved, stage, folder, request.trial)),
    )?;
    Ok(())
}

// rq-3de08c05 rq-340771e5
/// Writes the job folder of an external stage, unless it exists, and returns its path. `warn`
/// receives warnings (large inputs).
pub fn write_job_folder_with(
    root: &Path,
    request: &SealRequest,
    warn: &mut dyn FnMut(&str),
) -> Result<PathBuf, ExternalError> {
    let resolved = load_external(root, request)?;
    let stage = resolved.stage(&request.stage)?;
    let folder = folder_of(&resolved, &stage.name, request.trial)?;
    // Never overwritten: the human may have edited its #SBATCH lines.
    if folder.exists() {
        return Ok(folder);
    }
    let inputs = input_files(&resolved, stage, request.trial)?;
    if let Some(w) = large_inputs_warning(inputs.iter().map(|f| f.size).sum()) {
        warn(&w);
    }
    let parent = folder.parent().expect("inside the output directory");
    fs::create_dir_all(parent).map_err(io_error(parent))?;
    let name = folder.file_name().expect("named").to_string_lossy();
    let tmp = parent.join(format!(".{name}.tophat-tmp-{}", std::process::id()));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).map_err(io_error(&tmp))?;
    }
    let written = populate(root, &resolved, stage, request, &inputs, &tmp, &folder)
        .and_then(|()| fs::rename(&tmp, &folder).map_err(io_error(&folder)));
    match written {
        Ok(()) => Ok(folder),
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            // Another process may have written the folder first.
            if folder.is_dir() {
                Ok(folder)
            } else {
                Err(e)
            }
        }
    }
}

// rq-3de08c05
/// Writes the job folder of an external stage, unless it exists, printing warnings to standard
/// error. Returns its path.
pub fn write_job_folder(root: &Path, request: &SealRequest) -> Result<PathBuf, ExternalError> {
    write_job_folder_with(root, request, &mut |w| eprintln!("warning: {w}"))
}

/// Regular files below `dir`, recursively, as sorted `/`-separated relative paths. Other entries
/// (symbolic links, devices) are reported in `other`.
fn walk(
    dir: &Path,
    skip_top: &[&str],
    other: &mut Vec<String>,
) -> std::io::Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![(String::new(), dir.to_path_buf())];
    while let Some((prefix, current)) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if prefix.is_empty() && skip_top.contains(&name.as_str()) {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let t = entry.file_type()?;
            if t.is_dir() {
                pending.push((rel, entry.path()));
            } else if t.is_file() {
                out.insert(rel, entry.path());
            } else {
                other.push(rel);
            }
        }
    }
    Ok(out)
}

/// Whether `path` holds exactly `expected` (size, then hash).
fn holds(path: &Path, expected: &OutputFile) -> Result<bool, ExternalError> {
    let meta = fs::metadata(path).map_err(io_error(path))?;
    if meta.len() != expected.size {
        return Ok(false);
    }
    let (_, sha256) = crate::data::hash_file(path).map_err(io_error(path))?;
    Ok(sha256 == expected.sha256)
}

/// Moves a file, copying when a rename is impossible.
fn move_file(from: &Path, to: &Path) -> Result<(), ExternalError> {
    fs::create_dir_all(to.parent().expect("inside the output directory")).map_err(io_error(to))?;
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).map_err(io_error(from))?;
    fs::remove_file(from).map_err(io_error(from))
}

// rq-0ab3c68d rq-a1948f0a
/// Verifies a returned job folder against its receipt, moves the returned outputs into the output
/// directory and seals the stage. Returns the record's path.
pub fn ingest(root: &Path, request: &SealRequest) -> Result<PathBuf, ExternalError> {
    let resolved = load_external(root, request)?;
    let stage = resolved.stage(&request.stage)?;
    let external = stage.external.as_ref().expect("checked");
    let study = &resolved.config.study;
    let key = resolved.key(&stage.name).to_string();
    let out_dir = resolved.out_dir(&stage.name, request.trial)?;
    let folder = folder_of(&resolved, &stage.name, request.trial)?;
    let folder_name = JOB_DIR.to_string();

    let receipt_path = folder.join(RECEIPT_FILE);
    let receipt_text = fs::read_to_string(&receipt_path).map_err(io_error(&receipt_path))?;
    let receipt = parse_receipt(&receipt_text).map_err(ExternalError::ReceiptInvalid)?;
    if receipt.study != *study || receipt.stage != stage.name {
        return Err(ExternalError::ReceiptInvalid(format!(
            "it is for stage `{}` of study `{}`, not `{}` of `{study}`",
            receipt.stage, receipt.study, stage.name
        )));
    }
    if receipt.key != key {
        return Err(ExternalError::KeyMismatch {
            expected: key,
            actual: receipt.key,
        });
    }
    let inputs = input_files(&resolved, stage, request.trial)?;
    if script_sha256(&job_script(&resolved, stage, &inputs, &[])?) != receipt.script {
        return Err(ExternalError::ScriptModified);
    }
    if receipt.exit != "0" {
        return Err(ExternalError::JobFailed {
            exit: receipt.exit.clone(),
        });
    }

    let mut problems = Vec::new();
    for o in &receipt.outputs {
        let file_name = o.path.rsplit('/').next().unwrap_or(&o.path);
        if is_kept(&external.keep, &o.path) {
            problems.push(format!(
                "{}: listed as an output but matches `keep`",
                o.path
            ));
        } else if file_name == RECORD_FILE || o.path.split('/').next() == Some(&folder_name) {
            problems.push(format!("{}: an output cannot have this name", o.path));
        }
    }
    for k in &receipt.kept {
        if !is_kept(&external.keep, &k.path) {
            problems.push(format!(
                "{}: listed as kept but matches no `keep` pattern",
                k.path
            ));
        }
    }
    let job_out = folder.join("out");
    let mut other = Vec::new();
    let present = walk(&job_out, &[], &mut other).map_err(io_error(&job_out))?;
    let outputs: BTreeMap<&str, &OutputFile> = receipt
        .outputs
        .iter()
        .map(|o| (o.path.as_str(), o))
        .collect();
    let kept: BTreeMap<&str, &OutputFile> =
        receipt.kept.iter().map(|k| (k.path.as_str(), k)).collect();
    // Outputs are in out/, or already moved by an earlier, interrupted ingest.
    let mut to_move = Vec::new();
    for o in &receipt.outputs {
        let returned = job_out.join(&o.path);
        let moved = out_dir.join(&o.path);
        if present.contains_key(&o.path) {
            if holds(&returned, o)? {
                to_move.push((returned, moved));
            } else {
                problems.push(format!("{}: differs from its `output` line", o.path));
            }
        } else if !(moved.is_file() && holds(&moved, o)?) {
            problems.push(format!("{}: missing from out/", o.path));
        }
    }
    for (rel, path) in &present {
        if outputs.contains_key(rel.as_str()) {
            continue;
        }
        match kept.get(rel.as_str()) {
            Some(k) if holds(path, k)? => {}
            Some(_) => problems.push(format!("{rel}: differs from its `kept` line")),
            None => problems.push(format!("{rel}: not listed in the receipt")),
        }
    }
    problems.extend(
        other
            .into_iter()
            .map(|rel| format!("{rel}: not a regular file")),
    );
    // Nothing else may be in the output directory, or it would be taken for an output.
    let mut stray = Vec::new();
    let in_out_dir =
        walk(&out_dir, &[&folder_name, RECORD_FILE], &mut stray).map_err(io_error(&out_dir))?;
    for rel in in_out_dir.keys().chain(stray.iter()) {
        if !outputs.contains_key(rel.as_str()) && !rel.starts_with('.') {
            problems.push(format!(
                "{rel}: in the output directory {} but not listed in the receipt",
                out_dir.display()
            ));
        }
    }
    if !problems.is_empty() {
        return Err(ExternalError::OutputMismatch(problems));
    }
    if receipt.outputs.is_empty() {
        return Err(PipelineError::NoOutputs(format!(
            "{} (every file matched `keep`)",
            job_out.display()
        ))
        .into());
    }
    if let Some(result) = &stage.result {
        let path = if present.contains_key(result) {
            job_out.join(result)
        } else {
            out_dir.join(result)
        };
        let text = fs::read_to_string(path).unwrap_or_default();
        if text.lines().filter(|l| !l.trim().is_empty()).count() != 1 {
            return Err(PipelineError::BadResult(format!(
                "`{result}` must contain exactly one non-empty line"
            ))
            .into());
        }
    }

    for (from, to) in &to_move {
        move_file(from, to)?;
    }
    let record_external = json!({
        "host": receipt.host,
        "job": receipt.job,
        "start": receipt.start,
        "end": receipt.end,
        "modules": receipt.modules,
        "setup": external.setup,
        "script_sha256": receipt.script,
        "receipt_sha256": sha256_hex(&receipt_text),
        "kept": receipt.kept.iter().map(|k| json!({"path": k.path, "size": k.size, "sha256": k.sha256})).collect::<Vec<_>>(),
    });
    let content = RecordContent {
        run: job_run(stage)?,
        validate: None,
        outputs: receipt
            .outputs
            .iter()
            .map(|o| (o.clone(), out_dir.join(&o.path)))
            .collect(),
        items: None,
        external: Some(record_external),
    };
    Ok(write_record(
        root, &resolved, stage, request, &out_dir, &content,
    )?)
}

// rq-530610d9
/// `tophat stage external`: ingests the returned job folder when it has a receipt; otherwise
/// writes the job folder if missing and returns [`ExternalError::Waiting`] with the steps.
pub fn stage_external(
    root: &Path,
    request: &SealRequest,
    warn: &mut dyn FnMut(&str),
) -> Result<PathBuf, ExternalError> {
    let resolved = load_external(root, request)?;
    let stage = resolved.stage(&request.stage)?;
    let folder = folder_of(&resolved, &stage.name, request.trial)?;
    if folder.join(RECEIPT_FILE).is_file() {
        return ingest(root, request);
    }
    let folder = write_job_folder_with(root, request, warn)?;
    Err(ExternalError::Waiting(steps_of(
        &resolved,
        stage,
        &folder,
        request.trial,
    )))
}

// rq-9eddd05a
/// `tophat stage instructions`: the steps for the stage's current key. Read-only.
pub fn instructions(
    root: &Path,
    study: &str,
    stage: &str,
    variant: &str,
) -> Result<String, ExternalError> {
    let trial = super::summary::read_study(root, study)?.status() == Some(Status::Draft);
    let resolved = ResolvedStage::load_mode(root, study, variant, Some(stage), trial)?;
    let request = SealRequest {
        study: study.to_string(),
        stage: stage.to_string(),
        key: resolved.key(stage).to_string(),
        variant: Some(variant.to_string()),
        item: None,
        trial,
    };
    let resolved = load_external(root, &request)?;
    let config = resolved.stage(stage)?;
    let folder = folder_of(&resolved, stage, trial)?;
    let mut text = steps_of(&resolved, config, &folder, trial);
    if !folder.exists() {
        text.push_str(&format!(
            "\nThe job folder has not been written yet: `tophat run {study}{}` writes it when the \
             pipeline reaches this stage.\n",
            if trial { " --draft" } else { "" }
        ));
    } else if folder.join(RECEIPT_FILE).is_file() {
        text.push_str(&format!(
            "\nThe job folder has returned with its {RECEIPT_FILE}: continue with step 4.\n"
        ));
    }
    Ok(text)
}

// rq-9f1e97a5
/// The external stages of a variant whose job folder is written for the current key and has not
/// been returned (no receipt), and that are not sealed.
pub fn waiting_jobs(root: &Path, study: &str, variant: &str) -> Vec<String> {
    let Ok(resolved) = ResolvedStage::load(root, study, variant) else {
        return Vec::new();
    };
    resolved
        .config
        .stages
        .iter()
        .filter(|s| s.external.is_some())
        .filter(|s| {
            let (Ok(dir), Ok(folder)) = (
                resolved.out_dir(&s.name, false),
                folder_of(&resolved, &s.name, false),
            ) else {
                return false;
            };
            let sealed =
                ProvenanceRecord::read(&dir).is_some_and(|r| r.key == resolved.key(&s.name));
            !sealed && folder.is_dir() && !folder.join(RECEIPT_FILE).exists()
        })
        .map(|s| s.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    fn receipt(extra: &str) -> String {
        format!(
            "tophat-receipt 1\nstudy x\nstage md\nkey sk-3f9a1c2b4d5e6f70\nscript {HASH}\nhost n1\n\
             job -\nstart 2026-10-21T08:00:03Z\nend 2026-10-21T19:42:51Z\nexit 0\n{extra}"
        )
    }

    #[test] // rq-d39d6009 rq-52dc4a39
    fn receipts_parse() {
        let r = parse_receipt(&receipt(&format!(
            "module gromacs/2024.2\noutput {HASH}  48211  md.edr\nkept {HASH}  98234221312  sub dir/md.trr\n"
        )))
        .unwrap();
        assert_eq!(r.key, "sk-3f9a1c2b4d5e6f70");
        assert_eq!(r.modules, vec!["gromacs/2024.2"]);
        assert_eq!(r.outputs[0].size, 48211);
        assert_eq!(r.kept[0].path, "sub dir/md.trr");
        let failed = parse_receipt(&receipt("").replace("exit 0", "exit input-mismatch")).unwrap();
        assert_eq!(failed.exit, "input-mismatch");
    }

    #[test] // rq-82d88391 rq-d39d6009
    fn receipts_are_parsed_strictly() {
        let e = parse_receipt(&receipt("output 9f86d0  48211  md.edr\n")).unwrap_err();
        assert!(e.contains("line 11") && e.contains("malformed hash"), "{e}");
        for bad in [
            receipt("").replace("tophat-receipt 1", "tophat-receipt 2"),
            receipt("").replace("study x\n", ""),
            receipt("host n2\n"),
            receipt("color blue\n"),
            receipt(&format!("output {HASH}  1  ../x\n")),
            receipt(&format!("output {HASH}  1  a\nkept {HASH}  1  a\n")),
            receipt("").replace("exit 0", "exit failed"),
            receipt("\n"),
        ] {
            assert!(parse_receipt(&bad).is_err(), "{bad}");
        }
    }

    #[test] // rq-fd86ea5e
    fn keep_patterns_do_not_cross_directories() {
        let keep = vec!["*.trr".to_string(), "traj/[a-c]?.xtc".to_string()];
        assert!(is_kept(&keep, "md.trr"));
        assert!(!is_kept(&keep, "sub/md.trr"));
        assert!(is_kept(&keep, "traj/b1.xtc"));
        assert!(!is_kept(&keep, "traj/d1.xtc"));
        assert!(!is_kept(&keep, "md.edr"));
        assert!(is_kept(&["[!m]*".to_string()], "x.log"));
        assert!(!is_kept(&["[!m]*".to_string()], "md.log"));
    }

    #[test] // rq-964a4533
    fn stage_options_override_defaults_by_name() {
        let stage = StageConfig {
            name: "md".into(),
            run: String::new(),
            code: vec![],
            inputs: Default::default(),
            params: Default::default(),
            env: "default".into(),
            env_set: false,
            tool: None,
            threads: 8,
            foreach: None,
            external: None,
            validate: None,
            review: None,
            trial: None,
            result: None,
            file: String::new(),
            line: None,
        };
        let defaults = vec!["--account=e123".to_string(), "--time=01:00:00".to_string()];
        let options = sbatch_options(&defaults, &stage, &["--time=24:00:00".to_string()]);
        assert_eq!(
            options,
            vec![
                "--account=e123",
                "--time=24:00:00",
                "--cpus-per-task=8",
                "--job-name=md"
            ]
        );
        let options = sbatch_options(&[], &stage, &["--cpus-per-task=4".to_string()]);
        assert_eq!(options, vec!["--cpus-per-task=4", "--job-name=md"]);
    }

    #[test] // rq-3f18bffc rq-340771e5
    fn large_inputs_warn_with_their_size() {
        assert_eq!(large_inputs_warning(LARGE_INPUTS_BYTES), None);
        let w = large_inputs_warning(12_345_000_000).unwrap();
        assert!(w.contains("12.3 GB") && w.contains("12345000000"), "{w}");
    }

    #[test] // rq-13602cac
    fn script_hash_ignores_sbatch_lines() {
        let a = "#!/bin/sh\n#SBATCH --time=1\necho hi\n";
        let b = "#!/bin/sh\n#SBATCH --time=2\n#SBATCH --qos=x\necho hi";
        assert_eq!(script_sha256(a), script_sha256(b));
        assert_ne!(script_sha256(a), script_sha256("#!/bin/sh\necho ho\n"));
    }
}
