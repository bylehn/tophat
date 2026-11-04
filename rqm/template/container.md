# Feature: Dev Container and Launcher <!-- rq-4d2bc439 -->

All work in a TopHat project happens inside a dev container: the agent, pixi, Snakemake and the
`tophat` CLI all run there, never on the host. The project's launcher, `th.sh`, builds the
container image with Podman, starts an interactive session with the workspace and the project's
data roots mounted, and gives the container no credentials of the human's: no SSH keys, no agent
sockets, no git or cloud credentials, and no git identity.

This file specifies the launcher and its options, the image, the session (mounts, environment,
pixi, git hooks), run options, and the check that keeps agents from running on the host. GPU
passthrough is specified in `rqm/template/gpu.md` and running the image under Apptainer in
`rqm/template/apptainer.md`; both extend the launcher described here. File ownership (managed vs
project-owned) follows `rqm/template/project.md`; the agent configuration files that register the
container check are specified in `rqm/template/skills.md` and `rqm/template/agents.md`; data roots
in `rqm/data.md`.

## Files <!-- rq-48a62199 -->

```text
th.sh                                     managed   adapter: exec .tophat/managed/launch/th.sh
.tophat/project-id                        project   the project's UUID, created on first launch
.tophat/managed/
  launch/th.sh                            managed   the launcher
  launch/run-options.sh                   managed   run-options validation
  launch/enter.sh                         managed   session start inside the container
  container/Containerfile                 managed   the base image
  container/Agent.Containerfile           managed   the agent layer
  container/versions.yaml                 managed   pinned pixi (with checksums), Copier and agent versions
  hooks/check-container.sh                managed   refuses agent prompts outside a container
.tophat/user/
  container/Containerfile                 project   project additions to the image
  podman/run-options                      project   extra `podman run` options
.tophat/state/                            ignored   machine-local launcher state
```

- `th.sh` at the project root changes to the project root and runs `.tophat/managed/launch/th.sh`;
  it is listed in `.tophat/managed/ownership-exceptions`.
- `.tophat/user/container/Containerfile` and `.tophat/user/podman/run-options` are project-owned
  (written once, in `_skip_if_exists`); the run-options file starts with only comments.
- The launcher assembles each build in `.tophat/state/build/`.
- `.tophat/state/` is git-ignored (see the ignore rules in `rqm/template/project.md`).

## Launcher <!-- rq-71727623 -->

`th.sh` is a POSIX `sh` script run from the project root on a Linux host, on macOS with a Podman
machine, or in WSL2 on Windows.

| Invocation | Effect |
|---|---|
| `th.sh` | Builds the image if needed, sets up git hooks, and starts an interactive session |
| `th.sh --check` | Runs `tophat check` in a throwaway container with the project mounted read-only, and exits with its exit code; builds the image only when none exists |
| `th.sh --dry-run` | Builds nothing and starts no session: prints, one per line, the build and session commands it would run |

- `--dry-run` still does what the session's commands depend on and what a launch always does: it
  creates the project ID, sets the hook path, and runs the read-only `tophat data roots` query (see
  *Data roots*) from the image of an earlier launch. Without such an image it refuses, saying to
  launch once first.
- Printed commands quote only the words that need it.

- Unknown options are refused with the usage text, before anything runs.
- Every refusal names its cause and exits 2 before any container starts. Exit 1 is left to
  `tophat check` finding errors, so a caller of `--check` (such as the pre-commit hook) can tell a
  check that failed from one that could not run.
- On first launch the launcher writes a new random UUID to `.tophat/project-id`; later launches
  read it. A malformed ID is refused. Image, volume and state names include it, so projects never
  share them.
- Under WSL2, when `podman` is the Windows binary, mount sources are translated with `wslpath -w`.

## Image <!-- rq-5c404476 -->

The image is built in three layers, all for the host's architecture (`linux-64` or
`linux-aarch64`):

