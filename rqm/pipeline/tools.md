# Feature: Tool Stages <!-- rq-c989449c -->

A **tool** is general-purpose code that lives in its own git repository, with its own requirements,
tests and release process: a method implementation, a simulation wrapper, an analysis library. A
study never contains tool code and never changes it; a stage uses a tool by pinning one exact commit.
This file specifies how tools are declared, pinned, fetched into a local cache, and run in their own
environments, and the rules `tophat check` applies to them. Stage configuration, keys and provenance
are specified in `rqm/pipeline/provenance.md`; rule generation in `rqm/pipeline/snakemake.md`.

## Declaring Tools <!-- rq-da50d924 -->

`.tophat/tools.yaml` (committed) names the project's tools and where their repositories are:

```yaml
tools:
  method-x:
    url: https://github.com/example/method-x.git
  docking-wrapper:
    url: https://gitlab.example.org/group/docking-wrapper.git
```

- Tool names match the study ID pattern and are 3 to 64 characters.
- `url` is a repository that can be cloned without credentials: an `https://` URL of a public
  repository, or an absolute path or `file://` URL of a local repository. SSH locations
  (`ssh://…`, `<user>@<host>:<path>`) and URLs with embedded user information
  (`https://<user>[:<secret>]@…`) are configuration errors: the dev container holds no credentials
  (see `rqm/template/container.md`), and TopHat stores none.

## Pinning a Tool in a Stage <!-- rq-cfae36f4 -->

A stage uses a tool with `tool: <name>@<commit>`:

```yaml
  - name: score
    tool: method-x@3f9a1c2
    run: x-score --level {params.level} {item} {out}
    inputs: {structures: stage:prepare}
    foreach: structures
    params: {level: b3lyp}
    env: default
```

- `<commit>` is 7 to 40 lowercase hexadecimal characters: a full commit hash or a prefix of one.
- Branch and tag names are not accepted: a pin never moves.
- Variants may pin a different commit of the same tool, so two variants can compare tool versions
  side by side.

### The tools lock <!-- rq-ab9a813f -->

`.tophat/tools.lock` (committed) records the full commit each pinned prefix resolved to:

```yaml
method-x:
  3f9a1c2: 3f9a1c2b4d5e6f708192a3b4c5d6e7f801234567
```

- `tophat tools sync` writes it; nobody edits it by hand.
- A full 40-character pin needs no lock entry.
- A stage's **resolved commit** is the full pin, or the lock entry for its prefix.
- Once recorded, a prefix's resolution never changes. If the prefix later matches more than one
  commit in the tool repository, `tophat tools sync` reports it as ambiguous and leaves the lock as
  it is.

## The Tool Cache <!-- rq-c5b9e99a -->

Tool code is checked out in a per-machine, git-ignored cache at
`.tophat/tools/<name>/<full commit>/`, one directory per pinned commit, so several versions of a
tool can be used side by side.

- A cached checkout is **pristine** when it is exactly the commit: no modified, added or removed
  files. Running or sealing a stage whose checkout is not pristine is refused.
- Deleting the cache is always safe; `tophat tools sync` recreates it.
- `tophat run`, `tophat pipeline rules` and `tophat stage begin` never fetch anything. If a pinned
  commit is not cached they fail with a message telling the user to run `tophat tools sync`.

## Tool Environments <!-- rq-a96b0574 -->

A tool stage runs in a pixi environment that belongs to the tool, not to the project:

1. If the tool's repository has `pixi.toml` and `pixi.lock` at its root (at the pinned commit),
   those are used.
2. Otherwise the project supplies them as committed files `envs/<tool name>/pixi.toml` and
   `envs/<tool name>/pixi.lock`.
3. Otherwise the stage has no environment, which is a configuration error.

`env` names the environment within that manifest (default `default`). The stage's command runs from
the study directory as `pixi run --frozen --manifest-path <manifest> -e <env> -- sh -c <command>`.

## Running a Tool Stage <!-- rq-f9ae0061 -->

- The placeholder `{tool}` is the path of the tool's cached checkout. It is available only in tool
  stages. Data shipped inside the tool repository is read through `{tool}`, so the commit fixes it.
