<!-- TopHat-managed: the rules of every agent session in this project. `copier update` changes this
file; put project conventions in AGENTS.md instead. -->

# Working in a TopHat project

This is a TopHat research project. Its promise is that every reported result can be traced,
mechanically and without trusting you, to the pre-registered question and endpoint it answers, the
code and environment that computed it, and the data it consumed. You help the scientist get there;
the `tophat` CLI does everything that must be exactly right.

## How the research is organized

The project has one overarching question, broken down into sub-questions in `PROJECT.md` (the
question tree), each with a stable ID. A **study** (`studies/<id>/STUDY.md`) answers one question
with one pre-registered primary endpoint and decision rule, and runs end to end as a pipeline of
**stages** declared in `config/`. A **variant** is the same study under a different setting. A
**dataset** is registered raw data or a published study output with a stable name and version. A
**tool** is general-purpose code in its own repository, pinned by commit. `exploration/` holds
scratch work that is never cited.

## Hard rules

1. Never invent a question, primary endpoint, decision rule, threshold, data source, licence or
   provenance statement: ask the human.
2. Never run `tophat study preregister`, `tophat stage qc` or `tophat tools sync`, and never write a
   QC verdict file. Give the human the exact command.
3. Never connect to a remote machine, transfer data to or from one, or submit a batch job. For an
   external stage, show the human the steps from `tophat stage instructions`.
4. Never edit the frozen sections (Question, Primary endpoint, Decision rule, Scenarios) of a study
   that has a `preregistered` block, never hand-write Result lines, and never edit `SUMMARY.md`,
   provenance records, `.tophat/tools.lock` or the `Snakefile` stub.
5. Never put a step a human is asked to run into a script you wrote; human steps are `tophat`
   commands or the steps `tophat` prints.
6. Never change tool code from within a study; changes go through the tool's own repository.
7. Nothing in `exploration/` is cited as a result.
8. Run `tophat check` before telling the human that work is done, and report its errors.
9. Never commit, push or change git identity: the human commits on the host. Ask the human to
   commit, naming the files, when a step needs committed code.

`tophat` computes every ID, hash, stage key, seal and result line. Never compute or write one
yourself.

## Skills

- `th-next`: find the next action with `tophat status` and carry it out (start here).
- `th-question`: write, stamp, answer or drop questions in `PROJECT.md`.
- `th-study`: plan a study in `STUDY.md` (plan mode) or build its pipeline (build mode).
- `th-run`: run a study's pipeline, hand QC and external jobs to the human, and report results.
- `th-data`: data roots and datasets: register, declare, publish, verify, retire.
- `th-tool`: declare tools and pin stages to tool commits.
- `th-adopt`: bring an existing repository into TopHat.

When the human has not said what to do, start with `th-next`.
