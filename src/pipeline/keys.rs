//! Stage keys, output locations and command substitution.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use yaml_rust2::{Yaml, YamlLoader};

use super::config::{parse_template, PipelineConfig, StageConfig, StageInput, TemplatePart};
use super::PipelineError;
use crate::check::Project;
use crate::data::{hash_file, DatasetRef, Manifest, Roots, RESULTS_ROOT};
use crate::study::STUDIES_DIR;

/// Version line at the start of every key text.
pub const KEY_FORMAT: &str = "tophat-stage-key/1";

// rq-e6252079
/// `sk-` followed by the first 16 hex characters of the SHA-256 of `text`.
pub fn stage_key(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("sk-{}", &crate::data::to_hex(&digest)[..16])
}

/// One code file entering a key: path relative to the study directory, and its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeFile {
    pub path: String,
    /// Project-relative path, for cleanliness checks.
    pub project_path: String,
    pub sha256: String,
}

/// Normalizes a path lexically. Returns `None` if it climbs above its starting point.
pub(crate) fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(p) => out.push(p),
            _ => return None,
        }
    }
    Some(out)
}

fn slash(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// What the key derivation needs from outside the configuration.
pub struct KeyContext {
    root: PathBuf,
    manifests: BTreeMap<DatasetRef, Manifest>,
    lock: Result<BTreeMap<String, String>, String>,
}

/// Reads the environment hashes from `pixi.lock`: environment name → `sha256:<hex>` over its sorted
/// package entries (`<platform> <kind> <location>`, one per line) across all platforms.
fn environment_hashes(text: &str) -> Result<BTreeMap<String, String>, String> {
    let doc = YamlLoader::load_from_str(text)
        .map_err(|e| format!("pixi.lock is not valid YAML: {e}"))?
        .into_iter()
        .next()
        .ok_or("pixi.lock is empty")?;
    let envs = doc["environments"]
        .as_hash()
        .ok_or("pixi.lock has no `environments`")?;
    let mut out = BTreeMap::new();
    for (name, env) in envs {
        let Yaml::String(name) = name else { continue };
        let mut entries = Vec::new();
        if let Some(platforms) = env["packages"].as_hash() {
            for (platform, packages) in platforms {
                let Yaml::String(platform) = platform else {
                    continue;
                };
                for package in packages.as_vec().map(Vec::as_slice).unwrap_or_default() {
                    if let Some(hash) = package.as_hash() {
                        for (kind, location) in hash {
                            if let (Yaml::String(kind), Yaml::String(location)) = (kind, location) {
                                entries.push(format!("{platform} {kind} {location}"));
                            }
                        }
                    }
                }
            }
        }
        entries.sort();
        let mut hasher = Sha256::new();
        for e in &entries {
            hasher.update(e.as_bytes());
            hasher.update(b"\n");
        }
        out.insert(
            name.clone(),
            format!("sha256:{}", crate::data::to_hex(&hasher.finalize())),
        );
    }
    Ok(out)
}

impl KeyContext {
    /// Prepares key derivation for the project at `root`: reads dataset manifests and `pixi.lock`.
    pub fn new(root: &Path) -> KeyContext {
        let project = Project::new(root);
        let lock = fs::read_to_string(root.join("pixi.lock"))
            .map_err(|_| "pixi.lock is missing".to_string())
            .and_then(|t| environment_hashes(&t));
        KeyContext {
            root: root.to_path_buf(),
            manifests: crate::data::manifests_by_ref(&project),
            lock,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // rq-3ac5de89
    /// The environment hash of a pixi environment.
    pub fn environment_hash(&self, env: &str) -> Result<String, String> {
        match &self.lock {
            Err(e) => Err(e.clone()),
            Ok(envs) => envs
                .get(env)
                .cloned()
                .ok_or_else(|| format!("pixi.lock has no environment `{env}`")),
        }
    }

    pub fn manifest(&self, r: &DatasetRef) -> Option<&Manifest> {
        self.manifests.get(r)
    }

    /// The code files of a stage, sorted by path. Fails if an entry is missing or outside the
    /// project.
    pub fn code_files(&self, study: &str, stage: &StageConfig) -> Result<Vec<CodeFile>, String> {
        let study_dir = PathBuf::from(STUDIES_DIR).join(study);
        let mut files = Vec::new();
        for entry in &stage.code {
            let project_rel = normalize(&study_dir.join(entry))
                .ok_or_else(|| format!("code `{entry}` lies outside the project"))?;
            let absolute = self.root.join(&project_rel);
            let meta = fs::symlink_metadata(&absolute)
                .map_err(|_| format!("code `{entry}` does not exist"))?;
            let mut found = Vec::new();
            if meta.is_dir() {
                let mut pending = vec![project_rel.clone()];
                while let Some(dir) = pending.pop() {
                    let mut names: Vec<_> = fs::read_dir(self.root.join(&dir))
                        .map_err(|e| e.to_string())?
                        .filter_map(Result::ok)
                        .collect();
                    names.sort_by_key(|e| e.file_name());
                    for e in names {
                        let rel = dir.join(e.file_name());
                        match e.file_type() {
                            Ok(t) if t.is_dir() => pending.push(rel),
                            Ok(t) if t.is_file() => found.push(rel),
                            _ => {}
                        }
                    }
                }
            } else if meta.is_file() {
                found.push(project_rel.clone());
            } else {
                return Err(format!("code `{entry}` is not a file or directory"));
            }
            for project_path in found {
                let (_, sha256) =
                    hash_file(&self.root.join(&project_path)).map_err(|e| e.to_string())?;
                let path = match project_path.strip_prefix(&study_dir) {
                    Ok(inside) => slash(inside),
                    Err(_) => format!("../../{}", slash(&project_path)),
                };
                files.push(CodeFile {
                    path,
                    project_path: slash(&project_path),
                    sha256,
                });
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        Ok(files)
    }
}

/// Key texts and keys of every stage of a pipeline, in stage order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageKeys {
    pub texts: BTreeMap<String, String>,
    pub keys: BTreeMap<String, String>,
}

fn key_error(stage: &StageConfig, message: String) -> PipelineError {
    PipelineError::Config {
        path: stage.file.clone(),
        line: stage.line,
        message: format!("stage `{}`: {message}", stage.name),
    }
}

/// What a tool stage runs with: its pin, resolved commit, pristine checkout and environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    pub pin: crate::tools::ToolPin,
    pub commit: String,
    pub checkout: PathBuf,
    /// The environment's `pixi.toml`.
    pub manifest: PathBuf,
    /// The environment's `pixi.lock`.
    pub lock: PathBuf,
}

impl KeyContext {
    // rq-f9ae0061 rq-a96b0574
    /// The tool a stage runs with, or why it cannot be used (unresolved, not cached, modified, or
    /// without an environment).
    pub fn tool_info(&self, stage: &StageConfig) -> Result<Option<ToolInfo>, String> {
        let Some(raw) = &stage.tool else {
            return Ok(None);
        };
        let pin = crate::tools::ToolPin::parse(raw)
            .ok_or_else(|| crate::tools::ToolsError::InvalidPin(raw.clone()).to_string())?;
        let checkout = crate::tools::tool_checkout(&self.root, &pin).map_err(|e| e.to_string())?;
        let commit = crate::tools::resolve_pin(&self.root, &pin).map_err(|e| e.to_string())?;
        let (manifest, lock) = crate::tools::tool_environment(&self.root, &pin.name, &checkout)
            .ok_or_else(|| {
                format!(
                    "tool `{}` at {commit} has no pixi.toml and pixi.lock, and the project has no envs/{}/",
                    pin.name, pin.name
                )
            })?;
        Ok(Some(ToolInfo {
            pin,
            commit,
            checkout,
            manifest,
            lock,
        }))
    }

    /// The environment hash of a stage: from the tool's environment for a tool stage, otherwise
    /// from the project's `pixi.lock`.
    pub fn stage_environment_hash(&self, stage: &StageConfig) -> Result<String, String> {
        match self.tool_info(stage)? {
            None => self.environment_hash(&stage.env),
            Some(tool) => {
                let text = fs::read_to_string(&tool.lock).map_err(|e| e.to_string())?;
                environment_hashes(&text)?
                    .get(&stage.env)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "the environment of tool `{}` has no environment `{}`",
                            tool.pin.name, stage.env
                        )
                    })
            }
        }
    }
}

