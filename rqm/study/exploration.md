# Feature: Exploration Before the Plan <!-- rq-a213aa7e -->

Science usually explores before it commits: an endpoint, deliverable or criterion chosen before
the data is understood is a guess. In TopHat exploring is recorded rather than thrown away, and
how it is recorded follows the study's mode (see `rqm/study/format.md`):

- In a mode that pre-registers (`confirmatory`, `descriptive`, `methods`, `replication`),
  exploring happens while the study is a `draft`: the pipeline runs as trials (see
  `rqm/workflow.md`), those trials can use pilot data, their findings are collected in a
  generated `EXPLORATION.md`, and scratch work lives in `exploration/<study>/`. When the human
  locks the plan (`tophat study preregister`), TopHat records what the exploration saw,
  including whether it already saw the data that will decide the question. Nothing is refused
  for having explored: a result whose plan was informed by its own deciding data is labelled as
  such, the way an amendment says whether results had been seen.
- In `exploratory` and `retrospective` mode there is no plan to lock and no trial runs: the
  study's runs are real from the start, keep QC gates, and report from their summaries (see
  `rqm/workflow.md`). A finding becomes citable when a study reports it as a labelled result
  line (see `rqm/study/format.md`); which results may be cited, and with which labels, is
  specified in `rqm/question-tree.md`.

This file specifies trial inputs, the deciding data and when it counts as seen, `EXPLORATION.md`,
what pre-registration records, the labels on results, and the check rules. The study format,
modes and lifecycle are specified in `rqm/study/format.md`, trial runs in `rqm/workflow.md`,
stage fields and keys in `rqm/pipeline/provenance.md`, and datasets in `rqm/data.md`.

## Trial Inputs <!-- rq-52742c3b -->

A stage of a study in a mode that pre-registers may declare `trial`, a mapping with `inputs` and
`params` that apply only in trial runs:

```yaml
  - name: prepare
    run: python scripts/prepare.py {in.raw} {out}
    inputs: {raw: dataset:bench@v2}
    trial:
      inputs: {raw: dataset:bench-pilot@v1}
```

- In a trial run, `trial.inputs` and `trial.params` are merged into the stage key by key (as a
  variant merges, see `rqm/pipeline/provenance.md`) before its key is computed, so a trial computes
  with the pilot data and stores its outputs under the trial key.
- A run after pre-registration ignores `trial`. The `trial` mapping does not enter the non-trial
  key, so adding or changing it never re-runs a locked study.
- `trial.inputs` may only name inputs the stage declares, and `trial` may hold no other keys; both
  are configuration errors otherwise (`pipeline.config-invalid`).
- `trial` on a stage of an `exploratory` or `retrospective` mode study is a configuration error
  (`pipeline.config-invalid`), because those modes have no trial runs.

A pilot is typically a small dataset registered next to the full one (for example a few systems of
a benchmark): `tophat data add bench-pilot …` and `tophat data add bench …` (see `rqm/data.md`).

## The Deciding Data <!-- rq-45ae65c9 -->

The **deciding data** of a study in a mode that pre-registers is what its locked pipeline (all
listed variants, without `trial` overrides) computes from: the dataset versions its stages
consume, and the outputs of its stages. For a `replication` study this includes the datasets its
reproduction target consumed, because its pipeline re-runs the target's stages (see
`rqm/pipeline/provenance.md`); a trial run without `trial` overrides computes the reproduction
itself and counts as seen.

The deciding data counts as **seen** when any trial run of the study, in any variant:

- consumed a dataset version that the locked pipeline consumes; or
- sealed an output under the same stage key as a stage of the locked pipeline, meaning it already
  computed exactly what the locked run will compute (for example synthetic data generated with the
  same seed).

TopHat determines this from the trial runs' provenance records; it does not need the human's
account.

## EXPLORATION.md <!-- rq-983f740b -->

`studies/<study>/EXPLORATION.md` of a study in a mode that pre-registers is generated,
committed, and never hand-edited; a study in `exploratory` or `retrospective` mode has none:

```markdown
---
study: median-vs-mean
generated: "2026-10-23T10:12:44Z"
tophat_version: "0.1.0"
deciding_data_seen: false
---

## Trial results

- base/analyse: n=10 mean RMSE(mean) - RMSE(median) = -0.0251, 95% bootstrap interval [...] (sk-1a2b…)

## Data used in trial runs

- dataset bench-pilot@v1, by stage prepare

## Exploration files

- exploration/median-vs-mean/look-at-noise.ipynb sha256:9f86d0…

## Deciding data

- dataset bench@v2: not used by any trial run
- no trial run computed an output of the locked pipeline
```

