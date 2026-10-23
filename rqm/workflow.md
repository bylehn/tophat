# Feature: Status and Run <!-- rq-168e854a -->

Two commands make the study lifecycle easy to follow. `tophat status` shows where each study is and
the single next action, naming who takes it (the human or anyone). `tophat run` runs a study's
pipeline with one short command, refusing when the study is not in a state to produce reportable
results, and ends by printing the next action. Both build on the other features: the study
lifecycle (`rqm/study/format.md`), the question tree (`rqm/question-tree.md`), data
(`rqm/data.md`), and pipelines (`rqm/pipeline/provenance.md`, `rqm/pipeline/snakemake.md`).

## Trial Runs <!-- rq-0d5e77fe -->

A **trial run** runs a draft study's pipeline to try it out before pre-registration, without
producing anything that can be reported.

- Trial outputs live under `<results root>/<study>/.trial/<stage>/<key>/` and their provenance
  records have `"trial": true` (see `rqm/pipeline/provenance.md`).
- Trial runs use each stage's `trial` inputs and parameters (see `rqm/study/exploration.md`), skip
  QC gates, never write `SUMMARY.md`, and their outputs cannot be published. The trial target ends
  by writing `EXPLORATION.md`.
- Outputs of a trial run never satisfy a non-trial run: after pre-registration the pipeline runs
  again into the non-trial locations.
- The rules generated for a `draft` study contain only trial targets (see
  `rqm/pipeline/snakemake.md`), so running Snakemake directly cannot bypass this either.

## Next Action <!-- rq-956cc0ad -->

The next action for a study is the first row of this table whose condition holds. `you` marks
actions only a human may take; `anyone` marks actions a human or an agent may take.

| # | Condition | Next action | Who |
|---|---|---|---|
| 1 | Status is `superseded` | None: superseded by `<study>` | — |
| 2 | `tophat check` reports errors for the study (see *Study scope*) | Fix the errors: `tophat check studies/<study>` | anyone |
| 3 | Status is `draft` and any of Question, Primary endpoint, Decision rule is empty, or there are no scenarios | Write the plan in `STUDY.md` | anyone |
| 4 | Status is `draft` | Pre-register: `tophat study preregister <study>` | you |
| 5 | The study has no stages | Add stages to `config/base.yaml` | anyone |
| 6 | The `results` root is configured (in `.tophat/roots.local.yaml` or `TOPHAT_ROOT_RESULTS`) at a directory that does not exist | Create `<location>`, or fix the `results` entry of `.tophat/roots.local.yaml` | anyone |
| 7 | A tool stage's pinned commit is unresolved or not cached on this machine | Fetch tools: `tophat tools sync` | you |
| 8 | An external stage's bundle is written for a variant's current key and the job has not been returned | Run the external job for `<stage>` (`<variant>`): `tophat stage instructions <study> <stage> --variant <variant>` | you |
| 9 | A gated stage (one with a `review` question, see `rqm/pipeline/snakemake.md`) is sealed for a variant's current key and has no verdict, and either a stage consuming it has no sealed output or the study is not yet reported | Give a QC verdict: `tophat stage qc <study> <stage> --variant <variant>` | you |
| 10 | A QC verdict for a current key is `fail` | Fix the cause of the failed QC of `<stage>` (`<variant>`); the next run uses a new key | anyone |
| 11 | Some stage of some listed variant has no sealed output for its current key, or `SUMMARY.md` is missing or stale | Run the pipeline: `tophat run <study>` | anyone |
| 12 | `SUMMARY.md` is dirty | Commit the code listed as unclean, then `tophat run <study>` | anyone |
| 13 | Status is `preregistered` | Report: `tophat study report <study>` | anyone |
| 14 | The study's question is `open` and no answered question cites the study | Answer question `<question>` in `PROJECT.md`, citing this study | anyone |
| 15 | Otherwise | None: done | — |

Retrospective studies without stages skip rows 5 to 13. A study whose `STUDY.md` cannot be parsed
has the next action of row 2.

