---
name: th-run
description: Run a study's pipeline and see it through to a reported result, handing QC verdicts and external (cluster) jobs to the human. Use when a study has stages and the human wants to run it, continue a stopped run, fix a failed run, or report its results. Not for designing the study or writing its stages; use th-study for that.
---

<!-- TopHat-managed adapter. The canonical skill lives under .tophat/managed/skills/. -->

Read `.tophat/managed/skills/th-run/SKILL.md` completely and follow it as the authoritative workflow.

When it says to ask the human, use the AskUserQuestion tool for a choice between options, and ask in
plain text otherwise. Other TopHat skills are invoked as `/th-<name>`, for example `/th-study`.
