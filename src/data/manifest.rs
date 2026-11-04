//! Dataset manifests: `datasets/<name>/v<n>.yaml`.

use sha2::{Digest, Sha256};
use yaml_rust2::{Yaml, YamlLoader};

use super::{DATASETS_DIR, DatasetRef, is_valid_dataset_name, to_hex};
use crate::check::Project;

// rq-eab2ab92 rq-4c85d6f4
/// How a dataset came to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetKind {
    Raw,
    Declared,
    Published,
}

impl DatasetKind {
    pub fn parse(s: &str) -> Option<DatasetKind> {
        Some(match s {
            "raw" => DatasetKind::Raw,
            "declared" => DatasetKind::Declared,
            "published" => DatasetKind::Published,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DatasetKind::Raw => "raw",
            DatasetKind::Declared => "declared",
            DatasetKind::Published => "published",
        }
    }
}

/// One file of a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative, `/`-separated path below the dataset directory.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Where a raw or declared dataset came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Raw {
        source: String,
        date: Option<String>,
        license: Option<String>,
    },
    Declared {
        statement: String,
    },
    Published,
}

/// The stage output a published dataset was taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedBy {
    pub study: String,
    pub stage: String,
    pub key: String,
}

/// The `retired` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retired {
    pub date: String,
    pub reason: String,
}

// rq-b2fecc6c rq-011facdf
/// A parsed, validated manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: u32,
    pub kind: DatasetKind,
    pub visibility: String,
    pub root: String,
    pub path: String,
    pub created: String,
    pub origin: Origin,
    pub produced_by: Option<ProducedBy>,
    pub files: Vec<FileEntry>,
    pub content_hash: String,
    pub retired: Option<Retired>,
}

impl Manifest {
    pub fn reference(&self) -> DatasetRef {
        DatasetRef {
            name: self.name.clone(),
            version: self.version,
        }
    }

    /// Renders the manifest as YAML. Strings are double-quoted (JSON escaping is valid YAML).
    pub fn render(&self) -> String {
        let q = |s: &str| serde_json::to_string(s).expect("strings serialize");
        let mut out = format!(
            "dataset: {}\nversion: {}\nkind: {}\nvisibility: {}\nroot: {}\npath: {}\ncreated: {}\n",
            q(&self.name),
            self.version,
            self.kind.as_str(),
            self.visibility,
            q(&self.root),
            q(&self.path),
            q(&self.created)
        );
        match &self.origin {
            Origin::Raw {
                source,
                date,
                license,
            } => {
                out.push_str(&format!("origin:\n  source: {}\n", q(source)));
                if let Some(date) = date {
                    out.push_str(&format!("  date: {}\n", q(date)));
                }
                if let Some(license) = license {
                    out.push_str(&format!("  license: {}\n", q(license)));
                }
            }
            Origin::Declared { statement } => {
                out.push_str(&format!("origin:\n  statement: {}\n", q(statement)))
            }
            Origin::Published => {}
        }
        if let Some(p) = &self.produced_by {
            out.push_str(&format!(
                "produced_by:\n  study: {}\n  stage: {}\n  key: {}\n",
                q(&p.study),
                q(&p.stage),
                q(&p.key)
            ));
        }
        out.push_str("files:\n");
        for f in &self.files {
            out.push_str(&format!(
                "  - path: {}\n    size: {}\n    sha256: {}\n",
                q(&f.path),
                f.size,
                q(&f.sha256)
            ));
        }
        out.push_str(&format!("content_hash: {}\n", q(&self.content_hash)));
        if let Some(r) = &self.retired {
            out.push_str(&retired_block(r));
        }
        out
    }
}

/// The YAML lines of a `retired` block.
pub(super) fn retired_block(r: &Retired) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("strings serialize");
    format!(
        "retired:\n  date: {}\n  reason: {}\n",
        q(&r.date),
        q(&r.reason)
    )
}