### Study scope <!-- rq-b1f6d54f -->

The errors that block a study (row 2, and `tophat run`) are the `error` diagnostics of
`tophat check` whose path is under `studies/<study>/`, under `datasets/<name>/` for a dataset one of
the study's stages references, or is `.tophat/roots.yaml` or `.tophat/roots.local.yaml`, except
`pipeline.summary-stale`: running the pipeline is what resolves a stale summary (row 11), so it never
blocks a run.

## Feature API <!-- rq-07a0e0ea -->

### Commands <!-- rq-079894b1 -->

- `tophat status [<study>...] [--json]` <!-- rq-6733b7c0 -->
  - Read-only. With no studies named, covers every study; superseded studies are listed on one
    line each at the end.
  - Prints, first, a project line: the number of questions by status, and the number of check
    errors, warnings and legacy items for the whole project.
  - Then, per study in ID order, a block:

    ```
    x-vs-docking  preregistered  q-9c0d1e2f  Does X rank binders better than docking?
      pipeline: base 3/3 sealed, pbe 1/3 sealed
      next (you): Give a QC verdict: tophat stage qc x-vs-docking score --variant pbe
    ```

    The pipeline line counts, per listed variant, stages with a sealed non-trial output for the
    current key out of all stages; it is omitted for studies without stages.
  - `--json` prints `{"project": {...}, "studies": [...]}` with, per study, `id`, `status`,
    `question`, `pipeline` (per variant: `sealed`, `total`, or `null`), and `next`
    (`{"row", "who", "text", "command"}`, `command` being `null` when the action is not a single
    command).
  - Exits 0, or 2 when run outside a project. Naming a study that does not exist exits 1.
- `tophat run <study> [--variant <variant>] [--cores <n>] [--draft] [-- <snakemake args>...]` <!-- rq-cdc39471 -->
  - Refuses (exit 1, Snakemake not started) when:
    - the study does not exist or has no stages;
    - its status is `draft` and `--draft` is not given (the message explains pre-registration and
      `--draft`);
    - `--draft` is given and its status is not `draft`;
    - its status is `superseded`;
    - `tophat check` reports errors for the study (listed in the message);
    - the `--variant` is not listed in `STUDY.md`.
  - Otherwise runs, from the project root and with the terminal attached,
    `pixi run snakemake -s studies/<study>/Snakefile --cores <n> <target> <snakemake args>`, where
    `<n>` defaults to the number of available CPUs and `<target>` is:

    | Mode | Without `--variant` | With `--variant <v>` |
    |---|---|---|
    | normal | `summary` | `variant_<v>` |
    | `--draft` | `trial` | `trial_<v>` |

  - After Snakemake exits, prints a blank line and the study's next action (as in
    `tophat status`). When Snakemake stopped at a QC gate, the next action is that gate's row 9;
    when it stopped at an external stage, it is that stage's row 8.
  - Exits with Snakemake's exit code; exits 2 if `pixi` cannot be started.

### Types <!-- rq-5cc877ea -->

- `NextAction` — `row: u8`, `who: Who` (`You`, `Anyone`, `Nobody`), `text: String`, <!-- rq-6ceae812 -->
  `command: Option<String>`.
- `StudyStatus` — the per-study part of `tophat status`: ID, status, question, per-variant <!-- rq-d511d6c8 -->
  pipeline progress, next action.

### Functions <!-- rq-e09ced9c -->

- `next_action(root: &Path, study: &str) -> Result<NextAction, StudyError>` — evaluates the <!-- rq-bf93285a -->
  *Next Action* table for one study.
- `study_status(root: &Path, studies: &[String]) -> Result<Vec<StudyStatus>, StudyError>` — the <!-- rq-02e65d51 -->
  data behind `tophat status`.

## Assumptions <!-- rq-3573382c -->

- `tophat run` delegates execution entirely to Snakemake; it adds checks and messages around it
  and does not run stages itself.
