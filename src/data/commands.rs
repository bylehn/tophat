//! The `tophat data` commands.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde_json::{json, Value};

use super::cache::HashCache;
use super::manifest::{is_safe_relative_path, list_manifests, retired_block};
use super::refs::study_dataset_refs;
use super::roots::load_roots;
use super::{
    content_hash, hash_files, is_valid_dataset_name, parse_manifest, DataError, DatasetKind,
    DatasetRef, FileEntry, Manifest, Origin, RESULTS_ROOT,
};
use crate::check::Project;
use crate::fsutil::write_atomic;
use crate::study::STUDIES_DIR;

/// What to register with `tophat data add` or `tophat data declare`.
#[derive(Debug, Clone)]
pub struct RegisterOptions {
    pub name: String,
    pub root: String,
    pub path: String,
    pub visibility: String,
    /// `Origin::Raw` for `add`, `Origin::Declared` for `declare`.
    pub origin: Origin,
}

/// Lists every regular file below `dir`, sorted by relative `/`-separated path. Symbolic links
/// are an error.
fn collect_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, DataError> {
    let mut out = Vec::new();
    let mut pending = vec![(String::new(), dir.to_path_buf())];
    while let Some((prefix, current)) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(DataError::SymbolicLink(rel));
            } else if file_type.is_dir() {
                pending.push((rel, entry.path()));
            } else if file_type.is_file() {
                out.push((rel, entry.path()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The next version number for a dataset name.
fn next_version(project: &Project, name: &str) -> u32 {
    list_manifests(project)
        .into_iter()
        .filter(|m| m.dir_name == name)
        .filter_map(|m| m.file_version)
        .max()
        .unwrap_or(0)
        + 1
}

// rq-af5ed3df rq-d89c8a6b
/// Registers a directory in a data root as the next version of a raw or declared dataset.
pub fn add(project_root: &Path, options: &RegisterOptions) -> Result<DatasetRef, DataError> {
    let RegisterOptions {
        name,
        root,
        path,
        visibility,
        origin,
    } = options;
    if !is_valid_dataset_name(name) {
        return Err(DataError::InvalidName(name.clone()));
    }
    if visibility != "public" && visibility != "private" {
        return Err(DataError::InvalidArgument(format!(
            "visibility must be public or private, not `{visibility}`"
        )));
    }
    let kind = match origin {
        Origin::Raw { source, date, .. } => {
            if source.trim().is_empty() {
                return Err(DataError::InvalidArgument(
                    "--source must not be empty".into(),
                ));
            }
            if date
                .as_deref()
                .is_some_and(|d| d.len() != 10 || NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err())
            {
                return Err(DataError::InvalidArgument(
                    "--date must be YYYY-MM-DD".into(),
                ));
            }
            DatasetKind::Raw
        }
        Origin::Declared { statement } => {
            if statement.trim().is_empty() {
                return Err(DataError::InvalidArgument(
                    "--statement must not be empty".into(),
                ));
            }
            DatasetKind::Declared
        }
        Origin::Published => {
            return Err(DataError::InvalidArgument(
                "published datasets are created with `tophat data publish`".into(),
            ))
        }
    };
    let roots = load_roots(project_root)?;
    let location = roots.require(root)?;
    let path = path.trim_end_matches('/');
    if !is_safe_relative_path(path) {
        return Err(DataError::PathOutsideRoot(path.to_string()));
    }
    let dir = location.join(path);
    if !dir.is_dir() {
        return Err(DataError::NoFiles(format!(
            "{root}/{path} (not a directory)"
        )));
    }
    let files = collect_files(&dir)?;
    if files.is_empty() {
        return Err(DataError::NoFiles(format!("{root}/{path}")));
    }

    let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
    let mut entries = Vec::with_capacity(files.len());
    for ((rel, _), hashed) in files.iter().zip(hash_files(&paths)) {
        let (size, sha256) = hashed?;
        entries.push(FileEntry {
            path: rel.clone(),
            size,
            sha256,
        });
    }
    let pairs: Vec<(String, String)> = entries
        .iter()
        .map(|e| (e.path.clone(), e.sha256.clone()))
        .collect();

    let project = Project::new(project_root);
    let version = next_version(&project, name);
    let manifest = Manifest {
        name: name.clone(),
        version,
        kind,
        visibility: visibility.clone(),
        root: root.clone(),
        path: path.to_string(),
        created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        origin: origin.clone(),
        produced_by: None,
        content_hash: content_hash(&pairs),
        files: entries,
        retired: None,
    };
    let reference = manifest.reference();
    let manifest_path = project_root.join(reference.manifest_path());
    fs::create_dir_all(
        manifest_path
            .parent()
            .expect("manifest paths have a parent"),
    )?;
    write_atomic(&manifest_path, &manifest.render())?;

    let mut cache = HashCache::load(project_root);
    for (entry, (_, file)) in manifest.files.iter().zip(&files) {
        cache.record(
            root,
            &format!("{path}/{}", entry.path),
            file,
            entry.size,
            &entry.sha256,
        );
    }
    cache.save();
    Ok(reference)
}

/// Registers data of unknown history as a declared dataset (see [`add`]).
pub fn declare(project_root: &Path, options: &RegisterOptions) -> Result<DatasetRef, DataError> {
    if !matches!(options.origin, Origin::Declared { .. }) {
        return Err(DataError::InvalidArgument(
            "declared datasets need a statement of origin".into(),
        ));
    }
    add(project_root, options)
}

/// The outcome of `tophat data verify`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// One line per problem or skipped dataset.
    pub lines: Vec<String>,
    pub verified: usize,
    pub problems: usize,
    pub skipped: usize,
}

impl VerifyReport {
    pub fn summary(&self) -> String {
        format!(
            "{} dataset version(s) verified, {} problem(s), {} skipped",
            self.verified, self.problems, self.skipped
        )
    }
}

/// A manifest's project-relative path and its parse result.
type SelectedManifest = (String, Result<Manifest, String>);

/// Selects manifests by `<name>` or `<name>@v<n>`; `None` selects all.
fn select(project: &Project, target: Option<&str>) -> Result<Vec<SelectedManifest>, DataError> {
    let all: Vec<_> = list_manifests(project)
        .into_iter()
        .filter(|m| m.file_version.is_some())
        .collect();
    let selected: Vec<_> = match target {
        None => all,
        Some(t) if t.contains('@') => {
            let r = DatasetRef::parse(t).ok_or_else(|| DataError::InvalidReference(t.into()))?;
            all.into_iter()
                .filter(|m| m.dir_name == r.name && m.file_version == Some(r.version))
                .collect()
        }
        Some(t) => {
            if !is_valid_dataset_name(t) {
                return Err(DataError::InvalidName(t.into()));
            }
            all.into_iter().filter(|m| m.dir_name == t).collect()
        }
    };
    if let (Some(t), true) = (target, selected.is_empty()) {
        return Err(DataError::DatasetNotFound(t.into()));
    }
    Ok(selected.into_iter().map(|m| (m.path, m.parsed)).collect())
}

// rq-33c3cc6a rq-597bb615
/// Re-hashes datasets and compares them to their manifests, updating the hash cache for matches.
pub fn verify(project_root: &Path, target: Option<&str>) -> Result<VerifyReport, DataError> {
    let project = Project::new(project_root);
    let roots = load_roots(project_root)?;
    let mut cache = HashCache::load(project_root);
    let mut report = VerifyReport::default();

    for (path, parsed) in select(&project, target)? {
        let manifest = match parsed {
            Ok(m) => m,
            Err(message) => {
                report.problems += 1;
                report.lines.push(format!("INVALID  {path}: {message}"));
                continue;
            }
        };
        let reference = manifest.reference();
        let Some(location) = roots.location(&manifest.root) else {
            report.skipped += 1;
            report.lines.push(format!(
                "SKIPPED  {reference} (root `{}` is not configured)",
                manifest.root
            ));
            continue;
        };
        let dir = location.join(&manifest.path);
        let before = report.problems;

        let present: Vec<&FileEntry> = manifest
            .files
            .iter()
            .filter(|e| dir.join(&e.path).is_file())
            .collect();
        for e in manifest
            .files
            .iter()
            .filter(|e| !dir.join(&e.path).is_file())
        {
            report.problems += 1;
            report
                .lines
                .push(format!("MISSING  {reference}  {}", e.path));
        }
        let paths: Vec<PathBuf> = present.iter().map(|e| dir.join(&e.path)).collect();
        for (entry, hashed) in present.iter().zip(hash_files(&paths)) {
            match hashed {
                Ok((size, sha)) if size == entry.size && sha == entry.sha256 => {
                    let rel = format!("{}/{}", manifest.path, entry.path);
                    cache.record(&manifest.root, &rel, &dir.join(&entry.path), size, &sha);
                }
                _ => {
                    report.problems += 1;
                    report
                        .lines
                        .push(format!("MISMATCH {reference}  {}", entry.path));
                }
            }
        }
        let recorded: BTreeSet<&str> = manifest.files.iter().map(|e| e.path.as_str()).collect();
        let extras = match collect_files(&dir) {
            Ok(files) => files.into_iter().map(|(rel, _)| rel).collect(),
            Err(DataError::SymbolicLink(rel)) => vec![rel],
            Err(_) => Vec::new(),
        };
        for rel in extras
            .into_iter()
            .filter(|r| !recorded.contains(r.as_str()))
        {
            report.problems += 1;
            report.lines.push(format!("EXTRA    {reference}  {rel}"));
        }
        if report.problems == before {
            report.verified += 1;
        }
    }
    cache.save();
    Ok(report)
}

// rq-a7709a28
/// Marks a dataset version as retired, appending the `retired` block to its manifest.
pub fn retire(
    project_root: &Path,
    reference: &str,
    reason: &str,
    today: NaiveDate,
) -> Result<(), DataError> {
    let r = DatasetRef::parse(reference)
        .ok_or_else(|| DataError::InvalidReference(reference.to_string()))?;
    if reason.trim().is_empty() {
        return Err(DataError::InvalidArgument(
            "--reason must not be empty".into(),
        ));
    }
    let path = project_root.join(r.manifest_path());
    let text = fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DataError::DatasetNotFound(reference.to_string())
        } else {
            e.into()
        }
    })?;
    let manifest = parse_manifest(&text).map_err(|message| DataError::Manifest {
        path: r.manifest_path(),
        message,
    })?;
    if manifest.retired.is_some() {
        return Err(DataError::AlreadyRetired(reference.to_string()));
    }
    let mut new_text = text;
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    new_text.push_str(&retired_block(&super::Retired {
        date: today.format("%Y-%m-%d").to_string(),
        reason: reason.trim().to_string(),
    }));
    write_atomic(&path, &new_text)?;
    Ok(())
}