// rq-37a6119e
/// Computes the key texts and keys of all stages.
pub fn compute_keys(config: &PipelineConfig, ctx: &KeyContext) -> Result<StageKeys, PipelineError> {
    compute_keys_until(config, ctx, None)
}

/// Computes the keys of the stages up to and including `last` (all stages when `None`). A stage's
/// key depends only on itself and earlier stages, so later stages cannot prevent it.
pub fn compute_keys_until(
    config: &PipelineConfig,
    ctx: &KeyContext,
    last: Option<&str>,
) -> Result<StageKeys, PipelineError> {
    let mut texts = BTreeMap::new();
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    let end = match last {
        None => config.stages.len(),
        Some(name) => {
            config
                .stages
                .iter()
                .position(|s| s.name == name)
                .ok_or_else(|| PipelineError::UnknownStage(name.to_string()))?
                + 1
        }
    };
    for stage in &config.stages[..end] {
        // rq-23d21623
        let env = match &stage.external {
            Some(_) => "external".to_string(),
            None => ctx
                .stage_environment_hash(stage)
                .map_err(|e| key_error(stage, e))?,
        };
        let mut lines = vec![KEY_FORMAT.to_string(), format!("run {}", stage.run)];
        // rq-0a10fc96
        // Only stages with `validate` have a `validate` line, so other stages' keys are unaffected.
        if let Some(validate) = &stage.validate {
            lines.push(format!("validate {validate}"));
        }
        // Only tool stages have a `tool` line, so other stages' keys are unaffected.
        if let Some(tool) = ctx.tool_info(stage).map_err(|e| key_error(stage, e))? {
            lines.push(format!("tool {}@{}", tool.pin.name, tool.commit));
        }
        lines.push(format!("env {env}"));
        // Only external stages have an `external` line, so other stages' keys are unaffected.
        if let Some(external) = &stage.external {
            lines.push(format!("external {}", external.key_json()));
        }
        lines.extend([
            format!("threads {}", stage.threads),
            format!("foreach {}", stage.foreach.as_deref().unwrap_or("-")),
            format!("result {}", stage.result.as_deref().unwrap_or("-")),
        ]);
        for code in ctx
            .code_files(&config.study, stage)
            .map_err(|e| key_error(stage, e))?
        {
            lines.push(format!("code {} {}", code.path, code.sha256));
        }
        lines.push(format!(
            "params {}",
            serde_json::to_string(&stage.params).expect("JSON values always serialize")
        ));
        for (name, input) in &stage.inputs {
            lines.push(match input {
                StageInput::Stage(upstream) => format!(
                    "input {name} stage {}",
                    keys.get(upstream).ok_or_else(|| key_error(
                        stage,
                        format!("input `{name}` references unknown stage `{upstream}`")
                    ))?
                ),
                StageInput::Dataset(r) => format!(
                    "input {name} dataset {r} {}",
                    ctx.manifest(r)
                        .ok_or_else(|| key_error(stage, format!("dataset `{r}` does not exist")))?
                        .content_hash
                ),
            });
        }
        let text = lines.join("\n") + "\n";
        keys.insert(stage.name.clone(), stage_key(&text));
        texts.insert(stage.name.clone(), text);
    }
    Ok(StageKeys { texts, keys })
}