1. **Base** (`.tophat/managed/container/Containerfile`): `ubuntu:24.04` with `bash`, `git`,
   `ca-certificates`, `curl`, `pipx`, and `less` and `vim` for reading and editing in a session; pixi at the version in `versions.yaml`, whose archive's
   SHA-256 must match the one listed there for the architecture; Copier at its version; and the `tophat`
   binary from `.tophat/managed/tophat-release.yaml` for the image's architecture. The build
   downloads the binary, compares its SHA-256 with the pinned hash, and fails on a mismatch. When
   `TOPHAT_DEV_BINARY` is set on the host to a Linux `tophat` binary (for example one built from the
   TopHat repository), the launcher copies it into the build context and the image uses it instead,
   downloading nothing; this is the only way to build an image while the pin is
   `version: unreleased`, which otherwise refuses the launch. An image with a development binary sets
   `TOPHAT_DEV_BUILD=1` in its environment. The
   image's system git configuration sets `user.useConfigOnly = true` and no identity, so git
   refuses to commit in the container (see *Commits happen on the host*). Programs
   live under an image-owned home, `/opt/tophat/home`, readable by any user, so the image works
   both as root (Podman) and as the invoking user (Apptainer).
2. **Agents** (`Agent.Containerfile`): the agents selected by the `agents` Copier question, at the
   versions in `versions.yaml`, with their self-updaters disabled. Their configuration homes are
   left empty: they are mount points for the session's agent volumes.
3. **Project** (`.tophat/user/container/Containerfile`): built `FROM` the agent layer; it starts with
   only that `FROM` line and a comment, and the project adds system packages there. Language
   environments belong in pixi, not here.

- Image names are `localhost/tophat-<project-id>-<layer>:latest`.
- A failed build leaves the previous image in place and stops the launch.
- The pixi environment is not part of the image: it lives in the workspace's `.pixi/` directory
  (see *Session*), so the image does not change when the environment does.

## Session <!-- rq-5fb69e3a -->

The session runs `podman run --rm -it` with exactly these mounts and variables:

| Mount or variable | Purpose |
|---|---|
| the project root at `/work`, working directory `/work` | the workspace |
| each data root location at the same path (see *Data roots*) | data roots |
| volume `tophat-<project-id>-pixi-cache` at the pixi cache directory | package cache across sessions |
| volume `tophat-<project-id>-<agent>` at each selected agent's configuration home | the agent's own login and session state |
| `TOPHAT_ROOT_<NAME>` | passed through when set on the host (see `rqm/data.md`) |
| each local tool repository, read-only, at the same path (see *Local tools*) | so `tophat tools sync` can fetch from it |
| each personal skill, read-only, in the agent's skills directory (see *Personal skills*) | the human's own skills, in every project |
| the personal instructions file, read-only, as the agent's user-level instructions (see *Personal instructions*) | the human's own standing instructions, in every project |
| `.tophat/user/podman/run-options` | appended last, so the project's options win |

Nothing else from the host enters the container. In particular the session never mounts `~/.ssh`,
`~/.gitconfig`, `~/.git-credentials`, `~/.netrc`, `~/.config/gh`, cloud credential directories or
the host's agent configuration, and never passes `SSH_AUTH_SOCK`, `GIT_*`, `EMAIL` or other host
variables. The container has network access, which pixi,
the agents and `tophat tools sync` (public repositories only, see `rqm/pipeline/tools.md`) use.

The session's command is `.tophat/managed/launch/enter.sh`, which:

1. runs `pixi install --locked`, which installs exactly the committed lock and refuses a lock that
   does not satisfy `pixi.toml`; on failure it prints pixi's error and a line saying that
   `pixi.toml` and `pixi.lock` disagree or a package could not be fetched, and continues;
2. prints `tophat status`, preceded, when `TOPHAT_DEV_BUILD=1`, by a line saying that this session
   runs a development build of `tophat` and its version;
3. replaces itself with the command given as its arguments or, without arguments (as in a session),
   with an interactive `bash` in which `pixi`, `tophat` and, through `pixi run`, Snakemake are
   available.

