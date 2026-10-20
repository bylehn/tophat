//! The per-machine hash cache `.tophat/cache/hashes.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};

use crate::fsutil::write_atomic_unique;

/// Path of the hash cache, relative to the project root.
pub const CACHE_FILE: &str = ".tophat/cache/hashes.json";

/// A cached hash of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime_ns: u64,
    pub sha256: String,
    pub verified: String,
}

// rq-731766dd
/// The hash cache, keyed by `<root>:<path>` where `path` is relative to the root.
#[derive(Debug, Clone, Default)]
pub struct HashCache {
    path: PathBuf,
    entries: BTreeMap<String, CacheEntry>,
}

/// Modification time of a file in nanoseconds since the Unix epoch.
pub(crate) fn mtime_ns(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    u64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_nanos()).ok()
}

fn key(root: &str, rel: &str) -> String {
    format!("{root}:{rel}")
}

impl HashCache {
    /// Loads the cache of a project. A missing or unreadable cache is empty.
    pub fn load(project: &Path) -> HashCache {
        let path = project.join(CACHE_FILE);
        let mut entries = BTreeMap::new();
        let parsed: Option<Value> = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        if let Some(Value::Object(map)) = parsed.as_ref().and_then(|v| v.get("entries")) {
            for (k, v) in map {
                let entry = (|| {
                    Some(CacheEntry {
                        size: v.get("size")?.as_u64()?,
                        mtime_ns: v.get("mtime_ns")?.as_u64()?,
                        sha256: v.get("sha256")?.as_str()?.to_string(),
                        verified: v.get("verified")?.as_str()?.to_string(),
                    })
                })();
                if let Some(entry) = entry {
                    entries.insert(k.clone(), entry);
                }
            }
        }
        HashCache { path, entries }
    }

    pub fn get(&self, root: &str, rel: &str) -> Option<&CacheEntry> {
        self.entries.get(&key(root, rel))
    }

    /// Records a verified hash for a file, taking its current modification time.
    pub fn record(&mut self, root: &str, rel: &str, file: &Path, size: u64, sha256: &str) {
        if let Some(mtime_ns) = mtime_ns(file) {
            self.entries.insert(
                key(root, rel),
                CacheEntry {
                    size,
                    mtime_ns,
                    sha256: sha256.to_string(),
                    verified: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                },
            );
        }
    }

    /// Writes the cache. Failures are ignored: the cache is an optimisation and safe to lose.
    pub fn save(&self) {
        let entries: serde_json::Map<String, Value> = self
            .entries
            .iter()
            .map(|(k, e)| {
                (
                    k.clone(),
                    json!({"size": e.size, "mtime_ns": e.mtime_ns, "sha256": e.sha256, "verified": e.verified}),
                )
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({"format_version": 1, "entries": entries}))
            .expect("JSON values always serialize");
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Concurrent seals may save at once; each uses its own temporary file.
        let _ = write_atomic_unique(&self.path, &text);
    }
}
