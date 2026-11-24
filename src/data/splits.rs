//! Data splits: the committed registry `datasets/splits.yaml`, and split derivation from lineage.
//! Specified in `rqm/data.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use yaml_rust2::{Yaml, YamlLoader};

use super::refs::manifests_by_ref;
use super::{DataError, DatasetRef, RESULTS_ROOT, is_valid_dataset_name, load_roots};
use crate::check::Project;

/// The split registry, relative to the project root.
pub const SPLITS_FILE: &str = "datasets/splits.yaml";

// rq-9673eeec
/// One registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    pub name: String,
    pub datasets: Vec<DatasetRef>,
    pub partition: Option<String>,
    pub source: Option<DatasetRef>,
}

// rq-bda61e61
/// The parsed split registry, sorted by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Splits {
    pub splits: Vec<Split>,
}

impl Splits {
    pub fn get(&self, name: &str) -> Option<&Split> {
        self.splits.iter().find(|s| s.name == name)
    }
}

fn scalar(value: &Yaml) -> Option<&str> {
    match value {
        Yaml::String(s) => Some(s),
        _ => None,
    }
}

// rq-f9dd9929
/// Parses and validates the split registry.
pub fn parse_splits(text: &str) -> Result<Splits, String> {
    let doc = YamlLoader::load_from_str(text)
        .map_err(|e| format!("invalid YAML: {}", e.info()))?
        .into_iter()
        .next()
        .unwrap_or(Yaml::Null);
    if matches!(doc, Yaml::Null) {
        return Ok(Splits::default());
    }
    let Yaml::Hash(top) = doc else {
        return Err("the registry must be a mapping with `splits`".into());
    };
    let splits = top
        .get(&Yaml::String("splits".into()))
        .unwrap_or(&Yaml::Null);
    let Yaml::Hash(entries) = splits else {
        if matches!(splits, Yaml::Null) {
            return Ok(Splits::default());
        }
        return Err("`splits` must be a mapping".into());
    };
    let mut out = Vec::new();
    for (key, value) in entries {
        let Some(name) = scalar(key) else {
            return Err("split names must be text".into());
        };
        if !is_valid_dataset_name(name) {
            return Err(format!("split name `{name}` is malformed"));
        }
        let Yaml::Hash(entry) = value else {
            return Err(format!("split `{name}` must be a mapping"));
        };
        let mut datasets = Vec::new();
        let mut partition = None;
        let mut source = None;
        for (k, v) in entry {
            let Some(k) = scalar(k) else {
                return Err(format!("split `{name}`: keys must be text"));
            };
            match k {
                "datasets" => {
                    let Yaml::Array(items) = v else {
                        return Err(format!("split `{name}`: `datasets` must be a list"));
                    };
                    for item in items {
                        let text = scalar(item).ok_or_else(|| {
                            format!("split `{name}`: dataset entries must be text")
                        })?;
                        let r = DatasetRef::parse(text).ok_or_else(|| {
                            format!("split `{name}`: `{text}` is not a dataset reference")
                        })?;
                        datasets.push(r);
                    }
                }
                "partition" => {
                    let p = scalar(v)
                        .ok_or_else(|| format!("split `{name}`: `partition` must be text"))?;
                    if !is_valid_dataset_name(p) {
                        return Err(format!("split `{name}`: partition `{p}` is malformed"));
                    }
                    partition = Some(p.to_string());
                }
                "source" => {
                    let s = scalar(v)
                        .ok_or_else(|| format!("split `{name}`: `source` must be text"))?;
                    source = Some(DatasetRef::parse(s).ok_or_else(|| {
                        format!("split `{name}`: `{s}` is not a dataset reference")
                    })?);
                }
                other => {
                    return Err(format!(
                        "split `{name}`: unknown key `{other}`; allowed keys are datasets, partition and source"
                    ));
                }
            }
        }
        if datasets.is_empty() {
            return Err(format!("split `{name}` has no members"));
        }
        out.push(Split {
            name: name.to_string(),
            datasets,
            partition,
            source,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Splits { splits: out })
}

/// Loads the registry. A missing file means no splits.
pub fn load_splits(project: &Project) -> Result<Splits, String> {
    let path = project.root().join(SPLITS_FILE);
    match fs::read_to_string(path) {
        Ok(text) => parse_splits(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Splits::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Renders the registry.
pub fn render_splits(splits: &Splits) -> String {
    if splits.splits.is_empty() {
        return "splits: {}\n".to_string();
    }
    let mut out = String::from("splits:\n");
    for split in &splits.splits {
        out.push_str(&format!("  {}:\n    datasets:\n", split.name));
        for member in &split.datasets {
            out.push_str(&format!("      - {member}\n"));
        }
        if let Some(partition) = &split.partition {
            out.push_str(&format!("    partition: {partition}\n"));
        }
        if let Some(source) = &split.source {
            out.push_str(&format!("    source: {source}\n"));
        }
    }
    out
}

/// Writes the registry atomically.
pub fn write_splits(project: &Project, splits: &Splits) -> Result<(), DataError> {
    crate::fsutil::write_atomic(&project.root().join(SPLITS_FILE), &render_splits(splits))
        .map_err(|e| DataError::Io(e.to_string()))
}

// rq-70fdb927
/// The split a consumed set lies on: the unique split with at least one member in it.
pub fn split_of(splits: &Splits, consumed: &[DatasetRef]) -> Option<String> {
    let matches: Vec<&Split> = splits
        .splits
        .iter()
        .filter(|s| s.datasets.iter().any(|m| consumed.contains(m)))
        .collect();
    match matches.as_slice() {
        [one] => Some(one.name.clone()),
        _ => None,
    }
}

/// The parsed file hashes of a dataset version's manifest.
fn manifest_hashes(project: &Project, r: &DatasetRef) -> Option<BTreeSet<String>> {
    manifests_by_ref(project)
        .get(r)
        .map(|m| m.files.iter().map(|f| f.sha256.clone()).collect())
}

/// Creates or replaces a split, validating partition, containment and disjointness.
pub fn declare_split(
    project: &Project,
    name: &str,
    datasets: Vec<DatasetRef>,
    partition: Option<String>,
    source: Option<DatasetRef>,
) -> Result<(), DataError> {
    if !is_valid_dataset_name(name) {
        return Err(DataError::InvalidName(name.to_string()));
    }
    if datasets.is_empty() {
        return Err(DataError::InvalidArgument(
            "a split needs at least one dataset".into(),
        ));
    }
    if let Some(p) = &partition
        && !is_valid_dataset_name(p)
    {
        return Err(DataError::InvalidName(p.clone()));
    }
    let manifests = manifests_by_ref(project);
    for r in &datasets {
        match manifests.get(r) {
            None => return Err(DataError::DatasetNotFound(r.to_string())),
            Some(m) if m.retired.is_some() => {
                return Err(DataError::RetiredReference(r.to_string()));
            }
            _ => {}
        }
    }
    if let Some(s) = &source
        && !manifests.contains_key(s)
    {
        return Err(DataError::DatasetNotFound(s.to_string()));
    }
    let mut splits = load_splits(project).map_err(|message| DataError::Manifest {
        path: SPLITS_FILE.to_string(),
        message,
    })?;
    if let Some(p) = &partition {
        for other in splits
            .splits
            .iter()
            .filter(|s| s.partition.as_deref() == Some(p.as_str()) && s.name != name)
        {
            if other.source != source {
                return Err(DataError::SplitSourceConflict {
                    partition: p.clone(),
                });
            }
        }
    }
    // Containment.
    if let Some(s) = &source {
        let Some(source_hashes) = manifest_hashes(project, s) else {
            return Err(DataError::DatasetNotFound(s.to_string()));
        };
        for r in &datasets {
            let Some(hashes) = manifest_hashes(project, r) else {
                return Err(DataError::DatasetNotFound(r.to_string()));
            };
            if !hashes.is_subset(&source_hashes) {
                return Err(DataError::SplitNotContained {
                    split: name.to_string(),
                    source: s.to_string(),
                });
            }
        }
    }
    // Disjointness.
    if let Some(p) = &partition {
        let mut member_hashes: BTreeSet<String> = BTreeSet::new();
        for r in &datasets {
            if let Some(hashes) = manifest_hashes(project, r) {
                member_hashes.extend(hashes);
            }
        }
        for other in splits
            .splits
            .iter()
            .filter(|s| s.partition.as_deref() == Some(p.as_str()) && s.name != name)
        {
            let other_hashes: BTreeSet<String> = other
                .datasets
                .iter()
                .filter_map(|r| manifest_hashes(project, r))
                .flatten()
                .collect();
            if member_hashes.intersection(&other_hashes).next().is_some() {
                return Err(DataError::SplitOverlap {
                    split: name.to_string(),
                    other: other.name.clone(),
                });
            }
        }
    }
    splits.splits.retain(|s| s.name != name);
    splits.splits.push(Split {
        name: name.to_string(),
        datasets,
        partition,
        source,
    });
    splits.splits.sort_by(|a, b| a.name.cmp(&b.name));
    write_splits(project, &splits)
}

/// Removes a split from the registry.
pub fn remove_split(project: &Project, name: &str) -> Result<(), DataError> {
    let mut splits = load_splits(project).map_err(|message| DataError::Manifest {
        path: SPLITS_FILE.to_string(),
        message,
    })?;
    let before = splits.splits.len();
    splits.splits.retain(|s| s.name != name);
    if splits.splits.len() == before {
        return Err(DataError::SplitUnknown(name.to_string()));
    }
    write_splits(project, &splits)
}

/// The data structure used to walk provenance records while collecting consumed datasets.
struct Walker {
    results: Option<std::path::PathBuf>,
    manifests: BTreeMap<DatasetRef, super::Manifest>,
    visited: BTreeSet<(String, String, String)>,
    out: Vec<DatasetRef>,
}

impl Walker {
    fn stage(&mut self, study: &str, stage: &str, key: &str) -> Option<()> {
        if !self
            .visited
            .insert((study.into(), stage.into(), key.into()))
        {
            return Some(());
        }
        let results = self.results.as_ref()?;
        let path = results
            .join(study)
            .join(stage)
            .join(key)
            .join("provenance.json");
        let text = fs::read_to_string(path).ok()?;
        let record: serde_json::Value = serde_json::from_str(&text).ok()?;
        let inputs = record.get("inputs")?.as_array()?;
        for input in inputs {
            match input.get("kind").and_then(|k| k.as_str()) {
                Some("stage") => {
                    let s = input.get("stage")?.as_str()?;
                    let k = input.get("key")?.as_str()?;
                    self.stage(study, s, k)?;
                }
                Some("dataset") => {
                    let text = input.get("ref")?.as_str()?;
                    let r = DatasetRef::parse(text)?;
                    if !self.out.contains(&r) {
                        self.out.push(r.clone());
                    }
                    let producing = self.manifests.get(&r).and_then(|m| m.produced_by.clone());
                    if let Some(p) = producing {
                        self.stage(&p.study, &p.stage, &p.key)?;
                    }
                }
                _ => {}
            }
        }
        Some(())
    }
}

// rq-367917d9
/// The dataset versions in the lineage of a stage output, or `Ok(None)` when a record the walk
/// needs is missing or unreadable.
pub fn consumed_datasets(
    project: &Project,
    study: &str,
    stage: &str,
    key: &str,
) -> Result<Option<Vec<DatasetRef>>, DataError> {
    let results = load_roots(project.root())?
        .location(RESULTS_ROOT)
        .map(std::path::Path::to_path_buf);
    let mut walker = Walker {
        results,
        manifests: manifests_by_ref(project),
        visited: BTreeSet::new(),
        out: Vec::new(),
    };
    match walker.stage(study, stage, key) {
        Some(()) => Ok(Some(walker.out)),
        None => Ok(None),
    }
}

/// The names of the splits a dataset version belongs to, sorted.
pub fn splits_of_dataset(splits: &Splits, r: &DatasetRef) -> Vec<String> {
    splits
        .splits
        .iter()
        .filter(|s| s.datasets.contains(r))
        .map(|s| s.name.clone())
        .collect()
}