- The next-action table is evaluated from committed and local files only; it never runs pipelines,
  hashes data, or contacts a network.

## Out of Scope <!-- rq-a7314db6 -->

- Cluster profiles and other executor configuration for Snakemake (pass them after `--`).
- Partial trial runs over a subset of items.
- Notifications or dashboards beyond the terminal output.

## Gherkin Scenarios <!-- rq-0b009184 -->

```gherkin
Feature: Status and run

  Background:
    Given a TopHat project in a git repository with root "results" configured
    And PROJECT.md has question "q-9c0d1e2f" with status "open"

  # --- Next action ---

  @rq-a3f2b786
  Scenario: A fresh draft needs its plan written
    Given "tophat study new x-vs-docking --question q-9c0d1e2f" was run
    When "tophat status x-vs-docking" is run
    Then the next action is row 3 "Write the plan in STUDY.md" for "anyone"

  @rq-2ec453d8
  Scenario: A complete draft waits for the human to pre-register
    Given draft study "x-vs-docking" has its plan written and no check errors
    When "tophat status x-vs-docking" is run
    Then the next action is "(you)" "tophat study preregister x-vs-docking"

  @rq-5a59811a
  Scenario: Check errors come first
    Given draft study "x-vs-docking" has an unknown front-matter status
    When "tophat status x-vs-docking" is run
    Then the next action is row 2 "tophat check studies/x-vs-docking"

  @rq-4d4f5fd4
  Scenario: Errors in another study do not block this one
    Given study "other-study" has check errors
    And draft study "x-vs-docking" has its plan written and no check errors of its own
    When "tophat status x-vs-docking" is run
    Then the next action is row 4

  @rq-ee88aae5
  Scenario: Errors in a referenced dataset block the study
    Given preregistered study "x-vs-docking" references "dataset:benchmark-set@v1"
    And "datasets/benchmark-set/v1.yaml" has a "data.manifest-invalid" error
    When "tophat status x-vs-docking" is run
    Then the next action is row 2

  @rq-eebf1864
  Scenario: Preregistered study without stages
    Given preregistered study "x-vs-docking" has "stages: []"
    When "tophat status x-vs-docking" is run
    Then the next action is row 5 "Add stages to config/base.yaml"

  @rq-cde76c0f
  Scenario: A configured results location that does not exist
    Given preregistered study "x-vs-docking" has stages
    And ".tophat/roots.local.yaml" locates root "results" at a directory that does not exist
    When "tophat status x-vs-docking" is run
    Then the next action is row 6 naming that directory

  @rq-1bd248f0
  Scenario: The default results folder needs no configuration
    Given preregistered study "x-vs-docking" has stages
    And root "results" is not configured and "results/" does not exist yet
    When "tophat status x-vs-docking" is run
    Then the next action is not row 6
    And the pipeline line counts the sealed stages of each variant

  @rq-9bb38d37
  Scenario: Waiting for a QC verdict
    Given preregistered study "x-vs-docking" has stages "score" (gated) and "analyse"
    And "score" is sealed for variant "pbe" with no verdict and "analyse" is not sealed for "pbe"
    When "tophat status x-vs-docking" is run
    Then the next action is "(you)" "tophat stage qc x-vs-docking score --variant pbe"

  @rq-032bd279
  Scenario: Tools not fetched on this machine
    Given preregistered study "x-vs-docking" has a tool stage whose pinned commit is not cached
    When "tophat status x-vs-docking" is run
    Then the next action is row 7 "(you)" "Fetch tools: tophat tools sync"

  @rq-f3b637ee
  Scenario: External job waiting for the human
    Given preregistered study "x-vs-docking" has an external stage "md" whose bundle is written and whose job has not been returned
    When "tophat status x-vs-docking" is run
    Then the next action is "(you)" "tophat stage instructions x-vs-docking md --variant base"

  @rq-14542671
  Scenario: Failed QC
    Given the current verdict for "score" in variant "base" is "fail"
    When "tophat status x-vs-docking" is run
    Then the next action is row 10 naming "score" and "base"

  @rq-4b2809d2
  Scenario: Stages left to run
    Given preregistered study "x-vs-docking" has 3 stages and only "prepare" is sealed
    When "tophat status x-vs-docking" is run
    Then the pipeline line reads "base 1/3 sealed"
    And the next action is "tophat run x-vs-docking"

  @rq-b1a89dcb
  Scenario: Stale summary means run again
    Given every stage is sealed but SUMMARY.md has keys that no longer match
    When "tophat status x-vs-docking" is run
    Then the next action is row 11

  @rq-82ac8f57
  Scenario: Dirty summary means commit and run again
    Given every stage is sealed and SUMMARY.md is current but dirty
    When "tophat status x-vs-docking" is run
    Then the next action is row 12

  @rq-f0049ad3
  Scenario: Ready to report
    Given preregistered study "x-vs-docking" has a current, clean SUMMARY.md
    When "tophat status x-vs-docking" is run
    Then the next action is "tophat study report x-vs-docking"

  @rq-13a88566
  Scenario: Reported study whose question is still open
    Given study "x-vs-docking" is reported with question "q-9c0d1e2f"
    And no question cites "x-vs-docking"
    When "tophat status x-vs-docking" is run
    Then the next action is row 14 naming "q-9c0d1e2f"

  @rq-62e72400
  Scenario: Reported study that is cited is done
    Given study "x-vs-docking" is reported
    And question "q-9c0d1e2f" is answered citing "x-vs-docking"
    When "tophat status x-vs-docking" is run
    Then the next action is row 15 "None: done"

  @rq-dffba6fd
  Scenario: Superseded study
    Given study "old-study" is superseded by "x-vs-docking"
    When "tophat status" is run
    Then "old-study" is listed on one line at the end, naming "x-vs-docking"

  @rq-2b6e16c6
  Scenario: Retrospective study without a pipeline
    Given retrospective study "old-benchmark" has no stages and its question is answered citing it
    When "tophat status old-benchmark" is run
    Then the next action is row 15

  # --- Status output ---

  @rq-018e0aed
  Scenario: Project line
    Given PROJECT.md has 2 open, 1 answered and 1 dropped questions
    And the project has 1 check error and 2 warnings
    When "tophat status" is run
    Then the first line reports the question counts and "1 error(s), 2 warning(s), 0 legacy item(s)"

  @rq-682322e9
  Scenario: JSON status for agents
    Given preregistered study "x-vs-docking" waits for a QC verdict on "score" for "pbe"
    When "tophat status x-vs-docking --json" is run
    Then the study's "next" is {"row": 9, "who": "you", "command": "tophat stage qc x-vs-docking score --variant pbe"} with its text

  @rq-8a1808bb
  Scenario: Status is read-only
    Given studies in several states
    When "tophat status" is run
    Then no file in the project or the results root is created, modified or deleted

  @rq-6e632e7d
  Scenario: Unknown study
    When "tophat status no-such-study" is run
    Then the exit code is 1

  @rq-7b3ae395
  Scenario: Unparseable study
    Given study "broken" has invalid front matter
    When "tophat status broken" is run
    Then the next action is row 2

  # --- tophat run ---

  @rq-c8940b0d
  Scenario: Run a preregistered study
    Given preregistered study "x-vs-docking" has stages and no check errors
    And a fake "pixi" on PATH records its arguments and exits 0
    When "tophat run x-vs-docking --cores 4" is run
    Then pixi was started from the project root with "run snakemake -s studies/x-vs-docking/Snakefile --cores 4 summary"
    And the output ends with the study's next action
    And the exit code is 0

  @rq-a3466977
  Scenario: Run one variant
    Given preregistered study "x-vs-docking" lists variants ["base", "pbe"]
    When "tophat run x-vs-docking --variant pbe" is run with a fake pixi
    Then the Snakemake target is "variant_pbe"

  @rq-24702b2d
  Scenario: Default cores
    When "tophat run x-vs-docking" is run with a fake pixi
    Then "--cores" is the number of available CPUs

  @rq-a9d102bf
  Scenario: Extra Snakemake arguments are passed through
    When "tophat run x-vs-docking -- --dry-run --printshellcmds" is run with a fake pixi
    Then the arguments end with "summary --dry-run --printshellcmds"

  @rq-d1d30204
  Scenario: Snakemake's exit code is returned
    Given a fake pixi that exits 1
    When "tophat run x-vs-docking" is run
    Then the exit code is 1
    And the study's next action is still printed

  @rq-5e257285
  Scenario: Stopped at a QC gate
    Given a fake pixi that exits 1 after which "score" is sealed for "base" without a verdict
    When "tophat run x-vs-docking" is run
    Then the last line names "tophat stage qc x-vs-docking score --variant base" for "you"

  @rq-d76deef1
  Scenario: Draft refused without --draft
    Given draft study "x-vs-docking" has stages
    When "tophat run x-vs-docking" is run
    Then the exit code is 1
    And the message explains pre-registration and the "--draft" option
    And pixi is not started

  @rq-23d4abbf
  Scenario: Trial run of a draft
    Given draft study "x-vs-docking" has stages and no check errors
    When "tophat run x-vs-docking --draft" is run with a fake pixi
    Then the Snakemake target is "trial"
    And the output reminds that trial results cannot be reported

  @rq-eb7be7fe
  Scenario: Trial run of one variant
    When "tophat run x-vs-docking --draft --variant pbe" is run on a draft with a fake pixi
    Then the Snakemake target is "trial_pbe"

  @rq-88a10bea
  Scenario: --draft on a preregistered study is refused
    Given preregistered study "x-vs-docking"
    When "tophat run x-vs-docking --draft" is run
    Then the exit code is 1 and pixi is not started

  @rq-d14816cc
  Scenario: Superseded study is refused
    Given study "old-study" is superseded
    When "tophat run old-study" is run
    Then the exit code is 1 and pixi is not started

  @rq-fd9f1a7c
  Scenario: Check errors are refused
    Given preregistered study "x-vs-docking" has a "pipeline.config-invalid" error
    When "tophat run x-vs-docking" is run
    Then the exit code is 1
    And the message lists the error
    And pixi is not started

  @rq-2e3b70f6
  Scenario: Warnings do not block a run
    Given preregistered study "x-vs-docking" has only warnings
    When "tophat run x-vs-docking" is run with a fake pixi
    Then pixi is started

  @rq-6e13314c
  Scenario: Unknown variant
    When "tophat run x-vs-docking --variant nope" is run
    Then the exit code is 1 and pixi is not started

  @rq-d17bfed2
  Scenario: Study without stages
    Given preregistered study "x-vs-docking" has "stages: []"
    When "tophat run x-vs-docking" is run
    Then the exit code is 1 and the message says to add stages

  @rq-3d60791d
  Scenario: pixi not available
    Given no "pixi" executable is on PATH
    When "tophat run x-vs-docking" is run for a runnable study
    Then the exit code is 2
    And the message says pixi could not be started

  # --- Trial runs (requires Snakemake) ---

  @rq-2a741327
  Scenario: Trial outputs are separate and unreportable
    Given Snakemake is available
    And draft study "x-vs-docking" has stages including a gated one and one with "result"
    When "tophat run x-vs-docking --draft" is run
    Then every stage is sealed under "<results root>/x-vs-docking/.trial/"
    And every provenance record has "trial": true
    And no QC verdict was required
    And SUMMARY.md was not written
    When the study is pre-registered and "tophat run x-vs-docking" is run
    Then every stage runs again into the non-trial locations

  @rq-256e64ad
  Scenario: Direct Snakemake on a draft only offers trial targets
    Given draft study "x-vs-docking" has stages
    When its rules are generated
    Then the rules file has targets "trial" and "trial_<variant>" only
```
