//! Tools: declared repositories, pinned commits, the tools lock, the per-machine cache, and
//! `tophat tools sync` / `tophat tools list`. Specified in `rqm/pipeline/tools.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use yaml_rust2::{Yaml, YamlLoader};

use crate::check::Project;
use crate::fsutil::write_atomic;
use crate::study::{STUDIES_DIR, is_valid_study_id};

/// The committed tool declarations.
pub const TOOLS_FILE: &str = ".tophat/tools.yaml";
/// The committed resolutions of short pins.
pub const LOCK_FILE: &str = ".tophat/tools.lock";
/// The per-machine cache of tool checkouts.
pub const CACHE_DIR: &str = ".tophat/tools";

// rq-d7bdbaae
/// A tool pin as written in a stage: `<name>@<commit or prefix>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolPin {
    pub name: String,
    pub pin: String,
}

impl ToolPin {
    // rq-cfae36f4
    /// Parses `<name>@<7 to 40 lowercase hex characters>`.
    pub fn parse(s: &str) -> Option<ToolPin> {
        let (name, pin) = s.split_once('@')?;
        let hex = pin
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        (is_valid_study_id(name) && (7..=40).contains(&pin.len()) && hex).then(|| ToolPin {
            name: name.to_string(),
            pin: pin.to_string(),
        })
    }

    pub fn is_full(&self) -> bool {
        self.pin.len() == 40
    }
}

impl fmt::Display for ToolPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.pin)
    }
}

// rq-3c1af8c0
/// Error returned by the tool commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolsError {
    UnknownTool(String),
    InvalidPin(String),
    Ambiguous {
        tool: String,
        pin: String,
    },
    NotFound {
        tool: String,
        pin: String,
    },
    /// A prefix pin has no entry in the tools lock.
    Unlocked {
        tool: String,
        pin: String,
    },
    Fetch {
        tool: String,
        message: String,
    },
    NotCached {
        tool: String,
        commit: String,
    },
    NotPristine {
        tool: String,
        commit: String,
    },
    /// `.tophat/tools.yaml` or `.tophat/tools.lock` is invalid.
    Invalid(String),
    Io(String),
}

impl fmt::Display for ToolsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolsError::UnknownTool(t) => write!(f, "tool `{t}` is not declared in {TOOLS_FILE}"),
            ToolsError::InvalidPin(p) => {
                write!(
                    f,
                    "`{p}` is not a tool pin: use <tool>@<7 to 40 lowercase hex characters>"
                )
            }
            ToolsError::Ambiguous { tool, pin } => {
                write!(
                    f,
                    "{tool}@{pin} matches more than one commit; pin more characters"
                )
            }
            ToolsError::NotFound { tool, pin } => write!(f, "{tool}@{pin} matches no commit"),
            ToolsError::Unlocked { tool, pin } => {
                write!(
                    f,
                    "{tool}@{pin} is not resolved yet: run `tophat tools sync`"
                )
            }
            ToolsError::Fetch { tool, message } => write!(f, "cannot fetch `{tool}`: {message}"),
            ToolsError::NotCached { tool, commit } => write!(
                f,
                "{tool}@{commit} is not cached on this machine: run `tophat tools sync`"
            ),
            ToolsError::NotPristine { tool, commit } => write!(
                f,
                "the cached checkout of {tool}@{commit} has local changes; delete {CACHE_DIR}/{tool}/{commit} and run `tophat tools sync`"
            ),
            ToolsError::Invalid(m) => write!(f, "{m}"),
            ToolsError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ToolsError {}