/// One input recorded in a `provenance.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedInput {
    Stage { stage: String, key: String },
    Dataset(String),
}

/// Reads the inputs of the provenance record of `<study>/<stage>@<key>`, if available.
fn recorded_inputs(
    results: Option<&Path>,
    study: &str,
    stage: &str,
    key: &str,
) -> Option<Vec<RecordedInput>> {
    let path = results?
        .join(study)
        .join(stage)
        .join(key)
        .join("provenance.json");
    let record: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let inputs = record.get("inputs")?.as_array()?;
    Some(
        inputs
            .iter()
            .filter_map(|i| match i.get("kind")?.as_str()? {
                "stage" => Some(RecordedInput::Stage {
                    stage: i.get("stage")?.as_str()?.to_string(),
                    key: i.get("key")?.as_str()?.to_string(),
                }),
                "dataset" => Some(RecordedInput::Dataset(i.get("ref")?.as_str()?.to_string())),
                _ => None,
            })
            .collect(),
    )
}

/// A node of a lineage tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageNode {
    /// `<name>@v<n>` or `<study>/<stage>@<key>`.
    pub label: String,
    /// `raw`, `declared`, `published`, `stage`, or `unknown`.
    pub kind: String,
    /// A marker such as `[declared, not verified]` or `[unavailable]`.
    pub mark: Option<String>,
    pub inputs: Vec<LineageNode>,
}

