# Feature: Dev Container and Launcher <!-- rq-4d2bc439 -->

All work in a TopHat project happens inside a dev container: the agent, pixi, Snakemake and the
`tophat` CLI all run there, never on the host. The project's launcher, `th.sh`, builds the
container image with Podman, starts an interactive session with the workspace and the project's
data roots mounted, passes a GPU through when the host has a usable one, and gives the container no
credentials of the human's: no SSH keys, no agent sockets, no git or cloud credentials. On a cluster,
where Podman is usually unavailable, the same image runs under Apptainer after being exported on a
build host.

This file specifies the launcher and its options, the image, the session (mounts, environment,
pixi, git hooks), GPU detection and passthrough, the Apptainer path, and the check that keeps agents
from running on the host. File ownership (managed vs project-owned) follows
`rqm/template/project.md`; the agent configuration files that register the container check are
specified in `rqm/template/skills.md`; data roots in `rqm/data.md`.

## Files <!-- rq-48a62199 -->

```text
th.sh                                     managed   adapter: exec .tophat/managed/launch/th.sh
.tophat/project-id                        project   the project's UUID, created on first launch
.tophat/managed/
  launch/th.sh                            managed   the launcher
  launch/gpu-detect.sh                    managed   GPU detection
  launch/run-options.sh                   managed   run-options validation
  launch/enter.sh                         managed   session start inside the container
  container/Containerfile                 managed   the base image
  container/Agent.Containerfile           managed   the agent layer
  container/versions.yaml                 managed   pinned pixi, Copier and agent versions
  hooks/check-container.sh                managed   refuses agent prompts outside a container
.tophat/user/
  container/Containerfile                 project   project additions to the image
  container/gpu                           project   the project's GPU default: auto, on or off
  podman/run-options                      project   extra `podman run` options
  apptainer/run-options                   project   extra `apptainer exec` options
.tophat/state/                            ignored   machine-local launcher state (exported images)
```

- `th.sh` at the project root is listed in `.tophat/managed/ownership-exceptions`.
- `.tophat/user/container/gpu` is written once from the `gpu` Copier answer.
- `.tophat/state/` is git-ignored (see the ignore rules in `rqm/template/project.md`).

## Launcher <!-- rq-71727623 -->

`th.sh` is a POSIX `sh` script run from the project root on a Linux host, on macOS with a Podman
machine, or in WSL2 on Windows.

| Invocation | Effect |
|---|---|
| `th.sh` | Builds the image if needed, sets up git hooks, and starts an interactive session |
| `th.sh --export-sif` | Builds the image and exports it to `.tophat/state/apptainer/tophat-<project-id>.sif` |
| `th.sh --run-sif` | Starts an interactive session from the exported image under Apptainer (no build) |
| `th.sh --check` | Runs `tophat check` in a throwaway container with the project mounted read-only, and exits with its exit code |
| `th.sh --dry-run [--run-sif]` | Prints, one per line, the container commands it would run, and runs none |

- Unknown options are refused with the usage text, before anything runs.
- Every refusal names its cause and exits non-zero before any container starts.
- On first launch the launcher writes a new random UUID to `.tophat/project-id`; later launches
  read it. A malformed ID is refused. Image, volume and state names include it, so projects never
  share them.
- Under WSL2, when `podman` is the Windows binary, mount sources are translated with `wslpath -w`.

## Image <!-- rq-5c404476 -->

The image is built in three layers, all for the host's architecture (`linux-64` or
`linux-aarch64`):

1. **Base** (`.tophat/managed/container/Containerfile`): `ubuntu:24.04` with `bash`, `git`,
   `ca-certificates`, `curl`; pixi and Copier at the versions in `versions.yaml`; and the `tophat`
   binary from `.tophat/managed/tophat-release.yaml` for the image's architecture. The build
   downloads the binary, compares its SHA-256 with the pinned hash, and fails on a mismatch. The
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
| GPU options | see *GPU* |
| `.tophat/user/podman/run-options` | appended last, so the project's options win |