// rq-da50d924
/// Why a tool `url` cannot be fetched without credentials, if it cannot: allowed are an `https://`
/// URL without user information, an absolute path, and a `file://` URL.
pub fn url_problem(url: &str) -> Option<String> {
    let public =
        "use a public https:// URL, or a local path; the dev container holds no credentials";
    if let Some(rest) = url.strip_prefix("https://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        // The user information may hold a secret, so it is never repeated.
        return authority.rsplit_once('@').map(|(_, host)| {
            let tail = &rest[authority.len()..];
            format!(
                "`https://<redacted>@{host}{tail}` contains user information (credentials); {public}"
            )
        });
    }
    if url.starts_with("file://") || url.starts_with('/') {
        return None;
    }
    if url.starts_with("ssh://") || url.starts_with("git+ssh://") {
        return Some(format!("`{url}` needs SSH credentials; {public}"));
    }
    // scp-like syntax: `[user@]host:path` with no scheme.
    if !url.contains("://") && url.split('/').next().is_some_and(|head| head.contains(':')) {
        return Some(format!(
            "`{url}` is an SSH location and needs credentials; {public}"
        ));
    }
    Some(format!(
        "`{url}` is not an https:// URL or a local path; {public}"
    ))
}

// rq-da50d924
/// Reads the declared tools: name → URL.
pub fn declared_tools(root: &Path) -> Result<BTreeMap<String, String>, ToolsError> {
    let path = root.join(TOOLS_FILE);
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    let invalid = |m: String| ToolsError::Invalid(format!("{TOOLS_FILE}: {m}"));
    let doc = YamlLoader::load_from_str(&text)
        .map_err(|e| invalid(format!("invalid YAML: {e}")))?
        .into_iter()
        .next()
        .unwrap_or(Yaml::Null);
    let mut out = BTreeMap::new();
    match &doc["tools"] {
        Yaml::BadValue | Yaml::Null => {}
        Yaml::Hash(tools) => {
            for (name, entry) in tools {
                let Yaml::String(name) = name else {
                    return Err(invalid("tool names must be text".into()));
                };
                if !is_valid_study_id(name) {
                    return Err(invalid(format!("tool name `{name}` is malformed")));
                }
                let Some(url) = entry["url"].as_str().filter(|u| !u.trim().is_empty()) else {
                    return Err(invalid(format!("tool `{name}` has no `url`")));
                };
                if let Some(problem) = url_problem(url) {
                    return Err(invalid(format!("tool `{name}`: {problem}")));
                }
                out.insert(name.clone(), url.to_string());
            }
        }
        _ => return Err(invalid("`tools` must be a mapping".into())),
    }
    Ok(out)
}

fn is_full_commit(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// rq-ab9a813f
/// Reads the tools lock: tool → prefix → full commit.
pub fn read_lock(root: &Path) -> Result<BTreeMap<String, BTreeMap<String, String>>, ToolsError> {
    let path = root.join(LOCK_FILE);
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    let invalid = |m: String| ToolsError::Invalid(format!("{LOCK_FILE}: {m}"));
    let doc = YamlLoader::load_from_str(&text)
        .map_err(|e| invalid(format!("invalid YAML: {e}")))?
        .into_iter()
        .next()
        .unwrap_or(Yaml::Null);
    let mut out = BTreeMap::new();
    let Yaml::Hash(tools) = doc else {
        return if doc.is_null() {
            Ok(out)
        } else {
            Err(invalid("must be a mapping".into()))
        };
    };
    for (tool, entries) in tools {
        let (Yaml::String(tool), Yaml::Hash(entries)) = (tool, entries) else {
            return Err(invalid("each tool maps prefixes to commits".into()));
        };
        let mut map = BTreeMap::new();
        for (prefix, commit) in entries {
            let (Yaml::String(prefix), Yaml::String(commit)) = (prefix, commit) else {
                return Err(invalid(format!(
                    "tool `{tool}`: prefixes and commits must be quoted text"
                )));
            };
            if !is_full_commit(&commit) || !commit.starts_with(prefix.as_str()) {
                return Err(invalid(format!(
                    "tool `{tool}`: `{commit}` is not a full commit extending `{prefix}`"
                )));
            }
            map.insert(prefix, commit);
        }
        out.insert(tool, map);
    }
    Ok(out)
}

fn render_lock(lock: &BTreeMap<String, BTreeMap<String, String>>) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("strings serialize");
    let mut out = String::from("# Written by `tophat tools sync`. Do not edit.\n");
    for (tool, entries) in lock {
        out.push_str(&format!("{tool}:\n"));
        for (prefix, commit) in entries {
            out.push_str(&format!("  {}: {}\n", q(prefix), q(commit)));
        }
    }
    out
}

