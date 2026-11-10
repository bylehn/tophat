# Feature: Generated Project Skeleton <!-- rq-859c4b9b -->

TopHat is a Copier template. `copier copy` turns it into a research project: a repository with a
question tree, a place for studies, data roots, a locked pixi environment with Snakemake, the
pinned `tophat` release, an integrity check in CI, and a non-blocking pre-commit hook. A freshly
generated project contains no questions, studies or datasets, and passes `tophat check` with no
diagnostics.

This file specifies the template's layout in this repository, the Copier questions, the files of a
generated project, which of them TopHat manages and which belong to the project, and the
project's initial state. The agent skills and their per-agent adapters are specified in
`rqm/template/skills.md`; the dev container, its launcher and GPU handling in
`rqm/template/container.md`; render tests and the other test gates in `rqm/testing.md`. The files
the CLI reads (`PROJECT.md`, `.tophat/roots.yaml`, studies, datasets) are specified in
`rqm/question-tree.md`, `rqm/data.md` and `rqm/study/format.md`.

## Template Layout <!-- rq-e68d06e8 -->

- `copier.yml` at the repository root defines the questions and sets `_subdirectory: template`, so
  only `template/` is rendered into projects.
- `template/` holds the project files, with Jinja templating (`.jinja` suffix) where a file depends
  on answers.
- The template's pre-solved lock files (see *Environment*) live under `template/` as one lock per
  language selection, and template tooling regenerates them for each template release.
- The answers are stored in `.copier-answers.yml` at the project root, so `copier update` finds
  them without extra arguments.
- `template/.licenses/` holds the texts of the licenses a project can choose (see *LICENSE*), and
  `template/TEMPLATE-LICENSE` grants the contents of `template/` under MIT No Attribution (MIT-0), so
  the files a project receives from the template come with no conditions and the project's own
  license covers all of it. Both are in `_exclude` and never rendered into a project. The rest of
  this repository (the `tophat` tool) stays under the BSD 3-Clause license of the root `LICENSE`,
  and the README says which license covers which part.

## Copier Questions <!-- rq-99c9f467 -->

| Question | Type | Default | Validation |
|---|---|---|---|
| `project_name` | text | — | Non-empty |
| `project_slug` | text | `project_name` lowercased, runs of other characters replaced by `-` | Lowercase letters, digits and single hyphens, starting with a letter |
| `project_description` | text | — | Non-empty, one paragraph |
| `author_name` | text | — | Non-empty |
| `author_email` | text | — | Contains `@` |
| `languages` | multi-select: `julia` | none | — |
| `agents` | multi-select: `claude-code` (and the agents of `rqm/template/agents.md`) | `claude-code` | At least one |
| `gpu` | choice: `auto`, `on`, `off` | `auto` | — |
| `license` | choice: `MIT`, `BSD-3-Clause`, `Apache-2.0`, `GPL-3.0-or-later`, `none` | `none` | — |
| `copyright_year` | text, asked unless `license` is `none` | the current year | Four digits |

- Python is always installed, because Snakemake runs on it; `languages` lists what is added beyond
  it.
- `agents` selects the skill adapters (see `rqm/template/skills.md`); `gpu` is the launcher's
  default (see `rqm/template/container.md`). This file only records them.
- `license` defaults to `none` because choosing a license is the human's decision: the template
  never picks one for them. Its help text says that `none` means nobody else may reuse the project
  until a license is added, and that MIT, BSD-3-Clause and Apache-2.0 let others reuse and adapt
  the work freely, while GPL-3.0-or-later requires that changed versions stay open.
- An invalid answer is rejected by Copier at the prompt, before any file is written.

## Generated Project Layout <!-- rq-fc2247e5 -->