// rq-f40ffa2b rq-4aa0232e
/// The key text of one stage.
pub fn stage_key_text(
    config: &PipelineConfig,
    stage: &str,
    ctx: &KeyContext,
) -> Result<String, PipelineError> {
    compute_keys_until(config, ctx, Some(stage))?
        .texts
        .remove(stage)
        .ok_or_else(|| PipelineError::UnknownStage(stage.to_string()))
}

// rq-a56c1f8f
/// The output directory of a stage under a results root.
pub fn output_dir(results: &Path, study: &str, stage: &str, key: &str, trial: bool) -> PathBuf {
    let base = if trial {
        results.join(study).join(".trial")
    } else {
        results.join(study)
    };
    base.join(stage).join(key)
}

/// The location of the `results` root.
pub fn results_root(roots: &Roots) -> Result<&Path, PipelineError> {
    roots
        .location(RESULTS_ROOT)
        .ok_or_else(|| PipelineError::UnconfiguredRoot(RESULTS_ROOT.to_string()))
}

/// The directory of an input: an upstream output directory or a dataset directory.
pub fn input_dir(
    config: &PipelineConfig,
    keys: &StageKeys,
    roots: &Roots,
    ctx: &KeyContext,
    input: &StageInput,
    trial: bool,
) -> Option<PathBuf> {
    match input {
        StageInput::Stage(upstream) => Some(output_dir(
            roots.location(RESULTS_ROOT)?,
            &config.study,
            upstream,
            keys.keys.get(upstream)?,
            trial,
        )),
        StageInput::Dataset(r) => {
            let m = ctx.manifest(r)?;
            Some(roots.location(&m.root)?.join(&m.path))
        }
    }
}

