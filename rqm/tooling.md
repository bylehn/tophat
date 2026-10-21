# Feature: Template Tooling <!-- rq-044e98a7 -->

The template tooling is the development-only Rust code that maintains the TopHat template: it
renders projects from the working tree, lints the rendered skills, regenerates the template's
pre-solved pixi locks, fills in the `tophat` release pin, and runs the conformance, render,
adoption and eval suites (specified in `rqm/testing.md`). It is never shipped to generated
projects and never published.

The repository is a Cargo workspace with two members: the `tophat` package at the root and the
tooling crate `xtask/`. `.cargo/config.toml` defines the alias `xtask = "run --package xtask --"`, so
every tool is run as `cargo xtask <command>`. The template it operates on is specified in
`rqm/template/project.md`, `rqm/template/skills.md` and `rqm/template/container.md`.

## Conventions <!-- rq-e2f42b59 -->

- Every command prints what it did and exits 0 on success, 1 when it ran and found problems, and 2
  when it could not run (bad arguments, a missing external program).
- External programs are found on `PATH`: `copier` (9.x), `pixi`, `git`, and for the commands that
  need them `podman`, `apptainer` and the agents' CLIs. A missing or wrong-version program is reported
  by name with exit 2.
- Commands that write files write them atomically and only when their content changes.
- The tooling has no dependencies beyond those of the `tophat` package plus what argument parsing and
  process control need from the standard library.

## Answer Sets <!-- rq-a4246ba4 -->

An **answer set** is a named set of Copier answers. `xtask/answers/<name>.yaml` holds the answers
used by render tests (see `rqm/testing.md`); each sets every question of `copier.yml`. The committed
sets are:

| Name | `languages` | `agents` | `gpu` |
|---|---|---|---|
| `minimal-claude` | none | `claude-code` | `auto` |
| `minimal-codex` | none | `codex` | `auto` |
| `minimal-opencode` | none | `opencode` | `auto` |
| `julia-all-agents` | `julia` | all three | `auto` |
| `gpu-off` | none | all three | `off` |

All sets use `project_name: Example Project`, a fixed description and a fixed author.

## Commands <!-- rq-ff41b0bc -->

### `cargo xtask render` <!-- rq-8fd5db8d -->

`cargo xtask render --answers <name|file> --out <dir> [--set <question>=<value>]...`

- Runs `copier copy --vcs-ref HEAD --defaults --data-file <answers>` from the repository root into
  `<dir>`, so uncommitted template changes are rendered; `--set` overrides single answers.
- Refuses an existing non-empty `<dir>`.
- Prints the rendered project's path.

### `cargo xtask lint-skills` <!-- rq-b4296d6d -->

`cargo xtask lint-skills <project dir> [--tophat <binary>]`

- Applies the skill lint rules of `rqm/template/skills.md` to a rendered project and prints one line
  per finding: `<rule> <path>[:<line>] <message>`.
- `skill.cli-unknown` compares against the usage text of `<binary> --help`; the default binary is
  the `tophat` built from the workspace.
- Exits 1 when there is a finding.

### `cargo xtask lock` <!-- rq-6fc05e55 -->

`cargo xtask lock [--check]`

- For each language selection (none, `julia`), renders a project, runs `pixi lock` in it, and copies
  the resulting `pixi.lock` to the template's pre-solved lock for that selection.
- With `--check`, writes nothing and exits 1 when any pre-solved lock differs from what the rendered
  `pixi.toml` requires (`pixi lock --check`), naming the selection.

### `cargo xtask release-pin` <!-- rq-be3c6abc -->

`cargo xtask release-pin --version <version> --url <base url> --binary <target>=<path>...`

- Hashes each given binary and writes the template's `tophat-release.yaml` with `version`, `url` and
  one `sha256` per target.
- Requires exactly the targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, and that
  each binary's `--version` output names `<version>` (checked only for a binary that runs on this
  machine).
- Exits 1 without writing when a target is missing or a version disagrees.

### `cargo xtask conformance` <!-- rq-f5dee2b3 -->

`cargo xtask conformance [--tophat <binary>] [--case <name>]...` runs the conformance suite (see
`rqm/testing.md`) against `<binary>`, by default the `tophat` built from the workspace. Prints one
line per case, `<case> ok|FAILED|skipped`, and the differences for each failure.

### `cargo xtask render-test` <!-- rq-5040a08a -->

`cargo xtask render-test [--answers <name>]... [--no-image]` renders each answer set (all by default)
into a temporary directory and runs the render checks of `rqm/testing.md` on it. `--no-image` skips
the container image build.

### `cargo xtask adoption` <!-- rq-5da5a173 -->

`cargo xtask adoption [--fixture <name>]...` runs the adoption tests of `rqm/testing.md`.

