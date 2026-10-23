# Feature: Stage Configuration, Keys and Provenance <!-- rq-de005c10 -->

A study's pipeline is a sequence of **stages** declared in its configuration files. Each stage has a
**stage key**: a fingerprint of everything that determines its output. Outputs live in a directory
named by the key, so changing anything that matters produces a new directory and never overwrites
an old result. When a stage finishes, `tophat stage seal` hashes its outputs and writes a
**provenance record** beside them. This file specifies the stage configuration schema, key
derivation, output locations, provenance records, and the `pipeline` rules evaluated by
`tophat check` for configuration and keys. Snakemake rule generation, fan-out execution, QC gates
and the summary are specified in `rqm/pipeline/snakemake.md`; datasets and data roots in
`rqm/data.md`.

## Stage Configuration <!-- rq-2f3eea8e -->

### Files <!-- rq-bb4108e3 -->

- `studies/<study>/config/base.yaml` holds the `stages` list.
- `studies/<study>/config/<variant>.yaml` holds only what a variant changes (see *Variants*).

### Stage fields <!-- rq-081e737a -->

```yaml
stages:
  - name: prepare
    run: julia --project=. scripts/prepare.jl {in.raw} {out}
    code: [scripts/prepare.jl]
    inputs:
      raw: dataset:benchmark-set@v2
    params: {cutoff: 2.0}
    validate: julia --project=. scripts/validate_prepare.jl {out}
  - name: score
    run: python scripts/score.py {item} {out} --level {params.level}
    code: [scripts/score.py, scripts/common/]
    inputs:
      structures: stage:prepare
    foreach: structures
    params: {level: b3lyp}
    env: chem
    threads: 4
  - name: analyse
    run: Rscript scripts/analyse.R {in.scores} {out}
    code: [scripts/analyse.R]
    inputs:
      scores: stage:score
    result: result.txt
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Matches the study ID pattern; unique within the study |
| `run` | yes | Shell command template (see *Placeholders*), run from the study directory |
| `code` | no | Files or directories, relative to the study directory, whose contents determine the stage's behaviour; each must exist inside the project |
| `inputs` | no | Mapping from input name (study ID pattern) to `stage:<earlier stage>` or `dataset:<name>@v<n>` |
| `params` | no | Mapping of parameters; values are YAML scalars, lists or mappings |
| `env` | no | pixi environment name; default `default` |
| `threads` | no | Positive integer passed to Snakemake; default 1 |
| `foreach` | no | Name of an input; run the stage once per item of that input (see `rqm/pipeline/snakemake.md`) |
| `validate` | no | Command, with the placeholders of `run`, that checks the stage's output after `run` and exits non-zero when it is wrong; the job then fails and nothing is sealed (see `rqm/pipeline/snakemake.md`). Its scripts belong in `code` |
| `review` | no | The question a human answers by looking at the stage's output; it gives the stage a QC gate (see `rqm/pipeline/snakemake.md`). It does not enter the stage key |
| `result` | no | File, relative to the output directory, whose single line is the stage's result line |
| `tool` | no | `<tool>@<commit>`: run the stage with a pinned tool (see `rqm/pipeline/tools.md`) |
| `external` | no | Run the stage on a remote machine (see `rqm/pipeline/external.md`) |

- `stage:` inputs name a stage that appears **earlier** in the list, so the pipeline has no cycles.
- `foreach` and `result` are not allowed on the same stage.
- `review` only decides whether the stage is gated (see `rqm/pipeline/snakemake.md`), so adding,
  removing or rewording it never re-runs anything. `validate` decides what is sealed, so it is part
  of the key.
- `tool` makes the stage a tool stage (see `rqm/pipeline/tools.md`) and `external` makes it an
  external stage run on a remote machine (see `rqm/pipeline/external.md`); a stage cannot have both.
- Unknown keys are configuration errors.

### Variants <!-- rq-a17aa884 -->

A variant file has the form `stages: [ { name: <existing stage>, <fields to change> } ]`. For each
entry, mappings (`params`, `inputs`) are merged key by key into the base stage and every other field
replaces the base value. A variant cannot add, remove or reorder stages. The `base` variant is
`config/base.yaml` unchanged.

### Placeholders <!-- rq-6891672b -->

`run` may contain:

| Placeholder | Value |
|---|---|
| `{out}` | The stage's output directory (for fan-out: the item's output directory) |
| `{in.<input>}` | The input's directory: the upstream stage's output directory, or the dataset directory in its root |
| `{item}` | Fan-out only: path of the current item file |
| `{item_name}` | Fan-out only: name of the current item |
| `{params.<key>}` | A scalar parameter value |
| `{params_file}` | Path of a JSON file holding all parameters |
| `{threads}` | The stage's thread count |

Substituted values are shell-quoted. Any other `{…}` in `run` is a configuration error; a literal
brace is written `{{` or `}}`.

## Stage Keys <!-- rq-4aa0232e -->

A stage key is `sk-` followed by the first 16 lowercase hex characters of the SHA-256 of the stage's
**key text**: these lines, joined with LF and ending with LF.

1. `tophat-stage-key/1`
2. `run <run template exactly as configured>`
3. Stages with `validate` only: `validate <validate template exactly as configured>`
4. Tool stages only: `tool <tool>@<resolved commit>` (see `rqm/pipeline/tools.md`)
5. `env <environment hash>` (see below), or `env external` for an external stage
6. External stages only: `external <canonical JSON>` (see `rqm/pipeline/external.md`)
7. `threads <n>`
8. `foreach <input name>` if fan-out, otherwise `foreach -`
9. `result <file>` if set, otherwise `result -`
10. One line `code <path> <sha256>` per file reached through `code` (directories expanded to all
   files recursively), sorted by path, with paths relative to the study directory and `/`-separated
11. `params <canonical JSON>`: the merged parameters, serialized with sorted keys and no whitespace
12. One line per input, sorted by input name: `input <name> stage <upstream key>` or
   `input <name> dataset <name>@v<n> <content_hash>`

The study ID, variant name and `review` question are not part of the key, so two variants with
identical effective settings for a stage share one output.

### Environment hash <!-- rq-3ac5de89 -->

Lines 3, 4 and 6 appear only for stages with `validate`, tool stages and external stages, so other
stages have exactly the key text they would have without those features.

The environment hash is `sha256:` followed by the SHA-256 of the sorted list of package entries
recorded in the project's `pixi.lock` (for a tool stage, the tool environment's `pixi.lock`) for the
stage's environment, across all platforms, one entry
per line. It does not depend on the machine computing it, so keys agree across platforms. A missing
lock file or environment is a configuration error for stages that use it.

### Output location <!-- rq-a56c1f8f -->

A stage's output directory is `<results root>/<study>/<stage>/<key>/`, where `<results root>` is the
location of the `results` data root (see `rqm/data.md`). Fan-out item outputs are in
`<output directory>/items/<item name>/`. Outputs of a trial run (see `rqm/workflow.md`) use
`<results root>/<study>/.trial/<stage>/<key>/` instead; the key is the same, the location is not.

## Provenance Records <!-- rq-533606d2 -->

`tophat stage seal` writes `provenance.json` in the output directory (and, for a fan-out item, in the
item directory). It is written last and atomically; its presence marks the output as complete.

```json
{
  "format_version": 1,
  "tophat_version": "0.1.0",
  "study": "x-vs-docking",
  "stage": "score",
  "key": "sk-3f9a1c2b4d5e6f70",
  "item": "1abc",
  "run": "python scripts/score.py '/res/x-vs-docking/prepare/sk-…/1abc.pdb' '/res/…/items/1abc' --level 'b3lyp'",
  "env": {"name": "chem", "hash": "sha256:…"},
  "code": [{"path": "scripts/score.py", "sha256": "…"}],
  "params": {"level": "b3lyp"},
  "inputs": [{"name": "structures", "kind": "stage", "stage": "prepare", "key": "sk-…"}],
  "outputs": [{"path": "score.json", "size": 812, "sha256": "…"}],
  "items": null,
  "clean": true,
  "unclean_files": [],
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "host": "laptop",
  "platform": "linux-64",
  "trial": false,
  "sealed": "2026-10-20T10:31:07Z"
}
```

- `run` is the command as executed, with placeholders substituted.
- `inputs` records dataset inputs as `{"name", "kind": "dataset", "ref": "<name>@v<n>",
  "content_hash"}`.
- `outputs` lists every regular file below the directory, recursively, except `provenance.json`
  and, for a fan-out stage record, the `items/` subtree. Paths are relative and `/`-separated.
- `items` is `null` except in the stage-level record of a fan-out stage, where it lists
  `{"name", "key": <stage key>, "outputs_hash"}` per item, `outputs_hash` being the
  `content_hash` (see `rqm/data.md`) of that item's outputs.
- `commit` is the project's `HEAD` commit, or `null` outside a git repository.
- `platform` is the pixi platform name of the machine that sealed the record.

### Clean <!-- rq-9f3a6366 -->

A record is **clean** when every file whose contents enter the key or configure the stage is
committed unchanged at `commit`: the stage's `code` files, the study's `config/base.yaml`, the
variant's config file if any, `pixi.toml` and `pixi.lock`. `unclean_files` lists those that are
untracked or have uncommitted changes. Outside a git repository, or when `git` is unavailable,
`clean` is `false` and `unclean_files` holds the single entry `"<no git>"`.

## Feature API <!-- rq-fc3ff911 -->

### Commands <!-- rq-9057ff4f -->

- `tophat stage seal <study> <stage> --key <key> [--variant <variant>] [--item <item name>] [--trial]` <!-- rq-719acfdc -->
  - Recomputes the stage key for the variant (default `base`) and fails if it differs from
    `--key` (the configuration or code changed while the stage ran).
  - Requires the output directory (or item directory) to contain at least one file besides
    `provenance.json`. For a stage with `result`, requires that file to exist and contain exactly
    one non-empty line.
  - For a fan-out stage without `--item`, requires every item to be sealed and writes the
    stage-level record with `items`.
  - Hashes the outputs, determines `clean`, writes `provenance.json`, and updates the hash cache.
  - With `--trial`, seals the trial output location and records `"trial": true`.
  - Used by generated rules; not normally run by hand.
- `tophat pipeline keys <study> [--variant <variant>] [--json]` <!-- rq-c8637010 -->
  - Prints, for each stage in order, its name, key, output directory, and whether a sealed
    non-trial output exists for that key (`sealed`, `missing`, or `unavailable` when the results
    root is unconfigured). Read-only.

### Types <!-- rq-01092d90 -->

- `StageConfig` — one stage after variant merging: name, run, code, inputs, params, env, threads, <!-- rq-9950b389 -->
  foreach, validate, review, result, tool, external, and the 1-based line of the stage in its
  configuration file.
- `StageInput` — `Stage(String)` or `Dataset(DatasetRef)`. <!-- rq-5cd91e7d -->
- `PipelineConfig` — the stages of a study for one variant. <!-- rq-cbd35529 -->
- `ProvenanceRecord` — a parsed `provenance.json`. <!-- rq-9f8a5a5e -->
- `PipelineError` — error type of the pipeline commands, with at minimum: <!-- rq-297a1f84 -->
  - `Config { path: String, line: Option<u32>, message: String }`.
  - `UnknownVariant(String)`, `UnknownStage(String)`.
  - `KeyMismatch { expected: String, actual: String }`.
  - `NoOutputs(String)`, `BadResult(String)`, `ItemsUnsealed(Vec<String>)`.
  - `UnconfiguredRoot(String)`, `Io(String)`.

### Functions <!-- rq-a2a20f9a -->

- `load_pipeline(root: &Path, study: &str, variant: &str) -> Result<PipelineConfig, PipelineError>` <!-- rq-f8dfe570 -->
  — reads and merges the configuration and validates it.
- `stage_key_text(config: &PipelineConfig, stage: &str, ctx: &KeyContext) -> Result<String, PipelineError>` <!-- rq-f40ffa2b -->
  — the key text of a stage; `KeyContext` supplies file hashing, the environment hash and dataset
  content hashes.
- `stage_key(text: &str) -> String` — `sk-` and the first 16 hex characters of the text's SHA-256. <!-- rq-e6252079 -->
- `pipeline_rules(project: &Project) -> Vec<Diagnostic>` — the `pipeline` rule family registered <!-- rq-55bef4eb -->
  in `RULE_FAMILIES`.

## Check Rules (`pipeline` family, configuration) <!-- rq-22c3d944 -->

Diagnostics are reported on the configuration file, at the offending stage's line where possible.
A study with no `stages` (or an empty list) has no pipeline and no `pipeline` diagnostics.

| Rule | Severity | Condition |
|---|---|---|
| `pipeline.config-invalid` | error | A configuration file cannot be parsed, a stage is malformed (missing `name` or `run`, unknown or key, `tool` together with `external`, duplicate name, bad `threads`, unknown placeholder, `foreach` naming no input, `foreach` with `result`, an empty `review`, `validate` on an external stage, or one of the removed QC fields `qc`, `qc_reason` and `check`), an input references a later or unknown stage, or a variant names an unknown stage or adds stages |
| `pipeline.validate-missing` | warning | A stage that another stage consumes, other than an external stage, has no `validate`, so nothing checks its output before it is used |
| `pipeline.code-missing` | error | A `code` entry does not exist or lies outside the project |
| `pipeline.env-missing` | error | `pixi.lock` is missing or lacks the stage's environment |

## Assumptions <!-- rq-3d588765 -->

- `git` is available on the machine that seals stages; it is used read-only to find `HEAD` and the
  status of the files that determine cleanliness.
- Upgrading an environment changes the keys of every stage that uses it, so those stages re-run.
- Keys do not depend on the platform: a stage run on Linux and checked on macOS has the same key.
  The platform that produced an output is recorded in its provenance.

## Out of Scope <!-- rq-d1d175cd -->

- Snakemake rule generation, fan-out execution, QC gates and summaries (`rqm/pipeline/snakemake.md`).
- Deleting old output directories.

## Gherkin Scenarios <!-- rq-80f63e5c -->

```gherkin
Feature: Stage configuration, keys and provenance

  Background:
    Given a TopHat project in a git repository with a pixi.lock containing environments "default" and "chem"
    And root "results" is configured
    And study "x-vs-docking" has the stages "prepare" (with validate), "score" (fan-out) and "analyse" (result) from this specification
    And all its files are committed

  # --- Configuration ---

  @rq-bbe1941a
  Scenario: A valid configuration has no pipeline diagnostics
    When "tophat check" is run
    Then no "pipeline" diagnostic is reported

  @rq-a5fd6b70
  Scenario: A study without stages has no pipeline
    Given study "notes-only" has "stages: []"
    When "tophat check" is run
    Then no "pipeline" diagnostic is reported for "notes-only"

  @rq-8d671e50
  Scenario: Input referencing a later stage
    Given stage "prepare" has the input "scores: stage:analyse"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported at stage "prepare"

  @rq-19e53a7b
  Scenario: Unknown placeholder
    Given stage "prepare" has run "julia prepare.jl {in.raw} {output}"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "{output}" is reported

  @rq-ccaa4e30
  Scenario: Escaped braces are allowed
    Given stage "prepare" has run "awk '{{print $1}}' {in.raw}/list > {out}/ids"
    When the configuration is loaded
    Then no error is reported

  @rq-eda3d443
  Scenario: A stage cannot be both a tool stage and an external stage
    Given stage "score" has "tool: method-x@a1b2c3d" and an "external" mapping
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "tool" and "external" is reported

  @rq-a5ab4b78
  Scenario: Fan-out stage cannot declare a result
    Given stage "score" has both "foreach: structures" and "result: r.txt"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported

  @rq-6101fec0
  Scenario: Missing code file
    Given stage "prepare" lists code "scripts/missing.jl"
    When "tophat check" is run
    Then an error "pipeline.code-missing" is reported

  @rq-c3a5cbef
  Scenario: Code outside the project
    Given stage "prepare" lists code "../../../etc/passwd"
    When "tophat check" is run
    Then an error "pipeline.code-missing" is reported

  @rq-003d3d0f
  Scenario: Missing environment
    Given stage "score" has "env: gpu" and pixi.lock has no environment "gpu"
    When "tophat check" is run
    Then an error "pipeline.env-missing" is reported

  @rq-180689f2
  Scenario: A consumed stage without validation is flagged
    Given stage "prepare" has no "validate" and stage "score" consumes it
    When "tophat check" is run
    Then a warning "pipeline.validate-missing" is reported at stage "prepare"
    And none is reported for "analyse", which nothing consumes

  @rq-0a10fc96
  Scenario: The validate command enters the key
    Given the keys of all stages are recorded
    When the "validate" command of "prepare" changes
    Then the keys of "prepare" and of every stage downstream of it change
    And a stage without "validate" has no "validate" line in its key text

  @rq-617974f2
  Scenario: The review question does not enter the key
    Given the key of "score" is recorded
    When its "review" question is reworded
    Then the key of "score" is unchanged

  @rq-2a51782f
  Scenario: The removed QC fields are rejected
    Given stage "score" has "qc: skip", "qc_reason" or "check"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported whose message names "review" and "validate"

  @rq-fd086402
  Scenario: An empty review question is rejected
    Given stage "score" has "review: ''"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported

  # --- Variants ---

  @rq-43b7a9e6
  Scenario: Variant overrides merge into the base stage
    Given "config/pbe.yaml" contains "stages: [{name: score, params: {level: pbe}}]"
    And the base stage "score" has params {level: b3lyp, grid: fine}
    When the pipeline is loaded for variant "pbe"
    Then stage "score" has params {level: pbe, grid: fine}
    And every other stage equals the base stage

  @rq-3fd889d8
  Scenario: Variant cannot add stages
    Given "config/pbe.yaml" contains "stages: [{name: extra, run: echo}]"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported on "config/pbe.yaml"

  # --- Keys ---

  @rq-59d28b94
  Scenario: Key text follows the specified construction
    When the key text of stage "prepare" is computed
    Then it starts with "tophat-stage-key/1"
    And it contains one "code" line for "scripts/prepare.jl" with the file's SHA-256
    And its "params" line is '{"cutoff":2.0}'
    And it contains "input raw dataset benchmark-set@v2" followed by that dataset's content hash
    And the key is "sk-" followed by the first 16 hex characters of the text's SHA-256

  @rq-6f38a675
  Scenario: Changing a script changes its key and every downstream key
    Given the keys of all stages are recorded
    When "scripts/prepare.jl" is edited
    Then the keys of "prepare", "score" and "analyse" all change

  @rq-e35a8bdd
  Scenario: Changing a parameter changes only that stage and downstream
    Given the keys of all stages are recorded
    When the "level" parameter of "score" changes
    Then the key of "prepare" is unchanged
    And the keys of "score" and "analyse" change

  @rq-21dcc0c2
  Scenario: Parameter order does not matter
    Given stage "score" has params {level: b3lyp, grid: fine}
    When the params are rewritten as {grid: fine, level: b3lyp}
    Then the key of "score" is unchanged

  @rq-9f5b536b
  Scenario: Variants with identical stage settings share a key
    Given variant "pbe" changes only the params of "score"
    Then stage "prepare" has the same key and output directory in variants "base" and "pbe"

  @rq-85aa702f
  Scenario: A new dataset version changes the key
    Given stage "prepare" consumes "dataset:benchmark-set@v2"
    When the input is changed to "dataset:benchmark-set@v3"
    Then the key of "prepare" changes

  @rq-e23a3422
  Scenario: A code directory is hashed file by file
    Given stage "score" lists code "scripts/common/"
    When a file inside "scripts/common/" is edited
    Then the key of "score" changes

  @rq-9e24cb69
  Scenario: Gating a stage does not change keys
    Given the keys of all stages are recorded
    When stage "prepare" gets a "review" question
    Then the keys of all stages are unchanged

  @rq-6d9d6c2d
  Scenario: Updating the environment changes keys of stages using it
    Given the "chem" environment entries in pixi.lock change
    Then the key of "score" changes
    And the key of "prepare" (environment "default") is unchanged

  @rq-04f01464
  Scenario: Output directory is named by the key
    When "tophat pipeline keys x-vs-docking" is run
    Then the output directory of "prepare" is "<results root>/x-vs-docking/prepare/<key of prepare>/"

  # --- Sealing ---

  @rq-de9819a0
  Scenario: Seal a completed stage
    Given the output directory of "prepare" contains "1abc.pdb" and "2xyz.pdb"
    When "tophat stage seal x-vs-docking prepare --key <key of prepare>" is run
    Then "provenance.json" is written in the output directory
    And it records both outputs with sizes and SHA-256 hashes
    And it records the key, the substituted command, the code hashes, the params, the inputs and the HEAD commit
    And "clean" is true and "unclean_files" is empty

  @rq-9fa80dbe
  Scenario: Seal refuses when the configuration changed during the run
    Given the run of "prepare" started with key K
    And "scripts/prepare.jl" was edited before sealing
    When "tophat stage seal x-vs-docking prepare --key K" is run
    Then the command fails with KeyMismatch
    And no provenance.json is written

  @rq-672050d2
  Scenario: Seal refuses an empty output
    Given the output directory of "prepare" is empty
    When "tophat stage seal x-vs-docking prepare --key <key of prepare>" is run
    Then the command fails with NoOutputs

  @rq-a54ab3d8
  Scenario: Seal checks the result file
    Given the output directory of "analyse" contains "result.txt" with two non-empty lines
    When "tophat stage seal x-vs-docking analyse --key <key of analyse>" is run
    Then the command fails with BadResult

  @rq-383b2154
  Scenario: Uncommitted code makes the record unclean
    Given "scripts/prepare.jl" has uncommitted changes
    And "prepare" ran and produced outputs under its current key
    When it is sealed
    Then "clean" is false
    And "unclean_files" contains "scripts/prepare.jl"

  @rq-6a6e86ca
  Scenario: Untracked config makes the record unclean
    Given "config/pbe.yaml" is not tracked by git
    When stage "score" of variant "pbe" is sealed
    Then "clean" is false and "unclean_files" contains "config/pbe.yaml"

  @rq-c85070d7
  Scenario: Changes elsewhere do not make a record unclean
    Given another study's script has uncommitted changes
    When stage "prepare" of "x-vs-docking" is sealed
    Then "clean" is true

  @rq-29ee60f1
  Scenario: Outside a git repository records are unclean
    Given the project is not a git repository
    When stage "prepare" is sealed
    Then "clean" is false and "unclean_files" is ["<no git>"]
    And "commit" is null

  @rq-96ee56c2
  Scenario: Seal a fan-out item and the stage
    Given "prepare" produced items "1abc.pdb" and "2xyz.pdb"
    And the item directories of "score" for "1abc" and "2xyz" contain outputs
    When each item is sealed with "--item"
    And the stage is sealed without "--item"
    Then each item directory has a provenance.json with its "item" name
    And the stage-level provenance.json lists both items with their outputs hashes

  @rq-2f5e3dcb
  Scenario: Stage-level seal of a fan-out stage requires all items
    Given only item "1abc" of "score" is sealed
    When "tophat stage seal x-vs-docking score --key <key of score>" is run without "--item"
    Then the command fails with ItemsUnsealed naming "2xyz"

  # --- Keys command ---

  @rq-3b86c7d4
  Scenario: Keys command reports sealed and missing outputs
    Given "prepare" is sealed under its current key and "score" is not
    When "tophat pipeline keys x-vs-docking" is run
    Then "prepare" is listed as "sealed" and "score" as "missing"

  @rq-11940cec
  Scenario: Keys command with the default results root
    Given root "results" is not configured
    When "tophat pipeline keys x-vs-docking" is run
    Then every stage's output directory lies under "<project root>/results/"
```
