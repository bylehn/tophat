---
name: th-study
description: Plan a study in its STUDY.md (plan mode, the default), or build its pipeline from that plan (build mode). Use when the human wants to start, design, amend, extend or supersede a study, or to implement a study's stages, scripts and scenario tests. Not for running a pipeline or reporting results; use th-run for those.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-study/local.md. -->

# th-study

A study's `STUDY.md` plays the role a requirements file plays for code: it says what must be true
before any code is written, and the scenario tests check the code against it. A complete model
study, with its pipeline and tests, is `.tophat/managed/examples/median-vs-mean/`; read it before
planning or building your first study here.

There are two modes. Say at the start which one you are in.

## Plan mode (default)

Plan mode writes only `STUDY.md`: no configuration, scripts or tests.

1. **Find the question.** The study answers one stamped question of `PROJECT.md`
   (`tophat question tree`). If it does not exist yet, use `th-question` first.
2. **Look for what exists.** Datasets in `datasets/` and who uses them (`tophat data used-by
   <name>`), published outputs of other studies, and tools (`tophat tools list`). Prefer pinning an
   existing dataset or output over planning a new computation.
3. **Create the study** when it does not exist: `tophat study new <id> --question <question ID>`
   (add `--visibility private` when it touches private data). The ID uses lowercase letters, digits
   and single hyphens.
4. **Explore first if needed.** If the human cannot yet say what to measure, or which threshold
   would decide, they have not explored enough, and any endpoint chosen now would be a guess.
   Suggest exploring first: scratch analyses in `exploration/<id>/`, or writing the stages now and
   trial runs of the draft (`tophat run <id> --draft`), preferably on a pilot: a small dataset
   registered next to the full one, used in trial runs through the stage's `trial` field
   (`trial: {inputs: {raw: dataset:<pilot>@v1}}`, or `trial: {params: {...}}` for fewer or
   differently seeded synthetic data), keeping the rest for the run that decides the question.
   Explain why: what is seen before the plan is locked shapes the plan, and TopHat labels a result
   whose plan was informed by its own deciding data (`EXPLORATION.md` records what the trial runs
   saw; `tophat study exploration <id>` rewrites it). Never fill the gap with a suggestion of your
   own.
5. **Ask the human** for everything only they can decide, and wait for the answers:
   - the **primary endpoint**: the single quantity that answers the question, and how it is
     measured;
   - the **decision rule**: how the endpoint's value maps to an answer, including every threshold;
   - which **variants** matter (the same endpoint under different settings);
   - the audience and whether the study is public or private.

   Never invent an endpoint, a threshold or a decision rule, and never fill those sections with a
   suggestion the human has not confirmed. Leave a section empty until the human has answered.
6. **Write the plan.** Fill `## Question`, `## Primary endpoint`, `## Decision rule`, `## Variants`
   (and list them in the front matter) and `## Stages` (prose; the machine-readable stages come in
   build mode). In `## Scenarios`, write each testable expectation as a checklist item,
   `- S1: <expectation>`, `- S2: …`, never reusing a number (Gherkin blocks tagged `@S1` also work,
   if the human prefers them). Scenarios cover the endpoint computation, the data preparation that
   the result depends on, and anything else the result relies on.
7. **Check.** Run `tophat check studies/<id>` and fix what it reports.
8. **Hand over.** When the plan is complete, first follow the quiz policy in
   `.tophat/user/skills/th-quiz/local.md`: by default offer `th-quiz`, a short quiz on the plan,
   and run it only if the human accepts; under `always` run it without asking; under `never` do not
   mention it. Then, whether or not the quiz was taken and whatever its answers were, ask the human
   to commit the plan on the host and give them the command to lock it themselves:
   `tophat study preregister <id>`. Explain what it does in
   plain words: it locks the Question, Primary endpoint, Decision rule and Scenarios, so the result
   is judged by the plan made before it was seen; the pipeline can still change. Only a human runs
   it.

### After pre-registration

Never edit the frozen sections. A change goes into `## Amendments` as one line:

```markdown
- YYYY-MM-DD — results seen: yes|no — <the reason>
```

with today's date and whether any answer-stage result had been seen. Unless `local.md` says
otherwise, decide what a change is like this, and confirm with the human:

- a new question or a new primary endpoint is a **new study**: plan it, and once it is
  pre-registered, mark the old one with `tophat study supersede <old> --by <new>`;
- a fix that does not change what is measured is an **amendment**;
- a different setting under the same endpoint (method, parameter, tool version, data subset) is a
  **variant**.

## Build mode

Build mode implements the pipeline of a pre-registered study, or of a draft for a trial run only
(trial results can never be reported). It never edits the frozen sections: if the pipeline cannot
meet them, stop and tell the human.

1. **Stages** go in `config/base.yaml`, in pipeline order, each with a `name`, a `run` command
   (run from the study directory, with placeholders such as `{in.<input>}`, `{out}`,
   `{params.<key>}`, `{item}`), its `code` files, `inputs` (`stage:<earlier stage>` or
   `dataset:<name>@v<n>`), `params`, and where needed `threads`, `foreach`, `result`. Each other
   variant gets `config/<variant>.yaml` holding only what differs from the base. The keys,
   environments and provenance follow from this; never compute them.
2. **Scripts** go in `scripts/`, in any language the environment has. They take every input and
   output path as an argument and never hard-code a path.
3. **Validation.** Give every stage that another stage consumes (except an external stage) a
   `validate` command: a script that checks its output mechanically (file counts, shapes, missing
   or non-finite values, ranges) and exits non-zero, saying what is wrong, when a check fails. It
   runs after the stage, every time; a failing validation stops the stage before anything is
   sealed. List the script in `code`. External stages are checked by scenario tests instead.
4. **Review.** Ask the human which stages need their own judgment, and give only those a `review`
   question: what a person decides by looking (does this plot look right, does this fit make
   sense), never something `validate` could check, and without hinting at the study's answer. If
   it helps the human look, have the stage write a figure or a short summary for the purpose. A
   reviewed stage waits for the human's verdict before its consumers run and before the study is
   reported.
5. **Tools.** Every stage that a tool computes is a tool stage, `tool: <tool>@<commit>` with the
   commit the human chose (see `th-tool`). Never copy tool code into the study.
6. **Cluster compute** is an external stage (`external:` with `setup`, `slurm` and `keep`): the
   pipeline writes a job folder that the human runs on the cluster. An external stage never fans
   out, and its `code` lies inside the study directory.
7. **Scenario tests** go in `tests/`. Every scenario `S<n>` has at least one test carrying the
   token `th:S<n>`, and no test names a scenario that does not exist.
8. **Check.** Run `tophat check studies/<id>` and `tophat pipeline keys <id>`, and fix what they
   report.
9. **Hand over.** Ask the human to commit the configuration, scripts and tests on the host before
   the run whose results will be reported, naming the files: results computed from uncommitted code
   are dirty and cannot be reported. Then continue with `th-run`.

Never edit the `Snakefile` stub: the rules are generated from `config/`.

## Project Extensions

Read `.tophat/user/skills/th-study/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