// rq-ccd30a25
/// The resolved commit of a pin, from the pin itself or the lock, without network access.
pub fn resolve_pin(root: &Path, pin: &ToolPin) -> Result<String, ToolsError> {
    if pin.is_full() {
        return Ok(pin.pin.clone());
    }
    read_lock(root)?
        .get(&pin.name)
        .and_then(|m| m.get(&pin.pin))
        .cloned()
        .ok_or_else(|| ToolsError::Unlocked {
            tool: pin.name.clone(),
            pin: pin.pin.clone(),
        })
}

/// The cache directory of a tool commit.
pub fn checkout_dir(root: &Path, tool: &str, commit: &str) -> PathBuf {
    root.join(CACHE_DIR).join(tool).join(commit)
}

fn mirror_dir(root: &Path, tool: &str) -> PathBuf {
    root.join(CACHE_DIR).join(tool).join(".git")
}

// rq-052f542d
/// Runs git without any way to obtain credentials: no terminal prompt, no askpass program and no
/// credential helper, so a repository that needs credentials fails at once.
fn git(args: &[&str], dir: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(std::process::Stdio::null())
        .args(["-c", "advice.detachedHead=false"])
        .args(["-c", "core.askPass=", "-c", "credential.helper="])
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Whether a cached checkout is exactly its commit, with no local changes.
fn is_pristine(dir: &Path, commit: &str) -> bool {
    let head = git(&["rev-parse", "HEAD"], Some(dir)).map(|h| h.trim().to_string());
    let status = git(
        &["status", "--porcelain", "--untracked-files=all"],
        Some(dir),
    );
    head.as_deref() == Ok(commit) && status.is_ok_and(|s| s.trim().is_empty())
}

// rq-f8136bd3 rq-c5b9e99a
/// The cached, pristine checkout of a pin, or why it cannot be used.
pub fn tool_checkout(root: &Path, pin: &ToolPin) -> Result<PathBuf, ToolsError> {
    let commit = resolve_pin(root, pin)?;
    let dir = checkout_dir(root, &pin.name, &commit);
    if !dir.is_dir() {
        return Err(ToolsError::NotCached {
            tool: pin.name.clone(),
            commit,
        });
    }
    if !is_pristine(&dir, &commit) {
        return Err(ToolsError::NotPristine {
            tool: pin.name.clone(),
            commit,
        });
    }
    Ok(dir)
}

// rq-a96b0574
/// The environment of a tool commit: `(manifest, lock)`. The tool's own `pixi.toml`/`pixi.lock`
/// win; otherwise the project's `envs/<tool>/`.
pub fn tool_environment(root: &Path, tool: &str, checkout: &Path) -> Option<(PathBuf, PathBuf)> {
    let own = (checkout.join("pixi.toml"), checkout.join("pixi.lock"));
    if own.0.is_file() && own.1.is_file() {
        return Some(own);
    }
    let supplied = (
        root.join("envs").join(tool).join("pixi.toml"),
        root.join("envs").join(tool).join("pixi.lock"),
    );
    (supplied.0.is_file() && supplied.1.is_file()).then_some(supplied)
}

/// How a prefix pin resolves against the commits it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Resolved to this commit; `newly` when it was not in the lock before.
    Resolved {
        commit: String,
        newly: bool,
        now_ambiguous: bool,
    },
    Ambiguous,
    NotFound,
}

