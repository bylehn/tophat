//! Stage configuration: `studies/<study>/config/base.yaml` and variant files.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Number, Value};
use yaml_rust2::{Yaml, YamlLoader};

use super::PipelineError;
use crate::check::Project;
use crate::data::DatasetRef;
use crate::study::{matches_id_pattern, STUDIES_DIR};

// rq-5cd91e7d
/// Where a stage input comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageInput {
    /// The output of an earlier stage of the same study.
    Stage(String),
    /// A dataset version.
    Dataset(DatasetRef),
}

// rq-fd86ea5e
/// The `external` mapping of an external stage (see `rqm/pipeline/external.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalConfig {
    /// Shell lines run in the job before the command.
    pub setup: Vec<String>,
    /// `sbatch` options of the stage.
    pub slurm: Vec<String>,
    /// Glob patterns, relative to `out/`, of output files that stay on the cluster.
    pub keep: Vec<String>,
}

impl ExternalConfig {
    // rq-23d21623
    /// The canonical JSON that enters the stage key: `setup` and `keep`, sorted keys, no
    /// whitespace. `slurm` is left out: resources do not change what a job computes.
    pub fn key_json(&self) -> String {
        serde_json::json!({"setup": self.setup, "keep": self.keep}).to_string()
    }
}

// rq-9950b389 rq-081e737a
/// One stage after variant merging.
#[derive(Debug, Clone, PartialEq)]
pub struct StageConfig {
    pub name: String,
    pub run: String,
    /// Code paths as written, relative to the study directory.
    pub code: Vec<String>,
    pub inputs: BTreeMap<String, StageInput>,
    /// The parameters as a JSON object (keys sorted).
    pub params: Map<String, Value>,
    pub env: String,
    /// Whether `env` was written in the configuration (rather than defaulted).
    pub env_set: bool,
    /// The tool pin as written (`<tool>@<commit>`), for a tool stage.
    pub tool: Option<String>,
    pub threads: u32,
    pub foreach: Option<String>,
    /// The `external` mapping, for an external stage.
    pub external: Option<ExternalConfig>,
    /// Whether the stage declares `qc: skip`, exempting it from its default QC gate.
    pub qc_skip: bool,
    /// Why the stage needs no QC verdict (with `qc: skip`).
    pub qc_reason: Option<String>,
    /// What the human checks in the stage's output at its QC gate. Not part of the key.
    pub check: Option<String>,
    pub result: Option<String>,
    /// Project-relative path of the file that defines the stage.
    pub file: String,
    /// 1-based line of the stage in `file`, when it can be located.
    pub line: Option<u32>,
}

// rq-cbd35529
/// The stages of a study for one variant.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineConfig {
    pub study: String,
    pub variant: String,
    pub stages: Vec<StageConfig>,
}

impl PipelineConfig {
    pub fn stage(&self, name: &str) -> Option<&StageConfig> {
        self.stages.iter().find(|s| s.name == name)
    }

    // rq-dc1e521d
    /// Whether a stage is gated: another stage consumes it (as a `stage:` input, including as a
    /// fan-out source) and it does not declare `qc: skip`.
    pub fn is_gated(&self, name: &str) -> bool {
        self.stage(name).is_some_and(|s| !s.qc_skip)
            && self.stages.iter().any(|consumer| {
                consumer
                    .inputs
                    .values()
                    .any(|i| matches!(i, StageInput::Stage(s) if s == name))
            })
    }
}

/// A problem found while loading or validating a configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}

impl From<ConfigProblem> for PipelineError {
    fn from(p: ConfigProblem) -> Self {
        PipelineError::Config {
            path: p.file,
            line: p.line,
            message: p.message,
        }
    }
}

/// One piece of a `run` template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    Text(String),
    Placeholder(String),
}

// rq-6891672b
/// Splits a `run` template into text and placeholders; `{{` and `}}` are literal braces.
pub fn parse_template(run: &str) -> Result<Vec<TemplatePart>, String> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = run.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') | None => return Err(format!("unclosed placeholder `{{{name}`")),
                        Some(c) => name.push(c),
                    }
                }
                if !text.is_empty() {
                    parts.push(TemplatePart::Text(std::mem::take(&mut text)));
                }
                parts.push(TemplatePart::Placeholder(name));
            }
            '}' => return Err("unmatched `}`; write `}}` for a literal brace".into()),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(TemplatePart::Text(text));
    }
    Ok(parts)
}

