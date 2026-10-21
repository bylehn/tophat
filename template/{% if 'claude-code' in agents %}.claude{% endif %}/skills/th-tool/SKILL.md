---
name: th-tool
description: Declare the project's tools (general-purpose code in its own git repository) and pin stages to exact tool commits. Use when a stage should run a method implementation, simulation wrapper or analysis library instead of study scripts, or when the human wants to change a tool version. Not for changing tool code; that happens in the tool's own repository.
---

<!-- TopHat-managed adapter. The canonical skill lives under .tophat/managed/skills/. -->

Read `.tophat/managed/skills/th-tool/SKILL.md` completely and follow it as the authoritative workflow.

When it says to ask the human, use the AskUserQuestion tool for a choice between options, and ask in
plain text otherwise. Other TopHat skills are invoked as `/th-<name>`, for example `/th-study`.
