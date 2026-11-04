---
name: th-tool
description: Declare the project's tools (general-purpose code in its own git repository) and pin stages to exact tool commits. Use when a stage should run a method implementation, simulation wrapper or analysis library instead of study scripts, or when the human wants to change a tool version. Not for changing tool code; that happens in the tool's own repository.
---

<!-- TopHat-managed adapter. The canonical skill lives under .tophat/managed/skills/. -->

Read `.tophat/managed/skills/th-tool/SKILL.md` completely and follow it as the authoritative workflow.

When it says to ask the human, use Codex's structured user-input mechanism, or ask a concise textual
question when only clarification is needed. Other TopHat skills are invoked as `$th-<name>`, for
example `$th-tool`.