```text
<project>/
  .copier-answers.yml            managed   Copier's record of the answers and template version
  PROJECT.md                     project   question tree: title and description, no questions
  README.md                      project   what the project is and how to start
  LICENSE                        project   the chosen license; absent when `license` is `none`
  pixi.toml                      project   the environment: Python, Snakemake, selected languages
  pixi.lock                      project   the pre-solved lock for the selected languages
  .gitignore                     project   root ignore rules (Snakemake and pixi working dirs)
  exploration/README.md          project   the area for unregistered work
  .github/workflows/tophat-check.yaml
                                 managed   CI: `tophat check` as a required gate
  .tophat/
    .gitignore                   managed   ignore rules for machine-local TopHat state
    roots.yaml                   project   data roots: declares `results`
    managed/
      GUIDE.md                   managed   how a study works, from question to report
      tophat-release.yaml        managed   the pinned `tophat` release and its checksums
      hooks/pre-commit           managed   runs `tophat check` and never blocks
      ownership-exceptions       managed   managed files that live outside .tophat/managed/
    user/                        project   project-owned extensions (skill `local.md` files)
```

`studies/` and `datasets/` are not created: `tophat study new` and `tophat data add` create them.
Agent files (skills, adapters, agent instruction files) and container files are added by the
features that specify them, under the same ownership rules.

## Ownership <!-- rq-cafa162c -->

Every file of a generated project is either **managed** or **project-owned**.

- **Managed** files belong to TopHat. `copier update` brings them to the new template version;
  local edits to them are lost or surface as update conflicts. Managed files live under
  `.tophat/managed/`, except those whose path a tool mandates, which are listed one per line in
  `.tophat/managed/ownership-exceptions` (`.copier-answers.yml`, `.tophat/.gitignore`,
  `.github/workflows/tophat-check.yaml`, and the files other features add). Every managed file in a
  format with comments carries the marker `TopHat-managed` within its first five lines (within five
  lines after the front matter, for a file that must begin with front matter), except the
  files of the worked example study (see `rqm/template/skills.md`), which are copied into `studies/`
  and then belong to the project.
- **Project-owned** files belong to the project. They are written once, by `copier copy`, and never
  again: every one is listed in `_skip_if_exists`, so `copier update` leaves them unchanged even when
  the template's version of them changed, and copying the template into an existing repository
  never overwrites a file of that name.
- `pixi.toml` and `pixi.lock` are project-owned because the environment hash is part of every stage
  key (see `rqm/pipeline/provenance.md`): a template update never changes an environment, so it
  never changes a key or re-runs a stage.

## Files <!-- rq-f40fa696 -->

### PROJECT.md <!-- rq-e660df68 -->

```markdown
# {{ project_name }}

{{ project_description }}

<!-- The question tree. Each question is a heading (the overarching question at level 2,
sub-questions deeper), followed by a `status: open` line; `tophat question stamp` gives new
questions their IDs. The agent elicits the questions from you; it never invents them. -->
```

No line of the comment starts with `#`, so the tree is empty (see `rqm/question-tree.md`).

### README.md <!-- rq-485c120f -->

Names the project, repeats its description, and gives the first steps: open the dev container,
run `tophat status`, and ask the agent to help write the overarching question. It says that stage
outputs go to the git-ignored `results/` folder unless `.tophat/roots.local.yaml` puts them
elsewhere, that results are reported only from pre-registered studies, and that
`.tophat/managed/GUIDE.md` explains how a study works.

### GUIDE.md <!-- rq-226c596c -->