impl LineageNode {
    /// Renders the tree, two spaces of indentation per level.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.render_into(0, &mut out);
        out
    }

    fn render_into(&self, depth: usize, out: &mut String) {
        out.push_str(&"  ".repeat(depth));
        out.push_str(&self.label);
        out.push_str(&format!(" ({})", self.kind));
        if let Some(mark) = &self.mark {
            out.push(' ');
            out.push_str(mark);
        }
        out.push('\n');
        for input in &self.inputs {
            input.render_into(depth + 1, out);
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "label": self.label,
            "kind": self.kind,
            "mark": self.mark,
            "inputs": self.inputs.iter().map(LineageNode::to_json).collect::<Vec<_>>(),
        })
    }
}

struct LineageContext {
    manifests: BTreeMap<DatasetRef, Manifest>,
    results: Option<PathBuf>,
    depth_limit: usize,
    /// Studies whose plan was informed by their deciding data.
    informed: BTreeSet<String>,
}

impl LineageContext {
    fn dataset(&self, text: &str, depth: usize) -> LineageNode {
        let unknown = || LineageNode {
            label: text.to_string(),
            kind: "unknown".into(),
            mark: Some("[unknown]".into()),
            inputs: Vec::new(),
        };
        let Some(m) = DatasetRef::parse(text).and_then(|r| self.manifests.get(&r)) else {
            return unknown();
        };
        let mut node = LineageNode {
            label: text.to_string(),
            kind: m.kind.as_str().to_string(),
            mark: None,
            inputs: Vec::new(),
        };
        match (m.kind, &m.produced_by) {
            (DatasetKind::Declared, _) => node.mark = Some("[declared, not verified]".into()),
            (DatasetKind::Published, Some(p)) => {
                node.inputs
                    .push(self.stage(&p.study, &p.stage, &p.key, depth + 1))
            }
            _ => {}
        }
        node
    }

