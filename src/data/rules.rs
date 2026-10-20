//! The `data` rule family evaluated by `tophat check`.

use std::collections::{BTreeMap, BTreeSet};

use super::cache::{mtime_ns, HashCache};
use super::manifest::list_manifests;
use super::refs::{accepts, config_occurrences, manifests_by_ref, study_status};
use super::roots::{load_roots_lenient, ROOTS_FILE};
use super::{DatasetKind, DatasetRef, DATASETS_DIR};
use crate::check::{Diagnostic, Project, Severity};
use crate::study::{parse_study_text, Status, STUDIES_DIR, STUDY_FILE};

fn diag(
    rule: &'static str,
    severity: Severity,
    path: &str,
    line: Option<u32>,
    message: String,
    hint: &str,
) -> Diagnostic {
    Diagnostic {
        rule,
        severity,
        path: path.to_string(),
        line,
        message,
        hint: Some(hint.to_string()),
    }
}

// rq-b2eccf77 rq-5b6f6e50
/// The `data` rule family.
pub fn data_rules(project: &Project) -> Vec<Diagnostic> {
    data_rules_with(project, &|var| std::env::var(var).ok())
}

/// [`data_rules`] with an explicit environment lookup for root locations.
pub fn data_rules_with(project: &Project, env: &dyn Fn(&str) -> Option<String>) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let (roots, problems) = load_roots_lenient(project.root(), env);
    for (file, message) in problems {
        out.push(diag(
            "data.roots-invalid",
            Severity::Error,
            &file,
            None,
            message,
            "Fix the roots file: names use lowercase letters, digits and hyphens; locations are absolute paths",
        ));
    }
    check_manifests(project, &roots, &mut out);
    check_references(project, &mut out);
    out
}

// rq-011facdf rq-84e20fdc
fn check_manifests(project: &Project, roots: &super::Roots, out: &mut Vec<Diagnostic>) {
    let cache = HashCache::load(project.root());
    let mut versions: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut unconfigured_reported: BTreeSet<String> = BTreeSet::new();

    for file in list_manifests(project) {
        let manifest = match (&file.parsed, file.file_version) {
            (_, None) => {
                out.push(diag(
                    "data.manifest-invalid",
                    Severity::Error,
                    &file.path,
                    None,
                    "Manifest file name is not of the form `v<n>.yaml`".into(),
                    "Name manifests v1.yaml, v2.yaml, …",
                ));
                continue;
            }
            (Err(message), Some(_)) => {
                out.push(diag(
                    "data.manifest-invalid",
                    Severity::Error,
                    &file.path,
                    None,
                    format!("Manifest is invalid: {message}"),
                    "Manifests are written by `tophat data`; restore it from version control",
                ));
                continue;
            }
            (Ok(m), Some(_)) => m,
        };
        let file_version = file.file_version.expect("checked above");
        versions
            .entry(file.dir_name.clone())
            .or_default()
            .insert(file_version);
        if manifest.name != file.dir_name || manifest.version != file_version {
            out.push(diag(
                "data.version-mismatch",
                Severity::Error,
                &file.path,
                None,
                format!(
                    "Manifest says `{}@v{}` but its file is `{}`",
                    manifest.name, manifest.version, file.path
                ),
                "Make the dataset name and version agree with the file name",
            ));
        }

        let Some(location) = roots.declared.get(&manifest.root) else {
            out.push(diag(
                "data.root-unknown",
                Severity::Error,
                &file.path,
                None,
                format!("Root `{}` is not declared", manifest.root),
                "Declare the root in .tophat/roots.yaml",
            ));
            continue;
        };
        let Some(location) = location else {
            if unconfigured_reported.insert(manifest.root.clone()) {
                out.push(diag(
                    "data.root-unconfigured",
                    Severity::Warning,
                    ROOTS_FILE,
                    None,
                    format!(
                        "Root `{}` is not configured on this machine; its data cannot be checked",
                        manifest.root
                    ),
                    "Set TOPHAT_ROOT_<NAME> or add the root to .tophat/roots.local.yaml",
                ));
            }
            continue;
        };

        let dir = location.join(&manifest.path);
        for entry in &manifest.files {
            let path = dir.join(&entry.path);
            let meta = std::fs::symlink_metadata(&path)
                .ok()
                .filter(|m| m.is_file());
            let Some(meta) = meta else {
                out.push(diag(
                    "data.file-missing",
                    Severity::Error,
                    &file.path,
                    None,
                    format!(
                        "File `{}` is missing from root `{}`",
                        entry.path, manifest.root
                    ),
                    "Restore the file, or retire this dataset version",
                ));
                continue;
            };
            if meta.len() != entry.size {
                out.push(diag(
                    "data.file-changed",
                    Severity::Error,
                    &file.path,
                    None,
                    format!(
                        "File `{}` has size {} but the manifest records {}",
                        entry.path,
                        meta.len(),
                        entry.size
                    ),
                    "Data is never modified in place; restore the file or register a new version",
                ));
                continue;
            }
            let rel = format!("{}/{}", manifest.path, entry.path);
            if let Some(cached) = cache.get(&manifest.root, &rel) {
                if cached.size == entry.size && Some(cached.mtime_ns) != mtime_ns(&path) {
                    out.push(diag(
                        "data.unverified",
                        Severity::Warning,
                        &file.path,
                        None,
                        format!(
                            "File `{}` was modified since it was last verified",
                            entry.path
                        ),
                        &format!("Run `tophat data verify {}`", manifest.reference()),
                    ));
                }
            }
        }
    }

    for (name, present) in versions {
        let max = *present.iter().max().expect("non-empty");
        if present.len() as u32 != max {
            out.push(diag(
                "data.version-gap",
                Severity::Error,
                &format!("{DATASETS_DIR}/{name}"),
                None,
                format!("Versions of `{name}` are not numbered 1 to {max} without gaps"),
                "Restore the missing manifests; versions are never deleted",
            ));
        }
    }
}