/// Decides a prefix pin's resolution. A recorded resolution never changes; a new prefix resolves
/// only to a unique matching commit.
pub fn resolve_prefix(locked: Option<&str>, candidates: &[String]) -> Resolution {
    match (locked, candidates) {
        (Some(commit), _) => Resolution::Resolved {
            commit: commit.to_string(),
            newly: false,
            now_ambiguous: candidates.len() > 1,
        },
        (None, [one]) => Resolution::Resolved {
            commit: one.clone(),
            newly: true,
            now_ambiguous: false,
        },
        (None, []) => Resolution::NotFound,
        (None, _) => Resolution::Ambiguous,
    }
}

/// Every tool pin in every study's configuration (all variants), with where it is used:
/// pin → `(study, variant, stage)`.
pub fn pins_in_use(root: &Path) -> BTreeMap<ToolPin, Vec<(String, String, String)>> {
    let project = Project::new(root);
    let mut out: BTreeMap<ToolPin, Vec<(String, String, String)>> = BTreeMap::new();
    for study in project.list_dir(STUDIES_DIR).unwrap_or_default() {
        let config_dir = format!("{STUDIES_DIR}/{study}/config");
        let mut variants: Vec<String> = project
            .list_dir(&config_dir)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| f.strip_suffix(".yaml").map(str::to_string))
            .collect();
        variants.sort_by_key(|v| (v != "base", v.clone()));
        for variant in variants {
            let Some(config) = crate::pipeline::load_variant_lenient(&project, &study, &variant)
            else {
                continue;
            };
            for stage in &config.stages {
                if let Some(pin) = stage.tool.as_deref().and_then(ToolPin::parse) {
                    out.entry(pin).or_default().push((
                        study.clone(),
                        variant.clone(),
                        stage.name.clone(),
                    ));
                }
            }
        }
    }
    out
}

/// The outcome of `tophat tools sync`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub lines: Vec<String>,
    pub ok: bool,
}

/// Full commits in a mirror whose hash starts with `prefix`.
fn commits_with_prefix(mirror: &Path, prefix: &str) -> Vec<String> {
    let Ok(out) = git(&["rev-parse", "--disambiguate", prefix], Some(mirror)) else {
        return Vec::new();
    };
    out.lines()
        .map(str::trim)
        .filter(|h| git(&["cat-file", "-t", h], Some(mirror)).is_ok_and(|t| t.trim() == "commit"))
        .map(str::to_string)
        .collect()
}

/// Explains a failed fetch; a repository asking for credentials is not publicly reachable.
fn fetch_error(message: String) -> String {
    let lower = message.to_ascii_lowercase();
    let asks_credentials = [
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "authentication failed",
        "authentication required",
    ]
    .iter()
    .any(|m| lower.contains(m));
    if asks_credentials {
        format!(
            "the repository is not publicly reachable (it asks for credentials, and tools are \
             fetched from public repositories only): {message}"
        )
    } else {
        message
    }
}

/// Makes sure the tool's mirror exists and, if `fetch` is set, is up to date.
fn ensure_mirror(root: &Path, tool: &str, url: &str, fetch: bool) -> Result<bool, String> {
    ensure_mirror_inner(root, tool, url, fetch).map_err(fetch_error)
}