    fn stage(&self, study: &str, stage: &str, key: &str, depth: usize) -> LineageNode {
        let mut node = LineageNode {
            label: format!("{study}/{stage}@{key}"),
            kind: "stage".into(),
            mark: None,
            inputs: Vec::new(),
        };
        if depth > self.depth_limit {
            node.mark = Some("[truncated]".into());
            return node;
        }
        // rq-643d82cb
        if self.informed.contains(study) {
            node.mark = Some("[plan informed by its deciding data]".into());
        }
        match recorded_inputs(self.results.as_deref(), study, stage, key) {
            None => node.mark = Some("[unavailable]".into()),
            Some(inputs) => {
                for input in inputs {
                    node.inputs.push(match input {
                        RecordedInput::Stage { stage, key } => {
                            self.stage(study, &stage, &key, depth + 1)
                        }
                        RecordedInput::Dataset(r) => self.dataset(&r, depth + 1),
                    });
                }
            }
        }
        node
    }
}

/// The studies whose `preregistered` block says the plan was informed by its deciding data.
fn informed_studies(root: &Path) -> BTreeSet<String> {
    let dir = root.join(crate::study::STUDIES_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|e| crate::study::parse_study(&e.path()).ok())
        .filter(|s| s.deciding_data_seen())
        .map(|s| s.id)
        .collect()
}

/// Parses `<study>/<stage>@<key>`.
fn parse_stage_target(s: &str) -> Option<(&str, &str, &str)> {
    let (study, rest) = s.split_once('/')?;
    let (stage, key) = rest.split_once('@')?;
    (crate::study::is_valid_study_id(study)
        && crate::study::matches_id_pattern(stage)
        && key.starts_with("sk-"))
    .then_some((study, stage, key))
}

// rq-1a97b91b
/// The lineage tree of a dataset version or a stage output `<study>/<stage>@<key>`.
pub fn lineage(project_root: &Path, target: &str) -> Result<LineageNode, DataError> {
    let project = Project::new(project_root);
    let ctx = LineageContext {
        manifests: super::refs::manifests_by_ref(&project),
        results: load_roots(project_root)?
            .location(RESULTS_ROOT)
            .map(Path::to_path_buf),
        depth_limit: 256,
        informed: informed_studies(project_root),
    };
    if let Some((study, stage, key)) = parse_stage_target(target) {
        return Ok(ctx.stage(study, stage, key, 0));
    }
    let r = DatasetRef::parse(target).ok_or_else(|| DataError::InvalidReference(target.into()))?;
    if !ctx.manifests.contains_key(&r) {
        return Err(DataError::DatasetNotFound(target.into()));
    }
    Ok(ctx.dataset(target, 0))
}