// rq-f1d5b29a rq-5512a963
fn check_references(project: &Project, out: &mut Vec<Diagnostic>) {
    let manifests = manifests_by_ref(project);
    let mut newest: BTreeMap<&str, u32> = BTreeMap::new();
    for (r, m) in &manifests {
        if m.retired.is_none() {
            let v = newest.entry(r.name.as_str()).or_default();
            *v = (*v).max(r.version);
        }
    }
    let existing: BTreeSet<DatasetRef> = list_manifests(project)
        .into_iter()
        .filter_map(|f| {
            Some(DatasetRef {
                name: f.dir_name,
                version: f.file_version?,
            })
        })
        .collect();

    for id in project.list_dir(STUDIES_DIR).unwrap_or_default() {
        let study_file = format!("{STUDIES_DIR}/{id}/{STUDY_FILE}");
        let Some(study) = project
            .read_to_string(&study_file)
            .ok()
            .and_then(|t| parse_study_text(&id, &study_file, &t).ok())
        else {
            continue;
        };
        let status = study.status();
        let public = study
            .front
            .visibility
            .as_ref()
            .is_some_and(|v| v.value == "public");

        for occurrence in config_occurrences(project, &id) {
            let at = |rule, severity, message: String, hint: &str| {
                diag(
                    rule,
                    severity,
                    &occurrence.file,
                    Some(occurrence.line),
                    message,
                    hint,
                )
            };
            let Some(reference) = &occurrence.reference else {
                out.push(at(
                    "data.reference-unknown",
                    Severity::Error,
                    format!(
                        "`dataset:{}` is not a valid dataset reference",
                        occurrence.text
                    ),
                    "Use dataset:<name>@v<n>",
                ));
                continue;
            };
            let Some(manifest) = manifests.get(reference) else {
                if !existing.contains(reference) {
                    out.push(at(
                        "data.reference-unknown",
                        Severity::Error,
                        format!("Dataset `{reference}` does not exist"),
                        "Register or publish it first, or fix the reference",
                    ));
                }
                continue;
            };
            if manifest.retired.is_some() {
                out.push(at(
                    "data.reference-retired",
                    Severity::Error,
                    format!("Dataset `{reference}` is retired"),
                    "Pin a non-retired version, recorded as an amendment or a new variant",
                ));
            }
            if matches!(status, Some(Status::Draft | Status::Preregistered))
                && newest
                    .get(reference.name.as_str())
                    .is_some_and(|&n| n > reference.version)
            {
                out.push(at(
                    "data.newer-version",
                    Severity::Warning,
                    format!(
                        "A newer version of `{}` exists (v{})",
                        reference.name,
                        newest[reference.name.as_str()]
                    ),
                    "Consider pinning the newer version",
                ));
            }
            if manifest.kind == DatasetKind::Published {
                let producer = manifest.produced_by.as_ref().map(|p| p.study.as_str());
                let reported = producer
                    .and_then(|p| study_status(project, p))
                    .is_some_and(|s| s == Status::Reported);
                if !reported {
                    let blocking = status == Some(Status::Reported) && !accepts(&study, reference);
                    out.push(at(
                        "data.producer-unreported",
                        if blocking { Severity::Error } else { Severity::Warning },
                        format!(
                            "`{reference}` comes from study `{}`, which is not reported",
                            producer.unwrap_or("?")
                        ),
                        &format!(
                            "Report the producing study first, or accept it with an amendment containing `accept {reference}`"
                        ),
                    ));
                }
            }
            if public && manifest.visibility == "private" {
                out.push(at(
                    "data.visibility-leak",
                    Severity::Error,
                    format!("Public study consumes private dataset `{reference}`"),
                    "Make the study private, or move it to the private companion repository",
                ));
            }
            if manifest.kind == DatasetKind::Declared {
                out.push(at(
                    "data.declared-input",
                    Severity::Legacy,
                    format!("`{reference}` is declared data; its history is not verified"),
                    "Re-derive it from raw data in a study when possible",
                ));
            }
        }
    }
}
