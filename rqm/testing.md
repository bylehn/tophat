# Feature: Test Gates <!-- rq-0d8c85a8 -->

TopHat's tests establish that the CLI, the template and the skills do what the requirements say.
Six suites run as required CI gates on every pull request and on `main`: the CLI's unit and
integration tests (including those that need Snakemake), scenario traceability, render tests,
the CLI conformance suite, the end-to-end example study, and adoption tests. Skill evals run
on demand and on a schedule, never as a gate. A release is published only after the gates pass
against the release binaries.

This file specifies each suite, the formats of conformance cases, adoption fixtures and eval
definitions, and the CI workflows. The commands that run the suites are specified in
`rqm/tooling.md`.

## CLI Tests <!-- rq-20e3189f -->

- `cargo test` holds one test per Gherkin scenario of each CLI requirements file, annotated with the
  scenario's ID (see the traceability rules in this repository's riprap skills).
- Tests that need Snakemake read its path from `TOPHAT_TEST_SNAKEMAKE`. Without it they print a skip
  notice and pass, except when `TOPHAT_REQUIRE_SNAKEMAKE=1` is set, in which case they fail, so a CI
  job meant to run them cannot pass without running them.
- The job runs `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` on Linux, macOS and Windows; a second job on Linux installs Snakemake and
  runs the Snakemake-backed tests with both variables set.

## Traceability <!-- rq-5b01ac93 -->

`cargo xtask scenarios` (see `rqm/tooling.md`) is a gate: every scenario in `rqm/` is referenced by
a test, a conformance case, a render check, an adoption fixture or an eval definition, and no
reference dangles. Behaviour scenarios of `rqm/template/skills.md` are referenced by eval
definitions.

## Render Tests <!-- rq-351008a7 -->

`cargo xtask render-test` renders every answer set of `rqm/tooling.md` and, for each project:

1. checks every render scenario of `rqm/template/project.md`, `rqm/template/skills.md` and
   `rqm/template/container.md` that does not need a running container, each as a named check
   annotated with its scenario ID;
2. runs `tophat check --strict` (the workspace's binary) and requires no diagnostics;
3. runs `cargo xtask lint-skills` and requires no findings;
4. runs `pixi install --frozen` and requires success, so the pre-solved lock installs;
5. builds the container image with `th.sh` (unless `--no-image`) and runs, inside it,
   `tophat --version`, `pixi run snakemake --version` and, for `julia-all-agents`,
   `pixi run julia --version`;
6. runs the launcher scenarios of `rqm/template/container.md` with `th.sh --dry-run`, and the GPU
   detection scenarios against simulated system roots.

A project that fails any step fails the job, naming the answer set and the step.

## Conformance Suite <!-- rq-b06148e1 -->

The conformance suite is the executable definition of the provenance guarantees: committed example
projects, valid and deliberately broken, with the diagnostics each must produce. Its cases are
committed bytes, so a CLI change that alters a record format, a key derivation or a rule's verdict
shows up as a failing case.

### Case format <!-- rq-55a71d92 -->

```text
tests/conformance/<case>/
  case.yaml        what to set up and what to expect
  project/         the project files, including .tophat/
  roots/<root>/    the data of each root the case uses
```

`case.yaml`:

```yaml
description: A reported result whose summary was produced from uncommitted code
since: 0.1.0               # the first tophat version this case applies to
git:                        # replayed in order with a fixed identity and date
  - commit: initial
  - write: {path: studies/s/scripts/run.sh, content: "echo changed\n"}
command: [check, --json]    # default; any tophat command
expect:
  exit: 1
  diagnostics:              # compared as a set of (rule, severity, path); messages are not compared
    - {rule: study.summary-dirty, severity: error, path: studies/s/STUDY.md}
```

- `git` steps are `commit: <message>` (stage everything and commit), `write: {path, content}` and
  `delete: <path>`. Commits use author and committer `TopHat Conformance
  <conformance@tophat.invalid>` at `2026-01-01T00:00:00Z`, so commit hashes are the same on every
  run and can appear in committed records and summaries.
- The runner copies the case to a temporary directory, runs `git init`, replays the steps, sets
  `TOPHAT_ROOT_<NAME>` to each `roots/<root>/` directory, runs `<binary> <command>` from the
  project, and compares the exit code and, for `check --json`, the diagnostics. For other commands,
  `expect` may give `stdout_contains` and `stderr_contains` strings instead.
- A case whose `since` is newer than the binary's version is skipped, so an older release can be run
  against today's suite.

### Required cases <!-- rq-3ccce61c -->

Valid (each expects exit 0 and no error diagnostics): a fresh project; a reported study with a clean,
current summary built from a fan-out pipeline with a QC gate; a published dataset consumed by a
second reported study; a retrospective study consuming a declared dataset (legacy diagnostics
only); an ingested external stage.

Broken (each expects exactly its diagnostic or failure):

| Case | Expected |
|---|---|
| Frozen section edited after pre-registration | `study.frozen-modified` |
| Scenario without a test in a reported study | `study.scenario-untested` |
| Result from a dirty commit | `study.summary-dirty` |
| Dataset file whose content changed | `data.file-changed` |
| Missing QC verdict before a consuming stage | the generated rule stops before the consumer (`stage qc-status` exits 2) |
| Study citing a question that does not exist | `question.unknown-reference` |
| Stage consuming another study's unpublished output (a `stage:` input naming that study's stage) | `pipeline.config-invalid` |
| Pinned dataset retired for a defect | `data.reference-retired` |
| Study reported while a producer is unreported | `data.producer-unreported` (error) |
| Private dataset consumed by a public study | `data.visibility-leak` |
| External receipt for another key | `stage external` exits 1 with `KeyMismatch` |
| External returned output with a wrong hash | `stage external` exits 1 with `OutputMismatch` |
| Edited external job script | `stage external` exits 1 with `ScriptModified` |

