# Feature: GPU Passthrough <!-- rq-4d42c729 -->

The launcher (see `rqm/template/container.md`) passes the host's GPU into the dev container when
the host has a usable one, so stages and notebooks can use it; without one the session runs on CPU
and says so. The host supplies the driver; GPU runtime libraries come from the pixi environment,
not the image.

This file specifies the GPU setting, the detection script, and the options the launcher adds. The
Apptainer options are specified with the Apptainer path in `rqm/template/apptainer.md`.

## Files <!-- rq-75960728 -->

```text
.tophat/managed/launch/gpu-detect.sh   managed   GPU detection
.tophat/user/container/gpu             project   the project's GPU default: auto, on or off
```

- The Copier question `gpu` (choice `auto`, `on`, `off`; default `auto`) sets the content of
  `.tophat/user/container/gpu`, which is written once and is project-owned.

## Setting <!-- rq-380bb3d9 -->

The GPU setting is `TOPHAT_GPU` when set on the host, otherwise the content of
`.tophat/user/container/gpu`, otherwise `auto`. Any value other than `auto`, `on` or `off` is
refused.

## Detection <!-- rq-e363eed4 -->

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

## Passthrough <!-- rq-4cf7a956 -->

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

## Assumptions <!-- rq-b41ea1c5 -->

- NVIDIA hosts use the NVIDIA Container Toolkit for CDI specifications; AMD hosts expose `/dev/kfd`
  and `/dev/dri` to the user.

## Out of Scope <!-- rq-3eed2202 -->

- Selecting individual GPUs, or several vendors at once.
- GPU passthrough on macOS hosts.

## Gherkin Scenarios <!-- rq-f9b0c4db -->

```gherkin
Feature: GPU passthrough

  Background:
    Given a project rendered with default answers in a git repository

  # --- GPU detection ---

  @rq-7fb69dfd
  Scenario: GPU detection from simulated hosts
    Given simulated system roots containing these files
      | files                                                           | detected      |
      | dev/nvidiactl, etc/cdi/nvidia.yaml naming nvidia.com/gpu        | nvidia        |
      | dev/nvidiactl, var/run/cdi/nvidia.json naming nvidia.com/gpu    | nvidia        |
      | dev/nvidiactl                                                   | nvidia-no-cdi |
      | dev/kfd, dev/dri/renderD128                                     | amd           |
      | dev/kfd                                                         | none          |
      | dev/nvidiactl, etc/cdi/nvidia.yaml, dev/kfd, dev/dri/renderD128 | nvidia        |
      | nothing                                                         | none          |
    When gpu-detect.sh is run on each
    Then it prints the detected value of its row

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
```