### Data roots <!-- rq-fa54ed44 -->

The launcher asks `tophat` where the data roots are, so it reads `.tophat/roots.yaml` and
`.tophat/roots.local.yaml` exactly as the CLI does:

- After building, it runs `tophat data roots` in a throwaway container from the new image, with the
  workspace mounted read-only and the host's `TOPHAT_ROOT_<NAME>` variables passed in (see
  `rqm/data.md`).
- When that command fails (invalid roots files), the launch is refused with its message.
- Each configured location is bound read-write at the identical path inside the container, so a
  path in a manifest, provenance record or job folder means the same file on the host and in the
  container.
- A configured location that does not exist on the host refuses the launch, naming the root.
  Unconfigured roots bind nothing, and neither does a location inside the project (such as the
  default `results/`): the workspace mount already holds it.

### Local tools <!-- rq-0df094cd -->

A tool declared in `.tophat/tools.yaml` by an absolute path or a `file://` URL (see
`rqm/pipeline/tools.md`) is a repository on the host, which the container cannot see unless it is
mounted:

- The launcher runs `tophat tools list` in a throwaway container from the image, like the data-roots
  query, and binds each local tool repository **read-only** at the identical path, so the URL in
  `.tophat/tools.yaml` works inside the container and nothing in the container can change the tool.
- A local tool path that does not exist on the host gets a warning naming the tool and is not
  mounted; the session still starts (its `tophat tools sync` then reports the tool as failed).
- When `tophat tools list` fails (invalid tool files), the launcher warns and mounts no tools; the
  session still starts, so the human can fix the files there.
- Public `https://` tools need no mount, and neither does a path inside the project.
- Mounts are made when the launcher starts the container: a local tool declared during a session is
  mounted only after the human exits the container and runs `./th.sh` again; restarting the agent
  inside the container is not enough.
- `--check` mounts no tools: the check reads only the tool cache.

### Personal skills <!-- rq-c490844e -->

A human can keep agent skills of their own (a writing style guide, for example) on the host and have
them in every TopHat session they start, without adding them to any project. Nothing about them is
committed, so a project never imposes one human's skills on another.

- The personal skills folder is `$XDG_CONFIG_HOME/tophat/skills`, or `~/.config/tophat/skills` when
  `XDG_CONFIG_HOME` is unset or empty. When the folder does not exist, the launcher mounts nothing
  and prints nothing: the feature is opt-in per human.
- Each direct subdirectory holding a `SKILL.md` is one skill, named after the subdirectory. The
  launcher binds it **read-only** into each selected agent's user-level skills directory:
  `/opt/tophat/home/.claude/skills/<name>` for Claude Code and
  `/opt/tophat/home/.agents/skills/<name>` for Codex and OpenCode, which share that directory.
  Skills are mounted one by one, never the folder as a whole, so each agent's own skills directory
  in its configuration volume (for example skills synced from the human's account) stays visible.
- A subdirectory that is a symbolic link is followed and its target mounted. Symbolic links inside a
  skill that point outside it do not resolve in the container.
- These entries are skipped with a warning naming the entry and the reason, and the session still
  starts:
  - a file, or a subdirectory without a `SKILL.md`;
  - a name that is not lowercase letters, digits and hyphens;
  - a name used by a skill in the project's skills directory for any agent (`.claude/skills/`,
    `.agents/skills/` or `.opencode/skills/`, such as `th-study`): a personal skill never shadows a
    project or managed skill;
  - a path containing `:` or `,`, which a mount option cannot express.
- Mounts are made when the launcher starts the container: edits to the files of a mounted skill show
  up in the session at once, but a skill added or removed on the host takes effect only after the
  human exits and runs `./th.sh` again.
- The skill is mounted read-only, so nothing in the container can change it. The empty directory a
  mount leaves in the agent volume when the skill is later removed from the host is harmless: without
  a `SKILL.md`, the agent ignores it.
