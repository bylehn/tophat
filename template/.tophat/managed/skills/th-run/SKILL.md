---
name: th-run
description: Run a study's pipeline and see it through to a reported result, handing QC verdicts and external (cluster) jobs to the human. Use when a study has stages and the human wants to run it, continue a stopped run, fix a failed run, or report its results. Not for designing the study or writing its stages; use th-study for that.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-run/local.md. -->

# th-run

## Running

Run `tophat run <id>`. It refuses to start a run that cannot produce reportable results and says
why; fix that first (usually with `th-study` or `th-data`). For a draft study, run
`tophat run <id> --draft` and tell the human that trial results are kept apart and can never be
reported. Every run ends with the study's next action; act on it as below.

## QC hand-off

When the run stops at a QC gate, the human must look at the stage's output and give a verdict.

1. Show the human where the output is: `tophat pipeline keys <id> --variant <variant>` lists each
   stage's key and output directory.
2. If it helps, summarize what the output contains (files, sizes, a few values), without judging
   it.
3. Give the human the exact command and stop: `tophat stage qc <id> <stage> --variant <variant>`.

Never give, suggest or imply a verdict, and never write a file under `qc/`. When the verdict is
`fail`, the cause is fixed upstream (a new configuration or code, hence a new key), never by
changing the verdict.

## External hand-off

When the run stops at an external stage, the human runs its job on a cluster.

1. Show the human the steps: `tophat stage instructions <id> <stage> --variant <variant>`.
2. Never run the copy or submission steps yourself, and never connect to the cluster.
3. When the human says the job folder is back, run the pipeline again; it checks the returned folder
   and continues. If it reports a mismatch, explain it from the message and let the human decide.

## Failures

When a stage fails, read the Snakemake output, find the cause, and explain it. Fix study code under
the build rules of `th-study`, then run again. Never edit outputs or provenance records by hand.

## Stale or dirty results

A summary computed from uncommitted code is dirty and cannot be reported. Ask the human to commit
the files listed as unclean, on the host, naming them; then run again. A summary whose keys no
longer match the configuration is stale: run again.

## Reporting

When the next action is to report:

1. Run `tophat study report <id>`. It copies the result lines from `SUMMARY.md` into `## Result`;
   never write or change a result line yourself.
2. Apply the pre-registered decision rule to the primary endpoint exactly as written in the study,
   for each variant.
3. Draft `## Interpretation` for the human to edit: what the result means for the question, stating
   plainly when it is not useful or not significant. Diagnostics inform but decide nothing; only the
   primary endpoint answers the question.
4. Run `tophat check studies/<id>`, then use `th-question` to answer the question if the human
   agrees.

To show where a result came from, use `tophat data lineage <id>/<stage>@<key>`.

## Project Extensions

Read `.tophat/user/skills/th-run/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
