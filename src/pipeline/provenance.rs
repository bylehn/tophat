//! Provenance records and `tophat stage seal`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use super::config::{load_pipeline, PipelineConfig, StageConfig, StageInput};
use super::keys::{
    compute_keys, input_dir, items_of, output_dir, params_file, render_run, results_root,
    KeyContext, RunPaths, StageKeys,
};
use super::PipelineError;
use crate::data::{content_hash, hash_files, load_roots, HashCache, Roots, RESULTS_ROOT};
use crate::fsutil::write_atomic;
use crate::study::STUDIES_DIR;

/// File name of a provenance record.
pub const RECORD_FILE: &str = "provenance.json";

/// One output file of a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// One item listed in the stage-level record of a fan-out stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSummary {
    pub name: String,
    pub key: String,
    pub outputs_hash: String,
}

// rq-9f8a5a5e rq-533606d2
/// A parsed `provenance.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvenanceRecord {
    pub study: String,
    pub stage: String,
    pub key: String,
    pub item: Option<String>,
    pub run: String,
    pub inputs: Vec<Value>,
    pub outputs: Vec<OutputFile>,
    pub items: Option<Vec<ItemSummary>>,
    pub clean: bool,
    pub unclean_files: Vec<String>,
    pub commit: Option<String>,
    pub trial: bool,
    /// The whole record as written.
    pub json: Value,
}

