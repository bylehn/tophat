//! Data roots, dataset manifests, hashing and lineage. Specified in `rqm/data.md`.

mod cache;
mod commands;
mod manifest;
mod refs;
mod roots;
mod rules;

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

pub use cache::HashCache;
pub use commands::{
    add, declare, lineage, retire, used_by, verify, LineageNode, RegisterOptions, UsedBy,
    VerifyReport,
};
pub use manifest::{
    content_hash, list_manifests, parse_manifest, DatasetKind, FileEntry, Manifest, ManifestFile,
    Origin, ProducedBy, Retired,
};
pub use refs::{study_dataset_refs, unreported_dependencies, ConfigRef};
pub use roots::{load_roots, load_roots_with, Roots};
pub use rules::{data_rules, data_rules_with};

/// Directory, relative to the project root, holding one subdirectory of manifests per dataset.
pub const DATASETS_DIR: &str = "datasets";
/// The data root that holds stage outputs.
pub const RESULTS_ROOT: &str = "results";

// rq-0f8bd6f3
/// A parsed dataset reference `<name>@v<n>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DatasetRef {
    pub name: String,
    pub version: u32,
}

impl DatasetRef {
    /// Parses `<name>@v<n>` with a valid dataset name and a positive version without leading zeros.
    pub fn parse(s: &str) -> Option<DatasetRef> {
        let (name, version) = s.split_once("@v")?;
        if !is_valid_dataset_name(name)
            || version.is_empty()
            || version.starts_with('0')
            || !version.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        Some(DatasetRef {
            name: name.to_string(),
            version: version.parse().ok()?,
        })
    }

    /// The project-relative path of this version's manifest.
    pub fn manifest_path(&self) -> String {
        format!("{DATASETS_DIR}/{}/v{}.yaml", self.name, self.version)
    }
}

impl fmt::Display for DatasetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@v{}", self.name, self.version)
    }
}

/// Whether `s` is a valid dataset name (the study ID pattern, 3 to 64 characters).
pub fn is_valid_dataset_name(s: &str) -> bool {
    crate::study::is_valid_study_id(s)
}

// rq-bd19d141
/// Error returned by the data commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    InvalidName(String),
    InvalidReference(String),
    InvalidArgument(String),
    UnknownRoot(String),
    UnconfiguredRoot(String),
    PathOutsideRoot(String),
    NoFiles(String),
    SymbolicLink(String),
    DatasetNotFound(String),
    AlreadyRetired(String),
    NotSealed(String),
    NotClean(String),
    OutputMismatch(Vec<String>),
    /// `.tophat/roots.yaml` or `.tophat/roots.local.yaml` is invalid.
    Roots(String),
    Manifest {
        path: String,
        message: String,
    },
    Io(String),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::InvalidName(n) => write!(
                f,
                "invalid dataset name `{n}`: use 3 to 64 lowercase letters, digits and single hyphens"
            ),
            DataError::InvalidReference(r) => {
                write!(f, "invalid dataset reference `{r}`: expected <name>@v<n>")
            }
            DataError::InvalidArgument(m) => write!(f, "{m}"),
            DataError::UnknownRoot(r) => {
                write!(f, "root `{r}` is not declared in .tophat/roots.yaml")
            }
            DataError::UnconfiguredRoot(r) => write!(
                f,
                "root `{r}` is not configured on this machine: set TOPHAT_ROOT_{} or add it to \
                 .tophat/roots.local.yaml",
                roots::env_suffix(r)
            ),
            DataError::PathOutsideRoot(p) => {
                write!(f, "path `{p}` must be relative and stay inside the root")
            }
            DataError::NoFiles(p) => write!(f, "`{p}` contains no files"),
            DataError::SymbolicLink(p) => {
                write!(f, "`{p}` is a symbolic link; datasets must contain only regular files")
            }
            DataError::DatasetNotFound(r) => write!(f, "dataset `{r}` does not exist"),
            DataError::AlreadyRetired(r) => write!(f, "dataset `{r}` is already retired"),
            DataError::NotSealed(m) => write!(f, "stage output is not sealed: {m}"),
            DataError::NotClean(m) => write!(f, "stage output was produced from uncommitted code: {m}"),
            DataError::OutputMismatch(files) => write!(
                f,
                "stage output files differ from their provenance record: {}",
                files.join(", ")
            ),
            DataError::Roots(m) => write!(f, "invalid data roots: {m}"),
            DataError::Manifest { path, message } => write!(f, "{path}: {message}"),
            DataError::Io(m) => write!(f, "I/O error: {m}"),
        }
    }
}

impl std::error::Error for DataError {}

impl From<io::Error> for DataError {
    fn from(e: io::Error) -> Self {
        DataError::Io(e.to_string())
    }
}

// rq-6f389d0b
/// Size and lowercase hex SHA-256 of a file, read in fixed-size chunks.
pub fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buffer[..n]);
    }
    Ok((size, to_hex(&hasher.finalize())))
}

/// Lowercase hex encoding.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hashes many files in parallel, preserving order.
pub(crate) fn hash_files(paths: &[std::path::PathBuf]) -> Vec<io::Result<(u64, String)>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(paths.len().max(1));
    let mut results: Vec<Option<io::Result<(u64, String)>>> =
        (0..paths.len()).map(|_| None).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots = std::sync::Mutex::new(&mut results);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= paths.len() {
                    break;
                }
                let result = hash_file(&paths[i]);
                slots.lock().expect("no panics while holding the lock")[i] = Some(result);
            });
        }
    });
    results
        .into_iter()
        .map(|r| r.expect("every index is hashed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-0f8bd6f3
    fn dataset_references() {
        assert_eq!(
            DatasetRef::parse("benchmark-set@v2"),
            Some(DatasetRef {
                name: "benchmark-set".into(),
                version: 2
            })
        );
        for bad in [
            "benchmark-set",
            "benchmark-set@v0",
            "benchmark-set@v02",
            "ab@v1",
            "X@v1",
            "set@2",
        ] {
            assert_eq!(DatasetRef::parse(bad), None, "{bad}");
        }
        assert_eq!(DatasetRef::parse("abc@v10").unwrap().to_string(), "abc@v10");
    }

    #[test] // rq-6f389d0b
    fn hashes_files() {
        let dir = std::env::temp_dir().join(format!("tophat-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("abc.txt");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            hash_file(&path).unwrap(),
            (
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string()
            )
        );
        let many: Vec<_> = (0..20).map(|_| path.clone()).collect();
        assert!(hash_files(&many).iter().all(|r| r.as_ref().unwrap().0 == 3));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