Rule names are those of the requirements file of each feature; a case names the rule its feature
specifies.

## End-to-End Example Study <!-- rq-aea3767f -->

The end-to-end test runs the worked example study (see `rqm/template/skills.md`) in a project
rendered with `julia-all-agents`, inside its container image, the way a human and an agent would:

1. Write and stamp a question in `PROJECT.md`; copy the example into `studies/`; set its question.
2. Declare a fixture tool (a local git repository with two commits, providing the scoring script) and
   make `score` a tool stage pinned to the first commit; run `tophat tools sync`.
3. Commit on the host side of the test (outside the container) and pre-register through a
   pseudo-terminal, typing the study ID as a human would.
4. Run `tophat run`: it stops at the external stage `filter`. Copy the job folder to a directory
   standing in for the cluster, run `sh job.sh` there with a `PATH` of POSIX tools only, copy the
   folder back without kept files, and run again.
5. Give QC verdicts through a pseudo-terminal as the run stops at each gate; run until the summary
   is written; `tophat study report`; require `tophat check --strict` to report no errors.
6. Verify the outputs, provenance records (including the `external` record), the summary's keys and
   result lines, and that `tophat data lineage` leads from the result to the generated data.
7. Change the tool pin to the second commit, commit, and run: exactly `score`, `analyse` and the
   summary run again (Snakemake's job list), and `generate` and `filter` are untouched.
8. Exercise the failing returns: a job folder returned after a configuration change (stale key), a
   returned output altered after the run, and a job script whose command line was edited; each
   ingest fails with its error and writes no record.

## Adoption Tests <!-- rq-68df5b63 -->

Each fixture is a committed directory and recipe, in the conformance case format, standing for a
typical existing repository:

| Fixture | Contents |
|---|---|
| `scripts-notebooks` | Python scripts, Jupyter notebooks, a `requirements.txt` |
| `conda-env` | an `environment.yml` and R scripts |
| `julia-project` | a Julia `Project.toml` and `Manifest.toml` |
| `snakemake-pipeline` | its own `Snakefile`, `config.yaml` and `.gitignore` |
| `make-pipeline` | a `Makefile` and processed data in `results/` |

For each fixture, `cargo xtask adoption`:

1. copies the fixture into a temporary repository, commits it, and records the hash of every file;
2. applies the template in place (`copier copy` into the repository) with `minimal-claude`;
3. requires every recorded file to be unchanged and every file Copier skipped to be reported;
4. performs the mapping the adoption skill would propose, through the CLI: `tophat data declare`
   for the fixture's processed data and a `retrospective` study citing it;
5. requires `tophat check` to report no errors, and the declared dataset and the retrospective study
   as legacy items;
6. requires `tophat data lineage` of the retrospective study's output to show the declared dataset
   marked `[declared, not verified]`.

## Skill Evals <!-- rq-05d2bce1 -->

Evals check that agents following the skills behave as `rqm/template/skills.md` requires. They need
agent API credentials, run in the project's container image, and are not a gate.

### Eval definitions <!-- rq-aa97877d -->

`evals/<name>.yaml`:

```yaml
scenario: rq-xxxxxxxx           # the behaviour scenario this eval checks
answers: julia-all-agents
setup:                          # conformance-style steps, plus copy-example and tophat commands
  - copy-example: median-vs-mean
prompt: "Plan a study for question q-… comparing the two filters."
replies:                        # scripted human answers, matched against the agent's question
  - when: "(?i)endpoint"
    say: "The mean RMSE difference over signals; the median filter is better if its 95% interval lies above 0."
max_turns: 30
checks:
  - unchanged: studies/median-vs-mean/STUDY.md#Primary endpoint
  - not_run: ["git commit", "tophat study preregister"]
  - tophat_check: no-errors
```

- `checks` are mechanical: `unchanged` / `changed` (a file, or a `STUDY.md` section), `not_run` /
  `ran` (command prefixes, read from the agent's transcript), `contains` (file text),
  `tophat_check` (`no-errors` or a rule name).
- The harness runs each agent headlessly in its JSON transcript mode, answers each question the
  agent asks with the first matching reply (and fails the eval on a question no reply matches), and
  stops at `max_turns`.
- `--repeat <n>` (default 3) runs every eval `n` times; an eval passes only when every repetition
  passes.
- Every behaviour scenario of `rqm/template/skills.md` has at least one eval.

## CI Workflows <!-- rq-068b6895 -->

`.github/workflows/CI.yaml` runs on pull requests, on pushes to `main`, and weekly:

| Job | Runner | Runs |
|---|---|---|
| `cli` | Linux, macOS, Windows | fmt, clippy, `cargo test`, coverage |
| `snakemake` | Linux | `cargo test` with Snakemake installed and `TOPHAT_REQUIRE_SNAKEMAKE=1` |
| `traceability` | Linux | `cargo xtask scenarios`, `cargo xtask lock --check` |
| `render` | Linux, with Podman | `cargo xtask render-test` |
| `conformance` | Linux | `cargo xtask conformance` |
| `e2e` | Linux, with Podman | the end-to-end test |
| `adoption` | Linux | `cargo xtask adoption` |

`.github/workflows/evals.yaml` runs `cargo xtask eval` weekly and on manual dispatch, with agent
credentials from repository secrets; its result is reported, never required.

`.github/workflows/release.yaml` runs on a version tag: it builds `tophat` for
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, runs the conformance suite against the
x86_64 binary, writes the release pin with `cargo xtask release-pin`, runs the render and
end-to-end tests with that pin, and only then publishes the binaries with their SHA-256 checksums.

## Assumptions <!-- rq-5be14147 -->

- GitHub-hosted Linux runners provide Podman and allow rootless containers.
- Apptainer is not available in CI; the Apptainer scenarios of `rqm/template/container.md` run with
  `th.sh --dry-run --run-sif` and a stub `apptainer` that prints its arguments and image labels.

## Out of Scope <!-- rq-117c41af -->

- GPU passthrough on real hardware (CI has no GPU); detection is tested on simulated hosts.
- Performance and load testing.
- Evals as a merge gate.

## Gherkin Scenarios <!-- rq-f9939ba7 -->

```gherkin
Feature: Test gates

  # --- CLI tests ---

  @rq-b55aac3a
  Scenario: Snakemake tests cannot silently pass in CI
    Given TOPHAT_REQUIRE_SNAKEMAKE is "1" and TOPHAT_TEST_SNAKEMAKE is unset
    When a Snakemake-backed test runs
    Then it fails saying Snakemake is required

  @rq-20c233fd
  Scenario: Snakemake tests skip locally without Snakemake
    Given neither TOPHAT_REQUIRE_SNAKEMAKE nor TOPHAT_TEST_SNAKEMAKE is set
    When a Snakemake-backed test runs
    Then it prints a skip notice and passes

  # --- Conformance runner ---

  @rq-4c6f3b16
  Scenario: A valid case passes
    Given a case expecting exit 0 and no diagnostics, whose project passes the check
    When "cargo xtask conformance --case <it>" is run
    Then it prints "<case> ok" and exits 0

  @rq-23b93182
  Scenario: A missing expected diagnostic fails the case
    Given a case expecting "data.file-changed" whose project produces no diagnostics
    When the case is run
    Then it prints "<case> FAILED" listing "data.file-changed" as missing
    And it exits 1

  @rq-a819acba
  Scenario: An unexpected diagnostic fails the case
    Given a case expecting no diagnostics whose project produces "data.visibility-leak"
    When the case is run
    Then it prints "<case> FAILED" listing "data.visibility-leak" as unexpected

  @rq-58513972
  Scenario: Messages are not compared
    Given a case whose expected diagnostic matches the actual one in rule, severity and path but not message
    When the case is run
    Then it passes

  @rq-067bd77a
  Scenario: Replayed commits are reproducible
    Given a case with a "commit" step
    When it is replayed twice
    Then both replays produce the same commit hash

  @rq-5958144d
  Scenario: Cases newer than the binary are skipped
    Given a case with "since: 9.0.0"
    When the suite runs against a binary of version 0.1.0
    Then the case prints "skipped"
    And it does not fail the suite

  @rq-513a430f
  Scenario: Commands other than check are compared by exit code and output
    Given a case running "stage external" that expects exit 1 and stderr containing "receipt.txt is for key"
    When the case is run against a binary that reports a key mismatch
    Then it passes

  @rq-97bc377b
  Scenario: The required cases are present
    Then tests/conformance holds one case for each valid and each broken case listed in this file

  # --- Render tests ---

  @rq-cb8f92b1
  Scenario: A failing render check names the answer set and step
    Given the template's .tophat/.gitignore lacks "/cache/"
    When "cargo xtask render-test --no-image" is run
    Then it fails naming each answer set and the ignore-rules check

  @rq-5fa25ddc
  Scenario: A Julia project has Julia in its image
    When "cargo xtask render-test --answers julia-all-agents" is run
    Then "pixi run julia --version" succeeds inside its image

  # --- End to end ---

  @rq-2da2ae3f
  Scenario: The example study runs end to end
    When the end-to-end test runs
    Then the example study is reported, its summary is clean and current, and "tophat check --strict" reports no errors

  @rq-9b40e3a8
  Scenario: A changed tool pin re-runs exactly the downstream stages
    Given the end-to-end study has been reported
    When the score stage's pin changes to the tool's second commit and the pipeline runs
    Then Snakemake runs score, analyse and the summary and no other job

  @rq-6ffea465
  Scenario: Failing returns write no record
    When the end-to-end test returns a stale job folder, an altered output and an edited job script
    Then each ingest fails with KeyMismatch, OutputMismatch or ScriptModified respectively
    And no provenance.json is written for the external stage

  # --- Adoption ---

  @rq-f54f3fe1
  Scenario: Adoption overwrites no user file
    When "cargo xtask adoption --fixture snakemake-pipeline" is run
    Then every file of the fixture is unchanged, including its Snakefile and .gitignore
    And the files Copier skipped are reported

  @rq-fb67f0df
  Scenario: Adopted content is labelled legacy
    When "cargo xtask adoption --fixture make-pipeline" is run
    Then "tophat check" reports no errors and lists the declared dataset and the retrospective study as legacy
    And the lineage of the retrospective study's output shows "[declared, not verified]"

  # --- Evals ---

  @rq-cd345094
  Scenario: An unmatched agent question fails the eval
    Given an eval whose replies match no question the agent asks
    When the eval runs
    Then it fails naming the unanswered question

  @rq-6fcb1f55
  Scenario: An eval passes only if every repetition passes
    Given an eval that passes in two of three repetitions
    When it runs with "--repeat 3"
    Then it is reported as failed with "2/3"

  @rq-fd2bf270
  Scenario: Forbidden commands are detected from the transcript
    Given an eval with "not_run: [git commit]" and an agent transcript containing a "git commit -m x" command
    When the checks run
    Then the eval fails naming "git commit"

  @rq-7238187f
  Scenario: Every behaviour scenario has an eval
    Then every scenario of the behaviour feature in rqm/template/skills.md is named by an evals/*.yaml file

  # --- Release ---

  @rq-e6ed00d3
  Scenario: A release is published only after the gates pass
    Given a version tag whose x86_64 binary fails a conformance case
    When the release workflow runs
    Then no binary is published
```