### `cargo xtask eval` <!-- rq-03d2afb2 -->

`cargo xtask eval [--agent claude-code|codex|opencode] [--eval <name>]... [--repeat <n>]` runs the
skill evals of `rqm/testing.md`.

### `cargo xtask scenarios` <!-- rq-04882df1 -->

`cargo xtask scenarios` checks Gherkin traceability across the repository: every scenario ID
(`@rq-…`) in `rqm/` is referenced by at least one test, conformance case, render check, adoption
fixture or eval definition, and every such reference names an existing scenario. Prints the
unreferenced and the dangling IDs; exits 1 when there is either.

## Assumptions <!-- rq-6fd600ee -->

- The tooling runs inside this repository's dev container (riprap's), where Rust, Copier and git are
  installed; pixi, Podman, Apptainer and the agents' CLIs are installed by the CI jobs or the
  maintainer when the command needs them.

## Out of Scope <!-- rq-06209ad3 -->

- Publishing releases (the release workflow in `rqm/testing.md` does that), and editing the pinned
  versions of pixi, Copier and the agents, which maintainers change by hand in
  `versions.yaml`.

## Gherkin Scenarios <!-- rq-b354e7d6 -->

```gherkin
Feature: Template tooling

  @rq-71619977
  Scenario: Render from the working tree
    Given an uncommitted change to template/README.md.jinja
    When "cargo xtask render --answers minimal-claude --out <tmp>/p" is run
    Then <tmp>/p/README.md contains the change
    And <tmp>/p/.copier-answers.yml records the answers of minimal-claude

  @rq-b0f89137
  Scenario: Render refuses a non-empty output directory
    Given <tmp>/p contains a file
    When "cargo xtask render --answers minimal-claude --out <tmp>/p" is run
    Then it exits 2 and <tmp>/p is unchanged

  @rq-eeadb8c2
  Scenario: Answers can be overridden
    When "cargo xtask render --answers minimal-claude --out <tmp>/p --set languages=[julia]" is run
    Then the rendered pixi.toml has julia

  @rq-9b4b5f1e
  Scenario: A missing external program is named
    Given copier is not on PATH
    When "cargo xtask render --answers minimal-claude --out <tmp>/p" is run
    Then it exits 2 naming "copier"

  @rq-6dd4dd02
  Scenario: Skill lint passes on a rendered project
    Given a project rendered with julia-all-agents
    When "cargo xtask lint-skills <project>" is run
    Then it prints nothing and exits 0

  @rq-2e07e31f
  Scenario: Skill lint reports findings with their rule
    Given a project rendered with minimal-claude whose .claude/skills/th-study/SKILL.md was deleted
    When "cargo xtask lint-skills <project>" is run
    Then it prints a line starting "skill.adapter-missing" naming th-study
    And it exits 1

  @rq-cb49acfd
  Scenario: Lock check detects a stale pre-solved lock
    Given the template's pixi.toml requires a package the pre-solved lock for "julia" lacks
    When "cargo xtask lock --check" is run
    Then it exits 1 naming "julia"
    And no file is changed

  @rq-39d23d3d
  Scenario: The release pin is written from the binaries
    Given release binaries for both musl targets reporting version 0.2.0
    When "cargo xtask release-pin --version 0.2.0 --url https://example.org/download/v0.2.0 --binary x86_64-unknown-linux-musl=<a> --binary aarch64-unknown-linux-musl=<b>" is run
    Then the template's tophat-release.yaml has version 0.2.0, that url, and the SHA-256 of <a> and <b>

  @rq-ac4f41eb
  Scenario: The release pin refuses a missing target
    When "cargo xtask release-pin --version 0.2.0 --url https://example.org --binary x86_64-unknown-linux-musl=<a>" is run
    Then it exits 1 naming aarch64-unknown-linux-musl
    And tophat-release.yaml is unchanged

  @rq-2ed82bd1
  Scenario: The release pin refuses a version mismatch
    Given a runnable binary <a> whose --version prints 0.1.9
    When "cargo xtask release-pin --version 0.2.0 --url https://example.org --binary x86_64-unknown-linux-musl=<a> --binary aarch64-unknown-linux-musl=<b>" is run
    Then it exits 1 naming 0.1.9 and 0.2.0

  @rq-6602ff7a
  Scenario: Unreferenced scenarios are found
    Given a requirements file with a scenario ID that no test references
    When "cargo xtask scenarios" is run
    Then it prints that ID and exits 1

  @rq-59f0f637
  Scenario: Dangling scenario references are found
    Given a test annotated with a scenario ID that no requirements file declares
    When "cargo xtask scenarios" is run
    Then it prints that ID and exits 1
```