- `code` may still list study files (e.g. a small adapter script); the tool itself is fixed by its
  commit and is not listed in `code`.

## Keys and Provenance <!-- rq-37a6119e -->

For a tool stage:

- The key text (see `rqm/pipeline/provenance.md`) gains the line
  `tool <name>@<resolved commit>`, placed directly after the `run` line. Stages without a tool have
  no `tool` line, so their keys are unaffected by this feature.
- The `env` line hashes the tool environment's `pixi.lock` (the tool's own, or the project-supplied
  one), using the same construction as project environments.
- The provenance record gains `"tool": {"name", "commit": <resolved commit>, "url"}`.
- Cleanliness additionally covers `.tophat/tools.yaml`, `.tophat/tools.lock`, and the
  project-supplied `envs/<tool>/` files when they are used. The tool's own code is fixed by its
  commit and needs no cleanliness check, but its checkout must be pristine.

## Feature API <!-- rq-a7c6edbc -->

### Commands <!-- rq-9d5b8ddf -->

- `tophat tools sync` <!-- rq-052f542d -->
  - Run by a human: it is the only command through which `tophat` reaches the network, and agents
    never run it (see `rqm/template/skills.md`).
  - Runs git with interactive credential prompts disabled, so a repository that needs credentials
    fails at once for that pin, saying the repository is not publicly reachable, rather than waiting
    for input.
  - Collects every tool pin in every study's configuration (all variants).
  - For each tool, fetches its repository into the cache's mirror (`.tophat/tools/<name>/.git`) when
    a pinned commit is unknown locally.
  - Resolves each prefix pin that has no lock entry, and writes `.tophat/tools.lock` atomically.
  - Creates a pristine checkout for each resolved commit that is not cached.
  - Prints one line per pin: `<name>@<pin> -> <full commit> (cached|fetched|failed: <reason>)`.
  - Exits 0 if every pin resolved and is cached, 1 otherwise. Successful pins are kept even when
    others fail.
- `tophat tools list [--json]` <!-- rq-4ebf0587 -->
  - Read-only. Lists each declared tool, its URL, and every pin in use with its resolved commit,
    whether it is cached and pristine, and which studies, variants and stages use it.

### Types <!-- rq-d49687d5 -->

- `ToolPin` — `name: String`, `pin: String` (as written). <!-- rq-d7bdbaae -->
- `ToolsError` — error type of the tool commands, with at minimum: <!-- rq-3c1af8c0 -->
  - `UnknownTool(String)`, `InvalidPin(String)`.
  - `Ambiguous { tool: String, pin: String }`, `NotFound { tool: String, pin: String }`.
  - `Fetch { tool: String, message: String }`.
  - `NotCached { tool: String, commit: String }`, `NotPristine { tool: String, commit: String }`.
  - `Io(String)`.

### Functions <!-- rq-149b947a -->

- `resolve_pin(root: &Path, pin: &ToolPin) -> Result<String, ToolsError>` — the resolved commit, <!-- rq-ccd30a25 -->
  from the pin itself or the lock, without network access.
- `tool_checkout(root: &Path, pin: &ToolPin) -> Result<PathBuf, ToolsError>` — the cached, pristine <!-- rq-f8136bd3 -->
  checkout of a pin, or why it cannot be used.
- `sync(root: &Path) -> Result<SyncReport, ToolsError>` — performs `tophat tools sync`. <!-- rq-097c7d03 -->

## Check Rules (`pipeline` family, tools) <!-- rq-85215682 -->

Diagnostics are reported on the configuration file at the stage's line, or on `.tophat/tools.yaml`.

| Rule | Severity | Condition |
|---|---|---|
| `pipeline.tools-invalid` | error | `.tophat/tools.yaml` or `.tophat/tools.lock` cannot be parsed, a tool name is malformed, a tool has no `url`, a `url` needs credentials (SSH form or embedded user information), or a lock entry is not a full commit extending its prefix |
| `pipeline.tool-unknown` | error | A stage pins a tool that `.tophat/tools.yaml` does not declare |
| `pipeline.tool-pin-invalid` | error | A pin is not `<name>@<7 to 40 lowercase hex characters>` |
| `pipeline.tool-unlocked` | error | A prefix pin has no entry in `.tophat/tools.lock` |
| `pipeline.tool-not-cached` | warning | A pinned commit is not in this machine's cache |
| `pipeline.tool-not-pristine` | error | A cached checkout of a pinned commit has local modifications |
| `pipeline.env-missing` | error | A cached tool has no environment (neither its own lock nor `envs/<tool>/`) |

