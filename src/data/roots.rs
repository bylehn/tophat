//! Data roots: declared names and their locations on this machine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use yaml_rust2::{Yaml, YamlLoader};

use super::{DataError, RESULTS_ROOT};

/// Path of the committed roots declaration, relative to the project root.
pub const ROOTS_FILE: &str = ".tophat/roots.yaml";
/// Path of the per-machine root locations, relative to the project root.
pub const LOCAL_ROOTS_FILE: &str = ".tophat/roots.local.yaml";

// rq-bb98136e
/// Declared root names and their locations on this machine (`None` when unconfigured).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    pub declared: BTreeMap<String, Option<PathBuf>>,
}

impl Roots {
    pub fn is_declared(&self, name: &str) -> bool {
        self.declared.contains_key(name)
    }

    /// The location of a root on this machine.
    pub fn location(&self, name: &str) -> Option<&Path> {
        self.declared.get(name).and_then(|l| l.as_deref())
    }

    /// The location of a root, or the reason it is unusable.
    pub fn require(&self, name: &str) -> Result<&Path, DataError> {
        match self.declared.get(name) {
            None => Err(DataError::UnknownRoot(name.to_string())),
            Some(None) => Err(DataError::UnconfiguredRoot(name.to_string())),
            Some(Some(path)) => Ok(path),
        }
    }
}

/// The environment-variable suffix for a root: upper-cased, `-` replaced by `_`.
pub fn env_suffix(name: &str) -> String {
    name.to_ascii_uppercase().replace('-', "_")
}

fn is_valid_root_name(name: &str) -> bool {
    crate::study::matches_id_pattern(name)
}

