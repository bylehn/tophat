---
name: th-tool
description: Declare the project's tools (general-purpose code in its own git repository) and pin stages to exact tool commits. Use when a stage should run a method implementation, simulation wrapper or analysis library instead of study scripts, or when the human wants to change a tool version. Not for changing tool code; that happens in the tool's own repository.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-tool/local.md. -->

# th-tool

A tool is code with its own repository, tests and release process. A study never contains tool
code and never changes it; a stage uses a tool by pinning one exact commit.

## Declaring a tool

Add it to `.tophat/tools.yaml` with the URL the human gives:

```yaml
tools:
  method-x:
    url: https://github.com/example/method-x.git
```

The URL must be cloneable without credentials: a public `https://` repository, or a local path.
The dev container holds no credentials, so private repositories cannot be fetched; tell the human
so if they give one.

A local repository (an absolute path or a `file://` URL) is mounted read-only into the dev
container at the same path when the container starts. After declaring one, ask the human to exit
the dev container and launch it again with `./th.sh` before running `tophat tools sync`;
restarting only the agent is not enough, because the mount is made by the launcher. A local repository exists only on this machine, so
others who clone the project cannot fetch it; suggest a public repository for a tool that will be
shared.

## Pinning a stage

A tool stage names the tool and a commit: `tool: method-x@3f9a1c2` (7 to 40 hex characters of a
commit). Use the commit the human chooses, never a branch or tag name. In `run`, `{tool}` is the
checked-out tool. The tool's own `pixi.toml` and `pixi.lock` at that commit provide the
environment; when it has none, add `envs/<tool>/pixi.toml` and `envs/<tool>/pixi.lock` to the
project.

## Fetching

Fetching tool code is the human's step. After a new pin, or on a machine whose cache lacks a pinned
commit, give the human the command `tophat tools sync` and wait for them to run it. Inspect the
pins, their full commits and where they are used with `tophat tools list`.

Never edit a cached checkout under `.tophat/tools/` or `.tophat/tools.lock`. When a tool needs a
change, it is made and committed in the tool's repository, and then the new commit is pinned.

## Project Extensions

Read `.tophat/user/skills/th-tool/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
