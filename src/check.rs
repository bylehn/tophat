//! The project integrity check (`tophat check`): project discovery, the diagnostic model, scoping
//! and rule-family evaluation. Output formatting lives in `report`.

use std::fmt;
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};

/// Name of the directory that marks the root of a TopHat project.
pub const PROJECT_MARKER: &str = ".tophat";

// rq-fdb74980 rq-192a5ff9
/// How serious a diagnostic is, and whether it fails the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The project violates an integrity guarantee. Always fails the check.
    Error,
    /// Incomplete or suspicious, but not yet a violation. Fails only in strict mode.
    Warning,
    /// A tolerated gap in a partially adopted project. Never fails the check.
    Legacy,
}

impl Severity {
    /// The lowercase name used in output.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Legacy => "legacy",
        }
    }

    // rq-192a5ff9
    /// Whether a diagnostic of this severity fails the check.
    pub fn fails(self, strict: bool) -> bool {
        match self {
            Severity::Error => true,
            Severity::Warning => strict,
            Severity::Legacy => false,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// rq-84ef0512
/// One finding reported by a rule family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Rule identifier of the form `<family>.<rule>`.
    pub rule: &'static str,
    pub severity: Severity,
    /// Project-relative path, `/`-separated. `.` denotes the project as a whole.
    pub path: String,
    /// 1-based line number within the file, when the finding has a location.
    pub line: Option<u32>,
    /// One sentence stating what is wrong.
    pub message: String,
    /// One sentence stating how to fix it.
    pub hint: Option<String>,
}

// rq-c8046ff3
/// Failure to run the check at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// No ancestor directory contains `.tophat/`.
    NotInProject,
    /// A given scope path is not inside the project.
    ScopeOutsideProject(String),
    /// A failure reading project structure that prevents any check from running.
    Io(String),
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckError::NotInProject => write!(
                f,
                "no TopHat project found: neither the current directory nor any of its \
                 ancestors contains a `{PROJECT_MARKER}` directory"
            ),
            CheckError::ScopeOutsideProject(p) => {
                write!(f, "path `{p}` is outside the TopHat project")
            }
            CheckError::Io(msg) => write!(f, "cannot read the project: {msg}"),
        }
    }
}

impl std::error::Error for CheckError {}

// rq-d94a6490
/// A read-only view of a project, used by rule families. All paths are project-relative and
/// `/`-separated; paths that would escape the project root are rejected.
#[derive(Debug)]
pub struct Project {
    root: PathBuf,
}

impl Project {
    /// Creates a view of the project rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Project { root: root.into() }
    }

    /// The project root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a project-relative path, rejecting absolute paths and `..` components.
    fn resolve(&self, rel: &str) -> io::Result<PathBuf> {
        let mut out = self.root.clone();
        for component in Path::new(rel).components() {
            match component {
                Component::Normal(part) => out.push(part),
                Component::CurDir => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("path `{rel}` is not a relative path inside the project"),
                    ))
                }
            }
        }
        Ok(out)
    }

    /// Whether `rel` names an existing regular file.
    pub fn is_file(&self, rel: &str) -> bool {
        self.resolve(rel).map(|p| p.is_file()).unwrap_or(false)
    }

    /// Whether `rel` names an existing directory.
    pub fn is_dir(&self, rel: &str) -> bool {
        self.resolve(rel).map(|p| p.is_dir()).unwrap_or(false)
    }

    /// Reads a file as UTF-8 text.
    pub fn read_to_string(&self, rel: &str) -> io::Result<String> {
        fs::read_to_string(self.resolve(rel)?)
    }

    /// Reads a file as bytes.
    pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        fs::read(self.resolve(rel)?)
    }

    /// Lists the regular files below a directory, recursively, as sorted project-relative paths.
    /// Symbolic links are not followed or listed.
    pub fn walk_files(&self, rel: &str) -> io::Result<Vec<String>> {
        let mut files = Vec::new();
        let mut pending = vec![rel.trim_end_matches('/').to_string()];
        while let Some(dir) = pending.pop() {
            for name in self.list_dir(&dir)? {
                let child = format!("{dir}/{name}");
                let meta = fs::symlink_metadata(self.resolve(&child)?)?;
                if meta.is_dir() {
                    pending.push(child);
                } else if meta.is_file() {
                    files.push(child);
                }
            }
        }
        files.sort();
        Ok(files)
    }

    /// Lists the entry names of a directory, sorted. Names that are not valid UTF-8 are skipped.
    pub fn list_dir(&self, rel: &str) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(self.resolve(rel)?)? {
            if let Ok(name) = entry?.file_name().into_string() {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }
}