- **Trial results** lists, per listed variant and result stage, the result line of the stage's
  sealed trial output for its current trial key, with that key; a stage without a sealed trial
  output is listed as `not run`.
- **Data used in trial runs** lists every dataset version consumed by a sealed trial record of the
  study, with the stages that consumed it.
- **Exploration files** lists every regular file below `exploration/<study>/`, sorted, with its
  SHA-256. TopHat does not run or check them.
- **Deciding data** lists each dataset version of the locked pipeline and whether a trial run used
  it, and whether a trial run computed an output of the locked pipeline; `deciding_data_seen` is
  `true` when either applies.
- The trial target of a draft study's generated rules ends by writing it (see
  `rqm/pipeline/snakemake.md`), and `tophat study exploration <study>` writes it at any time
  while the study is a `draft` in a mode that pre-registers. Writing leaves the file untouched
  when only `generated` would change.

## Locking the Plan <!-- rq-fea6913a -->

`tophat study preregister` (see `rqm/study/format.md`) also:

1. says, before asking for confirmation, what the exploration saw: how many trial results, which
   data, how many exploration files, and, when the deciding data was seen, that the result will be
   labelled as informed by it;
2. on confirmation, writes `EXPLORATION.md` as above, as it stands at that moment, and records in
   the `preregistered` block `exploration_hash` (the SHA-256 of `EXPLORATION.md`, as
   `sha256:<hex>`) and `deciding_data_seen` (`true` or `false`). An aborted pre-registration writes
   nothing.

After pre-registration `EXPLORATION.md` is part of the locked record: changing it is an error.
Trial runs are not possible after pre-registration, so it stays as it was.

## Labels <!-- rq-3a21c28f -->

When the `preregistered` block has `deciding_data_seen: true`:

- `SUMMARY.md` carries `deciding_data_seen: true` in its front matter (see
  `rqm/pipeline/snakemake.md`);
- `tophat study report` prints that the plan was informed by results on the deciding data;
- `tophat data lineage` marks the study's outputs `[plan informed by its deciding data]`;
- `tophat status` shows the study's status as `reported (plan informed by its deciding data)`.

The label is independent of the epistemic tags of result lines (see `rqm/study/format.md`): a
line tagged `pre-specified` keeps its tag when the plan was informed by its deciding data. The
label says how the plan was made; the tag says how the line was derived.

A study whose `preregistered` block has no exploration fields (pre-registered before this feature)
has no label and no `EXPLORATION.md` requirement.

## Feature API <!-- rq-4a253518 -->

### Commands <!-- rq-d3e631be -->

- `tophat study exploration <study-id>` <!-- rq-bc7ec152 -->
  - Requires status `draft` and a mode that pre-registers; otherwise it fails with `WrongStatus`
    or `WrongMode` (see `rqm/study/format.md`). Writes `EXPLORATION.md` from the trial records,
    the locked pipeline's keys and inputs, and `exploration/<study>/`, and prints its path and
    whether the deciding data has been seen. Does not require an interactive terminal.

### Types <!-- rq-91c22cd5 -->

- `Exploration` — the content of `EXPLORATION.md`: trial result lines with keys, datasets used in <!-- rq-36d7d1e3 -->
  trial runs with their consumers, exploration files with hashes, the deciding datasets with whether
  each was used, whether a trial computed an output of the locked pipeline, and
  `deciding_data_seen`.

### Functions <!-- rq-e5152739 -->

- `explore(root: &Path, study: &str) -> Result<Exploration, StudyError>` — computes the exploration <!-- rq-d0c8b971 -->
  record of a study from its files and trial records, without writing.
- `write_exploration(root: &Path, exploration: &Exploration) -> Result<bool, StudyError>` — writes <!-- rq-79590bc8 -->
  `EXPLORATION.md` atomically; returns whether the file changed.

## Check Rules (`study` family) <!-- rq-4d6751e2 -->

| Rule | Severity | Condition |
|---|---|---|
| `study.exploration-invalid` | error | `EXPLORATION.md` cannot be parsed, or names another study |
| `study.exploration-unexpected` | error | `EXPLORATION.md` exists while the study's mode does not pre-register |
| `study.exploration-modified` | error | The `preregistered` block has an `exploration_hash` and `EXPLORATION.md` is missing or its hash differs |
| `study.exploration-label-mismatch` | error | `SUMMARY.md`'s `deciding_data_seen` differs from the `preregistered` block's |