/// Reads a YAML file's top-level `roots` mapping. A missing file yields `None`.
fn read_roots_mapping(path: &Path) -> Result<Option<Vec<(String, Yaml)>>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let docs = YamlLoader::load_from_str(&text).map_err(|e| format!("invalid YAML: {e}"))?;
    let roots = match docs.into_iter().next() {
        None | Some(Yaml::Null) => return Ok(Some(Vec::new())),
        Some(Yaml::Hash(mut top)) => top.remove(&Yaml::String("roots".into())),
        Some(_) => return Err("expected a mapping with a `roots` key".into()),
    };
    match roots {
        None | Some(Yaml::Null) => Ok(Some(Vec::new())),
        Some(Yaml::Hash(hash)) => hash
            .into_iter()
            .map(|(k, v)| match k {
                Yaml::String(k) => Ok((k, v)),
                _ => Err("root names must be text".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("`roots` must be a mapping".into()),
    }
}

/// Sets a root's location, recording a problem when it is not an absolute path.
fn set_location(
    declared: &mut BTreeMap<String, Option<PathBuf>>,
    name: &str,
    value: &str,
    source: &str,
    problems: &mut Vec<(String, String)>,
) {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        problems.push((
            source.to_string(),
            format!("location of root `{name}` must be an absolute path, not `{value}`"),
        ));
    } else if let Some(slot) = declared.get_mut(name) {
        *slot = Some(path);
    }
}

/// Loads the roots, collecting problems instead of failing: returns the roots that could be
/// determined and `(file, message)` for each problem.
pub(crate) fn load_roots_lenient(
    project: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> (Roots, Vec<(String, String)>) {
    let mut problems = Vec::new();
    let mut declared: BTreeMap<String, Option<PathBuf>> = BTreeMap::new();
    declared.insert(RESULTS_ROOT.to_string(), None);

    match read_roots_mapping(&project.join(ROOTS_FILE)) {
        Ok(entries) => {
            for (name, value) in entries.unwrap_or_default() {
                if !is_valid_root_name(&name) {
                    problems.push((
                        ROOTS_FILE.to_string(),
                        format!("root name `{name}` is malformed"),
                    ));
                    continue;
                }
                if !matches!(value, Yaml::Hash(_) | Yaml::Null) {
                    problems.push((
                        ROOTS_FILE.to_string(),
                        format!("root `{name}` must be a mapping (e.g. with `description`)"),
                    ));
                }
                declared.insert(name, None);
            }
        }
        Err(message) => problems.push((ROOTS_FILE.to_string(), message)),
    }

    match read_roots_mapping(&project.join(LOCAL_ROOTS_FILE)) {
        Ok(entries) => {
            for (name, value) in entries.unwrap_or_default() {
                match value {
                    Yaml::String(location) if declared.contains_key(&name) => set_location(
                        &mut declared,
                        &name,
                        &location,
                        LOCAL_ROOTS_FILE,
                        &mut problems,
                    ),
                    Yaml::String(_) => problems.push((
                        LOCAL_ROOTS_FILE.to_string(),
                        format!("root `{name}` is not declared in {ROOTS_FILE}"),
                    )),
                    _ => problems.push((
                        LOCAL_ROOTS_FILE.to_string(),
                        format!("location of root `{name}` must be a path"),
                    )),
                }
            }
        }
        Err(message) => problems.push((LOCAL_ROOTS_FILE.to_string(), message)),
    }

    let names: Vec<String> = declared.keys().cloned().collect();
    for name in names {
        let var = format!("TOPHAT_ROOT_{}", env_suffix(&name));
        if let Some(value) = env(&var).filter(|v| !v.is_empty()) {
            set_location(&mut declared, &name, &value, &var, &mut problems);
        }
    }
    (Roots { declared }, problems)
}

// rq-2960fd26 rq-e4355ed1 rq-7ec07143
/// Reads the declared roots and resolves their locations from the environment and
/// `.tophat/roots.local.yaml`.
pub fn load_roots(project: &Path) -> Result<Roots, DataError> {
    load_roots_with(project, &|var| std::env::var(var).ok())
}

/// [`load_roots`] with an explicit environment lookup.
pub fn load_roots_with(
    project: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Roots, DataError> {
    let (roots, problems) = load_roots_lenient(project, env);
    match problems.first() {
        None => Ok(roots),
        Some((file, message)) => Err(DataError::Roots(format!("{file}: {message}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(roots: Option<&str>, local: Option<&str>) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tophat-roots-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".tophat")).unwrap();
        if let Some(text) = roots {
            fs::write(dir.join(ROOTS_FILE), text).unwrap();
        }
        if let Some(text) = local {
            fs::write(dir.join(LOCAL_ROOTS_FILE), text).unwrap();
        }
        dir
    }

    #[test] // rq-7ec07143
    fn environment_overrides_local_file() {
        let dir = project(
            Some("roots:\n  raw:\n    description: x\n"),
            Some("roots:\n  raw: /data/local\n"),
        );
        let roots = load_roots_with(&dir, &|var| {
            (var == "TOPHAT_ROOT_RAW").then(|| "/data/elsewhere".to_string())
        })
        .unwrap();
        assert_eq!(roots.location("raw"), Some(Path::new("/data/elsewhere")));
        let roots = load_roots_with(&dir, &|_| None).unwrap();
        assert_eq!(roots.location("raw"), Some(Path::new("/data/local")));
        assert_eq!(roots.location("results"), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test] // rq-e4355ed1
    fn missing_file_declares_only_results() {
        let dir = project(None, None);
        let roots = load_roots_with(&dir, &|_| None).unwrap();
        assert_eq!(roots.declared.keys().collect::<Vec<_>>(), ["results"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test] // rq-7ec07143
    fn rejects_relative_and_undeclared_locations() {
        let dir = project(
            Some("roots:\n  raw: {}\n"),
            Some("roots:\n  raw: data/raw\n"),
        );
        assert!(matches!(
            load_roots_with(&dir, &|_| None),
            Err(DataError::Roots(_))
        ));
        let dir2 = project(Some("roots:\n  raw: {}\n"), Some("roots:\n  rwa: /x\n"));
        assert!(matches!(
            load_roots_with(&dir2, &|_| None),
            Err(DataError::Roots(_))
        ));
        let dir3 = project(Some("roots:\n  Raw: {}\n"), None);
        assert!(load_roots_with(&dir3, &|_| None).is_err());
        for d in [dir, dir2, dir3] {
            fs::remove_dir_all(d).unwrap();
        }
    }
}
