//! Tests for the dev container launcher (rqm/template/container.md).
//!
//! The launcher runs against a fake `podman` that logs every invocation and, for the throwaway
//! containers that run `tophat` (the data-roots query and `--check`), runs this workspace's
//! `tophat` in the mounted workspace. They need `copier` to render a project (see
//! tests/common/mod.rs).
//!
//! The image scenarios need real Podman and network access; they run only with
//! `TOPHAT_TEST_PODMAN=1`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

/// A fake podman: logs its arguments, answers `image exists` and `build`, and runs the `tophat`
/// command of a `run` in the directory mounted at /work.
const FAKE_PODMAN: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_PODMAN_LOG"
case "$1" in
  image) exit "${FAKE_PODMAN_IMAGE_EXISTS:-0}" ;;
  build) exit "${FAKE_PODMAN_BUILD_EXIT:-0}" ;;
  run)
    shift
    work=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        -v)
          case "$2" in *:/work | *:/work:ro) work=${2%:/work*} ;; esac
          shift 2 ;;
        -e | -w) shift 2 ;;
        localhost/*) shift; break ;;
        *) shift ;;
      esac
    done
    [ "$1" = tophat ] || { echo "fake podman cannot run $1" >&2; exit 125; }
    shift
    # Inside a container the workspace is /work: show its paths that way.
    cd "$work" || exit 125
    "$FAKE_TOPHAT" "$@" > "$FAKE_PODMAN_LOG.out" 2>&1
    status=$?
    sed "s#$(pwd)#/work#g" "$FAKE_PODMAN_LOG.out"
    exit "$status"
    ;;
esac
exit 0
"#;

struct Env {
    base: PathBuf,
}

impl Env {
    /// A rendered project in a git repository, with the fake podman on PATH.
    fn new() -> Env {
        static N: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "tophat-launcher-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("bin")).unwrap();
        let env = Env { base };
        let out = Command::new("copier")
            .args(["copy", "--defaults", "--quiet", "--vcs-ref", "HEAD"])
            .args(["-d", "project_name=Method X"])
            .args(["-d", "project_description=Does method X work?"])
            .args([
                "-d",
                "author_name=A. Scientist",
                "-d",
                "author_email=a@example.org",
            ])
            .arg(env!("CARGO_MANIFEST_DIR"))
            .arg(env.project())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        env.git(&["init", "-q"]);
        executable(&env.base.join("bin/podman"), FAKE_PODMAN);
        env
    }

    fn project(&self) -> PathBuf {
        self.base.join("project")
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.base.join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.project().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(self.project())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.path("podman.log")).unwrap_or_default()
    }

    /// The launcher's command: `./th.sh <args>` with the fake podman first on PATH, the workspace's
    /// tophat as the development binary, a home of its own (so the human's personal skills stay
    /// out), and no host root, git, email or XDG_CONFIG_HOME variables.
    fn launcher(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("./th.sh").args(args).current_dir(self.project());
        let path = format!(
            "{}:{}",
            self.base.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        for (key, _) in std::env::vars() {
            if key.starts_with("TOPHAT_ROOT_")
                || key.starts_with("GIT_")
                || key == "EMAIL"
                || key == "XDG_CONFIG_HOME"
            {
                cmd.env_remove(key);
            }
        }
        cmd.env("PATH", path)
            .env("HOME", self.path("home"))
            .env("FAKE_PODMAN_LOG", self.path("podman.log"))
            .env("FAKE_TOPHAT", env!("CARGO_BIN_EXE_tophat"))
            .env("TOPHAT_DEV_BINARY", env!("CARGO_BIN_EXE_tophat"));
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.launcher(args).output().unwrap()
    }

    /// The session command a dry run prints.
    fn session(&self, out: &Output) -> String {
        assert!(out.status.success(), "{}", stderr(out));
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| l.starts_with("podman run --rm -it"))
            .unwrap_or_else(|| panic!("no session command in:\n{}", stdout(out)))
            .to_string()
    }

    fn id(&self) -> String {
        fs::read_to_string(self.project().join(".tophat/project-id"))
            .unwrap()
            .trim()
            .to_string()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Writes a skill `<dir>/<name>/SKILL.md` and returns the skill's folder.
fn skill(dir: &Path, name: &str) -> PathBuf {
    let folder = dir.join(name);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
    folder
}

/// The binds the session command makes in Claude Code's skills directory.
fn skill_binds(session: &str) -> Vec<&str> {
    session
        .split(' ')
        .filter(|word| word.contains(":/opt/tophat/home/.claude/skills"))
        .collect()
}

/// Declares the roots `raw` and `results` in the project.
fn declare_raw(env: &Env) {
    env.write(".tophat/roots.yaml", "roots:\n  raw: {}\n  results: {}\n");
}

// --- Launcher -----------------------------------------------------------------------------------

#[test] // rq-22ab7a81 rq-71727623
fn the_project_id_is_created_once() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    assert!(!env.project().join(".tophat/project-id").exists());
    let first = env.run(&["--dry-run"]);
    let id = env.id();
    assert_eq!(id.len(), 36, "{id}");
    let second = env.run(&["--dry-run"]);
    assert_eq!(env.id(), id);
    let image = format!("localhost/tophat-{id}-project:latest");
    assert!(env.session(&first).contains(&image));
    assert!(env.session(&second).contains(&image));
}

#[test] // rq-df2a2b0e rq-71727623
fn a_malformed_project_id_is_refused() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.write(".tophat/project-id", "not-a-uuid\n");
    let out = env.run(&["--dry-run"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains(".tophat/project-id"),
        "{}",
        stderr(&out)
    );
    assert_eq!(env.log(), "");
}

#[test] // rq-5b3d8f35 rq-71727623
fn unknown_options_are_refused() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env.run(&["--frobnicate"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Usage: ./th.sh"), "{}", stderr(&out));
    assert_eq!(env.log(), "");
}

#[test] // rq-18896042 rq-71727623
fn a_dry_run_needs_an_image_from_an_earlier_launch() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env
        .launcher(&["--dry-run"])
        .env("FAKE_PODMAN_IMAGE_EXISTS", "1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("launch once first"),
        "{}",
        stderr(&out)
    );
    // The build commands were printed, not run.
    assert!(!env.log().contains("build"), "{}", env.log());
    assert!(stdout(&out).contains("podman build"));
}

// --- Image source -------------------------------------------------------------------------------

#[test] // rq-9c7772af rq-5c404476
fn an_unreleased_pin_needs_a_development_binary() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env
        .launcher(&["--dry-run"])
        .env_remove("TOPHAT_DEV_BINARY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(
        err.contains("no tophat release is pinned") && err.contains("TOPHAT_DEV_BINARY"),
        "{err}"
    );
    assert_eq!(env.log(), "");
}

#[test] // rq-bcd5b8bd rq-5c404476
fn a_missing_development_binary_is_refused() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env
        .launcher(&["--dry-run"])
        .env("TOPHAT_DEV_BINARY", "/nonexistent/tophat")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("/nonexistent/tophat"),
        "{}",
        stderr(&out)
    );
}

#[test] // rq-5c404476
fn the_development_binary_goes_into_the_build_context() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env.run(&["--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let copied = env
        .project()
        .join(".tophat/state/build/tophat-source/tophat");
    assert_eq!(
        fs::read(copied).unwrap(),
        fs::read(env!("CARGO_BIN_EXE_tophat")).unwrap()
    );
    let builds: Vec<String> = stdout(&out)
        .lines()
        .filter(|l| l.starts_with("podman build"))
        .map(str::to_string)
        .collect();
    assert_eq!(builds.len(), 3, "{builds:?}");
    assert!(
        builds[0].contains("--build-arg TOPHAT_DEV_BUILD=1"),
        "{}",
        builds[0]
    );
    assert!(
        builds[0].contains("--build-arg PIXI_VERSION=0.82.1"),
        "{}",
        builds[0]
    );
}

// --- Session ------------------------------------------------------------------------------------

#[test] // rq-1cc9f489 rq-5fb69e3a
fn the_session_mounts_only_the_workspace_roots_and_its_own_volumes() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let results = env.path("data/results");
    fs::create_dir_all(&results).unwrap();
    env.write(
        ".tophat/roots.local.yaml",
        &format!("roots:\n  results: {}\n", results.display()),
    );
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    let id = env.id();
    for expected in [
        format!("-v {}:/work ", env.project().display()),
        format!("-v {0}:{0} ", results.display()),
        format!("-v tophat-{id}-pixi-cache:/opt/tophat/cache/pixi"),
        format!("-v tophat-{id}-claude:/opt/tophat/home/.claude"),
        "/work/.tophat/managed/launch/enter.sh".to_string(),
    ] {
        assert!(
            session.contains(&expected),
            "{expected} missing from {session}"
        );
    }
    for forbidden in [
        ".ssh",
        ".gitconfig",
        ".git-credentials",
        ".netrc",
        "SSH_AUTH_SOCK",
    ] {
        assert!(!session.contains(forbidden), "{forbidden} in {session}");
    }
    // Exactly the workspace, the root and the two volumes are mounted.
    assert_eq!(session.matches(" -v ").count(), 4, "{session}");
}

#[test] // rq-6026766d rq-9f886d87
fn the_hosts_git_identity_does_not_enter_the_container() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let home = env.path("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = A. Scientist\n\temail = a@example.org\n",
    )
    .unwrap();
    let out = env
        .launcher(&["--dry-run"])
        .env("HOME", &home)
        .env("GIT_AUTHOR_NAME", "A. Scientist")
        .env("EMAIL", "a@example.org")
        .output()
        .unwrap();
    let session = env.session(&out);
    for forbidden in ["GIT_", "EMAIL", "A. Scientist", "a@example.org"] {
        assert!(!session.contains(forbidden), "{forbidden} in {session}");
    }
}

#[test] // rq-f9a0d3e0 rq-fa54ed44
fn root_variables_are_bound_and_passed_through() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    declare_raw(&env);
    let raw = env.path("scratch/raw");
    fs::create_dir_all(&raw).unwrap();
    let out = env
        .launcher(&["--dry-run"])
        .env("TOPHAT_ROOT_RAW", &raw)
        .output()
        .unwrap();
    let session = env.session(&out);
    assert!(
        session.contains(&format!("-v {0}:{0} ", raw.display())),
        "{session}"
    );
    assert!(session.contains("-e TOPHAT_ROOT_RAW"), "{session}");
}

#[test] // rq-1537c938 rq-fa54ed44
fn a_missing_root_location_stops_the_launch() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    declare_raw(&env);
    env.write(".tophat/roots.local.yaml", "roots:\n  raw: /nonexistent\n");
    let out = env.run(&["--dry-run"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("'raw'"), "{}", stderr(&out));
    assert!(!stdout(&out).contains("podman run --rm -it"));
}

#[test] // rq-aba117e6 rq-fa54ed44
fn invalid_roots_files_stop_the_launch() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.write(
        ".tophat/roots.local.yaml",
        "roots:\n  results: data/results\n",
    );
    let out = env.run(&["--dry-run"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("the data roots cannot be read") && err.contains(".tophat/roots.local.yaml"),
        "{err}"
    );
}

#[test] // rq-7d26723f rq-fa54ed44
fn every_yaml_form_tophat_accepts_is_understood() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let results = env.path("data/results");
    fs::create_dir_all(&results).unwrap();
    env.write(
        ".tophat/roots.local.yaml",
        &format!("roots: {{results: {}}}\n", results.display()),
    );
    let session = env.session(&env.run(&["--dry-run"]));
    assert!(
        session.contains(&format!("-v {0}:{0} ", results.display())),
        "{session}"
    );
}

#[test] // rq-6de57a60 rq-0df094cd
fn local_tool_repositories_are_mounted_read_only() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let method = env.path("src/method-x");
    let lib = env.path("src/lib-y");
    fs::create_dir_all(&method).unwrap();
    fs::create_dir_all(&lib).unwrap();
    env.write(
        ".tophat/tools.yaml",
        &format!(
            "tools:\n  method-x:\n    url: {}\n  lib-y:\n    url: file://{}\n  web-z:\n    url: https://github.com/example/web-z.git\n",
            method.display(),
            lib.display()
        ),
    );
    let session = env.session(&env.run(&["--dry-run"]));
    assert!(
        session.contains(&format!("-v {0}:{0}:ro ", method.display())),
        "{session}"
    );
    assert!(
        session.contains(&format!("-v {0}:{0}:ro ", lib.display())),
        "{session}"
    );
    assert!(!session.contains("web-z"), "{session}");
    // The workspace, the two volumes and the two tools.
    assert_eq!(session.matches(" -v ").count(), 5, "{session}");
}

#[test] // rq-a37e2c54 rq-0df094cd
fn a_missing_local_tool_repository_is_reported_not_fatal() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.write(
        ".tophat/tools.yaml",
        "tools:\n  method-x:\n    url: /nonexistent/method-x\n",
    );
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    let err = stderr(&out);
    assert!(
        err.contains("'method-x'") && err.contains("/nonexistent/method-x"),
        "{err}"
    );
    assert!(!session.contains("/nonexistent"), "{session}");
}

#[test] // rq-e5151256 rq-c490844e
fn personal_skills_are_mounted_read_only_one_by_one() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let folder = skill(&env.path("home/.config/tophat/skills"), "writing-clearly");
    let session = env.session(&env.run(&["--dry-run"]));
    assert_eq!(
        skill_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/skills/writing-clearly:ro",
            folder.display()
        )],
        "{session}"
    );
}

#[test] // rq-7be133fa rq-c490844e
fn xdg_config_home_locates_the_personal_skills_folder() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let folder = skill(&env.path("cfg/tophat/skills"), "writing-clearly");
    skill(&env.path("home/.config/tophat/skills"), "other");
    let out = env
        .launcher(&["--dry-run"])
        .env("XDG_CONFIG_HOME", env.path("cfg"))
        .output()
        .unwrap();
    let session = env.session(&out);
    assert_eq!(
        skill_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/skills/writing-clearly:ro",
            folder.display()
        )],
        "{session}"
    );
    assert!(!session.contains("other"), "{session}");
}

#[test] // rq-29a38dbc rq-c490844e
fn without_a_personal_skills_folder_nothing_changes() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    assert!(skill_binds(&session).is_empty(), "{session}");
    assert!(!stderr(&out).contains("personal skill"), "{}", stderr(&out));
}

#[test] // rq-59aee61f rq-c490844e
fn a_personal_skill_does_not_shadow_a_project_skill() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let dir = env.path("home/.config/tophat/skills");
    skill(&dir, "th-study");
    let folder = skill(&dir, "writing-clearly");
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    let err = stderr(&out);
    assert!(
        err.contains("'th-study'") && err.contains("name of a project skill"),
        "{err}"
    );
    assert_eq!(
        skill_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/skills/writing-clearly:ro",
            folder.display()
        )],
        "{session}"
    );
}

#[test] // rq-75ce0958 rq-c490844e
fn entries_that_are_not_skills_are_skipped_with_a_warning() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let dir = env.path("home/.config/tophat/skills");
    let folder = skill(&dir, "writing-clearly");
    skill(&dir, "My Skill");
    fs::create_dir_all(dir.join("drafts")).unwrap();
    fs::write(dir.join("notes.txt"), "notes\n").unwrap();
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    let err = stderr(&out);
    for name in ["'notes.txt'", "'drafts'", "'My Skill'"] {
        assert!(err.contains(name), "{name} in {err}");
    }
    assert_eq!(
        skill_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/skills/writing-clearly:ro",
            folder.display()
        )],
        "{session}"
    );
}

#[cfg(unix)]
#[test] // rq-6f6a1986 rq-c490844e
fn a_symbolic_link_to_a_skill_is_followed() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let target = skill(&env.path("src"), "writing-clearly");
    let dir = env.path("home/.config/tophat/skills");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&target, dir.join("writing-clearly")).unwrap();
    let session = env.session(&env.run(&["--dry-run"]));
    let target = fs::canonicalize(&target).unwrap();
    assert_eq!(
        skill_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/skills/writing-clearly:ro",
            target.display()
        )],
        "{session}"
    );
}

#[test] // rq-42f6f9ba rq-c490844e
fn the_check_mounts_no_personal_skills() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    skill(&env.path("home/.config/tophat/skills"), "writing-clearly");
    env.run(&["--check"]);
    let log = env.log();
    let check = log
        .lines()
        .find(|l| l.starts_with("run ") && l.ends_with(" tophat check"))
        .unwrap_or_else(|| panic!("{log}"));
    assert!(!check.contains(".claude/skills"), "{check}");
}

/// The binds the session command makes at Claude Code's user-level instructions.
fn instruction_binds(session: &str) -> Vec<&str> {
    session
        .split(' ')
        .filter(|word| word.contains(":/opt/tophat/home/.claude/CLAUDE.md"))
        .collect()
}

#[test] // rq-97a969e1 rq-59768c6a
fn personal_instructions_are_mounted_read_only() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let file = env.path("home/.config/tophat/CLAUDE.md");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "Apply the honey skill.\n").unwrap();
    let session = env.session(&env.run(&["--dry-run"]));
    assert_eq!(
        instruction_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/CLAUDE.md:ro",
            file.display()
        )],
        "{session}"
    );
}

#[test] // rq-8f775172 rq-59768c6a
fn xdg_config_home_locates_the_personal_instructions() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let file = env.path("cfg/tophat/CLAUDE.md");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "mine\n").unwrap();
    let home = env.path("home/.config/tophat");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("CLAUDE.md"), "other\n").unwrap();
    let out = env
        .launcher(&["--dry-run"])
        .env("XDG_CONFIG_HOME", env.path("cfg"))
        .output()
        .unwrap();
    let session = env.session(&out);
    assert_eq!(
        instruction_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/CLAUDE.md:ro",
            file.display()
        )],
        "{session}"
    );
    assert!(!session.contains(&home.display().to_string()), "{session}");
}

#[test] // rq-972c7f77 rq-59768c6a
fn without_personal_instructions_nothing_changes() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    assert!(instruction_binds(&session).is_empty(), "{session}");
    assert!(
        !stderr(&out).contains("personal instructions"),
        "{}",
        stderr(&out)
    );
}

#[test] // rq-b18523ab rq-59768c6a
fn a_personal_instructions_path_that_is_not_a_file_is_skipped_with_a_warning() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let dir = env.path("home/.config/tophat/CLAUDE.md");
    fs::create_dir_all(&dir).unwrap();
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    let err = stderr(&out);
    assert!(
        err.contains(&dir.display().to_string()) && err.contains("not a file"),
        "{err}"
    );
    assert!(instruction_binds(&session).is_empty(), "{session}");
}

#[cfg(unix)]
#[test] // rq-039ad741 rq-59768c6a
fn a_symbolic_link_to_personal_instructions_is_followed() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let target = env.path("src/notes/CLAUDE.md");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "notes\n").unwrap();
    let dir = env.path("home/.config/tophat");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&target, dir.join("CLAUDE.md")).unwrap();
    let session = env.session(&env.run(&["--dry-run"]));
    let target = fs::canonicalize(&target).unwrap();
    assert_eq!(
        instruction_binds(&session),
        [format!(
            "{}:/opt/tophat/home/.claude/CLAUDE.md:ro",
            target.display()
        )],
        "{session}"
    );
}

#[cfg(unix)]
#[test] // rq-59768c6a
fn a_symbolic_link_loop_is_skipped_with_a_warning() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let dir = env.path("home/.config/tophat");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(dir.join("CLAUDE.md"), dir.join("CLAUDE.md")).unwrap();
    let out = env.run(&["--dry-run"]);
    let session = env.session(&out);
    assert!(stderr(&out).contains("loop"), "{}", stderr(&out));
    assert!(instruction_binds(&session).is_empty(), "{session}");
}

#[test] // rq-b9501494 rq-59768c6a
fn the_check_mounts_no_personal_instructions() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let dir = env.path("home/.config/tophat");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("CLAUDE.md"), "mine\n").unwrap();
    env.run(&["--check"]);
    let log = env.log();
    let check = log
        .lines()
        .find(|l| l.starts_with("run ") && l.ends_with(" tophat check"))
        .unwrap_or_else(|| panic!("{log}"));
    assert!(!check.contains("CLAUDE.md"), "{check}");
}

#[test] // rq-3c33f183 rq-2b0ca72f
fn project_run_options_are_appended_and_validated() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.write(
        ".tophat/user/podman/run-options",
        "# options\n--memory=8g\n",
    );
    let session = env.session(&env.run(&["--dry-run"]));
    let image = format!(" localhost/tophat-{}-project:latest ", env.id());
    let option = session.find(" --memory=8g").unwrap();
    let image_at = session.find(&image).unwrap();
    assert_eq!(option + " --memory=8g".len(), image_at, "{session}");
}

#[test] // rq-5df9545e rq-2b0ca72f
fn an_invalid_run_option_is_refused() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.write(
        ".tophat/user/podman/run-options",
        "# options\n--memory 8g\n",
    );
    let out = env.run(&["--dry-run"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains(".tophat/user/podman/run-options line 2"),
        "{err}"
    );
    assert_eq!(env.log(), "");
}

#[test] // rq-b3ee805a rq-71727623
fn the_check_runs_in_a_throwaway_container() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.write(
        "PROJECT.md",
        "# Method X\n\n## q-1a2b3c4d · Does X work?\n\nNo status line here.\n",
    );
    let out = env.run(&["--check"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("question.status-missing"),
        "{}",
        stdout(&out)
    );
    let log = env.log();
    let check = log
        .lines()
        .find(|l| l.starts_with("run ") && l.ends_with(" tophat check"))
        .unwrap_or_else(|| panic!("{log}"));
    assert!(
        check.contains(&format!("-v {}:/work:ro", env.project().display())),
        "{check}"
    );
    assert!(!check.contains(" -it"), "{check}");
}

// --- Git hooks ----------------------------------------------------------------------------------

#[test] // rq-2f11c7ec rq-81e54133
fn the_hook_path_is_set_when_unset() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    assert_eq!(
        env.git(&["config", "--local", "--get", "core.hooksPath"]),
        ""
    );
    assert!(env.run(&["--dry-run"]).status.success());
    assert_eq!(
        env.git(&["config", "--local", "--get", "core.hooksPath"]),
        ".tophat/managed/hooks"
    );
}

#[test] // rq-f503e56e rq-81e54133
fn an_existing_hook_path_is_left_alone() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    env.git(&["config", "--local", "core.hooksPath", ".husky"]);
    let out = env.run(&["--dry-run"]);
    assert!(out.status.success());
    assert_eq!(
        env.git(&["config", "--local", "--get", "core.hooksPath"]),
        ".husky"
    );
    assert!(
        stderr(&out).contains("the TopHat pre-commit hook is not active"),
        "{}",
        stderr(&out)
    );
}

// --- Session start (enter.sh) -------------------------------------------------------------------

/// Runs enter.sh with a fake pixi (logging, exiting with `pixi_exit`) and the workspace's tophat.
fn enter(env: &Env, pixi_exit: i32, extra: &[(&str, &str)]) -> Output {
    let bin = env.path("enter-bin");
    fs::create_dir_all(&bin).unwrap();
    executable(
        &bin.join("pixi"),
        &format!(
            "#!/bin/sh\nprintf 'pixi %s\\n' \"$*\" >> '{}'\necho pixi-error >&2\nexit {pixi_exit}\n",
            env.path("pixi.log").display()
        ),
    );
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tophat"), bin.join("tophat"));
    Command::new("sh")
        .args([
            ".tophat/managed/launch/enter.sh",
            "sh",
            "-c",
            "echo shell-started",
        ])
        .current_dir(env.project())
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .envs(extra.iter().copied())
        .output()
        .unwrap()
}

#[test] // rq-e9865354 rq-5fb69e3a
fn the_environment_is_installed_when_the_session_starts() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = enter(&env, 0, &[("TOPHAT_DEV_BUILD", "1")]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        fs::read_to_string(env.path("pixi.log")).unwrap(),
        "pixi install --locked\n"
    );
    let text = stdout(&out);
    let status = text
        .find("questions: 0 open")
        .expect("tophat status was printed");
    let dev = text
        .find("development build of tophat")
        .expect("dev notice");
    let shell = text.find("shell-started").expect("the command ran");
    assert!(dev < status && status < shell, "{text}");
}

#[test] // rq-7478e01d rq-5fb69e3a
fn a_stale_lock_does_not_prevent_the_session() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let out = enter(&env, 1, &[]);
    let err = stderr(&out);
    assert!(err.contains("pixi-error"), "{err}");
    assert!(err.contains("pixi.toml and pixi.lock disagree"), "{err}");
    assert!(stdout(&out).contains("shell-started"));
    assert!(!stdout(&out).contains("development build"));
}

// --- Container check ----------------------------------------------------------------------------

fn check_container(env: &Env, root: &Path) -> Output {
    let mut cmd = Command::new("sh");
    cmd.arg(".tophat/managed/hooks/check-container.sh")
        .arg(root)
        .current_dir(env.project());
    for var in [
        "container",
        "APPTAINER_CONTAINER",
        "SINGULARITY_CONTAINER",
        "KUBERNETES_SERVICE_HOST",
    ] {
        cmd.env_remove(var);
    }
    cmd.output().unwrap()
}

#[test] // rq-c65f79e0 rq-1cf4eaf3
fn prompts_are_allowed_inside_the_container() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let root = env.path("sysroot");
    fs::create_dir_all(root.join("run")).unwrap();
    fs::write(root.join("run/.containerenv"), "").unwrap();
    assert_eq!(check_container(&env, &root).status.code(), Some(0));
}

#[test] // rq-36a97588 rq-1cf4eaf3
fn prompts_are_refused_on_the_host() {
    if !common::copier() {
        return;
    }
    let env = Env::new();
    let root = env.path("sysroot");
    fs::create_dir_all(root.join("proc/1")).unwrap();
    fs::write(root.join("proc/1/cgroup"), "0::/user.slice\n").unwrap();
    let out = check_container(&env, &root);
    assert_eq!(out.status.code(), Some(2));
    let decision: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(decision["decision"], "block");
    assert!(decision["reason"].as_str().unwrap().contains("th.sh"));
}

// --- Image (real Podman) ------------------------------------------------------------------------

/// Whether the image tests run: they build real images, which needs Podman and network access.
fn podman_enabled() -> bool {
    let enabled = std::env::var_os("TOPHAT_TEST_PODMAN").is_some_and(|v| v == "1");
    if !enabled {
        eprintln!("skipped: set TOPHAT_TEST_PODMAN=1 (with Podman installed) to build real images");
    }
    enabled && common::copier()
}

/// Builds the project's image with the real podman (through `th.sh --check`, which builds when no
/// image exists) and returns its name.
fn build_image(env: &Env) -> String {
    let out = Command::new("sh")
        .args(["./th.sh", "--check"])
        .current_dir(env.project())
        .env("TOPHAT_DEV_BINARY", env!("CARGO_BIN_EXE_tophat"))
        .output()
        .unwrap();
    let image = format!("localhost/tophat-{}-project:latest", env.id());
    assert!(
        Command::new("podman")
            .args(["image", "exists", &image])
            .status()
            .unwrap()
            .success(),
        "the image was not built: {}{}",
        stdout(&out),
        stderr(&out)
    );
    image
}

fn in_image(env: &Env, image: &str, script: &str) -> Output {
    Command::new("podman")
        .args(["run", "--rm", "-v"])
        .arg(format!("{}:/work", env.project().display()))
        .args(["-w", "/work", image, "sh", "-c", script])
        .output()
        .unwrap()
}

#[test] // rq-9f3d3812 rq-5c404476
fn a_development_binary_replaces_the_release() {
    if !podman_enabled() {
        return;
    }
    let env = Env::new();
    let image = build_image(&env);
    let out = in_image(
        &env,
        &image,
        "tophat --version && echo dev=$TOPHAT_DEV_BUILD",
    );
    let text = stdout(&out);
    assert!(
        text.contains(&format!("tophat {}", env!("CARGO_PKG_VERSION"))),
        "{text}"
    );
    assert!(text.contains("dev=1"), "{text}");
}

#[test] // rq-007bd449 rq-5c404476
fn claude_code_is_installed_in_the_image() {
    if !podman_enabled() {
        return;
    }
    let env = Env::new();
    let image = build_image(&env);
    assert!(in_image(&env, &image, "claude --version").status.success());
}

#[test] // rq-df0c42b9 rq-9f886d87
fn git_refuses_to_commit_inside_the_container() {
    if !podman_enabled() {
        return;
    }
    let env = Env::new();
    let image = build_image(&env);
    let out = in_image(
        &env,
        &image,
        "git add -A && if git commit -qm x; then echo committed; else echo refused; fi; git status --short >/dev/null && echo status-ok",
    );
    let text = stdout(&out);
    assert!(
        text.contains("refused") && !text.contains("committed"),
        "{text}{}",
        stderr(&out)
    );
    assert!(text.contains("status-ok"), "{text}");
}

#[test] // rq-ea93a50d rq-9f886d87
fn a_commit_made_on_the_host_is_seen_in_the_container() {
    if !podman_enabled() {
        return;
    }
    let env = Env::new();
    let image = build_image(&env);
    env.git(&["add", "-A"]);
    let before = stdout(&in_image(&env, &image, "git status --porcelain"));
    assert!(before.contains("PROJECT.md"), "{before}");
    Command::new("git")
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "host",
        ])
        .current_dir(env.project())
        .status()
        .unwrap();
    let after = stdout(&in_image(&env, &image, "git status --porcelain"));
    assert!(!after.contains("PROJECT.md"), "{after}");
}
