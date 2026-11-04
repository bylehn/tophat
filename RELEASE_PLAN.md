# TopHat 0.1.0 Release Plan

Track the work that takes TopHat from pre-release to a public release at
`github.com/bylehn/tophat`.

## Goal

A scientist runs

```sh
copier copy gh:bylehn/tophat my-project
```

and gets a working project whose container downloads a prebuilt, checksum-verified `tophat`
binary — no Rust toolchain and no `TOPHAT_DEV_BINARY`. Every requirement scenario is implemented
and referenced, all CI gates pass, and the release workflow publishes the binaries.

## Completion criteria

- `xtask/pending.txt` is empty: all 548 requirement scenarios under `rqm/` are referenced by a
  test, conformance case, render check, adoption fixture or eval (`cargo xtask scenarios` reports
  0 problems and 0 pending).
- All jobs in `.github/workflows/CI.yaml` are green on pull requests and `main`.
- `.github/workflows/release.yaml` builds both musl binaries, runs the gates against them, writes
  the release pin, and publishes only after they pass (rq-e6ed00d3).
- `template/.tophat/managed/tophat-release.yaml` pins the released version instead of
  `version: unreleased`.
- A project generated from the public template builds its container and runs the worked example
  end to end without a Rust toolchain.

## Scope decisions

- **0.1.0 includes everything currently specified**: GPU passthrough, Apptainer, Codex and
  OpenCode, skill evals, the test/release tooling, and all CI gates.
- **Full gate suite**, exactly as `rqm/testing.md` specifies.
- **Repository**: `gh:bylehn/tophat` (currently private), published to GitHub Releases.
- **First version**: `0.1.0` (the version the requirements use throughout).

## Phases

Each step implements one requirements file (or one aspect of it), is covered by its Gherkin
scenarios, and ends by removing the file from `xtask/pending.txt` so `cargo xtask scenarios`
stays green.

### Phase 1 — Platform features

- [x] **Codex and OpenCode** (`rqm/template/agents.md`): Copier choices, thin adapters for all
  eight skills, permission files with the same deny/allow lists as Claude Code, container-check
  hooks, image installation of only the selected agents with pinned versions and checksums, and
  one session volume per selected agent at its configuration home
  (`rqm/template/container.md` §Session).
- [x] **GPU passthrough** (`rqm/template/gpu.md`): NVIDIA/AMD detection against simulated hosts,
  CDI fallback, `auto`/`on`/`off` and host override, CPU fallback, Podman and Apptainer options.
- [ ] **Apptainer** (`rqm/template/apptainer.md`): `--export-sif` / `--run-sif`, GPU options,
  project-id label check, atomic export, host isolation and personal mounts.

### Phase 2 — Test and release tooling (`rqm/tooling.md`)

- [ ] `cargo xtask render` and committed answer sets in `xtask/answers/` (`minimal-claude`,
  `minimal-codex`, `minimal-opencode`, `julia-all-agents`, `gpu-off`).
- [ ] `cargo xtask lock` (regenerate and `--check` the pre-solved pixi locks).
- [ ] `cargo xtask conformance` plus the committed cases: 5 valid and 13 broken, per
  `rqm/testing.md` §Required cases, with deterministic git replay and `since:` versioning.
- [ ] `cargo xtask render-test`: render checks, `tophat check --strict`, skill lint,
  `pixi install --frozen`, image build, launcher and GPU scenarios.
- [ ] `cargo xtask adoption` plus the 5 fixtures (`scripts-notebooks`, `conda-env`,
  `julia-project`, `snakemake-pipeline`, `make-pipeline`).
- [ ] `cargo xtask eval` plus `evals/*.yaml` covering every behaviour scenario of
  `rqm/template/skills.md` (including the `th-quiz` scenarios).
- [ ] `cargo xtask release-pin`: hash the binaries, write `tophat-release.yaml`, refuse missing
  targets and version mismatches.

### Phase 3 — CI gates (`rqm/testing.md` §CI Workflows)

- [ ] `.github/workflows/CI.yaml` with the nine jobs: `cli` (Linux/macOS/Windows), `snakemake`,
  `template`, `image`, `traceability`, `render`, `conformance`, `e2e`, `adoption`.
- [ ] `.github/workflows/evals.yaml`: weekly and manual dispatch, credentials from secrets, never
  a required check.

### Phase 4 — Release

- [ ] `.github/workflows/release.yaml`: on a version tag, build `x86_64` and `aarch64` musl
  binaries, run conformance against the x86_64 binary, write the pin with
  `cargo xtask release-pin`, re-run render and e2e with that pin, then publish the binaries with
  SHA-256 checksums.
- [ ] Un-pend and test the release scenarios: `rqm/template/container.md`
  (image holds the pinned tophat; wrong checksum fails the build) and `rqm/template/project.md`
  (the release pin is complete).
- [ ] Tag `v0.1.0`, flip `tophat-release.yaml` from `unreleased` to the released pin, and confirm
  a generated project runs the worked example without `TOPHAT_DEV_BINARY`.

### Phase 5 — Go public

- [ ] README: replace `gh:<owner>/tophat` with `gh:bylehn/tophat`, remove the "Before the first
  release" section, and update the status note.
- [ ] Scan the (currently private) repository history before making it public.
- [ ] Make the repository public and verify `copier copy gh:bylehn/tophat` from a clean machine.

## Notes and caveats

- Implementation follows `rr-implement`: every requirement scenario gets at least one test
  annotated with its `rq-` ID, and `rqm.sh index` rebuilds `rqm/registry.json`.
- Commit `b08cf6b` (riprap `include_rust_skeleton: false`) deleted the root `Cargo.toml` and
  `src/lib.rs`, breaking the build from HEAD. Both were restored during Phase 1 and must be
  committed. That commit also removed riprap's default `.codecov.yml` and `CI.yaml`, which
  Phase 3 replaces with the TopHat-specific workflow.
- Phase 1's new files are untracked: `Agent.Containerfile.jinja`, the conditional
  `.codex`/`.agents`/`.opencode` template directories, `opencode.json` and this plan must be
  `git add`ed together with the modified files; a tracked-files-only commit would break the build
  and the template.
- Personal skills and instructions mount for Claude Code alone: `rqm/template/container.md` wrote
  that scope when `claude-code` was the only agent offered. Decide whether Codex and OpenCode
  (whose skills use the same SKILL.md format) should mount them too, and revise that section.
- The Codex and OpenCode integration points follow the assumptions of `rqm/template/agents.md`
  (`.codex/rules` prefix rules, `.codex/hooks.json`, `opencode.json` permissions, the
  `.opencode/plugins` check). Phase 2's evals should confirm them against the real agents.
- Claude Code installs via `curl | bash` without a checksum, unlike every other image binary;
  consider a hash-verified install before going public.
- Skill evals need agent API credentials and are reported weekly; they are never a release gate.
- The versioned contract (stage-key derivation, provenance and study formats) is unchanged by
  this plan, so released projects remain compatible with `copier update`.
