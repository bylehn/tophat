---
name: th-quiz
description: Quiz the human on a study's plan before they lock it, so they pre-register a plan they understand (how the decision rule decides, what the endpoint measures, what the study assumes and cannot show). Use when th-study or th-next offers it at hand-over, or when the human asks to be quizzed on a plan. Advice only, never a condition for pre-registration; not for planning or changing the study, use th-study for that.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-quiz/local.md. -->

# th-quiz

A short quiz on a study's plan. Its purpose is that the human locks a plan they understand: what it
tests, how its decision rule decides, what it assumes and what it cannot show. It is advice: it
writes nothing, keeps no record, and is never a condition for pre-registration.

## 1. Read the plan

Read the study's `STUDY.md` and its question in `PROJECT.md` (`tophat question tree`; `tophat
status` shows the study's status). Draw every question from these two, and from nothing else: never
read or mention results, `SUMMARY.md`, provenance records, or the outputs of trial runs.

If the Question, Primary endpoint or Decision rule section is empty, say the plan is incomplete,
ask no question, and point to `th-study`.

## 2. Ask the questions

Ask about five questions, one at a time, and wait for each answer before the next.

- **Three or four multiple-choice questions**, each with four options, exactly one of which follows
  from the plan's text. Together they cover:
  - applying the decision rule to an invented outcome, including a boundary case, such as a value
    exactly at a threshold or an interval that touches it;
  - what the primary endpoint measures, and over which unit (signals, patients, compounds, runs …);
  - what a variant changes, when the study has variants;
  - what the study can conclude when the rule gives "no difference".
- **One or two open questions**, asking the human in their own words for the assumptions the study
  relies on, its limitations, or the population and setting its answer extends to.

Label every invented outcome as invented ("Suppose, as an invented example, …"), and never present
one as an expected or likely result.

If writing a question shows that the plan does not decide it (an outcome the decision rule does not
cover, or wording with two readings), do not ask it: note it as a gap and move on.

## 3. Respond to the answers

- After a wrong multiple-choice answer, explain the right one, quoting the plan's wording that
  decides it. After a right one, confirm it in a line.
- Discuss open answers; never mark them right or wrong. The discussion may name what the plan
  leaves unstated, as a question for the human, never as a fix of your own.

## 4. Finish

End with a short list of the gaps found, or say there were none. For each gap, offer to return to
`th-study`, where the human decides any change: plan mode for a `draft` study, an amendment for a
pre-registered one.

Write nothing: create or change no file, and keep no record of the questions, answers or score.
Never edit `STUDY.md`, and never suggest wording for an endpoint, threshold or decision rule the
human has not given.

When the quiz was offered at hand-over, the hand-over continues afterwards, whatever the answers
were: the human gets the pre-registration command either way.

## Quiz policy

`.tophat/user/skills/th-quiz/local.md` sets when `th-study` and `th-next` offer the quiz at
hand-over:

- `offer` (the default, when `local.md` sets no policy): offer the quiz, and run it only if the
  human accepts;
- `always`: run the quiz without asking first;
- `never`: do not mention the quiz.

## Project Extensions

Read `.tophat/user/skills/th-quiz/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