## Assumptions <!-- rq-d087ce39 -->

- Exploration that never runs through the study's pipeline (notebooks, scripts in
  `exploration/<study>/`) is recorded by file hash only; TopHat cannot know which data it touched.
  The human states that in the plan if it matters.
- Trial result lines in `EXPLORATION.md` carry no epistemic tag: tags are written on `SUMMARY.md`
  result lines by `tophat study summarize` (see `rqm/study/format.md`), and nothing in an
  exploration record is cited.

## Out of Scope <!-- rq-2fd2cf95 -->

- Splitting one dataset into pilot and confirmation parts; pilots are registered as their own
  datasets.
- Refusing pre-registration because the deciding data was seen.
- Exploration shared across studies; the exploration area holds per-study directories.
- Which results may be cited as evidence for a question, and with which labels; exploratory
  findings are citable as labelled result lines of a study (`rqm/question-tree.md`).
- Recording the scratch files of a study whose mode does not pre-register, and any tooling for
  the exploration area beyond `EXPLORATION.md` (open question in `rqm/ARCHITECTURE.md`).

## Gherkin Scenarios <!-- rq-0e00e0de -->

```gherkin
Feature: Exploration before the plan

  Background:
    Given a TopHat project in a git repository with root "results" configured
    And datasets "bench@v2" and "bench-pilot@v1" are registered
    And draft confirmatory study "x-vs-docking" has stages "prepare" (input "raw" from "dataset:bench@v2") and "analyse" (result)

  # --- Trial inputs ---

  @rq-a65aea60
  Scenario: Trial runs use the trial inputs
    Given stage "prepare" has "trial: {inputs: {raw: dataset:bench-pilot@v1}}"
    When the study is run as a trial
    Then the trial record of "prepare" lists input "raw" from "bench-pilot@v1"

  @rq-a96b7d8f
  Scenario: The trial mapping does not enter the locked key
    Given the non-trial keys of all stages are recorded
    When stage "prepare" gets "trial: {params: {n: 10}}"
    Then the non-trial keys of all stages are unchanged
    And the trial key of "prepare" differs from its non-trial key

  @rq-864e9f23
  Scenario: A trial input must be an input of the stage
    Given stage "prepare" has "trial: {inputs: {other: dataset:bench-pilot@v1}}"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "other" is reported

  @rq-246da339
  Scenario: A trial mapping holds only inputs and params
    Given stage "prepare" has "trial: {run: echo x}"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "run" is reported

  @rq-dd5fc8f7
  Scenario: A study without pre-registration has no trial inputs
    Given exploratory study "outlier-dig" has a stage "prepare" with "trial: {params: {n: 10}}"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "trial" is reported

  # --- Deciding data and EXPLORATION.md ---

  @rq-3842f534
  Scenario: Exploring on a pilot leaves the deciding data unseen
    Given stage "prepare" has trial inputs from "bench-pilot@v1" and the study was run as a trial
    When "tophat study exploration x-vs-docking" is run
    Then EXPLORATION.md lists "bench-pilot@v1" as used and "bench@v2" as not used by any trial run
    And it has "deciding_data_seen: false"

  @rq-dc5dfbc8
  Scenario: Exploring on the deciding dataset is seen
    Given stage "prepare" has no trial inputs and the study was run as a trial
    When "tophat study exploration x-vs-docking" is run
    Then EXPLORATION.md lists "bench@v2" as used by a trial run
    And it has "deciding_data_seen: true"

  @rq-019ede34
  Scenario: An identical trial computation is seen
    Given a study whose first stage generates data from a seed, without trial overrides
    And the study was run as a trial
    When "tophat study exploration" is run
    Then EXPLORATION.md says a trial run computed an output of the locked pipeline
    And it has "deciding_data_seen: true"

  @rq-89cbcbc0
  Scenario: A different trial seed keeps synthetic data unseen
    Given that stage has "trial: {params: {seed: 2}}" and the study was run as a trial
    When "tophat study exploration" is run
    Then it has "deciding_data_seen: false"

  @rq-039a519b
  Scenario: A replication trial computes its reproduction
    Given replication study "rerun-score" has "reproduces: x-vs-docking@<full-commit>" and no trial overrides
    And the study was run as a trial
    When "tophat study exploration rerun-score" is run
    Then EXPLORATION.md says a trial run computed an output of the locked pipeline
    And it has "deciding_data_seen: true"

  @rq-ce8f7bac
  Scenario: Trial results and exploration files are listed
    Given the trial run sealed "analyse" with result line "rho=0.42" in variant "base"
    And "exploration/x-vs-docking/notes.ipynb" exists
    When "tophat study exploration x-vs-docking" is run
    Then EXPLORATION.md lists "base/analyse: rho=0.42" with the trial key
    And it lists "exploration/x-vs-docking/notes.ipynb" with its SHA-256

  @rq-d887a262
  Scenario: Trial result lines carry no epistemic tag
    Given stage "analyse" declares "epistemic: exploratory" and the study was run as a trial
    When "tophat study exploration x-vs-docking" is run
    Then the trial result line of "analyse" is listed verbatim, without a tag group

  @rq-2e420949
  Scenario: The trial target writes EXPLORATION.md
    Given Snakemake is available
    When "tophat run x-vs-docking --draft" is run
    Then EXPLORATION.md exists and lists the trial results

  @rq-e3a73fb1
  Scenario: Exploration only for drafts
    Given study "x-vs-docking" is pre-registered
    When "tophat study exploration x-vs-docking" is run
    Then it fails saying the exploration was recorded when the plan was locked

  @rq-a88d8dfe
  Scenario: Exploration needs a mode that pre-registers
    Given exploratory study "outlier-dig" has status "draft"
    When "tophat study exploration outlier-dig" is run
    Then it fails with WrongMode saying the study has no plan to lock

  @rq-2a055696
  Scenario: An exploration record without pre-registration is an error
    Given exploratory study "outlier-dig" has an EXPLORATION.md
    When "tophat check" is run
    Then an error "study.exploration-unexpected" is reported

  @rq-b3af0612
  Scenario: Rewriting an unchanged exploration changes nothing
    Given EXPLORATION.md was written
    When "tophat study exploration x-vs-docking" is run again without new trials
    Then EXPLORATION.md is byte-for-byte unchanged

  # --- Locking the plan ---

  @rq-7aef1eb3
  Scenario: Pre-registration records the exploration
    Given the study explored on "bench-pilot@v1" only
    When "tophat study preregister x-vs-docking" is run interactively and confirmed
    Then EXPLORATION.md is written
    And the preregistered block has "exploration_hash" equal to the SHA-256 of EXPLORATION.md and "deciding_data_seen: false"

  @rq-924b622d
  Scenario: Pre-registration of a descriptive study records its exploration
    Given descriptive study "bench-characterisation" has status "draft" and explored on "bench-pilot@v1" only
    When "tophat study preregister bench-characterisation" is run interactively and confirmed
    Then EXPLORATION.md is written for "bench-characterisation"
    And its preregistered block has "exploration_hash" and "deciding_data_seen: false"

  @rq-db0c3b99
  Scenario: Pre-registration says what the exploration saw
    Given the study's trial runs used "bench@v2"
    When "tophat study preregister x-vs-docking" is run interactively
    Then before asking for confirmation it says that the deciding data was seen and that the result will be labelled as informed by it
    And confirming still pre-registers the study

  @rq-5b211090
  Scenario: Editing the locked exploration is an error
    Given the study was pre-registered with an exploration record
    When EXPLORATION.md is edited
    Then "tophat check" reports an error "study.exploration-modified"

  @rq-647bcecc
  Scenario: An invalid exploration file is reported
    Given EXPLORATION.md names study "other-study"
    When "tophat check" is run
    Then an error "study.exploration-invalid" is reported

  # --- Labels ---

  @rq-643d82cb
  Scenario: A result informed by its deciding data is labelled
    Given the study was pre-registered with "deciding_data_seen: true", run and summarized
    Then SUMMARY.md has "deciding_data_seen: true"
    And "tophat study report x-vs-docking" prints that the plan was informed by results on the deciding data
    And "tophat data lineage" of its analyse output is marked "[plan informed by its deciding data]"
    And its result lines keep their epistemic tags

  @rq-6ee5a634
  Scenario: The summary must carry the study's label
    Given the preregistered block has "deciding_data_seen: true"
    And SUMMARY.md has "deciding_data_seen: false"
    When "tophat check" is run
    Then an error "study.exploration-label-mismatch" is reported

  @rq-2048dd2b
  Scenario: Studies pre-registered without an exploration record
    Given a study whose preregistered block has only "date" and "frozen_hash"
    When "tophat check" is run
    Then no exploration rule reports anything for it
```
