---
name: th-data
description: Manage data roots and datasets: configure where data lives, register raw data, declare data of unknown history, publish a study output as a dataset, verify, retire, and trace lineage. Use when a study needs data that is not registered yet, or when the human asks about data. Not for moving or copying data; TopHat never does that.
---

<!-- TopHat-managed adapter. The canonical skill lives under .tophat/managed/skills/. -->

Read `.tophat/managed/skills/th-data/SKILL.md` completely and follow it as the authoritative workflow.

When it says to ask the human, use the AskUserQuestion tool for a choice between options, and ask in
plain text otherwise. Other TopHat skills are invoked as `/th-<name>`, for example `/th-study`.