- `--check` mounts no personal skills: the check runs no agent.
- Personal skills are mounted for every selected agent. Codex and OpenCode share one mount at
  `/opt/tophat/home/.agents/skills`.

### Personal instructions <!-- rq-59768c6a -->

A human can keep standing instructions of their own (which personal skill to apply when, for
example) on the host and have every selected agent read them in every TopHat session they start,
without adding them to any project. Like personal skills, nothing about them is committed.

- The personal instructions file is `CLAUDE.md` in the folder that holds the personal skills folder:
  `$XDG_CONFIG_HOME/tophat/CLAUDE.md`, or `~/.config/tophat/CLAUDE.md` when `XDG_CONFIG_HOME` is
  unset or empty. When the file does not exist, the launcher mounts nothing and prints nothing. It is
  independent of the skills folder: either can exist without the other.
- The launcher binds it **read-only** as each selected agent's user-level instructions:
  `/opt/tophat/home/.claude/CLAUDE.md` for Claude Code, `/opt/tophat/home/.codex/AGENTS.md` for
  Codex and `/opt/tophat/home/.config/opencode/AGENTS.md` for OpenCode (`HOME` is
  `/opt/tophat/home`). Each agent reads it at the start of every session alongside the project's
  instructions; where the two conflict, the project's instructions take precedence, so personal
  instructions never override TopHat's rules.
- A symbolic link is followed and its target mounted.
- These cases are skipped with a warning naming the path and the reason, and the session still
  starts:
  - the path is a directory or other non-regular file;
  - the resolved path contains `:` or `,`, which a mount option cannot express.
- While the file is mounted, an instructions file an agent volume already holds (such as
  `~/.claude/CLAUDE.md` or `~/.codex/AGENTS.md`) is hidden but left unchanged, and writes to the
  user-level instructions from inside the container fail: the human edits the file on the host.
- Each agent reads the file when it starts, so an edit on the host takes effect at the next agent
  session; a file created or removed on the host takes effect only after the human exits and runs
  `./th.sh` again.
- `--check` does not mount it: the check runs no agent.

### Commits happen on the host <!-- rq-9f886d87 -->

The container carries no git identity and no git credentials, so every commit, push and pull is
made by the human on the host. This keeps a human in the loop for everything that can be reported:
a provenance record is clean only for code committed at `HEAD` (see `rqm/pipeline/provenance.md`),
so every reportable result sits on a commit a human made. Inside the container git still reads the
repository (`status`, `diff`, `log`) and `tophat` uses it read-only; the workspace is a bind mount,
so a commit made on the host is visible in the container at once. Agents are also denied
`git commit` and `git push` (see `rqm/template/skills.md`).

### Git hooks <!-- rq-81e54133 -->

Before the session starts, the launcher sets `git config --local core.hooksPath .tophat/managed/hooks`
when `core.hooksPath` is unset. When it is set to anything else, the launcher leaves it unchanged
and prints that the TopHat pre-commit hook is not active and that the existing hook can call
`.tophat/managed/hooks/pre-commit`. Outside a git repository it does nothing.

## Run Options <!-- rq-2b0ca72f -->

`.tophat/user/podman/run-options` holds one option per line;
blank lines and `#` comments are ignored. A line containing whitespace, or not starting with `-`, is
refused with the file name and line number before any container starts. The options are appended
after the launcher's own, so a runtime that takes the last occurrence of an option takes the
project's.

## Container Check <!-- rq-1cf4eaf3 -->

`.tophat/managed/hooks/check-container.sh` exits 0 inside a container (Podman's
`/run/.containerenv` or `container=podman`, Apptainer's `APPTAINER_CONTAINER` or `/.singularity.d`,
a container cgroup, or an overlay root filesystem). The files are looked up under a system root, `/`
by default or the directory given as its first argument, so tests can simulate hosts. Outside one it prints, as JSON, a block decision
whose reason says to start the session with `th.sh`, and exits 2. The agent configuration files
(see `rqm/template/skills.md`) run it before every prompt, so an agent started on the host refuses
to work.