/// Converts YAML to JSON for parameters. Mapping keys must be text.
fn yaml_to_json(value: &Yaml) -> Result<Value, String> {
    Ok(match value {
        Yaml::Null => Value::Null,
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Integer(i) => Value::Number((*i).into()),
        Yaml::Real(s) => {
            let f: f64 = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
            Value::Number(
                Number::from_f64(f).ok_or_else(|| format!("`{s}` is not a finite number"))?,
            )
        }
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Array(items) => {
            Value::Array(items.iter().map(yaml_to_json).collect::<Result<_, _>>()?)
        }
        Yaml::Hash(hash) => {
            let mut map = Map::new();
            for (k, v) in hash {
                let Yaml::String(k) = k else {
                    return Err("parameter names must be text".into());
                };
                map.insert(k.clone(), yaml_to_json(v)?);
            }
            Value::Object(map)
        }
        Yaml::Alias(_) | Yaml::BadValue => return Err("unsupported YAML value".into()),
    })
}

const STAGE_KEYS: [&str; 14] = [
    "name",
    "run",
    "code",
    "inputs",
    "params",
    "env",
    "threads",
    "foreach",
    "qc",
    "qc_reason",
    "check",
    "result",
    "tool",
    "external",
];

/// The fields given for one stage in one file, before merging.
#[derive(Debug, Default, Clone)]
struct StageFields {
    name: Option<String>,
    run: Option<String>,
    code: Option<Vec<String>>,
    inputs: Option<BTreeMap<String, String>>,
    params: Option<Map<String, Value>>,
    env: Option<String>,
    threads: Option<u32>,
    foreach: Option<String>,
    qc_skip: Option<bool>,
    qc_reason: Option<String>,
    check: Option<String>,
    result: Option<String>,
    tool: Option<String>,
    external: Option<ExternalConfig>,
    line: Option<u32>,
}

/// Finds the line of the stage named `name` (`- name: x` or `name: x`), searching from `from`.
fn locate_stage(lines: &[&str], name: &str, from: usize) -> Option<usize> {
    let matches = |l: &str| {
        let t = l.trim_start().trim_start_matches('-').trim_start();
        t.strip_prefix("name:")
            .map(|v| {
                v.split(" #")
                    .next()
                    .unwrap_or(v)
                    .trim()
                    .trim_matches(['"', '\''])
                    == name
            })
            .unwrap_or(false)
    };
    (from..lines.len())
        .chain(0..from)
        .find(|&i| matches(lines[i]))
}