/// The consumers of a dataset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsedBy {
    /// `(study, variant, stage, reference)` for each configuration that references the dataset.
    pub stages: Vec<(String, String, String, String)>,
    /// `(published reference, consumed reference)` for each published dataset whose producing
    /// stage consumed it.
    pub datasets: Vec<(String, String)>,
}

impl UsedBy {
    /// One line per consumer.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (study, variant, stage, r) in &self.stages {
            out.push_str(&format!(
                "study {study} variant {variant} stage {stage} uses {r}\n"
            ));
        }
        for (published, consumed) in &self.datasets {
            out.push_str(&format!(
                "dataset {published} was derived from {consumed}\n"
            ));
        }
        out
    }

    pub fn to_json(&self) -> Value {
        json!({
            "stages": self.stages.iter().map(|(study, variant, stage, r)| json!(
                {"study": study, "variant": variant, "stage": stage, "dataset": r})).collect::<Vec<_>>(),
            "datasets": self.datasets.iter().map(|(published, consumed)| json!(
                {"published": published, "consumed": consumed})).collect::<Vec<_>>(),
        })
    }
}

// rq-23b73104
/// Lists the studies, variants and stages that reference a dataset (or one version of it), and
/// the published datasets derived from it.
pub fn used_by(project_root: &Path, query: &str) -> Result<UsedBy, DataError> {
    let project = Project::new(project_root);
    let manifests = super::refs::manifests_by_ref(&project);
    let (name, version) = if query.contains('@') {
        let r =
            DatasetRef::parse(query).ok_or_else(|| DataError::InvalidReference(query.into()))?;
        (r.name, Some(r.version))
    } else if is_valid_dataset_name(query) {
        (query.to_string(), None)
    } else {
        return Err(DataError::InvalidName(query.into()));
    };
    if !manifests
        .keys()
        .any(|r| r.name == name && version.is_none_or(|v| v == r.version))
    {
        return Err(DataError::DatasetNotFound(query.into()));
    }
    let matches = |text: &str| {
        DatasetRef::parse(text)
            .is_some_and(|r| r.name == name && version.is_none_or(|v| v == r.version))
    };

    let mut result = UsedBy::default();
    for study in project.list_dir(STUDIES_DIR).unwrap_or_default() {
        for r in study_dataset_refs(&project, &study) {
            if matches(&r.text) {
                result.stages.push((r.study, r.variant, r.stage, r.text));
            }
        }
    }
    let results = load_roots(project_root)
        .ok()
        .and_then(|roots| roots.location(RESULTS_ROOT).map(Path::to_path_buf));
    for (published, m) in &manifests {
        let Some(p) = &m.produced_by else { continue };
        for input in
            recorded_inputs(results.as_deref(), &p.study, &p.stage, &p.key).unwrap_or_default()
        {
            if let RecordedInput::Dataset(consumed) = input {
                if matches(&consumed) {
                    result.datasets.push((published.to_string(), consumed));
                }
            }
        }
    }
    result.stages.sort();
    result.stages.dedup();
    result.datasets.sort();
    result.datasets.dedup();
    Ok(result)
}