Nothing else from the host enters the container. In particular the session never mounts `~/.ssh`,
`~/.gitconfig`, `~/.git-credentials`, `~/.netrc`, `~/.config/gh`, cloud credential directories or
the host's agent configuration, and never passes `SSH_AUTH_SOCK`, `GIT_*`, `EMAIL` or other host
variables. The container has network access, which pixi,
the agents and `tophat tools sync` (public repositories only, see `rqm/pipeline/tools.md`) use.

The session's command is `.tophat/managed/launch/enter.sh`, which:

1. runs `pixi install --frozen`; on failure it prints pixi's error and a line saying that
   `pixi.toml` and `pixi.lock` disagree or a package could not be fetched, and continues;
2. prints `tophat status`;
3. replaces itself with an interactive `bash` in which `pixi`, `tophat` and, through
   `pixi run`, Snakemake are available.

### Data roots <!-- rq-fa54ed44 -->

The launcher asks `tophat` where the data roots are, so it reads `.tophat/roots.yaml` and
`.tophat/roots.local.yaml` exactly as the CLI does:

- After building, it runs `tophat data roots` in a throwaway container from the new image, with the
  workspace mounted read-only and the host's `TOPHAT_ROOT_<NAME>` variables passed in (see
  `rqm/data.md`). Under `--run-sif` it runs the same command from the exported image.
- When that command fails (invalid roots files), the launch is refused with its message.
- Each configured location is bound read-write at the identical path inside the container, so a
  path in a manifest, provenance record or job folder means the same file on the host and in the
  container.
- A configured location that does not exist on the host refuses the launch, naming the root.
  Unconfigured roots bind nothing (`tophat status` then says what to configure).

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

## GPU <!-- rq-19750015 -->

### Setting <!-- rq-380bb3d9 -->

The GPU setting is `TOPHAT_GPU` when set on the host, otherwise the content of
`.tophat/user/container/gpu`, otherwise `auto`. Any value other than `auto`, `on` or `off` is
refused.

### Detection <!-- rq-e363eed4 -->

`.tophat/managed/launch/gpu-detect.sh` prints one word, from files under a system root (`/` by
default, or the directory given as its first argument, so tests can simulate hosts):

| Output | Condition |
|---|---|
| `nvidia` | `dev/nvidiactl` exists and a CDI specification for `nvidia.com/gpu` exists in `etc/cdi/` or `var/run/cdi/` |
| `nvidia-no-cdi` | `dev/nvidiactl` exists without such a specification |
| `amd` | `dev/kfd` exists and `dev/dri/` holds a `renderD*` node |
| `none` | otherwise |

NVIDIA is reported when both vendors are present. Under Apptainer, `nvidia-no-cdi` counts as
`nvidia`, because `--nv` needs no CDI specification.

### Passthrough <!-- rq-4cf7a956 -->

| Detected | Podman options | Apptainer options |
|---|---|---|
| `nvidia` | `--device nvidia.com/gpu=all` | `--nv` |
| `amd` | `--device /dev/kfd --device /dev/dri --group-add keep-groups` | `--rocm` |

- `auto`: passes the detected GPU through. With `none`, it starts on CPU and prints one line saying
  no usable GPU was found; with `nvidia-no-cdi` under Podman, it starts on CPU and prints that the
  NVIDIA GPU needs a CDI specification (`sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml`).
- `on`: like `auto`, but refuses to launch when no GPU can be passed through, with the same
  explanation.
- `off`: passes nothing through.
- On macOS no GPU can be passed through, so `auto` starts on CPU with the message and `on` refuses.
- The host supplies the driver; GPU runtime libraries come from the pixi environment.

## Apptainer <!-- rq-1031bf35 -->

- `th.sh --export-sif` builds the image as usual, saves it as an OCI archive, and converts it with
  `apptainer build` to `.tophat/state/apptainer/tophat-<project-id>.sif`, writing to a temporary file
  and renaming it when complete. Apptainer must be on the build host's `PATH`. The image carries the
  label `org.tophat.project-id`.
