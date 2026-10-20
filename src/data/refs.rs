//! Dataset references in study pipeline configurations (`stages[].inputs` values of the form
//! `dataset:<name>@v<n>`), and the producer-maturity check built on them.
//!
//! This reader is deliberately lenient: it extracts references from whatever parses and leaves
//! configuration errors to the pipeline rules.

use std::collections::BTreeMap;

use yaml_rust2::{Yaml, YamlLoader};

use super::manifest::list_manifests;
use super::{DatasetKind, DatasetRef, Manifest};
use crate::check::Project;
use crate::study::{parse_study_text, Status, Study, STUDIES_DIR, STUDY_FILE};

/// One dataset reference in a study configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRef {
    pub study: String,
    /// Project-relative path of the configuration file containing the reference.
    pub file: String,
    /// 1-based line of the reference in `file`.
    pub line: u32,
    pub variant: String,
    pub stage: String,
    pub input: String,
    /// The text after `dataset:`.
    pub text: String,
    /// The parsed reference, when well-formed.
    pub reference: Option<DatasetRef>,
}

/// A stage's dataset inputs in one configuration file: `(stage, input, text, line)`.
type FileInputs = Vec<(String, String, String, u32)>;

/// Extracts `(stage, input, dataset text, line)` from a configuration file's `stages`.
fn file_inputs(text: &str) -> FileInputs {
    let Some(Yaml::Hash(top)) = YamlLoader::load_from_str(text)
        .ok()
        .and_then(|docs| docs.into_iter().next())
    else {
        return Vec::new();
    };
    let Some(Yaml::Array(stages)) = top.get(&Yaml::String("stages".into())) else {
        return Vec::new();
    };
    // Lines with comments removed, for locating references.
    let lines: Vec<&str> = text
        .lines()
        .map(|l| match l.find(" #") {
            Some(i) => &l[..i],
            None if l.trim_start().starts_with('#') => "",
            None => l,
        })
        .collect();
    let mut cursor = 0usize;
    let mut out = Vec::new();
    for stage in stages {
        let Yaml::Hash(stage) = stage else { continue };
        let Some(Yaml::String(name)) = stage.get(&Yaml::String("name".into())) else {
            continue;
        };
        let Some(Yaml::Hash(inputs)) = stage.get(&Yaml::String("inputs".into())) else {
            continue;
        };
        for (input, value) in inputs {
            let (Yaml::String(input), Yaml::String(value)) = (input, value) else {
                continue;
            };
            let Some(reference) = value.strip_prefix("dataset:") else {
                continue;
            };
            let found = (cursor..lines.len())
                .chain(0..cursor)
                .find(|&i| lines[i].contains(value.as_str()));
            if let Some(i) = found {
                cursor = i + 1;
            }
            out.push((
                name.clone(),
                input.clone(),
                reference.to_string(),
                found.map_or(1, |i| i as u32 + 1),
            ));
        }
    }
    out
}

fn config_files(project: &Project, study: &str) -> Vec<(String, String)> {
    let dir = format!("{STUDIES_DIR}/{study}/config");
    project
        .list_dir(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| {
            let variant = f.strip_suffix(".yaml")?.to_string();
            project
                .is_file(&format!("{dir}/{f}"))
                .then(|| (variant, format!("{dir}/{f}")))
        })
        .collect()
}

/// Every dataset reference written in a study's configuration files, one per occurrence. The
/// `variant` is the file's variant (`base` for `config/base.yaml`).
pub(crate) fn config_occurrences(project: &Project, study: &str) -> Vec<ConfigRef> {
    let mut out = Vec::new();
    for (variant, file) in config_files(project, study) {
        let Ok(text) = project.read_to_string(&file) else {
            continue;
        };
        for (stage, input, text, line) in file_inputs(&text) {
            out.push(ConfigRef {
                study: study.to_string(),
                file: file.clone(),
                line,
                variant: variant.clone(),
                stage,
                input,
                reference: DatasetRef::parse(&text),
                text,
            });
        }
    }
    out
}

// rq-f1d5b29a
/// The dataset references in effect for each variant of a study (variant files override inputs of
/// the base stages key by key), one entry per variant, stage and input.
pub fn study_dataset_refs(project: &Project, study: &str) -> Vec<ConfigRef> {
    let occurrences = config_occurrences(project, study);
    let base: Vec<&ConfigRef> = occurrences.iter().filter(|r| r.variant == "base").collect();
    let mut variants: Vec<String> = config_files(project, study)
        .into_iter()
        .map(|(v, _)| v)
        .collect();
    variants.sort();
    let mut out = Vec::new();
    for variant in variants {
        // (stage, input) → reference, starting from base and overridden by the variant file.
        let mut effective: BTreeMap<(String, String), &ConfigRef> = base
            .iter()
            .map(|r| ((r.stage.clone(), r.input.clone()), *r))
            .collect();
        if variant != "base" {
            for r in occurrences.iter().filter(|r| r.variant == variant) {
                effective.insert((r.stage.clone(), r.input.clone()), r);
            }
        }
        out.extend(effective.into_values().map(|r| ConfigRef {
            variant: variant.clone(),
            ..r.clone()
        }));
    }
    out
}

/// Parsed manifests by reference (invalid manifests omitted).
pub(crate) fn manifests_by_ref(project: &Project) -> BTreeMap<DatasetRef, Manifest> {
    list_manifests(project)
        .into_iter()
        .filter_map(|m| m.parsed.ok())
        .map(|m| (m.reference(), m))
        .collect()
}

/// The status of a study, if it exists and parses.
pub(crate) fn study_status(project: &Project, id: &str) -> Option<Status> {
    let file = format!("{STUDIES_DIR}/{id}/{STUDY_FILE}");
    let text = project.read_to_string(&file).ok()?;
    parse_study_text(id, &file, &text).ok()?.status()
}

/// Whether the study has an amendment accepting a dependency on `reference`.
pub(crate) fn accepts(study: &Study, reference: &DatasetRef) -> bool {
    let needle = format!("accept {reference}");
    study.amendments.iter().any(|a| a.reason.contains(&needle))
}

// rq-5512a963
/// Published datasets the study consumes whose producing study is not reported and which the
/// study has not accepted in an amendment, as sorted, deduplicated references.
pub fn unreported_dependencies(project: &Project, study: &Study) -> Vec<DatasetRef> {
    let manifests = manifests_by_ref(project);
    let mut out: Vec<DatasetRef> = config_occurrences(project, &study.id)
        .into_iter()
        .filter_map(|r| r.reference)
        .filter(|r| {
            manifests.get(r).is_some_and(|m| {
                m.kind == DatasetKind::Published
                    && m.produced_by
                        .as_ref()
                        .is_some_and(|p| study_status(project, &p.study) != Some(Status::Reported))
            })
        })
        .filter(|r| !accepts(study, r))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-f1d5b29a
    fn extracts_dataset_inputs_with_lines() {
        let text = "# dataset:ignored@v1\nstages:\n  - name: prepare\n    run: x\n    inputs:\n      \
                    raw: dataset:benchmark-set@v2  # pinned\n      other: stage:x\n  - name: score\n    \
                    inputs: {extra: \"dataset:extra-set@v1\"}\n";
        assert_eq!(
            file_inputs(text),
            vec![
                ("prepare".into(), "raw".into(), "benchmark-set@v2".into(), 6),
                ("score".into(), "extra".into(), "extra-set@v1".into(), 9),
            ]
        );
    }
}
