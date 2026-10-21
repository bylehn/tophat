---
name: th-question
description: Write, stamp, answer or drop questions in the project's question tree (PROJECT.md). Use when the project has no overarching question yet, when the human wants to add or refine questions, or when a reported study can answer an open question. Not for planning the study that answers a question; use th-study for that.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-question/local.md. -->

# th-question

`PROJECT.md` holds the project's overarching question and its sub-questions. Each question is a
heading: the overarching question at level 2 (`##`), each sub-question one level deeper than its
parent. The line after the heading is its status: `status: open`, `status: answered` or
`status: dropped`. Show the human the current tree with `tophat question tree`.

## Writing questions

The questions belong to the human. Never invent one, and never rephrase one into a different
question.

1. Ask the human for the question in their own words. Help them make it answerable: one question
   per heading, specific enough that a single study (or the answers of its sub-questions) can
   settle it. Suggest a split into sub-questions when a question bundles several, and let the human
   decide.
2. Write each new question as a heading without an ID, followed by `status: open`:

   ```markdown
   ## Does method X rank binders better than docking?
   status: open
   ```

3. Run `tophat question stamp`. It gives every new question its ID (`q-` and 8 hex characters) and
   prints them. Never write an ID yourself.
4. Run `tophat check PROJECT.md` and fix what it reports.

## Answering a question

Answer a question only when the human agrees, and only from evidence:

- a leaf question from one or more `reported` studies that cite it;
- a parent question from the answers of its sub-questions.

Write, below the status line changed to `status: answered`:

```markdown
**Answer:** <what the cited results show, in one or two sentences>
Evidence: <study or question IDs, comma-separated>
```

State plainly when the cited results are not useful or not significant. Never cite anything from
`exploration/`. Run `tophat check PROJECT.md`; it verifies that the evidence exists and is reported.

## Dropping a question

Drop a question only when the human decides to. Change its status to `status: dropped` and add
`**Reason:** <the human's reason>`.

## Project Extensions

Read `.tophat/user/skills/th-question/local.md` and follow its instructions. Where they conflict
with this skill, `local.md` wins.