impl ProvenanceRecord {
    /// Parses a record; `None` if it is not a valid record.
    pub fn parse(text: &str) -> Option<ProvenanceRecord> {
        let json: Value = serde_json::from_str(text).ok()?;
        let s = |k: &str| json.get(k).and_then(Value::as_str).map(str::to_string);
        let outputs = json
            .get("outputs")?
            .as_array()?
            .iter()
            .map(|o| {
                Some(OutputFile {
                    path: o.get("path")?.as_str()?.to_string(),
                    size: o.get("size")?.as_u64()?,
                    sha256: o.get("sha256")?.as_str()?.to_string(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let items = match json.get("items") {
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .map(|i| {
                        Some(ItemSummary {
                            name: i.get("name")?.as_str()?.to_string(),
                            key: i.get("key")?.as_str()?.to_string(),
                            outputs_hash: i.get("outputs_hash")?.as_str()?.to_string(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            ),
            _ => None,
        };
        Some(ProvenanceRecord {
            study: s("study")?,
            stage: s("stage")?,
            key: s("key")?,
            item: s("item"),
            run: s("run").unwrap_or_default(),
            inputs: json
                .get("inputs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            outputs,
            items,
            clean: json.get("clean")?.as_bool()?,
            unclean_files: json
                .get("unclean_files")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            commit: s("commit"),
            trial: json.get("trial").and_then(Value::as_bool).unwrap_or(false),
            json,
        })
    }

    /// Reads the record in a directory, if present and valid.
    pub fn read(dir: &Path) -> Option<ProvenanceRecord> {
        Self::parse(&fs::read_to_string(dir.join(RECORD_FILE)).ok()?)
    }

    /// The `content_hash` of the record's outputs.
    pub fn outputs_hash(&self) -> String {
        let pairs: Vec<(String, String)> = self
            .outputs
            .iter()
            .map(|o| (o.path.clone(), o.sha256.clone()))
            .collect();
        content_hash(&pairs)
    }
}

/// Lists regular files below `dir`, recursively, as sorted relative paths, skipping
/// `provenance.json` at any level and, when `skip_items` is set, the top-level `items/` subtree.
fn list_outputs(dir: &Path, skip_items: bool) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut pending = vec![(String::new(), dir.to_path_buf())];
    while let Some((prefix, current)) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                if !(skip_items && prefix.is_empty() && name == "items") {
                    pending.push((rel, entry.path()));
                }
            } else if file_type.is_file() && name != RECORD_FILE {
                out.push((rel, entry.path()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The result of the cleanliness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleanliness {
    pub clean: bool,
    pub unclean_files: Vec<String>,
    pub commit: Option<String>,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

// rq-9f3a6366
/// Whether the given project-relative files are committed unchanged at `HEAD`.
pub fn cleanliness(root: &Path, files: &[String]) -> Cleanliness {
    let no_git = Cleanliness {
        clean: false,
        unclean_files: vec!["<no git>".into()],
        commit: None,
    };
    let Some(commit) = git(root, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string()) else {
        return no_git;
    };
    let Some(prefix) = git(root, &["rev-parse", "--show-prefix"]).map(|s| s.trim().to_string())
    else {
        return no_git;
    };
    let mut args = vec!["ls-files", "-z", "--"];
    args.extend(files.iter().map(String::as_str));
    let Some(tracked) = git(root, &args) else {
        return no_git;
    };
    let tracked: std::collections::BTreeSet<String> = tracked
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
    ];
    args.extend(files.iter().map(String::as_str));
    let Some(status) = git(root, &args) else {
        return no_git;
    };
    let changed: std::collections::BTreeSet<String> = status
        .split('\0')
        .filter(|s| s.len() > 3)
        .map(|s| {
            s[3..]
                .strip_prefix(prefix.as_str())
                .unwrap_or(&s[3..])
                .to_string()
        })
        .collect();
    let mut unclean: Vec<String> = files
        .iter()
        .filter(|f| !tracked.contains(f.as_str()) || changed.contains(f.as_str()))
        .cloned()
        .collect();
    unclean.sort();
    unclean.dedup();
    Cleanliness {
        clean: unclean.is_empty(),
        unclean_files: unclean,
        commit: Some(commit),
    }
}

/// The files whose state determines whether a stage's record is clean.
pub(crate) fn determining_files(
    ctx: &KeyContext,
    config: &PipelineConfig,
    stage: &StageConfig,
) -> Vec<String> {
    let mut files: Vec<String> = ctx
        .code_files(&config.study, stage)
        .map(|c| c.into_iter().map(|f| f.project_path).collect())
        .unwrap_or_default();
    files.push(format!("{STUDIES_DIR}/{}/config/base.yaml", config.study));
    if config.variant != "base" {
        files.push(format!(
            "{STUDIES_DIR}/{}/config/{}.yaml",
            config.study, config.variant
        ));
    }
    // An external stage's software comes from the cluster, not from pixi.
    for f in ["pixi.toml", "pixi.lock"] {
        if stage.external.is_none() && ctx.root().join(f).is_file() {
            files.push(f.to_string());
        }
    }
    // A tool stage also depends on the tool declarations, the lock, and a project-supplied
    // environment. The tool's own code is fixed by its commit.
    if let Ok(Some(tool)) = ctx.tool_info(stage) {
        for f in [crate::tools::TOOLS_FILE, crate::tools::LOCK_FILE] {
            if ctx.root().join(f).is_file() {
                files.push(f.to_string());
            }
        }
        // Only a project-supplied environment counts; the tool's own is fixed by its commit.
        let supplied = ctx.root().join("envs").join(&tool.pin.name);
        for env_file in [&tool.manifest, &tool.lock] {
            if let Ok(rel) = env_file.strip_prefix(ctx.root()) {
                if env_file.starts_with(&supplied) {
                    files.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    files
}

/// The pixi platform name of this machine.
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "win",
        other => other,
    };
    let arch = match (os, std::env::consts::ARCH) {
        (_, "x86_64") => "64",
        ("osx", "aarch64") => "arm64",
        (_, "aarch64") => "aarch64",
        (_, other) => other,
    };
    format!("{os}-{arch}")
}

pub(crate) fn host() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Everything about a stage that sealing and rule generation need.
pub(crate) struct ResolvedStage {
    pub config: PipelineConfig,
    pub keys: StageKeys,
    pub roots: Roots,
    pub ctx: KeyContext,
}

impl ResolvedStage {
    pub fn load(root: &Path, study: &str, variant: &str) -> Result<ResolvedStage, PipelineError> {
        Self::load_until(root, study, variant, None)
    }

    /// Like [`ResolvedStage::load`], computing keys only up to and including `last`.
    pub fn load_until(
        root: &Path,
        study: &str,
        variant: &str,
        last: Option<&str>,
    ) -> Result<ResolvedStage, PipelineError> {
        Self::load_mode(root, study, variant, last, false)
    }

    // rq-52742c3b
    /// Like [`ResolvedStage::load_until`]; for a trial run (`trial`), each stage's `trial` inputs
    /// and parameters are merged in before the keys are computed.
    pub fn load_mode(
        root: &Path,
        study: &str,
        variant: &str,
        last: Option<&str>,
        trial: bool,
    ) -> Result<ResolvedStage, PipelineError> {
        let config = load_pipeline(root, study, variant)?;
        let config = if trial { config.for_trial() } else { config };
        let ctx = KeyContext::new(root);
        let keys = super::keys::compute_keys_until(&config, &ctx, last)?;
        let roots = load_roots(root).map_err(|e| PipelineError::Io(e.to_string()))?;
        Ok(ResolvedStage {
            config,
            keys,
            roots,
            ctx,
        })
    }

    pub fn stage(&self, name: &str) -> Result<&StageConfig, PipelineError> {
        self.config
            .stage(name)
            .ok_or_else(|| PipelineError::UnknownStage(name.to_string()))
    }

    pub fn key(&self, name: &str) -> &str {
        &self.keys.keys[name]
    }

    pub fn out_dir(&self, stage: &str, trial: bool) -> Result<PathBuf, PipelineError> {
        Ok(output_dir(
            results_root(&self.roots)?,
            &self.config.study,
            stage,
            self.key(stage),
            trial,
        ))
    }

    /// The items of a fan-out stage: `(name, file)`.
    pub fn items(
        &self,
        stage: &StageConfig,
        trial: bool,
    ) -> Result<Vec<(String, PathBuf)>, PipelineError> {
        let foreach = stage.foreach.as_ref().expect("fan-out stage");
        let input = &stage.inputs[foreach];
        let dir = input_dir(
            &self.config,
            &self.keys,
            &self.roots,
            &self.ctx,
            input,
            trial,
        )
        .ok_or_else(|| PipelineError::Io(format!("input `{foreach}` is unavailable")))?;
        let items =
            items_of(&dir).map_err(|e| PipelineError::Io(format!("{}: {e}", dir.display())))?;
        let mut bad: Vec<String> = items
            .iter()
            .enumerate()
            .filter(|(i, (name, _))| {
                !is_valid_item_name(name) || items[..*i].iter().any(|(other, _)| other == name)
            })
            .map(|(_, (name, _))| name.clone())
            .collect();
        bad.dedup();
        if bad.is_empty() {
            Ok(items)
        } else {
            Err(PipelineError::BadItems(bad))
        }
    }

    /// The paths a stage's command is substituted with.
    pub fn run_paths(
        &self,
        stage: &StageConfig,
        item: Option<&str>,
        trial: bool,
    ) -> Result<RunPaths, PipelineError> {
        let stage_dir = self.out_dir(&stage.name, trial)?;
        let inputs = stage
            .inputs
            .iter()
            .filter_map(|(name, input)| {
                input_dir(
                    &self.config,
                    &self.keys,
                    &self.roots,
                    &self.ctx,
                    input,
                    trial,
                )
                .map(|d| (name.clone(), d))
            })
            .collect::<BTreeMap<_, _>>();
        let item = match item {
            None => None,
            Some(name) => Some(
                self.items(stage, trial)?
                    .into_iter()
                    .find(|(n, _)| n == name)
                    .ok_or_else(|| {
                        PipelineError::UnknownStage(format!("{}[{name}]", stage.name))
                    })?,
            ),
        };
        let out = match &item {
            Some((name, _)) => stage_dir.join("items").join(name),
            None => stage_dir.clone(),
        };
        let tool = self
            .ctx
            .tool_info(stage)
            .map_err(PipelineError::Io)?
            .map(|t| t.checkout);
        Ok(RunPaths {
            out,
            inputs,
            item,
            params_file: params_file(&stage_dir),
            tool,
        })
    }
}

/// Whether `name` is a valid item name.
pub fn is_valid_item_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// External stages are sealed only by ingesting their returned job folder.
fn refuse_external(stage: &StageConfig) -> Result<(), PipelineError> {
    match stage.external {
        None => Ok(()),
        Some(_) => Err(PipelineError::Config {
            path: stage.file.clone(),
            line: stage.line,
            message: format!(
                "stage `{}` is an external stage: it runs on the cluster and is sealed by \
                 `tophat stage external` when its job folder returns",
                stage.name
            ),
        }),
    }
}

/// What to seal.
#[derive(Debug, Clone, Default)]
pub struct SealRequest {
    pub study: String,
    pub stage: String,
    pub key: String,
    pub variant: Option<String>,
    pub item: Option<String>,
    pub trial: bool,
}

// rq-719acfdc
/// Seals a stage output (or one fan-out item): verifies the key, hashes the outputs, determines
/// cleanliness and writes `provenance.json`. Returns the record's path.
pub fn seal(root: &Path, request: &SealRequest) -> Result<PathBuf, PipelineError> {
    let variant = request.variant.as_deref().unwrap_or("base");
    let resolved = ResolvedStage::load_mode(
        root,
        &request.study,
        variant,
        Some(&request.stage),
        request.trial,
    )?;
    let stage = resolved.stage(&request.stage)?;
    let actual = resolved.key(&stage.name).to_string();
    if actual != request.key {
        return Err(PipelineError::KeyMismatch {
            expected: request.key.clone(),
            actual,
        });
    }
    refuse_external(stage)?;
    if let Some(item) = &request.item {
        if stage.foreach.is_none() {
            return Err(PipelineError::Config {
                path: stage.file.clone(),
                line: stage.line,
                message: format!("stage `{}` is not a fan-out stage", stage.name),
            });
        }
        if !is_valid_item_name(item) {
            return Err(PipelineError::NoOutputs(format!(
                "invalid item name `{item}`"
            )));
        }
    }
    let stage_dir = resolved.out_dir(&stage.name, request.trial)?;
    let dir = match &request.item {
        Some(item) => stage_dir.join("items").join(item),
        None => stage_dir.clone(),
    };
    let fan_out_stage_record = stage.foreach.is_some() && request.item.is_none();

    let items = if fan_out_stage_record {
        let expected = resolved.items(stage, request.trial)?;
        let mut missing = Vec::new();
        let mut summaries = Vec::new();
        for (name, _) in expected {
            match ProvenanceRecord::read(&stage_dir.join("items").join(&name)) {
                Some(record) if record.key == actual => summaries.push(ItemSummary {
                    outputs_hash: record.outputs_hash(),
                    name,
                    key: actual.clone(),
                }),
                _ => missing.push(name),
            }
        }
        if !missing.is_empty() {
            return Err(PipelineError::ItemsUnsealed(missing));
        }
        Some(summaries)
    } else {
        None
    };

    let files = if dir.is_dir() {
        list_outputs(&dir, fan_out_stage_record).map_err(|e| PipelineError::Io(e.to_string()))?
    } else {
        Vec::new()
    };
    if files.is_empty() && !fan_out_stage_record {
        return Err(PipelineError::NoOutputs(dir.display().to_string()));
    }
    if let (Some(result), None) = (&stage.result, &request.item) {
        let text = fs::read_to_string(dir.join(result)).unwrap_or_default();
        if text.lines().filter(|l| !l.trim().is_empty()).count() != 1 {
            return Err(PipelineError::BadResult(format!(
                "`{result}` must contain exactly one non-empty line"
            )));
        }
    }

    let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
    let mut outputs = Vec::with_capacity(files.len());
    for ((rel, path), hashed) in files.iter().zip(hash_files(&paths)) {
        let (size, sha256) = hashed.map_err(|e| PipelineError::Io(e.to_string()))?;
        outputs.push((
            OutputFile {
                path: rel.clone(),
                size,
                sha256,
            },
            path.clone(),
        ));
    }

    let run_paths = resolved.run_paths(stage, request.item.as_deref(), request.trial)?;
    let run = render_run(stage, &run_paths).map_err(PipelineError::Io)?;
    let validate = stage
        .validate
        .as_ref()
        .map(|v| super::keys::render_template(v, stage, &run_paths))
        .transpose()
        .map_err(PipelineError::Io)?;
    write_record(
        root,
        &resolved,
        stage,
        request,
        &dir,
        &RecordContent {
            run,
            validate,
            outputs,
            items,
            external: None,
        },
    )
}

/// What a record holds beyond what the stage's configuration determines.
pub(crate) struct RecordContent {
    /// The command as executed.
    pub run: String,
    /// The `validate` command that checked the output, as executed.
    pub validate: Option<String>,
    /// The outputs, with the file each was hashed from.
    pub outputs: Vec<(OutputFile, PathBuf)>,
    pub items: Option<Vec<ItemSummary>>,
    /// The `external` object of an external stage's record.
    pub external: Option<Value>,
}

// rq-533606d2
/// Determines cleanliness and writes the `provenance.json` of a stage (or item) in `dir`.
pub(crate) fn write_record(
    root: &Path,
    resolved: &ResolvedStage,
    stage: &StageConfig,
    request: &SealRequest,
    dir: &Path,
    content: &RecordContent,
) -> Result<PathBuf, PipelineError> {
    let actual = resolved.key(&stage.name).to_string();
    let cleanliness = cleanliness(
        root,
        &determining_files(&resolved.ctx, &resolved.config, stage),
    );
    let code = resolved
        .ctx
        .code_files(&resolved.config.study, stage)
        .map_err(PipelineError::Io)?;
    let inputs: Vec<Value> = stage
        .inputs
        .iter()
        .map(|(name, input)| match input {
            StageInput::Stage(upstream) => json!({
                "name": name, "kind": "stage", "stage": upstream, "key": resolved.key(upstream),
            }),
            StageInput::Dataset(r) => json!({
                "name": name, "kind": "dataset", "ref": r.to_string(),
                "content_hash": resolved.ctx.manifest(r).map(|m| m.content_hash.clone()),
            }),
        })
        .collect();
    // An external stage ran with the cluster's software, so it has no pixi environment.
    let env = match &stage.external {
        Some(_) => Value::Null,
        None => {
            let hash = resolved
                .ctx
                .environment_hash(&stage.env)
                .map_err(PipelineError::Io)?;
            json!({"name": stage.env, "hash": hash})
        }
    };
    let record = json!({
        "format_version": 1,
        "tophat_version": env!("CARGO_PKG_VERSION"),
        "study": resolved.config.study,
        "stage": stage.name,
        "key": actual,
        "item": request.item,
        "run": content.run,
        "validate": content.validate,
        "env": env,
        "code": code.iter().map(|c| json!({"path": c.path, "sha256": c.sha256})).collect::<Vec<_>>(),
        "params": Value::Object(stage.params.clone()),
        "inputs": inputs,
        "outputs": content.outputs.iter().map(|(o, _)| json!({"path": o.path, "size": o.size, "sha256": o.sha256})).collect::<Vec<_>>(),
        "items": content.items.as_ref().map(|items| items.iter().map(|i| json!({
            "name": i.name, "key": i.key, "outputs_hash": i.outputs_hash,
        })).collect::<Vec<_>>()),
        "clean": cleanliness.clean,
        "unclean_files": cleanliness.unclean_files,
        "commit": cleanliness.commit,
        "host": host(),
        "platform": platform(),
        "tool": resolved.ctx.tool_info(stage).ok().flatten().map(|t| {
            let url = crate::tools::declared_tools(root)
                .ok()
                .and_then(|d| d.get(&t.pin.name).cloned());
            json!({"name": t.pin.name, "commit": t.commit, "url": url})
        }),
        "external": content.external,
        "trial": request.trial,
        "sealed": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });

    fs::create_dir_all(dir).map_err(|e| PipelineError::Io(e.to_string()))?;
    let record_path = dir.join(RECORD_FILE);
    let text = serde_json::to_string_pretty(&record).expect("JSON values always serialize") + "\n";
    write_atomic(&record_path, &text).map_err(|e| PipelineError::Io(e.to_string()))?;

    let results = results_root(&resolved.roots)?;
    let mut cache = HashCache::load(root);
    for (output, path) in &content.outputs {
        if let Ok(rel) = path.strip_prefix(results) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            cache.record(RESULTS_ROOT, &rel, path, output.size, &output.sha256);
        }
    }
    cache.save();
    Ok(record_path)
}

/// What a generated rule needs to run a stage job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BegunJob {
    /// The command with placeholders substituted.
    pub run: String,
    /// The `validate` command with placeholders substituted, run after `run` when set.
    pub validate: Option<String>,
    /// The pixi environment.
    pub env: String,
    /// The directory to run in: the study directory.
    pub cwd: PathBuf,
    /// The environment's `pixi.toml` for a tool stage; `None` for the project environment.
    pub manifest: Option<PathBuf>,
}

/// Prepares a stage job: verifies the key, empties the job's output directory (the item directory
/// for a fan-out item), writes the parameters file, and returns the command to run.
pub fn begin(root: &Path, request: &SealRequest) -> Result<BegunJob, PipelineError> {
    let variant = request.variant.as_deref().unwrap_or("base");
    let resolved = ResolvedStage::load_mode(
        root,
        &request.study,
        variant,
        Some(&request.stage),
        request.trial,
    )?;
    let stage = resolved.stage(&request.stage)?;
    let actual = resolved.key(&stage.name).to_string();
    if actual != request.key {
        return Err(PipelineError::KeyMismatch {
            expected: request.key.clone(),
            actual,
        });
    }
    refuse_external(stage)?;
    if stage.foreach.is_some() != request.item.is_some() {
        return Err(PipelineError::Config {
            path: stage.file.clone(),
            line: stage.line,
            message: format!(
                "stage `{}`: --item is required exactly for fan-out stages",
                stage.name
            ),
        });
    }
    let paths = resolved.run_paths(stage, request.item.as_deref(), request.trial)?;
    let run = render_run(stage, &paths).map_err(PipelineError::Io)?;
    let io = |e: std::io::Error| PipelineError::Io(e.to_string());
    if paths.out.exists() {
        fs::remove_dir_all(&paths.out).map_err(io)?;
    }
    fs::create_dir_all(&paths.out).map_err(io)?;
    let params = serde_json::to_string_pretty(&Value::Object(stage.params.clone()))
        .expect("JSON values always serialize");
    // Parallel item jobs of one stage share this file; leave it alone when unchanged.
    let params = params + "\n";
    if fs::read_to_string(&paths.params_file).ok().as_deref() != Some(params.as_str()) {
        crate::fsutil::write_atomic_unique(&paths.params_file, &params).map_err(io)?;
    }
    let manifest = resolved
        .ctx
        .tool_info(stage)
        .map_err(PipelineError::Io)?
        .map(|t| t.manifest);
    let validate = stage
        .validate
        .as_ref()
        .map(|v| super::keys::render_template(v, stage, &paths))
        .transpose()
        .map_err(PipelineError::Io)?;
    Ok(BegunJob {
        run,
        validate,
        env: stage.env.clone(),
        cwd: root.join(STUDIES_DIR).join(&request.study),
        manifest,
    })
}

/// One stage in `tophat pipeline keys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStatus {
    pub stage: String,
    pub key: String,
    /// The output directory, or `None` when the results root is unconfigured.
    pub output: Option<PathBuf>,
    /// `sealed`, `missing`, or `unavailable`.
    pub status: &'static str,
}

// rq-c8637010
/// The keys, output directories and sealed state of a study's stages for a variant.
pub fn key_status(
    root: &Path,
    study: &str,
    variant: &str,
) -> Result<Vec<KeyStatus>, PipelineError> {
    let config = load_pipeline(root, study, variant)?;
    let ctx = KeyContext::new(root);
    let keys = compute_keys(&config, &ctx)?;
    let roots = load_roots(root).map_err(|e| PipelineError::Io(e.to_string()))?;
    let results = roots.location(RESULTS_ROOT);
    Ok(config
        .stages
        .iter()
        .map(|s| {
            let key = keys.keys[&s.name].clone();
            let output = results.map(|r| output_dir(r, study, &s.name, &key, false));
            let status = match &output {
                None => "unavailable",
                Some(dir)
                    if ProvenanceRecord::read(dir).is_some_and(|r| r.key == key && !r.trial) =>
                {
                    "sealed"
                }
                Some(_) => "missing",
            };
            KeyStatus {
                stage: s.name.clone(),
                key,
                output,
                status,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_names() {
        let p = platform();
        assert!(p.contains('-'), "{p}");
    }

    #[test]
    fn item_names() {
        assert!(is_valid_item_name("1abc"));
        assert!(is_valid_item_name("a.b_c-d"));
        assert!(!is_valid_item_name("a b"));
        assert!(!is_valid_item_name(".."));
    }
}