/// Parses one configuration file into per-stage fields, reporting problems.
fn parse_file(
    file: &str,
    text: &str,
    problems: &mut Vec<ConfigProblem>,
) -> Option<Vec<StageFields>> {
    let problem = |line: Option<u32>, message: String| ConfigProblem {
        file: file.to_string(),
        line,
        message,
    };
    let doc = match YamlLoader::load_from_str(text) {
        Ok(docs) => docs.into_iter().next().unwrap_or(Yaml::Null),
        Err(e) => {
            problems.push(problem(
                Some(e.marker().line() as u32),
                format!("invalid YAML: {}", e.info()),
            ));
            return None;
        }
    };
    let stages = match doc {
        Yaml::Null => return Some(Vec::new()),
        Yaml::Hash(mut top) => {
            let stages = top.remove(&Yaml::String("stages".into()));
            if let Some((k, _)) = top.iter().next() {
                problems.push(problem(
                    None,
                    format!("unknown top-level key {k:?}; only `stages` is allowed"),
                ));
            }
            stages.unwrap_or(Yaml::Null)
        }
        _ => {
            problems.push(problem(
                None,
                "the configuration must be a mapping with `stages`".into(),
            ));
            return None;
        }
    };
    let items = match stages {
        Yaml::Null => return Some(Vec::new()),
        Yaml::Array(items) => items,
        _ => {
            problems.push(problem(None, "`stages` must be a list".into()));
            return None;
        }
    };

    let lines: Vec<&str> = text.lines().collect();
    let mut cursor = 0;
    let mut out = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let Yaml::Hash(hash) = item else {
            problems.push(problem(
                None,
                format!("stage {} must be a mapping", index + 1),
            ));
            continue;
        };
        let mut fields = StageFields::default();
        if let Some(Yaml::String(name)) = hash.get(&Yaml::String("name".into())) {
            fields.name = Some(name.clone());
            if let Some(i) = locate_stage(&lines, name, cursor) {
                fields.line = Some(i as u32 + 1);
                cursor = i + 1;
            }
        }
        let at = fields.line;
        let label = fields
            .name
            .clone()
            .unwrap_or_else(|| format!("#{}", index + 1));
        if hash.contains_key(&Yaml::String("tool".into()))
            && hash.contains_key(&Yaml::String("external".into()))
        {
            problems.push(problem(
                at,
                format!("stage `{label}`: a stage cannot have both `tool` and `external`"),
            ));
        }
        for (key, value) in hash {
            let Yaml::String(key) = key else {
                problems.push(problem(at, format!("stage `{label}`: keys must be text")));
                continue;
            };
            let bad = |what: &str| problem(at, format!("stage `{label}`: `{key}` must be {what}"));
            match key.as_str() {
                "name" if !matches!(value, Yaml::String(_)) => problems.push(bad("text")),
                "name" => {}
                "run" => match value {
                    Yaml::String(s) => fields.run = Some(s),
                    _ => problems.push(bad("text")),
                },
                "code" => match value {
                    Yaml::String(s) => fields.code = Some(vec![s]),
                    Yaml::Array(items) => {
                        let strings: Option<Vec<String>> = items
                            .into_iter()
                            .map(|i| match i {
                                Yaml::String(s) => Some(s),
                                _ => None,
                            })
                            .collect();
                        match strings {
                            Some(s) => fields.code = Some(s),
                            None => problems.push(bad("a list of paths")),
                        }
                    }
                    _ => problems.push(bad("a list of paths")),
                },
                "inputs" => match value {
                    Yaml::Hash(h) => {
                        let mut inputs = BTreeMap::new();
                        for (k, v) in h {
                            match (k, v) {
                                (Yaml::String(k), Yaml::String(v)) => {
                                    inputs.insert(k, v);
                                }
                                _ => problems.push(bad("a mapping of input names to sources")),
                            }
                        }
                        fields.inputs = Some(inputs);
                    }
                    Yaml::Null => fields.inputs = Some(BTreeMap::new()),
                    _ => problems.push(bad("a mapping of input names to sources")),
                },
                "params" => match value {
                    Yaml::Null => fields.params = Some(Map::new()),
                    v @ Yaml::Hash(_) => match yaml_to_json(&v) {
                        Ok(Value::Object(map)) => fields.params = Some(map),
                        Ok(_) => unreachable!("a YAML mapping converts to a JSON object"),
                        Err(e) => {
                            problems.push(problem(at, format!("stage `{label}`: params: {e}")))
                        }
                    },
                    _ => problems.push(bad("a mapping")),
                },
                "env" => match value {
                    Yaml::String(s) => fields.env = Some(s),
                    _ => problems.push(bad("a pixi environment name")),
                },
                "threads" => match value {
                    Yaml::Integer(n) if n >= 1 && n <= u32::MAX as i64 => {
                        fields.threads = Some(n as u32)
                    }
                    _ => problems.push(bad("a positive whole number")),
                },
                "foreach" => match value {
                    Yaml::String(s) => fields.foreach = Some(s),
                    _ => problems.push(bad("an input name")),
                },
                "qc" => match value {
                    Yaml::String(s) if s == "skip" => fields.qc_skip = Some(true),
                    _ => problems.push(problem(
                        at,
                        format!(
                            "stage `{label}`: stages consumed by other stages are gated by default; \
                             the only `qc` setting is `qc: skip` (with a `qc_reason`)"
                        ),
                    )),
                },
                "qc_reason" => match value {
                    Yaml::String(s) => fields.qc_reason = Some(s),
                    _ => problems.push(bad("text")),
                },
                // rq-0a10fc96
                "check" => match value {
                    Yaml::String(s) => fields.check = Some(s),
                    _ => problems.push(bad("text")),
                },
                "result" => match value {
                    Yaml::String(s) => fields.result = Some(s),
                    _ => problems.push(bad("a file name")),
                },
                "tool" => match value {
                    Yaml::String(s) => fields.tool = Some(s),
                    _ => problems.push(bad("a pin `<tool>@<commit>`")),
                },
                "external" => match parse_external(value) {
                    Ok(external) => fields.external = Some(external),
                    Err(m) => problems.push(problem(at, format!("stage `{label}`: external: {m}"))),
                },
                other => problems.push(problem(
                    at,
                    format!(
                        "stage `{label}`: unknown key `{other}`; allowed keys are {}",
                        STAGE_KEYS.join(", ")
                    ),
                )),
            }
        }
        out.push(fields);
    }
    Some(out)
}