// rq-da8648ca
/// The `content_hash` of `(path, sha256)` pairs: SHA-256 over `<sha256>  <path>\n` lines sorted by
/// path.
pub fn content_hash(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (path, sha) in sorted {
        hasher.update(format!("{sha}  {path}\n").as_bytes());
    }
    format!("sha256:{}", to_hex(&hasher.finalize()))
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `s` is a relative, `/`-separated path with no empty, `.` or `..` segments.
pub(crate) fn is_safe_relative_path(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s.contains('\\')
        && s.split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn is_stage_key(s: &str) -> bool {
    s.strip_prefix("sk-").is_some_and(|hex| {
        hex.len() == 16
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn is_date(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Typed access to a YAML mapping, consuming known keys so unknown ones can be reported.
struct Fields {
    entries: Vec<(String, Yaml)>,
    context: &'static str,
}

impl Fields {
    fn from(value: Yaml, context: &'static str) -> Result<Fields, String> {
        match value {
            Yaml::Hash(hash) => Ok(Fields {
                entries: hash
                    .into_iter()
                    .map(|(k, v)| match k {
                        Yaml::String(k) => Ok((k, v)),
                        _ => Err(format!("{context}: keys must be text")),
                    })
                    .collect::<Result<_, _>>()?,
                context,
            }),
            _ => Err(format!("{context} must be a mapping")),
        }
    }

    fn take(&mut self, key: &str) -> Option<Yaml> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(i).1)
    }

    fn string(&mut self, key: &str) -> Result<String, String> {
        match self.take(key) {
            Some(Yaml::String(s)) => Ok(s),
            Some(_) => Err(format!("{}: `{key}` must be text", self.context)),
            None => Err(format!("{}: `{key}` is missing", self.context)),
        }
    }

    fn optional_string(&mut self, key: &str) -> Result<Option<String>, String> {
        match self.take(key) {
            None => Ok(None),
            Some(Yaml::String(s)) => Ok(Some(s)),
            Some(_) => Err(format!("{}: `{key}` must be text", self.context)),
        }
    }

    fn integer(&mut self, key: &str) -> Result<i64, String> {
        match self.take(key) {
            Some(Yaml::Integer(i)) => Ok(i),
            Some(_) => Err(format!("{}: `{key}` must be a whole number", self.context)),
            None => Err(format!("{}: `{key}` is missing", self.context)),
        }
    }

    fn finish(self) -> Result<(), String> {
        match self.entries.first() {
            None => Ok(()),
            Some((k, _)) => Err(format!("{}: unknown field `{k}`", self.context)),
        }
    }
}

// rq-adfff14c rq-011facdf
/// Parses and validates a manifest.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let doc = YamlLoader::load_from_str(text)
        .map_err(|e| format!("invalid YAML: {e}"))?
        .into_iter()
        .next()
        .ok_or("the manifest is empty")?;
    let mut f = Fields::from(doc, "manifest")?;

    let name = f.string("dataset")?;
    if !is_valid_dataset_name(&name) {
        return Err(format!("`dataset: {name}` is not a valid dataset name"));
    }
    let version = f.integer("version")?;
    let version = u32::try_from(version)
        .ok()
        .filter(|v| *v >= 1)
        .ok_or("`version` must be a positive whole number")?;
    let kind_text = match f.take("kind") {
        Some(Yaml::String(s)) => s,
        _ => return Err("`kind` must be raw, declared or published".into()),
    };
    let kind = DatasetKind::parse(&kind_text).ok_or("`kind` must be raw, declared or published")?;
    let visibility = match f.take("visibility") {
        Some(Yaml::String(s)) if s == "public" || s == "private" => s,
        _ => return Err("`visibility` must be public or private".into()),
    };
    let root = f.string("root")?;
    if !crate::study::matches_id_pattern(&root) {
        return Err(format!("`root: {root}` is not a valid root name"));
    }
    let path = f.string("path")?;
    if !is_safe_relative_path(&path) {
        return Err(format!(
            "`path: {path}` must be a relative path inside the root"
        ));
    }
    let created = f.string("created")?;
    if chrono::DateTime::parse_from_rfc3339(&created).is_err() {
        return Err("`created` must be an RFC 3339 timestamp".into());
    }

    let origin = match (kind, f.take("origin")) {
        (DatasetKind::Raw, Some(value)) => {
            let mut o = Fields::from(value, "origin")?;
            let source = o.string("source")?;
            let date = o.optional_string("date")?;
            let license = o.optional_string("license")?;
            o.finish()?;
            if source.trim().is_empty() {
                return Err("`origin.source` is empty".into());
            }
            if date.as_deref().is_some_and(|d| !is_date(d)) {
                return Err("`origin.date` must be YYYY-MM-DD".into());
            }
            Origin::Raw {
                source,
                date,
                license,
            }
        }
        (DatasetKind::Declared, Some(value)) => {
            let mut o = Fields::from(value, "origin")?;
            let statement = o.string("statement")?;
            o.finish()?;
            if statement.trim().is_empty() {
                return Err("`origin.statement` is empty".into());
            }
            Origin::Declared { statement }
        }
        (DatasetKind::Published, None) => Origin::Published,
        (DatasetKind::Published, Some(_)) => {
            return Err("a published dataset has `produced_by`, not `origin`".into());
        }
        (_, None) => return Err(format!("a {kind_text} dataset needs `origin`")),
    };

    let produced_by = match (kind, f.take("produced_by")) {
        (DatasetKind::Published, Some(value)) => {
            let mut p = Fields::from(value, "produced_by")?;
            let produced = ProducedBy {
                study: p.string("study")?,
                stage: p.string("stage")?,
                key: p.string("key")?,
            };
            p.finish()?;
            if !is_stage_key(&produced.key) {
                return Err("`produced_by.key` is not a stage key".into());
            }
            Some(produced)
        }
        (DatasetKind::Published, None) => {
            return Err("a published dataset needs `produced_by`".into());
        }
        (_, Some(_)) => return Err("only published datasets have `produced_by`".into()),
        (_, None) => None,
    };

    let files = match f.take("files") {
        Some(Yaml::Array(items)) => items
            .into_iter()
            .map(|item| {
                let mut e = Fields::from(item, "files entry")?;
                let path = e.string("path")?;
                let size = u64::try_from(e.integer("size")?)
                    .map_err(|_| "files entry: `size` must not be negative".to_string())?;
                let sha256 = e.string("sha256")?;
                e.finish()?;
                if !is_safe_relative_path(&path) {
                    return Err(format!("file path `{path}` must be relative"));
                }
                if !is_hex64(&sha256) {
                    return Err(format!("file `{path}` has a malformed sha256"));
                }
                Ok(FileEntry { path, size, sha256 })
            })
            .collect::<Result<Vec<_>, String>>()?,
        _ => return Err("`files` must be a list".into()),
    };
    if files.is_empty() {
        return Err("`files` is empty".into());
    }
    if files.windows(2).any(|w| w[0].path >= w[1].path) {
        return Err("`files` must be sorted by path without duplicates".into());
    }

    let content = f.string("content_hash")?;
    let pairs: Vec<(String, String)> = files
        .iter()
        .map(|e| (e.path.clone(), e.sha256.clone()))
        .collect();
    if content != content_hash(&pairs) {
        return Err("`content_hash` does not match `files`".into());
    }

    let retired = match f.take("retired") {
        None => None,
        Some(value) => {
            let mut r = Fields::from(value, "retired")?;
            let retired = Retired {
                date: r.string("date")?,
                reason: r.string("reason")?,
            };
            r.finish()?;
            if !is_date(&retired.date) || retired.reason.trim().is_empty() {
                return Err("`retired` needs a YYYY-MM-DD date and a reason".into());
            }
            Some(retired)
        }
    };
    f.finish()?;

    Ok(Manifest {
        name,
        version,
        kind,
        visibility,
        root,
        path,
        created,
        origin,
        produced_by,
        files,
        content_hash: content,
        retired,
    })
}

/// A manifest file found under `datasets/`.
#[derive(Debug, Clone)]
pub struct ManifestFile {
    /// Project-relative path.
    pub path: String,
    /// The directory name (dataset name).
    pub dir_name: String,
    /// The version from the file name, when it has the form `v<n>.yaml`.
    pub file_version: Option<u32>,
    pub parsed: Result<Manifest, String>,
}

/// Every `*.yaml` file directly inside a `datasets/<name>/` directory, sorted by path.
pub fn list_manifests(project: &Project) -> Vec<ManifestFile> {
    let mut out = Vec::new();
    for dir_name in project.list_dir(DATASETS_DIR).unwrap_or_default() {
        let dir = format!("{DATASETS_DIR}/{dir_name}");
        if !project.is_dir(&dir) {
            continue;
        }
        for file in project.list_dir(&dir).unwrap_or_default() {
            let Some(stem) = file.strip_suffix(".yaml") else {
                continue;
            };
            let path = format!("{dir}/{file}");
            let file_version = stem
                .strip_prefix('v')
                .filter(|n| {
                    !n.starts_with('0') && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
                })
                .and_then(|n| n.parse().ok());
            let parsed = project
                .read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| parse_manifest(&text));
            out.push(ManifestFile {
                path,
                dir_name: dir_name.clone(),
                file_version,
                parsed,
            });
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn sample() -> Manifest {
        let files = vec![
            FileEntry {
                path: "1abc.pdb".into(),
                size: 3,
                sha256: "a".repeat(64),
            },
            FileEntry {
                path: "sub/2xyz.pdb".into(),
                size: 5,
                sha256: "b".repeat(64),
            },
        ];
        let pairs: Vec<_> = files
            .iter()
            .map(|f| (f.path.clone(), f.sha256.clone()))
            .collect();
        Manifest {
            name: "benchmark-set".into(),
            version: 1,
            kind: DatasetKind::Raw,
            visibility: "public".into(),
            root: "raw".into(),
            path: "benchmark/2024-05".into(),
            created: "2026-10-20T09:12:00Z".into(),
            origin: Origin::Raw {
                source: "PDBbind \"refined\"".into(),
                date: Some("2024-05-02".into()),
                license: None,
            },
            produced_by: None,
            content_hash: content_hash(&pairs),
            files,
            retired: None,
        }
    }

    #[test] // rq-011facdf rq-adfff14c
    fn render_and_parse_round_trip() {
        let m = sample();
        assert_eq!(parse_manifest(&m.render()), Ok(m.clone()));
        let mut retired = m;
        retired.retired = Some(Retired {
            date: "2026-11-02".into(),
            reason: "wrong".into(),
        });
        assert_eq!(parse_manifest(&retired.render()), Ok(retired));
    }

    #[test] // rq-011facdf
    fn rejects_invalid_manifests() {
        let text = sample().render();
        for (from, to) in [
            ("kind: raw", "kind: cooked"),
            ("version: 1", "version: 0"),
            ("path: \"benchmark/2024-05\"", "path: \"../x\""),
            ("  source: \"PDBbind \\\"refined\\\"\"\n", ""),
            ("content_hash: \"sha256:", "content_hash: \"sha256:0"),
            ("visibility: public", "visibility: secret"),
            (
                "created: \"2026-10-20T09:12:00Z\"",
                "created: \"yesterday\"",
            ),
            (
                "dataset: \"benchmark-set\"",
                "dataset: \"benchmark-set\"\nextra: 1",
            ),
        ] {
            let bad = text.replacen(from, to, 1);
            assert_ne!(bad, text, "fixture lacks {from:?}");
            assert!(
                parse_manifest(&bad).is_err(),
                "accepted after replacing {from:?}"
            );
        }
    }

    #[test] // rq-da8648ca
    fn content_hash_is_order_independent_and_specified() {
        let a = vec![
            ("b".to_string(), "2".repeat(64)),
            ("a".to_string(), "1".repeat(64)),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(content_hash(&a), content_hash(&b));
        let expected_input = format!("{}  a\n{}  b\n", "1".repeat(64), "2".repeat(64));
        assert_eq!(
            content_hash(&a),
            format!(
                "sha256:{}",
                to_hex(&Sha256::digest(expected_input.as_bytes()))
            )
        );
    }

    #[test]
    fn safe_relative_paths() {
        assert!(is_safe_relative_path("a/b.pdb"));
        for bad in ["", "/a", "a/../b", "./a", "a//b", "..", "a\\b"] {
            assert!(!is_safe_relative_path(bad), "{bad}");
        }
    }
}