/// A rule family: inspects the project and returns its diagnostics.
pub type RuleFamily = fn(&Project) -> Vec<Diagnostic>;

// rq-9b44b88f
/// The static list of rule families evaluated by every check. Adding a family means adding a
/// function here; there is no runtime plugin mechanism.
pub static RULE_FAMILIES: &[RuleFamily] = &[
    crate::study::study_rules,
    crate::question::question_rules,
    crate::data::data_rules,
    crate::pipeline::pipeline_rules,
];

// rq-abbd00cd
/// Finds the project root: the nearest ancestor of `start` (including `start` itself) that
/// contains a `.tophat/` directory.
pub fn find_project_root(start: &Path) -> Result<PathBuf, CheckError> {
    start
        .ancestors()
        .find(|dir| dir.join(PROJECT_MARKER).is_dir())
        .map(Path::to_path_buf)
        .ok_or(CheckError::NotInProject)
}

/// Removes `.` and resolves `..` components lexically, without touching the filesystem.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Converts a relative path to the `/`-separated form used in diagnostics.
fn to_slash_string(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

// rq-cf68a3b8
/// Turns a user-supplied scope path (relative to `cwd`, or absolute) into a project-relative,
/// `/`-separated prefix. The empty string denotes the whole project.
pub fn resolve_scope(root: &Path, cwd: &Path, path: &Path) -> Result<String, CheckError> {
    let joined = normalize_lexically(&cwd.join(path));
    // Canonicalize when possible so symlinked spellings of the project path still match.
    let absolute = fs::canonicalize(&joined).unwrap_or(joined);
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    absolute
        .strip_prefix(&root)
        .map(to_slash_string)
        .map_err(|_| CheckError::ScopeOutsideProject(path.display().to_string()))
}

/// Whether a diagnostic path lies at or below a scope prefix.
fn in_scope(path: &str, scopes: &[String]) -> bool {
    scopes.is_empty()
        || scopes.iter().any(|scope| {
            scope.is_empty()
                || path == scope
                || (path.starts_with(scope.as_str())
                    && path.as_bytes().get(scope.len()) == Some(&b'/'))
        })
}

/// Whether a rule identifier has the form `<family>.<rule>` using `a-z`, `0-9` and `-`.
pub fn is_valid_rule_id(rule: &str) -> bool {
    let valid_part = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    matches!(rule.split_once('.'), Some((family, name)) if valid_part(family) && valid_part(name))
}

// rq-e54fa7fd
/// Runs one rule family, converting a panic into an error diagnostic so the remaining families
/// still run.
fn run_family(family: RuleFamily, project: &Project) -> Vec<Diagnostic> {
    match panic::catch_unwind(AssertUnwindSafe(|| family(project))) {
        Ok(diagnostics) => diagnostics,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown cause".to_string());
            vec![Diagnostic {
                rule: "check.rule-family-failed",
                severity: Severity::Error,
                path: ".".to_string(),
                line: None,
                message: format!("A rule family failed internally: {detail}"),
                hint: Some("Report this as a bug in tophat".to_string()),
            }]
        }
    }
}

/// Sorts diagnostics by path, then line (absent first), then rule, then message.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        (&a.path, a.line, a.rule, &a.message).cmp(&(&b.path, b.line, b.rule, &b.message))
    });
}

