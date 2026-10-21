# Feature: Apptainer <!-- rq-5e71ddd6 -->

On a cluster, where Podman is usually unavailable, the project's dev container image runs under
Apptainer. The image is built with Podman on a build host and exported as a single file, which the
human copies to the cluster; TopHat never copies it. The session there has the same mounts and the
same absence of credentials as under Podman (see `rqm/template/container.md`).

## Files <!-- rq-7801e56b -->

```text
.tophat/state/apptainer/tophat-<project-id>.sif   ignored   the exported image
.tophat/state/apptainer/agents/<agent>/           ignored   per-agent configuration homes
.tophat/user/apptainer/run-options                project   extra `apptainer exec` options
```

## Launcher <!-- rq-48bc7333 -->

| Invocation | Effect |
|---|---|
| `th.sh --export-sif` | Builds the image and exports it to `.tophat/state/apptainer/tophat-<project-id>.sif` |
| `th.sh --run-sif` | Starts an interactive session from the exported image under Apptainer (no build) |
| `th.sh --dry-run --run-sif` | Prints the commands `--run-sif` would run, and runs none |

- Under `--run-sif` the launcher reads the data roots by running `tophat data roots` from the
  exported image.
- `.tophat/user/apptainer/run-options` follows the format and validation of the Podman run options
  in `rqm/template/container.md`.
- With GPU passthrough (see `rqm/template/gpu.md`), the options are `--nv` for `nvidia` and for
  `nvidia-no-cdi` (Apptainer needs no CDI specification), and `--rocm` for `amd`.

## Export and Run <!-- rq-4f95e12e -->

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

## Assumptions <!-- rq-16024bf5 -->

- Apptainer 1.2 or later on the build host (for export) and on execution hosts.

## Out of Scope <!-- rq-9dc8c249 -->

- Building images on a cluster, and transferring exported images.

## Gherkin Scenarios <!-- rq-49a7a910 -->

```gherkin
Feature: Apptainer

  Background:
    Given a project rendered with default answers in a git repository

  @rq-5cf97dd8
  Scenario: Apptainer GPU options
    Given an exported image for this project and the GPU setting "auto"
    When "th.sh --dry-run --run-sif" is run with detection printing each value below
      | detected      | option |
      | nvidia        | --nv   |
      | nvidia-no-cdi | --nv   |
      | amd           | --rocm |
    Then the apptainer command contains the option of its row

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
```