## Assumptions <!-- rq-71385ea7 -->

- `git` and network access are available wherever `tophat tools sync` runs: inside the dev container,
  which holds no credentials.
- `.tophat/tools/` is git-ignored in every TopHat project; the project template lists it.

## Out of Scope <!-- rq-6ed3a5ea -->

- Developing a tool from inside a study, or using uncommitted tool changes.
- Private tool repositories, and any credentials for fetching tools.
- Tools that are not git repositories.
- Building or compiling tools; a tool's environment (or a step in its own `pixi.toml`) provides
  whatever it needs.

## Gherkin Scenarios <!-- rq-c7516f9a -->

```gherkin
Feature: Tool stages

  Background:
    Given a TopHat project in a git repository
    And ".tophat/tools.yaml" declares tool "method-x" at a local fixture repository
    And the fixture repository has commits A and B, each with its own pixi.toml and pixi.lock
    And study "x-vs-docking" has a stage "score" with "tool: method-x@<first 7 characters of A>"

  # --- Sync and pinning ---

  @rq-08c87cc4
  Scenario: Sync resolves a prefix and caches the checkout
    When "tophat tools sync" is run
    Then ".tophat/tools.lock" maps the prefix of A to the full commit A
    And ".tophat/tools/method-x/<A>/" is a checkout of A
    And the output line for "method-x" ends with "(fetched)"
    And the exit code is 0

  @rq-e2b68626
  Scenario: Sync is idempotent
    Given "tophat tools sync" has run
    When it is run again
    Then ".tophat/tools.lock" is unchanged
    And the output line for "method-x" ends with "(cached)"

  @rq-bd5b27af
  Scenario: A full pin needs no lock entry
    Given stage "score" has "tool: method-x@<full A>"
    When "tophat tools sync" is run
    Then ".tophat/tools.lock" has no entry for that pin
    And the checkout of A is cached

  @rq-f1e21338
  Scenario: Variants pin different versions side by side
    Given variant "new-x" pins "method-x@<prefix of B>" for stage "score"
    When "tophat tools sync" is run
    Then checkouts of A and B are both cached
    And stage "score" has different keys in variants "base" and "new-x"

  @rq-4f2cf51c
  Scenario: A recorded resolution never changes
    Given ".tophat/tools.lock" maps the prefix of A to A
    And the fixture repository gains a commit whose hash shares that prefix
    When "tophat tools sync" is run
    Then the output reports the prefix as ambiguous
    And ".tophat/tools.lock" still maps it to A

  @rq-77e1e92c
  Scenario: An unknown commit fails sync for that pin only
    Given variant "broken" pins "method-x@deadbee", which no commit matches
    When "tophat tools sync" is run
    Then the line for "deadbee" ends with "(failed: …)"
    And A is still resolved and cached
    And the exit code is 1

  @rq-582d60cc
  Scenario: Locations that need credentials are rejected
    Given ".tophat/tools.yaml" declares tool "method-x" with url "git@github.com:example/method-x.git"
    When "tophat check" is run
    Then an error "pipeline.tools-invalid" naming "method-x" and credentials is reported

  @rq-d4af23a7
  Scenario: URLs with embedded credentials are rejected
    Given ".tophat/tools.yaml" declares tool "method-x" with url "https://user:token@github.com/example/method-x.git"
    When "tophat check" is run
    Then an error "pipeline.tools-invalid" naming "method-x" is reported

  @rq-940c97c9
  Scenario: A repository that needs credentials fails sync without prompting
    Given tool "method-x" is declared with an https url whose server asks for credentials
    When "tophat tools sync" is run with no terminal input available
    Then it finishes without waiting for input
    And its line for "method-x@<pin>" reports "failed:" with a reason saying the repository is not publicly reachable
    And the exit code is 1

  @rq-bf589d85
  Scenario: Branch names are not pins
    Given stage "score" has "tool: method-x@main"
    When "tophat check" is run
    Then an error "pipeline.tool-pin-invalid" is reported

  @rq-5aaa8478
  Scenario: Undeclared tool
    Given stage "score" has "tool: other-tool@3f9a1c2"
    When "tophat check" is run
    Then an error "pipeline.tool-unknown" is reported

  @rq-648eee87
  Scenario: Prefix without a lock entry
    Given "tophat tools sync" has not been run
    When "tophat check" is run
    Then an error "pipeline.tool-unlocked" is reported at stage "score"

  @rq-692c1c23
  Scenario: Not cached on this machine is a warning
    Given ".tophat/tools.lock" maps the prefix to A
    And ".tophat/tools/" does not exist
    When "tophat check" is run
    Then a warning "pipeline.tool-not-cached" is reported
    And no error about the tool is reported

  @rq-df6d227b
  Scenario: A modified checkout is refused
    Given the checkout of A has a modified file
    When "tophat check" is run
    Then an error "pipeline.tool-not-pristine" is reported
    And "tophat stage begin" for stage "score" fails with NotPristine

  # --- Running ---

  @rq-7f815511
  Scenario: Generation refuses an uncached tool
    Given ".tophat/tools.lock" maps the prefix to A but the checkout is not cached
    When "tophat pipeline rules x-vs-docking --output rules.smk" is run
    Then the command fails telling the user to run "tophat tools sync"
    And "rules.smk" is not written

  @rq-175f7439
  Scenario: The tool runs in its own environment
    Given "tophat tools sync" has run
    When "tophat stage begin" runs for an item of stage "score"
    Then the returned environment manifest is "<cache>/method-x/<A>/pixi.toml"
    And the returned command has "{tool}" replaced by the checkout path

  @rq-80cee75b
  Scenario: Project-supplied environment for a tool without one
    Given the fixture commit C has no pixi.toml or pixi.lock
    And the project has "envs/method-x/pixi.toml" and "envs/method-x/pixi.lock"
    And stage "score" pins C
    When the stage key is computed
    Then its "env" line hashes "envs/method-x/pixi.lock"

  @rq-d7d4093c
  Scenario: A tool without any environment
    Given stage "score" pins commit C, which has no environment, and "envs/method-x/" does not exist
    When "tophat check" is run
    Then an error "pipeline.env-missing" is reported

  @rq-f8fa9801
  Scenario: The {tool} placeholder is only for tool stages
    Given stage "prepare" has no tool and its run uses "{tool}"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "{tool}" is reported

  # --- Keys and provenance ---

  @rq-71e65d00
  Scenario: The key records the resolved commit
    When the key text of stage "score" is computed
    Then the line directly after its "run" line is "tool method-x@<full A>"
    And stages without a tool have no "tool" line

  @rq-3b6d26cd
  Scenario: Existing keys are unchanged by this feature
    Given a stage without a tool and its key computed before tools were configured
    When ".tophat/tools.yaml" is added
    Then that stage's key is unchanged

  @rq-27bb0a4e
  Scenario: Changing the pinned commit re-runs the stage and downstream
    Given the keys of all stages are recorded
    When stage "score" pins B instead of A and "tophat tools sync" is run
    Then the keys of "score" and every downstream stage change
    And the key of "prepare" is unchanged

  @rq-a06c8348
  Scenario: Provenance records the tool
    Given stage "score" was run and sealed with tool A
    Then its provenance record has "tool" {"name": "method-x", "commit": "<full A>", "url": "<fixture url>"}

  @rq-59a106f1
  Scenario: An uncommitted lock makes the record unclean
    Given ".tophat/tools.lock" has uncommitted changes
    When stage "score" is sealed
    Then "clean" is false and "unclean_files" contains ".tophat/tools.lock"

  # --- Listing ---

  @rq-533b4dc3
  Scenario: List tools and their use
    Given "tophat tools sync" has run
    When "tophat tools list" is run
    Then the output names "method-x", its URL, the pin, commit A, "cached", and "x-vs-docking/base/score"
```