/// Runs the given rule families against the project at `root` and returns the diagnostics within
/// `scopes` (project-relative prefixes as produced by [`resolve_scope`]), sorted. Diagnostics of the
/// reserved `check` family are always reported, because they concern the run itself.
pub fn run_families(root: &Path, scopes: &[String], families: &[RuleFamily]) -> Vec<Diagnostic> {
    let project = Project::new(root);
    let mut diagnostics: Vec<Diagnostic> = families
        .iter()
        .flat_map(|&family| run_family(family, &project))
        .filter(|d| d.rule.starts_with("check.") || in_scope(&d.path, scopes))
        .collect();
    debug_assert!(
        diagnostics.iter().all(|d| is_valid_rule_id(d.rule)),
        "a rule family emitted an invalid rule identifier"
    );
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

// rq-6236fe4c
/// Runs every registered rule family against the project at `root` and returns the diagnostics in
/// scope, sorted. `scope` paths are absolute or relative to `root`; an empty slice means the whole
/// project.
pub fn run_check(root: &Path, scope: &[PathBuf]) -> Result<Vec<Diagnostic>, CheckError> {
    if !root.join(PROJECT_MARKER).is_dir() {
        return Err(CheckError::NotInProject);
    }
    let scopes = scope
        .iter()
        .map(|p| resolve_scope(root, root, p))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(run_families(root, &scopes, RULE_FAMILIES))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(rule: &'static str, path: &str, line: Option<u32>, message: &str) -> Diagnostic {
        Diagnostic {
            rule,
            severity: Severity::Error,
            path: path.to_string(),
            line,
            message: message.to_string(),
            hint: None,
        }
    }

    #[test] // rq-192a5ff9
    fn severities_fail_as_specified() {
        assert!(Severity::Error.fails(false));
        assert!(Severity::Error.fails(true));
        assert!(!Severity::Warning.fails(false));
        assert!(Severity::Warning.fails(true));
        assert!(!Severity::Legacy.fails(false));
        assert!(!Severity::Legacy.fails(true));
    }

    #[test] // rq-9b9f7ee6
    fn rule_identifier_format() {
        assert!(is_valid_rule_id("study.frozen-modified"));
        assert!(is_valid_rule_id("check.rule-family-failed"));
        assert!(is_valid_rule_id("data2.hash-0"));
        assert!(!is_valid_rule_id("study"));
        assert!(!is_valid_rule_id("Study.frozen"));
        assert!(!is_valid_rule_id("study.frozen_modified"));
        assert!(!is_valid_rule_id(".frozen"));
        assert!(!is_valid_rule_id("study."));
        assert!(!is_valid_rule_id("a.b.c"));
    }

    #[test] // rq-cf68a3b8
    fn scope_prefix_matching_respects_path_boundaries() {
        let scopes = vec!["studies/a".to_string()];
        assert!(in_scope("studies/a", &scopes));
        assert!(in_scope("studies/a/STUDY.md", &scopes));
        assert!(!in_scope("studies/ab/STUDY.md", &scopes));
        assert!(!in_scope("studies/b/STUDY.md", &scopes));
        assert!(in_scope("anything", &[]));
        assert!(in_scope("anything", &[String::new()]));
    }

    #[test] // rq-68a6dd72
    fn sorting_is_by_path_line_rule_message() {
        let mut ds = vec![
            diag("x.b", "b", Some(1), "m"),
            diag("x.a", "a", Some(2), "m"),
            diag("x.b", "a", Some(2), "a"),
            diag("x.a", "a", None, "z"),
            diag("x.a", "a", Some(2), "a"),
        ];
        sort_diagnostics(&mut ds);
        let keys: Vec<_> = ds
            .iter()
            .map(|d| (d.path.as_str(), d.line, d.rule, d.message.as_str()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("a", None, "x.a", "z"),
                ("a", Some(2), "x.a", "a"),
                ("a", Some(2), "x.a", "m"),
                ("a", Some(2), "x.b", "a"),
                ("b", Some(1), "x.b", "m"),
            ]
        );
    }

    #[test] // rq-d94a6490
    fn project_rejects_paths_escaping_the_root() {
        let project = Project::new("/nonexistent-root");
        assert!(project.read_to_string("../etc/passwd").is_err());
        assert!(project.read_to_string("/etc/passwd").is_err());
        assert!(!project.is_file("../x"));
    }

    #[test] // rq-e54fa7fd
    fn panicking_family_is_reported_and_others_still_run() {
        fn bad(_: &Project) -> Vec<Diagnostic> {
            panic!("boom")
        }
        fn good(_: &Project) -> Vec<Diagnostic> {
            vec![diag("test.found", "studies/a/STUDY.md", None, "found")]
        }
        let families: &[RuleFamily] = &[bad, good];
        let ds = run_families(Path::new("/nonexistent-root"), &[], families);
        assert_eq!(ds.len(), 2);
        assert!(ds.iter().any(|d| d.rule == "check.rule-family-failed"));
        assert!(ds.iter().any(|d| d.rule == "test.found"));
    }
}