// rq-4fc6c2a5
/// Publishes a sealed, clean, non-trial stage output as the next version of dataset `name`.
pub fn publish(
    project_root: &Path,
    study: &str,
    stage: &str,
    variant: Option<&str>,
    name: &str,
) -> Result<DatasetRef, DataError> {
    use crate::pipeline::{PipelineError, ProvenanceRecord, ResolvedStage, RECORD_FILE};

    if !is_valid_dataset_name(name) {
        return Err(DataError::InvalidName(name.to_string()));
    }
    let resolved = ResolvedStage::load(project_root, study, variant.unwrap_or("base")).map_err(
        |e| match e {
            PipelineError::UnconfiguredRoot(r) => DataError::UnconfiguredRoot(r),
            other => DataError::InvalidArgument(other.to_string()),
        },
    )?;
    resolved
        .stage(stage)
        .map_err(|e| DataError::InvalidArgument(e.to_string()))?;
    let key = resolved.key(stage).to_string();
    let dir = resolved.out_dir(stage, false).map_err(|e| match e {
        PipelineError::UnconfiguredRoot(r) => DataError::UnconfiguredRoot(r),
        other => DataError::InvalidArgument(other.to_string()),
    })?;
    let record = ProvenanceRecord::read(&dir)
        .filter(|r| r.key == key && !r.trial)
        .ok_or_else(|| DataError::NotSealed(format!("{study}/{stage}@{key}")))?;
    if !record.clean {
        return Err(DataError::NotClean(record.unclean_files.join(", ")));
    }

    // What the records say the outputs are, keyed by path below the output directory.
    let mut expected: BTreeMap<String, (u64, String)> = record
        .outputs
        .iter()
        .map(|o| (o.path.clone(), (o.size, o.sha256.clone())))
        .collect();
    for item in record.items.iter().flatten() {
        let item_dir = dir.join("items").join(&item.name);
        let item_record = ProvenanceRecord::read(&item_dir).ok_or_else(|| {
            DataError::NotSealed(format!("{study}/{stage}@{key} item {}", item.name))
        })?;
        for o in item_record.outputs {
            expected.insert(
                format!("items/{}/{}", item.name, o.path),
                (o.size, o.sha256),
            );
        }
    }

    let files = collect_files(&dir)?;
    let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
    let mut entries = Vec::with_capacity(files.len());
    let mut mismatches = Vec::new();
    for ((rel, _), hashed) in files.iter().zip(hash_files(&paths)) {
        let (size, sha256) = hashed?;
        let is_record = rel == RECORD_FILE || rel.ends_with(&format!("/{RECORD_FILE}"));
        if !is_record {
            match expected.remove(rel) {
                Some((s, h)) if s == size && h == sha256 => {}
                _ => mismatches.push(rel.clone()),
            }
        }
        entries.push(FileEntry {
            path: rel.clone(),
            size,
            sha256,
        });
    }
    mismatches.extend(expected.into_keys());
    if !mismatches.is_empty() {
        mismatches.sort();
        return Err(DataError::OutputMismatch(mismatches));
    }

    let project = Project::new(project_root);
    let study_file = format!(
        "{}/{study}/{}",
        crate::study::STUDIES_DIR,
        crate::study::STUDY_FILE
    );
    let visibility = project
        .read_to_string(&study_file)
        .ok()
        .and_then(|t| crate::study::parse_study_text(study, &study_file, &t).ok())
        .and_then(|s| s.front.visibility.map(|v| v.value))
        .filter(|v| v == "private")
        .unwrap_or_else(|| "public".into());
    let pairs: Vec<(String, String)> = entries
        .iter()
        .map(|e| (e.path.clone(), e.sha256.clone()))
        .collect();
    let manifest = Manifest {
        name: name.to_string(),
        version: next_version(&project, name),
        kind: DatasetKind::Published,
        visibility,
        root: RESULTS_ROOT.to_string(),
        path: format!("{study}/{stage}/{key}"),
        created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        origin: Origin::Published,
        produced_by: Some(super::ProducedBy {
            study: study.to_string(),
            stage: stage.to_string(),
            key,
        }),
        content_hash: content_hash(&pairs),
        files: entries,
        retired: None,
    };
    let reference = manifest.reference();
    let manifest_path = project_root.join(reference.manifest_path());
    fs::create_dir_all(
        manifest_path
            .parent()
            .expect("manifest paths have a parent"),
    )?;
    write_atomic(&manifest_path, &manifest.render())?;

    let mut cache = HashCache::load(project_root);
    for (entry, (_, file)) in manifest.files.iter().zip(&files) {
        cache.record(
            RESULTS_ROOT,
            &format!("{}/{}", manifest.path, entry.path),
            file,
            entry.size,
            &entry.sha256,
        );
    }
    cache.save();
    Ok(reference)
}