- The human copies the `.sif` file to the same path in the project's checkout on the cluster.
  TopHat never copies it.
- `th.sh --run-sif` builds nothing. It refuses when the image is missing (naming the expected path
  and how to export one) or when its `org.tophat.project-id` label differs from
  `.tophat/project-id`. It runs `apptainer exec --containall --no-home --cleanenv --writable-tmpfs`,
  binding the workspace at `/work`, the data roots at their own paths, and per-agent directories
  under `.tophat/state/apptainer/agents/` (created with mode `0700`) at the agents' configuration
  homes, followed by the GPU options and `.tophat/user/apptainer/run-options`, and runs
  `env HOME=/opt/tophat/home .tophat/managed/launch/enter.sh`.

## Run Options <!-- rq-2b0ca72f -->

`.tophat/user/podman/run-options` and `.tophat/user/apptainer/run-options` hold one option per line;
blank lines and `#` comments are ignored. A line containing whitespace, or not starting with `-`, is
refused with the file name and line number before any container starts. The options are appended
after the launcher's own, so a runtime that takes the last occurrence of an option takes the
project's.

## Container Check <!-- rq-1cf4eaf3 -->

`.tophat/managed/hooks/check-container.sh` exits 0 inside a container (Podman's
`/run/.containerenv` or `container=podman`, Apptainer's `APPTAINER_CONTAINER` or `/.singularity.d`,
a container cgroup, or an overlay root filesystem). Outside one it prints, as JSON, a block decision
whose reason says to start the session with `th.sh`, and exits 2. The agent configuration files
(see `rqm/template/skills.md`) run it before every prompt, so an agent started on the host refuses
to work.

## Assumptions <!-- rq-14cdc44c -->

- Podman 4.4 or later on the build host; Apptainer 1.2 or later on execution hosts.
- On macOS, data root locations are inside directories the Podman machine shares (by default
  `/Users`).
- NVIDIA hosts use the NVIDIA Container Toolkit for CDI specifications; AMD hosts expose `/dev/kfd`
  and `/dev/dri` to the user.

## Out of Scope <!-- rq-656aa36b -->

