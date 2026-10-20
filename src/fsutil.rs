//! Filesystem helpers.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Writes `contents` to `path` atomically: writes a temporary file in the same directory, then
/// renames it over `path`. On failure `path` is unchanged.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tophat-tmp"));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() && tmp.is_file() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Like [`write_atomic`], but with a temporary file name unique to this process, for files that
/// several processes may write at the same time.
pub fn write_atomic_unique(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tophat-tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() && tmp.is_file() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