fn ensure_mirror_inner(root: &Path, tool: &str, url: &str, fetch: bool) -> Result<bool, String> {
    let mirror = mirror_dir(root, tool);
    if !mirror.is_dir() {
        fs::create_dir_all(mirror.parent().expect("mirror has a parent"))
            .map_err(|e| e.to_string())?;
        git(
            &["clone", "--quiet", "--bare", url, &mirror.to_string_lossy()],
            None,
        )?;
        return Ok(true);
    }
    if fetch {
        git(
            &[
                "fetch",
                "--quiet",
                url,
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
            Some(&mirror),
        )?;
        return Ok(true);
    }
    Ok(false)
}

/// Creates a pristine checkout of `commit`, atomically.
fn create_checkout(root: &Path, tool: &str, commit: &str) -> Result<(), String> {
    let dir = checkout_dir(root, tool, commit);
    let tmp = dir.with_file_name(format!(".{commit}.tophat-tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let mirror = mirror_dir(root, tool);
    git(
        &[
            "clone",
            "--quiet",
            "--shared",
            "--no-checkout",
            &mirror.to_string_lossy(),
            &tmp.to_string_lossy(),
        ],
        None,
    )?;
    git(&["checkout", "--quiet", "--detach", commit], Some(&tmp))?;
    fs::rename(&tmp, &dir).map_err(|e| e.to_string())
}

// rq-097c7d03 rq-052f542d
/// Performs `tophat tools sync`.
pub fn sync(root: &Path) -> Result<SyncReport, ToolsError> {
    let declared = declared_tools(root)?;
    let mut lock = read_lock(root)?;
    let mut report = SyncReport {
        lines: Vec::new(),
        ok: true,
    };
    let mut fetched_tools: BTreeSet<String> = BTreeSet::new();
    for pin in pins_in_use(root).into_keys() {
        let fail = |report: &mut SyncReport, reason: String| {
            report.ok = false;
            report.lines.push(format!("{pin} -> ? (failed: {reason})"));
        };
        let Some(url) = declared.get(&pin.name) else {
            fail(
                &mut report,
                format!("tool `{}` is not declared in {TOOLS_FILE}", pin.name),
            );
            continue;
        };
        let mirror = mirror_dir(root, &pin.name);
        let locked = lock.get(&pin.name).and_then(|m| m.get(&pin.pin)).cloned();
        let known = |c: &str| {
            git(
                &["cat-file", "-e", &format!("{c}^{{commit}}")],
                Some(&mirror),
            )
            .is_ok()
        };
        // Fetch when the mirror is missing or does not know the commit yet.
        let needs_fetch = !mirror.is_dir()
            || match (&locked, pin.is_full()) {
                (Some(c), _) => !known(c),
                (None, true) => !known(&pin.pin),
                (None, false) => commits_with_prefix(&mirror, &pin.pin).is_empty(),
            };
        if needs_fetch && !fetched_tools.contains(&pin.name) {
            if let Err(e) = ensure_mirror(root, &pin.name, url, true) {
                fail(&mut report, format!("fetch failed: {e}"));
                continue;
            }
            fetched_tools.insert(pin.name.clone());
        }
        let (commit, note) = if pin.is_full() {
            if !known(&pin.pin) {
                fail(&mut report, "no such commit".into());
                continue;
            }
            (pin.pin.clone(), "")
        } else {
            match resolve_prefix(locked.as_deref(), &commits_with_prefix(&mirror, &pin.pin)) {
                Resolution::Resolved {
                    commit,
                    newly,
                    now_ambiguous,
                } => {
                    if newly {
                        lock.entry(pin.name.clone())
                            .or_default()
                            .insert(pin.pin.clone(), commit.clone());
                    }
                    (
                        commit,
                        if now_ambiguous {
                            ", prefix is now ambiguous; kept the recorded commit"
                        } else {
                            ""
                        },
                    )
                }
                Resolution::Ambiguous => {
                    fail(
                        &mut report,
                        "ambiguous: matches more than one commit".into(),
                    );
                    continue;
                }
                Resolution::NotFound => {
                    fail(&mut report, "no such commit".into());
                    continue;
                }
            }
        };
        let state = if checkout_dir(root, &pin.name, &commit).is_dir() {
            "cached"
        } else {
            match create_checkout(root, &pin.name, &commit) {
                Ok(()) => "fetched",
                Err(e) => {
                    fail(&mut report, format!("checkout failed: {e}"));
                    continue;
                }
            }
        };
        report
            .lines
            .push(format!("{pin} -> {commit} ({state}{note})"));
    }
    let text = render_lock(&lock);
    let path = root.join(LOCK_FILE);
    let unchanged = fs::read_to_string(&path).is_ok_and(|old| old == text);
    if !unchanged && (path.exists() || !lock.is_empty()) {
        write_atomic(&path, &text).map_err(|e| ToolsError::Io(e.to_string()))?;
    }
    Ok(report)
}

/// One pin in `tophat tools list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinUse {
    pub pin: ToolPin,
    pub commit: Option<String>,
    /// `cached`, `modified`, `not cached`, or `unresolved`.
    pub state: &'static str,
    pub used_by: Vec<(String, String, String)>,
}

// rq-4ebf0587
/// The data behind `tophat tools list`: declared tools with their URLs, and every pin in use.
pub fn list(root: &Path) -> Result<(BTreeMap<String, String>, Vec<PinUse>), ToolsError> {
    let declared = declared_tools(root)?;
    let mut pins = Vec::new();
    for (pin, used_by) in pins_in_use(root) {
        let (commit, state) = match resolve_pin(root, &pin) {
            Err(_) => (None, "unresolved"),
            Ok(commit) => {
                let state = match tool_checkout(root, &pin) {
                    Ok(_) => "cached",
                    Err(ToolsError::NotPristine { .. }) => "modified",
                    Err(_) => "not cached",
                };
                (Some(commit), state)
            }
        };
        pins.push(PinUse {
            pin,
            commit,
            state,
            used_by,
        });
    }
    Ok((declared, pins))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // rq-cfae36f4 rq-bf589d85
    fn pins() {
        assert_eq!(
            ToolPin::parse("method-x@3f9a1c2"),
            Some(ToolPin {
                name: "method-x".into(),
                pin: "3f9a1c2".into()
            })
        );
        for bad in [
            "method-x@main",
            "method-x@3f9a1c",
            "method-x@3F9A1C2",
            "mx@3f9a1c2",
            "method-x",
        ] {
            assert_eq!(ToolPin::parse(bad), None, "{bad}");
        }
        assert!(
            ToolPin::parse(&format!("method-x@{}", "a".repeat(40)))
                .unwrap()
                .is_full()
        );
        assert_eq!(
            ToolPin::parse(&format!("method-x@{}", "a".repeat(41))),
            None
        );
    }

    #[test] // rq-4f2cf51c rq-ab9a813f
    fn a_recorded_resolution_never_changes() {
        let a = "a".repeat(40);
        let other = format!("{}{}", &a[..7], "b".repeat(33));
        assert_eq!(
            resolve_prefix(Some(&a), &[a.clone(), other.clone()]),
            Resolution::Resolved {
                commit: a.clone(),
                newly: false,
                now_ambiguous: true
            }
        );
        assert_eq!(
            resolve_prefix(None, &[a.clone(), other]),
            Resolution::Ambiguous
        );
        assert_eq!(resolve_prefix(None, &[]), Resolution::NotFound);
        assert_eq!(
            resolve_prefix(None, std::slice::from_ref(&a)),
            Resolution::Resolved {
                commit: a,
                newly: true,
                now_ambiguous: false
            }
        );
    }

    #[test] // rq-ab9a813f
    fn lock_round_trip() {
        let mut lock = BTreeMap::new();
        lock.insert(
            "method-x".to_string(),
            BTreeMap::from([("1234567".to_string(), format!("1234567{}", "0".repeat(33)))]),
        );
        let dir = std::env::temp_dir().join(format!("tophat-lock-{}", std::process::id()));
        fs::create_dir_all(dir.join(".tophat")).unwrap();
        fs::write(dir.join(LOCK_FILE), render_lock(&lock)).unwrap();
        assert_eq!(read_lock(&dir).unwrap(), lock);
        fs::remove_dir_all(dir).unwrap();
    }
}