- Native Windows launchers (`.bat`, PowerShell); Windows is supported through WSL2.
- Docker as the container runtime.
- Building images on a cluster, and transferring exported images.
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

  @rq-f419a09b
  Scenario: Only selected agents are installed
    Given the project was rendered with agents ["codex"]
    When the image is built
    Then "codex --version" succeeds in the image
    And "claude" and "opencode" are not on PATH in the image

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

  @rq-7d26723f
  Scenario: Every YAML form tophat accepts is understood
    Given .tophat/roots.local.yaml contains "roots: {results: /data/results}" and /data/results exists
    When "th.sh --dry-run" is run
    Then the podman run command binds /data/results at /data/results

  @rq-e9865354
  Scenario: The environment is installed when the session starts
    Given the container starts with enter.sh
    Then "pixi install --frozen" runs before the interactive shell
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

  # --- GPU detection ---

  Scenario Outline: GPU detection from a simulated host
    Given a system root containing <files>
    When gpu-detect.sh is run on it
    Then it prints "<result>"

    Examples:
      | files                                                         | result        |
      | dev/nvidiactl, etc/cdi/nvidia.yaml naming nvidia.com/gpu      | nvidia        |
      | dev/nvidiactl, var/run/cdi/nvidia.json naming nvidia.com/gpu  | nvidia        |
      | dev/nvidiactl                                                 | nvidia-no-cdi |
      | dev/kfd, dev/dri/renderD128                                   | amd           |
      | dev/kfd                                                       | none          |
      | dev/nvidiactl, etc/cdi/nvidia.yaml, dev/kfd, dev/dri/renderD128 | nvidia      |
      | nothing                                                       | none          |

  # --- GPU passthrough ---

  @rq-31265ce2
  Scenario: An NVIDIA GPU is passed through
    Given the GPU setting is "auto" and detection prints "nvidia"
    When "th.sh --dry-run" is run
    Then the podman run command contains "--device nvidia.com/gpu=all"

  @rq-abe5670e
  Scenario: An AMD GPU is passed through
    Given the GPU setting is "auto" and detection prints "amd"
    When "th.sh --dry-run" is run
    Then the podman run command contains "--device /dev/kfd", "--device /dev/dri" and "--group-add keep-groups"

  @rq-46b1f245
  Scenario: No GPU falls back to CPU
    Given the GPU setting is "auto" and detection prints "none"
    When "th.sh --dry-run" is run
    Then the podman run command has no GPU options
    And the launcher prints that no usable GPU was found and the session runs on CPU

  @rq-5dc40f24
  Scenario: An NVIDIA GPU without CDI falls back with instructions
    Given the GPU setting is "auto" and detection prints "nvidia-no-cdi"
    When "th.sh --dry-run" is run
    Then the podman run command has no GPU options
    And the launcher prints "nvidia-ctk cdi generate"

  @rq-96018e0c
  Scenario: GPU on without a GPU refuses to launch
    Given the GPU setting is "on" and detection prints "none"
    When "th.sh --dry-run" is run
    Then it exits non-zero saying no usable GPU was found

  @rq-f6dc88ee
  Scenario: GPU off passes nothing through
    Given the GPU setting is "off" and detection prints "nvidia"
    When "th.sh --dry-run" is run
    Then the podman run command has no GPU options

  @rq-98385f5d
  Scenario: The host variable overrides the project default
    Given .tophat/user/container/gpu contains "on" and TOPHAT_GPU is "off"
    And detection prints "none"
    When "th.sh --dry-run" is run
    Then the launch is not refused and the podman run command has no GPU options

  @rq-3a78943e
  Scenario: An invalid GPU setting is refused
    Given TOPHAT_GPU is "maybe"
    When "th.sh --dry-run" is run
    Then it exits non-zero naming the allowed values auto, on and off

  Scenario Outline: Apptainer GPU options
    Given an exported image for this project and the GPU setting "auto"
    And detection prints "<detected>"
    When "th.sh --dry-run --run-sif" is run
    Then the apptainer command contains "<option>"

    Examples:
      | detected      | option |
      | nvidia        | --nv   |
      | nvidia-no-cdi | --nv   |
      | amd           | --rocm |

  # --- Apptainer ---

  @rq-ef1935b3
  Scenario: Export writes the image atomically
    When "th.sh --export-sif" is run and the conversion fails
    Then no file exists at .tophat/state/apptainer/tophat-<project-id>.sif

  @rq-55e2e7f7
  Scenario: An image of another project is refused
    Given .tophat/state/apptainer/tophat-<project-id>.sif is labelled with another project ID
    When "th.sh --run-sif" is run
    Then it exits non-zero naming both project IDs

  @rq-c8022e42
  Scenario: A missing image names its expected path
    Given no exported image exists
    When "th.sh --run-sif" is run
    Then it exits non-zero naming .tophat/state/apptainer/tophat-<project-id>.sif and "th.sh --export-sif"

  @rq-55cac2c5
  Scenario: Apptainer runs isolated from the host
    Given an exported image for this project
    When "th.sh --dry-run --run-sif" is run
    Then the apptainer command contains "--containall", "--no-home" and "--cleanenv"
    And binds the project root at /work and the configured data roots at their own paths
    And binds no host home directory

  # --- Container check ---

  @rq-c65f79e0
  Scenario: Prompts are allowed inside the container
    Given /run/.containerenv exists
    When check-container.sh runs
    Then it exits 0

  @rq-36a97588
  Scenario: Prompts are refused on the host
    Given none of the container markers is present
    When check-container.sh runs
    Then it prints a JSON block decision naming "th.sh"
    And it exits 2
```
