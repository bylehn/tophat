---
name: th-next
description: Find out where the project stands and take its next step. Use at the start of a session, when the human asks what to do next, or when unsure which TopHat skill applies. Not for a task the human has already described; use the skill for that task instead.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-next/local.md. -->

# th-next

When the human asks how a study works, or why a step exists, point them to
`.tophat/managed/GUIDE.md`.

## 1. Read the status

Run `tophat status --json`. For each study it gives `next`: the `row` of the next-action table,
`who` may take it (`you` means only the human), its `text`, and the `command` to run.

- If the question tree is empty and there are no studies, this is a new project: see *A new
  project* below.
- If the human named a study, take that study. Otherwise take the study whose next action is
  furthest along (the highest row below 15); if several are equally far, ask the human which one.
- If every study's row is 15 (done) and no question is open, say so and stop.

## 2. Act on the row

| Row | What it means | What you do |
|---|---|---|
| 1 | The study is superseded | Nothing; say which study replaced it |
| 2 | `tophat check` reports errors in the study's scope | Run `tophat check studies/<id>` and fix the errors with the skill that owns the file: `th-study` for a study, `th-data` for a dataset, `th-tool` for tools, `th-question` for `PROJECT.md` |
| 3 | The draft's plan is incomplete | `th-study`, plan mode |
| 4 | The plan is ready for pre-registration | Offer `th-quiz` as `th-study`'s hand-over does (following the quiz policy in `.tophat/user/skills/th-quiz/local.md`), then give the human `tophat study preregister <id>` and stop |
| 5 | The study has no stages | `th-study`, build mode |
| 6 | The configured `results` location does not exist | Tell the human; ask whether to create that directory or to change the location in `.tophat/roots.local.yaml` (see `th-data`) |
| 7 | Tool code is not on this machine | Give the human `tophat tools sync` and stop |
| 8 | An external job is waiting | `th-run`, external hand-off |
| 9 | A QC verdict is needed | Give the human the `tophat stage qc` command from `command` and stop |
| 10 to 13 | A failed QC, an unrun or dirty pipeline, a result to report | `th-run` |
| 14 | The study is reported and its question is still open | `th-question`, answering |

Never carry out a next action whose `who` is `you`: give the human its `command` and stop.

## A new project

Explain, in a few sentences, how a study goes: **explore** the data until you know what to
measure; **plan** the study (the question, the one number that answers it, and how that number
decides); **lock** the plan before the run that answers the question (`tophat study preregister`,
done by the human); **run** the pipeline; **report** the result. Point to
`.tophat/managed/GUIDE.md` for the details. Then offer two starts, and let the human choose:

- **A practice run on the worked example**, to see the whole path once on a study that is known to
  work. Add a practice question to `PROJECT.md` (for example "Practice: does a median filter
  recover noisy step signals better than a mean filter?") with `status: open`, stamp it with
  `tophat question stamp`, copy `.tophat/managed/examples/median-vs-mean/` to
  `studies/median-vs-mean/`, set its `question:` to the stamped ID, and follow its `README.md`,
  handing each human step to the human as it comes. Say at the start that the practice question and
  study can be removed afterwards (drop the question with the reason "practice"; delete the study
  directory and its outputs under `results/`).
- **Their own project**: use `th-question` to write the overarching question.

## 3. Finish

After acting, run `tophat status` again and tell the human the new next action in one line.

## Project Extensions

Read `.tophat/user/skills/th-next/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
