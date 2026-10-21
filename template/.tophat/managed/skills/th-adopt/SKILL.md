---
name: th-adopt
description: Bring an existing research repository into TopHat, labelling past work honestly. Use when TopHat has just been applied to a repository that already holds code, data, environments or analyses, or when the human asks how to adopt one. Not for planning new studies in a project that is already set up.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-adopt/local.md. -->

# th-adopt

Adoption maps existing content onto TopHat without rejecting it or presenting it as verified. The
human approves every step; never delete, move or rewrite a user file without their approval.

## 1. Inventory

List what the repository holds: reusable code, raw data, processed data, environment files
(`environment.yml`, `requirements.txt`, `Project.toml`, `pixi.toml`), pipelines (`Snakefile`,
`Makefile`, scripts), notebooks, and past analyses with results. Show the list to the human.

## 2. Choose a path

Recommend one, and let the human choose:

- **In place**: TopHat applied to this repository, on a branch. Best when the repository is mostly
  analyses.
- **Wrap**: a fresh TopHat project that pins this repository as a tool (see `th-tool`) and registers
  its data. Best when the repository is mostly reusable code.

## 3. Map, item by item

Propose, and apply only what the human approves:

- reusable code → a tool (`th-tool`);
- raw data with a known source → `tophat data add` (`th-data`);
- processed data of unknown history → `tophat data declare`, with the human's statement;
- past analyses → `retrospective` studies, written up with `th-study`; they are labelled
  retrospective wherever they are cited;
- environments → dependencies in `pixi.toml`;
- pipelines → stage configurations, where practical.

Content not mapped stays as it is and is listed as legacy in a short note for the human.

## 4. Reconcile the skeleton

When the template met existing files, it kept them. With the human's approval:

- add `python` and `snakemake-minimal` (channels `conda-forge` and `bioconda`) to an existing
  `pixi.toml`;
- add `/.snakemake/` and `/.pixi/` to an existing `.gitignore`;
- add the line pointing at `.tophat/managed/AGENTS.md` to an existing `AGENTS.md`, and, when the
  project's agent setup uses one, the import lines to its agent entry file.

Finish with `tophat check`; declared datasets and retrospective studies appear as legacy items,
which is expected.

## Project Extensions

Read `.tophat/user/skills/th-adopt/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