/// The items of a fan-out input: `(item name, file path)`, sorted by name. Items are the regular
/// files directly inside the input directory, except `provenance.json` and names starting with `.`;
/// an item's name is its file name without the last extension.
pub fn items_of(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut items = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name == "provenance.json"
            || file_name.starts_with('.')
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let name = match file_name.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem.to_string(),
            _ => file_name.clone(),
        };
        items.push((name, entry.path()));
    }
    items.sort();
    Ok(items)
}

/// Quotes a value for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The paths a stage's command is substituted with.
pub struct RunPaths {
    pub out: PathBuf,
    pub inputs: BTreeMap<String, PathBuf>,
    pub item: Option<(String, PathBuf)>,
    pub params_file: PathBuf,
    /// The tool checkout, for a tool stage.
    pub tool: Option<PathBuf>,
}

/// The `run` command with its placeholders substituted and shell-quoted.
pub fn render_run(stage: &StageConfig, paths: &RunPaths) -> Result<String, String> {
    render_template(&stage.run, stage, paths)
}

/// A command template of `stage` (its `run` or `validate`) with its placeholders substituted and
/// shell-quoted.
pub fn render_template(
    template: &str,
    stage: &StageConfig,
    paths: &RunPaths,
) -> Result<String, String> {
    let mut out = String::new();
    for part in parse_template(template)? {
        match part {
            TemplatePart::Text(t) => out.push_str(&t),
            TemplatePart::Placeholder(p) => {
                let value = match p.as_str() {
                    "out" => paths.out.display().to_string(),
                    "params_file" => paths.params_file.display().to_string(),
                    "tool" => paths.tool.as_ref().ok_or("no tool")?.display().to_string(),
                    "threads" => stage.threads.to_string(),
                    // Without a current item (the stage-level record of a fan-out stage), the
                    // item placeholders stay as written: no single item ran.
                    "item" | "item_name" if paths.item.is_none() => {
                        out.push_str(&format!("{{{p}}}"));
                        continue;
                    }
                    "item" => paths
                        .item
                        .as_ref()
                        .expect("checked")
                        .1
                        .display()
                        .to_string(),
                    "item_name" => paths.item.as_ref().expect("checked").0.clone(),
                    _ => {
                        if let Some(input) = p.strip_prefix("in.") {
                            paths
                                .inputs
                                .get(input)
                                .ok_or_else(|| format!("input `{input}` is unavailable"))?
                                .display()
                                .to_string()
                        } else if let Some(key) = p.strip_prefix("params.") {
                            match stage.params.get(key) {
                                Some(serde_json::Value::String(s)) => s.clone(),
                                Some(v) => v.to_string(),
                                None => return Err(format!("unknown parameter `{key}`")),
                            }
                        } else {
                            return Err(format!("unknown placeholder `{{{p}}}`"));
                        }
                    }
                };
                out.push_str(&shell_quote(&value));
            }
        }
    }
    Ok(out)
}

/// The path of a stage's parameters file, next to its output directory.
pub fn params_file(out_dir: &Path) -> PathBuf {
    let mut name = out_dir.file_name().unwrap_or_default().to_os_string();
    name.push(".params.json");
    out_dir.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-e6252079
    fn key_format() {
        let key = stage_key("x\n");
        assert!(key.starts_with("sk-") && key.len() == 19);
        let digest = crate::data::to_hex(&Sha256::digest(b"x\n"));
        assert_eq!(&key[3..], &digest[..16]);
    }

    #[test] // rq-3ac5de89
    fn environment_hash_is_platform_independent_and_sorted() {
        let a = "environments:\n  default:\n    packages:\n      linux-64:\n      - conda: https://x/a\n      \
                 - pypi: https://x/b\n      osx-arm64:\n      - conda: https://x/c\n";
        let b = "environments:\n  default:\n    packages:\n      osx-arm64:\n      - conda: https://x/c\n      \
                 linux-64:\n      - pypi: https://x/b\n      - conda: https://x/a\n";
        assert_eq!(
            environment_hashes(a).unwrap(),
            environment_hashes(b).unwrap()
        );
        let c = a.replace("x/c", "x/d");
        assert_ne!(
            environment_hashes(a).unwrap(),
            environment_hashes(&c).unwrap()
        );
    }

    #[test]
    fn quoting_and_items() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(
            params_file(Path::new("/r/s/st/sk-1")),
            PathBuf::from("/r/s/st/sk-1.params.json")
        );
    }
}