`.tophat/managed/GUIDE.md` explains, in one page, how a study goes from question to report, and
why each step exists. It starts with the path in five words (explore, plan, lock, run, report) and
with exploring first: endpoints and thresholds chosen before the data is understood are arbitrary,
so exploration in `exploration/` and trial runs of a draft come before the plan is locked. It ends
with a short glossary of the terms it uses (pre-register, primary endpoint, decision rule,
scenario, stage key, validation, review, variant, amendment). In between: the question tree; planning and pre-registration (what is frozen, and why the
plan comes before any result is seen); stages, stage keys and why outputs live in directories named
by their key; validation (the checks code runs after every stage) and review (the few QC gates where
a human's judgment is asked for, and why they block what comes after); external stages and their
job folders;
the summary, clean and dirty results, and reporting; who does what (the human pre-registers, gives
review verdicts, runs cluster jobs, commits, and writes the Interpretation; the agent does the
rest);
and the commands for each step. The README and the agent rules point to it.

### LICENSE <!-- rq-39c292b1 -->

`LICENSE` holds the full text of the chosen license:

- `MIT` and `BSD-3-Clause`: the standard text, with the copyright line
  `Copyright (c) {{ copyright_year }}, {{ author_name }}` (BSD) or
  `Copyright (c) {{ copyright_year }} {{ author_name }}` (MIT).
- `Apache-2.0` and `GPL-3.0-or-later`: the license text verbatim, as published by the Apache
  Software Foundation and the Free Software Foundation; neither text carries a copyright line.
- `none`: no `LICENSE` file is written.

`LICENSE` is project-owned: `copier update` never changes it, so a later change of the `license`
answer does not replace a license already in place, and copying the template into a repository that
has its own `LICENSE` keeps that file.

### exploration/README.md <!-- rq-45cc9712 -->

States that `exploration/` holds scratch analyses and notebooks, that anything in it may inform a
new study, and that nothing in it may be cited as a result.

### Data roots <!-- rq-cba13e20 -->

`.tophat/roots.yaml` declares the `results` root, with a comment explaining that each machine sets
the location in `.tophat/roots.local.yaml` (see `rqm/data.md`):

```yaml
# Data roots of this project. Locations are per machine: see .tophat/roots.local.yaml.
# Stage outputs go to results/ in the project unless that file puts them elsewhere.
roots:
  results: {}
```

### Ignore rules <!-- rq-0da9fa10 -->

`.tophat/.gitignore` (managed) holds TopHat's rules, relative to `.tophat/`, so they never collide
with a project's own `.gitignore`:

```gitignore
# TopHat-managed: machine-local state, never committed
/roots.local.yaml
/slurm.local.yaml
/cache/
/tools/
/generated/
/state/
```

The root `.gitignore` (project-owned) lists the directories that tools create at the project root:
`/.snakemake/`, `/.pixi/`, and `/results/`, the default location of stage outputs (see
`rqm/data.md`).

### Environment <!-- rq-260d84c9 -->

`pixi.toml` declares one workspace named after `project_slug`, the channels `conda-forge` and
`bioconda` (in that order; Snakemake is published on bioconda), the platforms `linux-64` and `linux-aarch64` (the dev container's platforms), and a `default`
environment with:

- `python` and `snakemake-minimal`, at the versions the template pins;
- `julia`, when `languages` includes `julia`.

`pixi.lock` is the template's pre-solved lock for that selection, so a generated project has a
committed lock from the start, `copier copy` needs no network access, and two projects generated
from the same template version with the same languages have identical environment hashes. The lock
covers both platforms.

### The tophat release <!-- rq-56d220b5 -->

`.tophat/managed/tophat-release.yaml` pins the `tophat` binary that the container installs and CI
uses:

```yaml
# TopHat-managed: the tophat release this project uses. `copier update` changes it.
version: 0.1.0
url: https://<release host>/download/v0.1.0
sha256:
  x86_64-unknown-linux-musl: <64 hex characters>
  aarch64-unknown-linux-musl: <64 hex characters>
```

- The binary for a target is `<url>/tophat-<target>`; its SHA-256 must equal the listed hash, or it
  is not used.
- Template tooling fills in `version`, `url` and the hashes when a template version is released.
- Before the first release the file holds only `version: unreleased`: there is no binary to download,
  the launcher requires a development binary (see `rqm/template/container.md`), and CI fails saying
  that no `tophat` release is pinned.

### Pre-commit hook <!-- rq-a35593b1 -->

`.tophat/managed/hooks/pre-commit` is a POSIX `sh` script:

- Commits are made on the host (see `rqm/template/container.md`), where `tophat` is usually not
  installed. The hook runs `tophat check` from the project root when `tophat` is on `PATH`, and
  otherwise `./th.sh --check`, which runs the check in a throwaway container. It prints the check's
  output; when the check has errors (exit 1), it ends with a line saying that CI will reject the
  commit until they are fixed. When the check could not run (any other non-zero status, such as the
  launcher refusing to start because no development binary is set), it ends with a line saying the
  check could not run, with the status, and does not claim errors.
- When neither `tophat` nor a container runtime is available, it prints one line saying the check
  was skipped.
- It always exits 0: a half-written draft can be committed, and CI is the gate.

Git runs it once `core.hooksPath` points at `.tophat/managed/hooks`; the launcher sets this (see
`rqm/template/container.md`).

### CI <!-- rq-72b63b68 -->

`.github/workflows/tophat-check.yaml` runs on every push and pull request, on `ubuntu-latest`:

1. Checks out the repository.
2. Reads `.tophat/managed/tophat-release.yaml`, downloads `tophat-x86_64-unknown-linux-musl`, and
   verifies its SHA-256; a mismatch fails the job before anything runs, and so does
   `version: unreleased`, with a message that no `tophat` release is pinned.
3. Runs `tophat check`. The job fails exactly when the check exits non-zero.

The job needs no container, pixi or Snakemake: `tophat check` reads only committed files. Data roots
are unconfigured in CI, so root-dependent checks report warnings, which do not fail the job.

## Initial State <!-- rq-4a8d3a7d -->

A generated project, before anything else is done:

- `tophat check` reports no diagnostics and exits 0, both with and without `--strict`;
- `tophat status` reports `0 open, 0 emerged, 0 answered, 0 dropped` questions and lists no studies;
- `tophat question tree` shows an empty tree.

`copier copy` does not run `git init` or any other command (it defines no `_tasks`), so generation
works without `--trust`.

## Assumptions <!-- rq-32776f33 -->

- Projects are hosted on a service that runs GitHub Actions workflows. Other CI providers are out of
  scope; the check is a single command, so adding one is a small change.
- Copier 9 or later generates and updates projects.
- A project adopted in place that already has a `pixi.toml` without Snakemake, or a `.gitignore`
  without the root rules, keeps its own files; the adoption skill (see `rqm/template/skills.md`)
  reconciles them.

## Out of Scope <!-- rq-4bd6ac72 -->

- The agent skills, adapters and agent instruction files (`rqm/template/skills.md`).
- The dev container, launcher, GPU handling and enabling the hook (`rqm/template/container.md`).
- The worked example study: it lives with the skills that copy from it, never in a new project's
  `studies/`.
- Languages other than Python and Julia.
- Licenses beyond the five choices, separate licenses for data or documentation (such as CC BY),
  and per-file license headers.
- Checking in `tophat check` that managed files are unmodified.

## Gherkin Scenarios <!-- rq-d4cc96df -->

```gherkin
Feature: Generated project skeleton

  Background:
    Given the template is rendered with project_name "Method X", project_description "Does method X improve affinity prediction?", author_name "A. Scientist" and author_email "a@example.org"

  # --- Questions ---

  @rq-911c9187
  Scenario: The slug is derived from the project name
    When the template is rendered with default answers otherwise
    Then the answers file records project_slug "method-x"
    And pixi.toml names the workspace "method-x"

  @rq-195607b5
  Scenario: A malformed slug is rejected
    When the template is rendered with project_slug "Method_X"
    Then Copier rejects the answer
    And no file is written

  @rq-db2ca072
  Scenario: At least one agent is required
    When the template is rendered with an empty agents selection
    Then Copier rejects the answer

  @rq-11ae7d55
  Scenario: A malformed copyright year is rejected
    When the template is rendered with license "MIT" and copyright_year "26"
    Then Copier rejects the answer
    And no file is written

  # --- Layout and ownership ---

  @rq-d0e11fee
  Scenario: A new project has the skeleton files and no studies or datasets
    When the template is rendered with default answers
    Then the project contains PROJECT.md, README.md, pixi.toml, pixi.lock, .gitignore, exploration/README.md, .github/workflows/tophat-check.yaml, .tophat/.gitignore, .tophat/roots.yaml, .tophat/managed/GUIDE.md, .tophat/managed/tophat-release.yaml, .tophat/managed/hooks/pre-commit and .tophat/managed/ownership-exceptions
    And it contains no studies/ and no datasets/ directory

  @rq-74365fac
  Scenario: Managed files carry the marker
    When the template is rendered with default answers
    Then every file under .tophat/managed/ and every path listed in ownership-exceptions, except those in formats without comments and those of the worked example, has "TopHat-managed" within its first five lines

  @rq-cae7fbe8
  Scenario: Every project-owned file is skipped on update
    When the template is rendered with license "MIT"
    Then every rendered file that is neither under .tophat/managed/ nor listed in ownership-exceptions is matched by _skip_if_exists

  @rq-4da71d7f
  Scenario: Machine-local state is ignored
    Given a rendered project in a git repository
    When .tophat/roots.local.yaml, .tophat/slurm.local.yaml, .tophat/cache/hashes.json, .tophat/tools/x/y, .tophat/generated/s.smk, .tophat/state/apptainer/x.sif, .snakemake/log, .pixi/envs/x and results/s/a/sk-0/x are created
    Then "git status --porcelain" lists none of them

  # --- Files ---

  @rq-5c6c0574
  Scenario: PROJECT.md starts with an empty question tree
    When the template is rendered with default answers
    Then PROJECT.md starts with "# Method X"
    And it contains "Does method X improve affinity prediction?"
    And "tophat question tree" shows no questions

  @rq-2b58b0cd
  Scenario: The chosen license is written with its notice
    When the template is rendered with copyright_year "2031" and each license other than "none"
    Then for license "MIT", LICENSE starts with "MIT License" and contains "Copyright (c) 2031 A. Scientist"
    And for license "BSD-3-Clause", LICENSE starts with "BSD 3-Clause License" and contains "Copyright (c) 2031, A. Scientist"
    And for license "Apache-2.0", LICENSE contains "Apache License" and "Version 2.0, January 2004"
    And for license "GPL-3.0-or-later", LICENSE contains "GNU GENERAL PUBLIC LICENSE" and "Version 3, 29 June 2007"

  @rq-8a5a143e
  Scenario: Long license texts are verbatim
    When the template is rendered with license "Apache-2.0", and again with license "GPL-3.0-or-later"
    Then each LICENSE equals the corresponding file of template/.licenses/ byte for byte

  @rq-63eb6559
  Scenario: No license is written by default
    When the template is rendered with default answers
    Then the project has no LICENSE file
    And the answers file records license "none" and no copyright_year

  @rq-ed4a5363
  Scenario: The template's own license files are not copied
    When the template is rendered with license "MIT"
    Then the project contains no TEMPLATE-LICENSE and no .licenses/ directory

  @rq-d1fe5c84
  Scenario: The results root is declared and defaults to the project
    When the template is rendered with default answers
    Then .tophat/roots.yaml declares exactly the root "results"
    And no .tophat/roots.local.yaml exists
    And "tophat data roots" locates "results" at the project's results/ folder

  @rq-e9156963
  Scenario: Python and Snakemake are always installed
    When the template is rendered with no languages selected
    Then the default environment of pixi.toml has python and snakemake-minimal and no julia
    And pixi.lock lists python and snakemake-minimal for linux-64 and linux-aarch64

  @rq-87749f65
  Scenario: Julia is added when selected
    When the template is rendered with languages ["julia"]
    Then the default environment of pixi.toml has julia
    And pixi.lock lists julia for linux-64 and linux-aarch64

  @rq-14a1f043
  Scenario: The lock is the template's pre-solved lock
    When the template is rendered twice with the same languages into different directories
    Then the two pixi.lock files are identical
    And the environment hash of "default" is the same in both projects

  @rq-06b39dff
  Scenario: The guide starts with exploring and explains its terms
    When the template is rendered with default answers
    Then .tophat/managed/GUIDE.md has a section on exploring before the plan, before the section on the plan
    And it has a glossary defining "pre-register", "primary endpoint", "decision rule" and "scenario"

  @rq-f2b4f4cc
  Scenario: The release pin is complete
    Given the template's release pin has been written by "cargo xtask release-pin"
    When the template is rendered with default answers
    Then .tophat/managed/tophat-release.yaml has a version, a url, and a 64-character lowercase hex sha256 for x86_64-unknown-linux-musl and aarch64-unknown-linux-musl

  @rq-99f3ccdf
  Scenario: An unreleased pin says so
    Given the template's release pin is "version: unreleased"
    When the template is rendered with default answers
    Then .tophat/managed/tophat-release.yaml holds "version: unreleased" and no url or sha256
    And the CI workflow's download step fails saying no tophat release is pinned

  # --- Hook and CI ---

  @rq-05e3b95c
  Scenario: The hook reports check errors without blocking
    Given a rendered project in a git repository with core.hooksPath set to .tophat/managed/hooks
    And tophat is on PATH
    And PROJECT.md has a question without a status line
    When a commit is made
    Then the commit succeeds
    And the hook output contains the check's error and says CI will reject the commit

  @rq-47e127bc
  Scenario: The hook uses the container on the host
    Given a rendered project in a git repository with core.hooksPath set to .tophat/managed/hooks
    And tophat is not on PATH and podman is
    And PROJECT.md has a question without a status line
    When a commit is made on the host
    Then the hook runs "./th.sh --check"
    And the commit succeeds
    And the hook output contains the check's error

  @rq-c0d8a5d1
  Scenario: The hook says when the check could not run
    Given a rendered project in a git repository with core.hooksPath set to .tophat/managed/hooks
    And tophat is not on PATH and podman is
    And "./th.sh --check" refuses to start and exits 2
    When a commit is made on the host
    Then the commit succeeds
    And the hook output contains the launcher's message and says the check could not run
    And it does not say that CI will reject the commit

  @rq-26afc755
  Scenario: The hook is skipped without tophat or a container runtime
    Given a rendered project in a git repository with core.hooksPath set to .tophat/managed/hooks
    And neither tophat nor podman is on PATH
    When a commit is made
    Then the commit succeeds
    And the hook prints that the check was skipped

  @rq-d4c74a32
  Scenario: CI verifies the binary before running the check
    When the template is rendered with default answers
    Then .github/workflows/tophat-check.yaml runs on push and pull_request
    And it downloads the binary named in tophat-release.yaml and compares its SHA-256 before running "tophat check"

  # --- Initial state ---

  @rq-1ec0c367
  Scenario: A new project passes the check
    When the template is rendered with default answers
    Then "tophat check --strict" in the project reports no diagnostics and exits 0

  @rq-6fd1f551
  Scenario: A new project has nothing to do but ask the question
    When the template is rendered with default answers
    Then "tophat status" reports "0 open, 0 answered, 0 dropped" and lists no studies

  @rq-83643799
  Scenario: Generation runs no commands
    When the template is rendered with default answers
    Then copier.yml defines no _tasks
    And rendering without --trust succeeds

  @rq-f57dc6fa
  Scenario: A study can be added to a new project
    Given a rendered project in a git repository
    When the question "Does X improve affinity prediction?" is added to PROJECT.md with "status: open" and "tophat question stamp" is run
    And "tophat study new x-vs-docking --question <the stamped ID>" is run
    Then "tophat check" reports no errors

  # --- Updates and adoption ---

  @rq-e7220b72
  Scenario: Updating keeps project-owned files and stage keys
    Given a project rendered from one template version, committed, with an edited README.md, a stamped question in PROJECT.md and a study with stages
    And the stage keys of that study are recorded
    When the project is updated to a template version whose PROJECT.md, README.md, pixi.toml and pixi.lock differ
    Then README.md, PROJECT.md, pixi.toml and pixi.lock are unchanged
    And the stage keys are unchanged
    And .tophat/managed/tophat-release.yaml is the new version's

  @rq-46aa2f57
  Scenario: Copying into an existing repository overwrites no user file
    Given an existing repository with README.md, .gitignore, pixi.toml and LICENSE of its own
    When the template is copied into it with license "MIT"
    Then README.md, .gitignore, pixi.toml and LICENSE are unchanged
    And .tophat/.gitignore and .tophat/roots.yaml are written
```