// rq-fd86ea5e
/// Whether `pattern` is a usable `keep` pattern: a relative path of literal characters and the
/// wildcards `*`, `?` and `[...]` (none of which match `/`), so that the job script, ingest and
/// `rsync --exclude` agree on what it matches.
fn is_valid_keep_pattern(pattern: &str) -> bool {
    pattern
        .split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && !pattern.contains("**")
        && pattern
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-*?[]!".contains(&b) || b == b'/')
}

// rq-fd86ea5e
/// Parses an `external` mapping: only `setup`, `slurm` and `keep`, each a list of text.
fn parse_external(value: Yaml) -> Result<ExternalConfig, String> {
    let hash = match value {
        Yaml::Hash(hash) => hash,
        _ => return Err("must be a mapping with `setup`, `slurm` and `keep`".into()),
    };
    let mut external = ExternalConfig::default();
    for (key, value) in hash {
        let Yaml::String(key) = key else {
            return Err("keys must be text".into());
        };
        let list = match value {
            Yaml::Null => Vec::new(),
            Yaml::Array(items) => items
                .into_iter()
                .map(|i| match i {
                    Yaml::String(s) => Ok(s),
                    _ => Err(format!("`{key}` must be a list of text")),
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(format!("`{key}` must be a list of text")),
        };
        if let Some(bad) = list.iter().find(|s| s.contains(['\n', '\r'])) {
            return Err(format!("`{key}` entry {bad:?} must be a single line"));
        }
        match key.as_str() {
            "setup" => external.setup = list,
            "slurm" => {
                if let Some(bad) = list.iter().find(|o| !o.starts_with('-')) {
                    return Err(format!(
                        "`slurm` option `{bad}` must be an sbatch option such as `--time=24:00:00`"
                    ));
                }
                external.slurm = list;
            }
            "keep" => {
                if let Some(bad) = list.iter().find(|p| !is_valid_keep_pattern(p)) {
                    return Err(format!(
                        "`keep` pattern `{bad}` must be a relative path using letters, digits, \
                         `._-` and the wildcards `*`, `?` and `[...]`"
                    ));
                }
                external.keep = list;
            }
            other => {
                return Err(format!(
                    "unknown key `{other}`; allowed keys are setup, slurm and keep. Connection \
                     details (hosts, remote paths, credentials) never go in the project"
                ))
            }
        }
    }
    Ok(external)
}

/// Builds a stage from complete fields, validating what does not depend on other stages.
fn build_stage(
    fields: StageFields,
    file: &str,
    problems: &mut Vec<ConfigProblem>,
) -> Option<StageConfig> {
    let problem = |message: String| ConfigProblem {
        file: file.to_string(),
        line: fields.line,
        message,
    };
    let Some(name) = fields.name.clone() else {
        problems.push(problem("a stage has no `name`".into()));
        return None;
    };
    if !matches_id_pattern(&name) {
        problems.push(problem(format!(
            "stage name `{name}` must use lowercase letters, digits and single hyphens"
        )));
    }
    let Some(run) = fields.run.clone() else {
        problems.push(problem(format!("stage `{name}` has no `run` command")));
        return None;
    };
    let mut inputs = BTreeMap::new();
    for (input, source) in fields.inputs.clone().unwrap_or_default() {
        if !matches_id_pattern(&input) {
            problems.push(problem(format!(
                "stage `{name}`: input name `{input}` is malformed"
            )));
        }
        let parsed = if let Some(stage) = source.strip_prefix("stage:") {
            Some(StageInput::Stage(stage.to_string()))
        } else if let Some(r) = source.strip_prefix("dataset:") {
            DatasetRef::parse(r).map(StageInput::Dataset)
        } else {
            None
        };
        match parsed {
            Some(p) => {
                inputs.insert(input, p);
            }
            None => problems.push(problem(format!(
                "stage `{name}`: input `{input}` must be `stage:<stage>` or `dataset:<name>@v<n>`, not `{source}`"
            ))),
        }
    }
    if let Some(result) = &fields.result {
        if !crate::data::is_safe_relative_path(result) {
            problems.push(problem(format!(
                "stage `{name}`: `result` must be a relative file path"
            )));
        }
    }
    let env = fields.env.clone().unwrap_or_else(|| "default".into());
    Some(StageConfig {
        name,
        run,
        code: fields.code.clone().unwrap_or_default(),
        inputs,
        params: fields.params.clone().unwrap_or_default(),
        env,
        env_set: fields.env.is_some(),
        threads: fields.threads.unwrap_or(1),
        foreach: fields.foreach.clone(),
        external: fields.external.clone(),
        qc_skip: fields.qc_skip.unwrap_or(false),
        qc_reason: fields.qc_reason.clone(),
        check: fields.check.clone(),
        tool: fields.tool.clone(),
        result: fields.result.clone(),
        file: file.to_string(),
        line: fields.line,
    })
}

/// Validates the relations between stages and each stage's `run` template.
fn validate(
    stages: &[StageConfig],
    file_of: &dyn Fn(&StageConfig) -> String,
    problems: &mut Vec<ConfigProblem>,
) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for stage in stages {
        let problem = |message: String| ConfigProblem {
            file: file_of(stage),
            line: stage.line,
            message,
        };
        if !seen.insert(&stage.name) {
            problems.push(problem(format!(
                "stage name `{}` is used more than once",
                stage.name
            )));
        }
        for (input, source) in &stage.inputs {
            if let StageInput::Stage(upstream) = source {
                if upstream == &stage.name
                    || !stages
                        .iter()
                        .take_while(|s| s.name != stage.name)
                        .any(|s| &s.name == upstream)
                {
                    problems.push(problem(format!(
                        "stage `{}`: input `{input}` references `stage:{upstream}`, which is not an earlier stage",
                        stage.name
                    )));
                }
            }
        }
        if stage.qc_reason.is_some() && !stage.qc_skip {
            problems.push(problem(format!(
                "stage `{}`: `qc_reason` is only allowed with `qc: skip`",
                stage.name
            )));
        }
        if let Some(f) = &stage.foreach {
            if !stage.inputs.contains_key(f) {
                problems.push(problem(format!(
                    "stage `{}`: `foreach: {f}` names no input",
                    stage.name
                )));
            }
            if stage.result.is_some() {
                problems.push(problem(format!(
                    "stage `{}`: a fan-out stage (`foreach`) cannot declare `result`",
                    stage.name
                )));
            }
        }
        // rq-fd86ea5e
        let external = stage.external.is_some();
        if external {
            let mut not_allowed = Vec::new();
            if stage.foreach.is_some() {
                not_allowed.push("`foreach` (external stages do not fan out)");
            }
            if stage.env_set {
                not_allowed
                    .push("`env` (its software comes from the cluster; use `external.setup`)");
            }
            if !not_allowed.is_empty() {
                problems.push(problem(format!(
                    "stage `{}`: an external stage cannot have {}",
                    stage.name,
                    not_allowed.join(", ")
                )));
            }
            if stage.tool.is_some() {
                problems.push(problem(format!(
                    "stage `{}`: a stage cannot have both `tool` and `external`",
                    stage.name
                )));
            }
            for entry in &stage.code {
                let normalized = super::keys::normalize(std::path::Path::new(entry));
                let top = normalized
                    .as_ref()
                    .and_then(|p| p.components().next())
                    .map(|c| c.as_os_str().to_string_lossy().into_owned());
                if let Some(top) =
                    top.filter(|t| super::external::RESERVED_NAMES.contains(&t.as_str()))
                {
                    problems.push(problem(format!(
                        "stage `{}`: code `{entry}` would take the place of `{top}` in the job folder",
                        stage.name
                    )));
                }
                if normalized.is_none() {
                    problems.push(problem(format!(
                        "stage `{}`: code `{entry}` of an external stage must lie inside the study \
                         directory, so it can travel with the job folder",
                        stage.name
                    )));
                }
            }
        }
        match parse_template(&stage.run) {
            Err(e) => problems.push(problem(format!("stage `{}`: run: {e}", stage.name))),
            Ok(parts) => {
                for part in parts {
                    let TemplatePart::Placeholder(p) = part else {
                        continue;
                    };
                    let valid = match p.as_str() {
                        "out" | "params_file" | "threads" => true,
                        "tool" => stage.tool.is_some() && !external,
                        "item" | "item_name" => stage.foreach.is_some() && !external,
                        _ => {
                            if let Some(input) = p.strip_prefix("in.") {
                                stage.inputs.contains_key(input)
                            } else if let Some(key) = p.strip_prefix("params.") {
                                stage
                                    .params
                                    .get(key)
                                    .is_some_and(|v| !v.is_object() && !v.is_array())
                            } else {
                                false
                            }
                        }
                    };
                    if !valid && external && ["tool", "item", "item_name"].contains(&p.as_str()) {
                        problems.push(problem(format!(
                            "stage `{}`: placeholder `{{{p}}}` is not available in an external stage",
                            stage.name
                        )));
                    } else if !valid {
                        problems.push(problem(format!(
                            "stage `{}`: unknown placeholder `{{{p}}}` in run",
                            stage.name
                        )));
                    }
                }
            }
        }
    }
}

/// Project-relative path of a study's configuration file for a variant.
pub fn config_path(study: &str, variant: &str) -> String {
    format!("{STUDIES_DIR}/{study}/config/{variant}.yaml")
}

/// Merges a variant's stage fields into the base stages.
fn merge_variant(
    base: &[StageConfig],
    variant_fields: Vec<StageFields>,
    file: &str,
    problems: &mut Vec<ConfigProblem>,
) -> Vec<StageConfig> {
    let mut stages = base.to_vec();
    let mut seen = BTreeSet::new();
    for fields in variant_fields {
        let problem = |message: String| ConfigProblem {
            file: file.to_string(),
            line: fields.line,
            message,
        };
        let Some(name) = fields.name.clone() else {
            problems.push(problem("a variant entry has no `name`".into()));
            continue;
        };
        if !seen.insert(name.clone()) {
            problems.push(problem(format!(
                "stage `{name}` appears more than once in the variant"
            )));
            continue;
        }
        let Some(stage) = stages.iter_mut().find(|s| s.name == name) else {
            problems.push(problem(format!(
                "variant names stage `{name}`, which is not in config/base.yaml; variants cannot add stages"
            )));
            continue;
        };
        if let Some(run) = fields.run {
            stage.run = run;
        }
        if let Some(code) = fields.code {
            stage.code = code;
        }
        for (input, source) in fields.inputs.unwrap_or_default() {
            let parsed = if let Some(s) = source.strip_prefix("stage:") {
                Some(StageInput::Stage(s.to_string()))
            } else {
                source
                    .strip_prefix("dataset:")
                    .and_then(DatasetRef::parse)
                    .map(StageInput::Dataset)
            };
            match parsed {
                Some(p) => {
                    stage.inputs.insert(input, p);
                }
                None => problems.push(problem(format!(
                    "stage `{name}`: input `{input}` must be `stage:<stage>` or `dataset:<name>@v<n>`"
                ))),
            }
        }
        for (k, v) in fields.params.unwrap_or_default() {
            stage.params.insert(k, v);
        }
        if let Some(env) = fields.env {
            stage.env = env;
            stage.env_set = true;
        }
        if let Some(threads) = fields.threads {
            stage.threads = threads;
        }
        if let Some(foreach) = fields.foreach {
            stage.foreach = Some(foreach);
        }
        if let Some(skip) = fields.qc_skip {
            stage.qc_skip = skip;
        }
        if let Some(reason) = fields.qc_reason {
            stage.qc_reason = Some(reason);
        }
        if let Some(check) = fields.check {
            stage.check = Some(check);
        }
        if let Some(tool) = fields.tool {
            stage.tool = Some(tool);
        }
        if let Some(external) = fields.external {
            stage.external = Some(external);
        }
        if let Some(result) = fields.result {
            stage.result = Some(result);
        }
        stage.file = file.to_string();
        stage.line = fields.line.or(stage.line);
    }
    stages
}

/// Loads the base stages, collecting every problem. Returns `None` when the base file is missing.
pub(crate) fn load_base(
    project: &Project,
    study: &str,
    problems: &mut Vec<ConfigProblem>,
) -> Option<Vec<StageConfig>> {
    let file = config_path(study, "base");
    let text = project.read_to_string(&file).ok()?;
    let fields = parse_file(&file, &text, problems)?;
    let stages: Vec<StageConfig> = fields
        .into_iter()
        .filter_map(|f| build_stage(f, &file, problems))
        .collect();
    validate(&stages, &|_| file.clone(), problems);
    Some(stages)
}

/// Loads a pipeline for a variant, collecting every problem.
pub(crate) fn load_variant(
    project: &Project,
    study: &str,
    variant: &str,
    problems: &mut Vec<ConfigProblem>,
) -> Option<PipelineConfig> {
    let base = load_base(project, study, problems)?;
    if variant == "base" {
        return Some(PipelineConfig {
            study: study.to_string(),
            variant: variant.to_string(),
            stages: base,
        });
    }
    let file = config_path(study, variant);
    let text = project.read_to_string(&file).ok()?;
    let mut variant_problems = Vec::new();
    let fields = parse_file(&file, &text, &mut variant_problems)?;
    let stages = merge_variant(&base, fields, &file, &mut variant_problems);
    // Re-validate the merged stages, attributing problems to the variant file.
    validate(&stages, &|_| file.clone(), &mut variant_problems);
    for p in variant_problems {
        if !problems.contains(&p) {
            problems.push(p);
        }
    }
    Some(PipelineConfig {
        study: study.to_string(),
        variant: variant.to_string(),
        stages,
    })
}

// rq-f8dfe570 rq-bb4108e3 rq-a17aa884
/// Reads and merges a study's configuration for a variant and validates it.
pub fn load_pipeline(
    root: &std::path::Path,
    study: &str,
    variant: &str,
) -> Result<PipelineConfig, PipelineError> {
    let project = Project::new(root);
    if variant != "base" && !project.is_file(&config_path(study, variant)) {
        return Err(PipelineError::UnknownVariant(variant.to_string()));
    }
    if !project.is_file(&config_path(study, "base")) {
        return Err(PipelineError::Config {
            path: config_path(study, "base"),
            line: None,
            message: "the study has no config/base.yaml".into(),
        });
    }
    let mut problems = Vec::new();
    let config = load_variant(&project, study, variant, &mut problems);
    match (problems.into_iter().next(), config) {
        (Some(problem), _) => Err(problem.into()),
        (None, Some(config)) => Ok(config),
        (None, None) => Err(PipelineError::Io(format!(
            "cannot read {}",
            config_path(study, variant)
        ))),
    }
}

/// Loads a study's pipeline for a variant, ignoring configuration problems (they are reported by
/// `tophat check`). `None` when the configuration cannot be read at all.
pub fn load_variant_lenient(
    project: &Project,
    study: &str,
    variant: &str,
) -> Option<PipelineConfig> {
    load_variant(project, study, variant, &mut Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-6891672b
    fn templates() {
        assert_eq!(
            parse_template("awk '{{print $1}}' {in.raw} > {out}/x").unwrap(),
            vec![
                TemplatePart::Text("awk '{print $1}' ".into()),
                TemplatePart::Placeholder("in.raw".into()),
                TemplatePart::Text(" > ".into()),
                TemplatePart::Placeholder("out".into()),
                TemplatePart::Text("/x".into()),
            ]
        );
        assert!(parse_template("{unclosed").is_err());
        assert!(parse_template("a } b").is_err());
    }

    #[test] // rq-081e737a
    fn params_become_canonical_json() {
        let doc = &YamlLoader::load_from_str("{level: b3lyp, cutoff: 2.0, n: 3, list: [1, x]}")
            .unwrap()[0];
        let json = yaml_to_json(doc).unwrap();
        assert_eq!(
            serde_json::to_string(&json).unwrap(),
            r#"{"cutoff":2.0,"level":"b3lyp","list":[1,"x"],"n":3}"#
        );
    }
}
