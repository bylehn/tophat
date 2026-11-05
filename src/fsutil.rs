//! Filesystem helpers.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The path as text with `/` separators, for instructions and generated text that must mean the
/// same thing on every platform.
pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `base` joined with `rel`, a relative path stored with `/` separators (as in manifests and
/// configuration), one component at a time so the result uses the platform's separator
/// throughout instead of mixing `\\` and `/` on Windows.
pub(crate) fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    path.extend(rel.split('/').filter(|c| !c.is_empty()));
    path
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_use_forward_slashes() {
        assert_eq!(path_text(Path::new(r"C:\work\job")), "C:/work/job");
        assert_eq!(path_text(Path::new("a/b/job")), "a/b/job");
    }

    #[test]
    fn relative_paths_join_by_component() {
        let joined = join_rel(Path::new("root"), "bench/v2/");
        assert_eq!(joined, Path::new("root").join("bench").join("v2"));
        assert_eq!(
            joined.to_string_lossy(),
            format!("root{0}bench{0}v2", std::path::MAIN_SEPARATOR)
        );
    }
}