## Assumptions <!-- rq-14cdc44c -->

- Podman 4.4 or later on the host.
- On macOS, data root locations are inside directories the Podman machine shares (by default
  `/Users`).

## Out of Scope <!-- rq-656aa36b -->

- Native Windows launchers (`.bat`, PowerShell); Windows is supported through WSL2.
- Docker as the container runtime.
- Forwarding credentials into the container for any command.
- Multi-node or MPI execution inside the container; cluster compute uses external stages.

## Gherkin Scenarios <!-- rq-7ab5b8f0 -->

```gherkin
Feature: Dev container and launcher

  Background:
    Given a project rendered with default answers in a git repository

  # --- Launcher ---

  @rq-22ab7a81
  Scenario: The project ID is created once
    Given .tophat/project-id does not exist
    When "th.sh --dry-run" is run twice
    Then .tophat/project-id holds one UUID
    And both runs name the image "localhost/tophat-<that UUID>-project:latest"

  @rq-df2a2b0e
  Scenario: A malformed project ID is refused
    Given .tophat/project-id contains "not-a-uuid"
    When "th.sh --dry-run" is run
    Then it exits non-zero naming .tophat/project-id

  @rq-5b3d8f35
  Scenario: Unknown options are refused
    When "th.sh --frobnicate" is run
    Then it prints the usage text and exits non-zero

  @rq-18896042
  Scenario: A dry run needs an image from an earlier launch
    Given no image of this project exists
    When "th.sh --dry-run" is run
    Then it exits non-zero saying to launch once first

  # --- Image ---

  @rq-099a04fb
  Scenario: The image holds the pinned tophat
    When the image is built
    Then "tophat --version" in the image prints the version in .tophat/managed/tophat-release.yaml

  @rq-21bea60c
  Scenario: A binary with the wrong checksum fails the build
    Given tophat-release.yaml lists a wrong sha256 for the host's architecture
    When the image is built
    Then the build fails naming the checksum mismatch
    And the previous image is unchanged

  @rq-9f3d3812
  Scenario: A development binary replaces the release
    Given TOPHAT_DEV_BINARY is set to a tophat binary built from the TopHat repository
    When the image is built
    Then nothing is downloaded for tophat
    And "tophat --version" in the image prints that binary's version
    And the session start says it runs a development build

  @rq-9c7772af
  Scenario: An unreleased pin needs a development binary
    Given .tophat/managed/tophat-release.yaml holds "version: unreleased"
    And TOPHAT_DEV_BINARY is unset
    When "th.sh --dry-run" is run
    Then it exits 2 saying no tophat release is pinned and naming TOPHAT_DEV_BINARY

  @rq-bcd5b8bd
  Scenario: A missing development binary is refused
    Given TOPHAT_DEV_BINARY is set to a path that does not exist
    When "th.sh --dry-run" is run
    Then it exits non-zero naming that path

  @rq-007bd449
  Scenario: Claude Code is installed in the image
    When the image is built
    Then "claude --version" succeeds in the image

  # --- Session ---

  @rq-1cc9f489
  Scenario: The session mounts only the workspace, roots and its own volumes
    Given .tophat/roots.local.yaml locates "results" at an existing directory /data/results
    When "th.sh --dry-run" is run
    Then the podman run command binds the project root at /work and /data/results at /data/results
    And mounts the pixi cache volume and one volume per selected agent
    And contains no ".ssh", ".gitconfig", ".git-credentials", ".netrc" or "SSH_AUTH_SOCK"

  @rq-6026766d
  Scenario: The host's git identity does not enter the container
    Given the host's git config has user.name "A. Scientist" and user.email "a@example.org"
    And the host environment sets GIT_AUTHOR_NAME and EMAIL
    When "th.sh --dry-run" is run
    Then the podman run command sets no GIT_ or EMAIL variable and contains neither "A. Scientist" nor "a@example.org"

  @rq-df0c42b9
  Scenario: Git refuses to commit inside the container
    Given a session in the container with a staged change
    When "git commit -m x" is run in the container
    Then it fails asking for an identity
    And "git status" and "git log" succeed

  @rq-ea93a50d
  Scenario: A commit made on the host is seen in the container
    Given a running session and a study whose code was changed and sealed while uncommitted
    When the human commits the change on the host
    Then "tophat check" in the container no longer reports the files as uncommitted

  @rq-b3ee805a
  Scenario: The check runs in a throwaway container
    When "th.sh --check" is run in a project with a check error
    Then it prints the diagnostics of "tophat check" and exits 1
    And the project directory is mounted read-only in the container it runs

  @rq-f9a0d3e0
  Scenario: Root variables are bound and passed through
    Given TOPHAT_ROOT_RAW is set on the host to an existing directory /scratch/raw
    When "th.sh --dry-run" is run
    Then the podman run command binds /scratch/raw at /scratch/raw and sets TOPHAT_ROOT_RAW

  @rq-1537c938
  Scenario: A missing root location stops the launch
    Given .tophat/roots.local.yaml locates "raw" at /nonexistent
    When "th.sh --dry-run" is run
    Then it exits non-zero naming the root "raw"

  @rq-aba117e6
  Scenario: Invalid roots files stop the launch
    Given .tophat/roots.local.yaml locates "results" at the relative path "data/results"
    When "th.sh --dry-run" is run
    Then it exits non-zero with the message of "tophat data roots"

  @rq-e5151256
  Scenario: Personal skills are mounted read-only, one by one
    Given XDG_CONFIG_HOME is unset
    And ~/.config/tophat/skills/writing-clearly/SKILL.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds ~/.config/tophat/skills/writing-clearly at /opt/tophat/home/.claude/skills/writing-clearly read-only
    And it binds nothing else under /opt/tophat/home/.claude/skills

  @rq-7be133fa
  Scenario: XDG_CONFIG_HOME locates the personal skills folder
    Given XDG_CONFIG_HOME is /cfg
    And /cfg/tophat/skills/writing-clearly/SKILL.md exists
    And ~/.config/tophat/skills/other/SKILL.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds /cfg/tophat/skills/writing-clearly
    And it mounts nothing for "other"

  @rq-29a38dbc
  Scenario: Without a personal skills folder nothing changes
    Given the personal skills folder does not exist
    When "th.sh --dry-run" is run
    Then the podman run command binds nothing under /opt/tophat/home/.claude/skills
    And the launcher prints no warning about personal skills

  @rq-59aee61f
  Scenario: A personal skill does not shadow a project skill
    Given the personal skills folder holds th-study/SKILL.md and writing-clearly/SKILL.md
    When "th.sh --dry-run" is run
    Then the launcher warns that the personal skill "th-study" has the name of a project skill and is not mounted
    And the podman run command binds writing-clearly and not th-study

  @rq-75ce0958
  Scenario: Entries that are not skills are skipped with a warning
    Given the personal skills folder holds notes.txt, a directory "drafts" without SKILL.md, a directory "My Skill" with SKILL.md, and writing-clearly/SKILL.md
    When "th.sh --dry-run" is run
    Then the launcher warns naming "notes.txt", "drafts" and "My Skill"
    And the podman run command binds only writing-clearly from the personal skills folder

  @rq-6f6a1986
  Scenario: A symbolic link to a skill is followed
    Given the personal skills folder holds a symbolic link "writing-clearly" to /src/writing-clearly, which holds SKILL.md
    When "th.sh --dry-run" is run
    Then the podman run command binds /src/writing-clearly at /opt/tophat/home/.claude/skills/writing-clearly read-only

  @rq-42f6f9ba
  Scenario: The check mounts no personal skills
    Given the personal skills folder holds writing-clearly/SKILL.md
    When "th.sh --check" is run
    Then its podman run command binds nothing under /opt/tophat/home/.claude/skills

  @rq-96259871
  Scenario: Personal skills are mounted for Codex
    Given the project is rendered with agents ["codex"]
    And ~/.config/tophat/skills/writing-clearly/SKILL.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds ~/.config/tophat/skills/writing-clearly at /opt/tophat/home/.agents/skills/writing-clearly read-only

  @rq-04529493
  Scenario: Personal skills are mounted for OpenCode
    Given the project is rendered with agents ["opencode"]
    And ~/.config/tophat/skills/writing-clearly/SKILL.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds ~/.config/tophat/skills/writing-clearly at /opt/tophat/home/.agents/skills/writing-clearly read-only

  @rq-3f41b27f
  Scenario: Codex and OpenCode share one personal skills mount
    Given the project is rendered with agents ["codex", "opencode"]
    And ~/.config/tophat/skills/writing-clearly/SKILL.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds the skill at /opt/tophat/home/.agents/skills/writing-clearly once

  @rq-bc93ff53
  Scenario: Every selected agent gets the personal skills
    Given the project is rendered with agents ["claude-code", "codex", "opencode"]
    And ~/.config/tophat/skills/writing-clearly/SKILL.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds the skill at /opt/tophat/home/.claude/skills/writing-clearly and at /opt/tophat/home/.agents/skills/writing-clearly

  @rq-a7475e28
  Scenario: A personal skill does not shadow Codex's project skills
    Given the project is rendered with agents ["codex"]
    And the personal skills folder holds th-study/SKILL.md and writing-clearly/SKILL.md
    When "th.sh --dry-run" is run
    Then the launcher warns that the personal skill "th-study" has the name of a project skill and is not mounted
    And the podman run command binds writing-clearly and not th-study

  @rq-97a969e1
  Scenario: Personal instructions are mounted read-only
    Given XDG_CONFIG_HOME is unset
    And ~/.config/tophat/CLAUDE.md exists
    And ~/.config/tophat/skills does not exist
    When "th.sh --dry-run" is run
    Then the podman run command binds ~/.config/tophat/CLAUDE.md at /opt/tophat/home/.claude/CLAUDE.md read-only

  @rq-8f775172
  Scenario: XDG_CONFIG_HOME locates the personal instructions
    Given XDG_CONFIG_HOME is /cfg
    And /cfg/tophat/CLAUDE.md exists
    And ~/.config/tophat/CLAUDE.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds /cfg/tophat/CLAUDE.md at /opt/tophat/home/.claude/CLAUDE.md read-only
    And it binds nothing from ~/.config/tophat

  @rq-972c7f77
  Scenario: Without personal instructions nothing changes
    Given ~/.config/tophat/CLAUDE.md does not exist
    When "th.sh --dry-run" is run
    Then the podman run command binds nothing at /opt/tophat/home/.claude/CLAUDE.md
    And the launcher prints no warning about personal instructions

  @rq-b18523ab
  Scenario: A personal instructions path that is not a file is skipped with a warning
    Given ~/.config/tophat/CLAUDE.md is a directory
    When "th.sh --dry-run" is run
    Then the launcher warns naming ~/.config/tophat/CLAUDE.md
    And the podman run command binds nothing at /opt/tophat/home/.claude/CLAUDE.md

  @rq-039ad741
  Scenario: A symbolic link to personal instructions is followed
    Given ~/.config/tophat/CLAUDE.md is a symbolic link to /src/notes/CLAUDE.md, a file
    When "th.sh --dry-run" is run
    Then the podman run command binds /src/notes/CLAUDE.md at /opt/tophat/home/.claude/CLAUDE.md read-only

  @rq-b9501494
  Scenario: The check mounts no personal instructions
    Given ~/.config/tophat/CLAUDE.md exists
    When "th.sh --check" is run
    Then its podman run command binds nothing at /opt/tophat/home/.claude/CLAUDE.md

  @rq-8ab5f8ee
  Scenario: Personal instructions are mounted for Codex
    Given the project is rendered with agents ["codex"]
    And ~/.config/tophat/CLAUDE.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds ~/.config/tophat/CLAUDE.md at /opt/tophat/home/.codex/AGENTS.md read-only

  @rq-406a9ba4
  Scenario: Personal instructions are mounted for OpenCode
    Given the project is rendered with agents ["opencode"]
    And ~/.config/tophat/CLAUDE.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds ~/.config/tophat/CLAUDE.md at /opt/tophat/home/.config/opencode/AGENTS.md read-only

  @rq-762ac795
  Scenario: Every selected agent gets the personal instructions
    Given the project is rendered with agents ["claude-code", "codex", "opencode"]
    And ~/.config/tophat/CLAUDE.md exists
    When "th.sh --dry-run" is run
    Then the podman run command binds it at /opt/tophat/home/.claude/CLAUDE.md, /opt/tophat/home/.codex/AGENTS.md and /opt/tophat/home/.config/opencode/AGENTS.md read-only

  @rq-6de57a60
  Scenario: Local tool repositories are mounted read-only
    Given ".tophat/tools.yaml" declares tool "method-x" at an existing directory /src/method-x and tool "lib-y" at "file:///src/lib-y"
    And tool "web-z" at "https://github.com/example/web-z.git"
    When "th.sh --dry-run" is run
    Then the podman run command binds /src/method-x at /src/method-x read-only and /src/lib-y at /src/lib-y read-only
    And it mounts nothing for "web-z"

  @rq-a37e2c54
  Scenario: A missing local tool repository is reported, not fatal
    Given ".tophat/tools.yaml" declares tool "method-x" at /nonexistent/method-x
    When "th.sh --dry-run" is run
    Then the launcher warns naming "method-x" and /nonexistent/method-x
    And the session command is printed without a mount for it

  @rq-7d26723f
  Scenario: Every YAML form tophat accepts is understood
    Given .tophat/roots.local.yaml contains "roots: {results: /data/results}" and /data/results exists
    When "th.sh --dry-run" is run
    Then the podman run command binds /data/results at /data/results

  @rq-e9865354
  Scenario: The environment is installed when the session starts
    Given the container starts with enter.sh
    Then "pixi install --locked" runs before the interactive shell
    And "tophat status" is printed

  @rq-7478e01d
  Scenario: A stale lock does not prevent the session
    Given pixi.toml lists a package that pixi.lock does not
    When enter.sh runs
    Then it prints pixi's error and that pixi.toml and pixi.lock disagree
    And the interactive shell starts

  @rq-3c33f183
  Scenario: Project run options are appended and validated
    Given .tophat/user/podman/run-options contains "--memory=8g"
    When "th.sh --dry-run" is run
    Then "--memory=8g" follows every launcher option in the podman run command

  @rq-5df9545e
  Scenario: An invalid run option is refused
    Given .tophat/user/podman/run-options line 2 is "--memory 8g"
    When "th.sh --dry-run" is run
    Then it exits non-zero naming the file and line 2

  # --- Git hooks ---

  @rq-2f11c7ec
  Scenario: The hook path is set when unset
    Given core.hooksPath is unset
    When "th.sh --dry-run" is run
    Then core.hooksPath is ".tophat/managed/hooks"

  @rq-f503e56e
  Scenario: An existing hook path is left alone
    Given core.hooksPath is ".husky"
    When "th.sh --dry-run" is run
    Then core.hooksPath is ".husky"
    And the launcher prints that the TopHat pre-commit hook is not active

  # --- Container check ---

  @rq-c65f79e0
  Scenario: Prompts are allowed inside the container
    Given a system root with run/.containerenv
    When check-container.sh runs
    Then it exits 0

  @rq-36a97588
  Scenario: Prompts are refused on the host
    Given a system root with none of the container markers, and no container variables set
    When check-container.sh runs
    Then it prints a JSON block decision naming "th.sh"
    And it exits 2
```
