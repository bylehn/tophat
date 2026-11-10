# Feature: Study Document Format and Lifecycle <!-- rq-2db7a1dd -->

A study is the smallest unit of research in a TopHat project: it addresses one question in one of
six modes, runs end-to-end with its own pipeline, and reports result lines that carry their
epistemic tag. Its `STUDY.md` is the study's requirements document. This file specifies the format
of `STUDY.md` (front matter, per-mode sections, links between studies), the study lifecycle and the
commands that move a study through it, the epistemic tags on result lines, the generated
`SUMMARY.md` that results are copied from, and the `study` rule family evaluated by `tophat check`
(see `rqm/check.md`).

Pipeline construction (Snakefile, stage keys, provenance), the question tree, and data rules are
specified elsewhere. This file depends on them only through the interfaces stated below.

## Study Layout <!-- rq-fc05344f -->

```
studies/<study-id>/
  STUDY.md          the study document (this file's subject)
  SUMMARY.md        written by `tophat study summarize`; committed
  EXPLORATION.md    written from the trial runs while a draft, locked at pre-registration
                    (see rqm/study/exploration.md); committed
  Snakefile         fixed stub (see rqm/pipeline/snakemake.md)
  config/
    base.yaml       base pipeline configuration (always present)
    <variant>.yaml  one override per non-base variant, holding only what differs
  qc/               human QC verdicts (see rqm/pipeline/snakemake.md)
  tests/            scenario tests, any language
```

The pipeline configuration, Snakefile and QC verdicts are governed by `rqm/pipeline/provenance.md`
and `rqm/pipeline/snakemake.md`. Other files (scripts) may exist and are not governed.

### Study ID <!-- rq-76ac5b2e -->

- The study ID is the name of the study's directory under `studies/`.
- It matches `^[a-z0-9]+(-[a-z0-9]+)*$` and is 3 to 64 characters long.
- It is unique within the project by construction (directory names are unique).
- It is part of the frozen fingerprint (see *Frozen Fingerprint*), so renaming a pre-registered
  study's directory is detected as a violation.

## STUDY.md Format <!-- rq-c66110cf -->

`STUDY.md` consists of YAML front matter delimited by `---` lines, followed by a Markdown body.

### Front Matter <!-- rq-a3b4939d -->

| Field | Type | Required | Meaning |
|---|---|---|---|
| `study` | string | always | Must equal the study directory name |
| `title` | string | always | Non-empty, one line |
| `question` | string | always | ID of the question in the project question tree this study addresses; matches `^q-[0-9a-f]{8}$` |
| `mode` | enum | always | `confirmatory`, `exploratory`, `descriptive`, `methods`, `replication`, or `retrospective` |
| `status` | enum | always | `draft`, `preregistered`, `reported`, or `superseded` |
| `visibility` | enum | always | `public` or `private` |
| `variants` | list of strings | always | Variant names; non-empty; each matches the study ID pattern; no duplicates |
| `preregistered` | mapping | modes with pre-registration, see *Lifecycle* | `date` (`YYYY-MM-DD`) and `frozen_hash` (`sha256:` followed by 64 lowercase hex characters); for studies locked with an exploration record also `exploration_hash` (same form) and `deciding_data_seen` (`true` or `false`), see `rqm/study/exploration.md`. Absent in `exploratory` and `retrospective` mode studies at every status |
| `reproduces` | string | iff `mode: replication` | `<study-id>@<pin>`, where `<pin>` is a 40-character lowercase hex project commit; the target study's stage keys are read from its `SUMMARY.md` front matter at that commit |
| `follows_up`, `reanalyses`, `refutes`, `extends`, `derived_from` | list of strings | optional | Study IDs this study relates to (see *Links*); each entry matches the study ID pattern |
| `superseded_by` | string | iff `status: superseded` | Study ID of the replacing study |
| `supersedes` | list of strings | optional | Study IDs this study replaces |

Unknown front-matter fields are a warning (they may be typos of known fields).

Whether the `question` ID exists in the question tree is checked by the question-tree feature, not
by this rule family.

### Body Sections <!-- rq-e9d4388a -->

Sections are `##` headings. Headings inside fenced code blocks are not headings. Section names are
matched exactly (case-sensitive). Every study carries these governed sections:

| Section | Frozen | Content |
|---|---|---|
| `## Question` | in modes that pre-register, at pre-registration | The question, in prose |
| `## Variants` | no | Prose describing each variant listed in front matter |
| `## Stages` | no | Prose describing the pipeline stages; the machine-readable definition is in `config/` |
| `## Result` | no | Result lines copied from `SUMMARY.md` by `tophat study report`; in `retrospective` mode also hand-written results |
| `## Interpretation` | no | Free human interpretation of the result |

and each mode that pre-registers adds its own governed sections:

| Mode | Mode sections | Content |
|---|---|---|
| `confirmatory` | `## Primary endpoint`, `## Decision rule`, `## Scenarios` | The single quantity that addresses the question and how it is measured; how the endpoint's value maps to an answer, including any thresholds; testable expectations (see *Scenarios*) |
| `descriptive` | `## Deliverables`, `## Scenarios` | What is measured, on what data, and published as what dataset or measurement; testable expectations |
| `methods` | `## Success criteria`, `## Scenarios` | The criteria a developed or evaluated method must meet; testable expectations |
| `replication` | `## Match criterion`, `## Scenarios` | What counts as a match with the reproduction target (exact hashes or stated tolerances on named quantities); testable expectations |

`exploratory` and `retrospective` mode studies add no mode sections. Their plan is never frozen.

- Every governed section heading of the study's mode must be present in every status, in any
  order; a section may be empty where the lifecycle allows.
- A governed heading must not appear more than once.
- A mode section belonging to another mode (e.g. `## Primary endpoint` in a descriptive study) is
  an ordinary free section, not governed.
- Any other `##` sections (e.g. `## Background`) are allowed and not governed.
- `## Amendments` is a governed section in modes that pre-register: dated changes after
  pre-registration (see *Amendments*). In `exploratory` and `retrospective` mode it is an ordinary
  free section.
- A section's content is every line after its heading up to the next `##` heading outside a fenced
  code block, or the end of the file. `###` and deeper headings are part of the section.
- A section is **empty** if its content contains only whitespace and HTML comments.

### Scenarios <!-- rq-04595081 -->

A scenario is one testable expectation of the study. Modes that pre-register carry a `## Scenarios`
section; `exploratory` and `retrospective` mode studies have none (a `## Scenarios` heading there is
an ordinary free section). The `## Scenarios` section writes scenarios in either or both of two
forms:

- **Checklist:** a top-level list item outside any fenced block, `- S<n>: <expectation>`, where
  `<n>` is a positive integer without leading zeros and the expectation is non-empty. This is the
  form the study template suggests:

  ```markdown
  - S1: Spearman rho of method X against experiment exceeds that of docking on the benchmark set.
  - S2: No scored structure has a heavy-atom clash below 2.0 Å.
  ```

  A list item `- S<digits>: …` whose number is zero or has leading zeros is an invalid tag; other
  list items are free text.
- **Gherkin:** ` ```gherkin ` fenced blocks, for those who prefer Given/When/Then.

For the Gherkin form:

- Each `Scenario:` (or `Scenario Outline:`) is immediately preceded, on its own line, by exactly one
  scenario tag of the form `@S<n>`, where `<n>` is a positive integer without leading zeros. Other
  Gherkin tags may appear on the same line. A tag is a scenario tag only if it is `@S` followed by
  digits only; tags such as `@Slow` or `@Smoke` are ordinary tags.
- Scenario numbers are unique within the study, across both forms. Gaps are allowed (a removed
  draft scenario's number need not be reused).

Gherkin example:

```markdown
## Scenarios

~~~gherkin
Feature: x-vs-docking

  @S1
  Scenario: Method X ranks the benchmark set better than docking
    Given the variant "base"
    When the score stage completes
    Then Spearman rho of X against experiment exceeds rho of docking

  @S2
  Scenario: Every scored structure passed the clash filter
    Given any variant
    When the filter stage completes
    Then no structure in the scored set has a heavy-atom clash below 2.0 Å
~~~
```

### Scenario Test References <!-- rq-aa0fe39a -->

- Test files are all regular files under `studies/<study-id>/tests/`, recursively, in any language.
- A test file references scenario `S<n>` by containing the token `th:S<n>` anywhere in its text
  (matched by `th:S([1-9][0-9]*)\b`). The token is normally placed in a comment.
- A scenario is **tested** if at least one test file references it.
- A reference to a scenario number that does not exist in `STUDY.md` is an **orphan reference**.

### Variants and Configuration <!-- rq-e5eda0dd -->

- `config/base.yaml` must exist.
- For every listed variant other than `base`, `config/<variant>.yaml` must exist.
- Every `config/*.yaml` file other than `base.yaml` must correspond to a listed variant.
- The content of configuration files is not validated by this rule family.

### Amendments <!-- rq-fd31255d -->

- Each amendment is one top-level list item:
  `- YYYY-MM-DD — results seen: yes|no — <reason>` (the separator is an em dash `—` surrounded by
  single spaces; `<reason>` is non-empty).
- Amendment dates are valid calendar dates, not earlier than the pre-registration date, and not later
  than the current date.
- The `## Amendments` section must be empty while the study has no `preregistered` block.

## Links <!-- rq-db6541c1 -->

A study relates to other studies through the relation fields in its front matter (`follows_up`,
`reanalyses`, `refutes`, `extends`, `derived_from`) and, in `replication` mode, through `reproduces`.

- Relations are stored once, on the relating study. The reverse direction — which studies follow up
  on, re-analyse, refute or extend a given one — is a query over the studies, not stored backlink
  fields.
- Relation fields are written by `tophat study link` and `reproduces` by `tophat study replicate`;
  they are never edited by hand. They may be added at any status, are not frozen, and do not enter
  the frozen fingerprint.
- Every relation entry must name an existing study other than the relating study itself.
- `reproduces` must name an existing study of this project, and its pin must be a 40-character
  lowercase hex commit. The target's stage keys are read from its committed `SUMMARY.md` front
  matter at that commit; those keys define what a reproduction compares against. How a replication
  study's pipeline re-runs the target's stages and how the comparison becomes result lines are
  specified in `rqm/pipeline/provenance.md` and `rqm/pipeline/snakemake.md`.

## Epistemic Tags <!-- rq-5cca5697 -->

Every result line — in `SUMMARY.md` and in the `## Result` section, including hand-written
retrospective results — ends with its **epistemic tag**: a final space-separated `[…]` group
holding the tag, or the tag, ` · ` (middle dot surrounded by single spaces), and `split:<name>`:

- `- base/score: n=120, rho=0.61 [0.49, 0.71], p<0.001 [pre-specified]`
- `- base/analyse: rho=0.42 [exploratory · split:held-out]`

- The tag is one of `pre-specified`, `exploratory`, or `post hoc`. The split name matches the study
  ID pattern. Which datasets form a split, and how the split a line was computed on is derived, are
  specified in `rqm/data.md`.
- A line's tag comes from the result stage that produced it: the stage's `epistemic` field in
  `config/` (see `rqm/pipeline/provenance.md`) overrides the mode default. The mode defaults are
  `exploratory` for every line of an `exploratory` mode study, `post hoc` for every line of a
  `retrospective` mode study, and `pre-specified` in `confirmatory`, `descriptive`, `methods` and
  `replication` mode.
- `epistemic: pre-specified` on a stage of an `exploratory` or `retrospective` mode study is a
  configuration error, as is `epistemic` on a stage without `result`.
- `tophat study summarize` derives each line's tag and writes it on the line; `tophat study report`
  copies result lines verbatim, so tags reach `## Result` unchanged.
- A `reported` `confirmatory` study has at least one result line tagged `pre-specified` — the
  primary endpoint's; its other lines may carry `exploratory` or `post hoc`. A line tagged
  `pre-specified` in an `exploratory` or `retrospective` mode study is an error: those modes never
  froze a plan.

## SUMMARY.md Format <!-- rq-09eda840 -->

`SUMMARY.md` is written by `tophat study summarize` (see `rqm/pipeline/snakemake.md`) and committed.
It is never hand-edited.

- YAML front matter with fields:
  - `study` — study ID; must equal the directory name.
  - `commit` — the 40-character lowercase hex project commit the pipeline ran at.
  - `dirty` — boolean; `true` if any provenance record the result lines derive from is not clean,
    i.e. some code, configuration or environment file that determined those outputs was not
    committed unchanged (see `rqm/pipeline/provenance.md`).
  - `generated` — RFC 3339 timestamp.
  - `tophat_version` — version string of the `tophat` binary that wrote it.
  - `keys` — optional mapping from `<variant>/<stage>` to the stage key each result line was read
    from. Summaries written by `tophat study summarize` always include it.
  - `check` — optional: the study's test key, present exactly when the study has test files (see
    `rqm/pipeline/snakemake.md`).
  - `deciding_data_seen` — optional boolean, copied from the study's `preregistered` block when it
    has one (see `rqm/study/exploration.md`).
- A body containing a `## Results` section whose content is one or more **result lines**, each a
  top-level list item `- <variant>/<stage>: <text> [<epistemic tag>]` (see *Epistemic Tags*), where
  `<variant>` is a listed variant and `<stage>` matches the study ID pattern.

## Frozen Fingerprint <!-- rq-08c7718c -->

The frozen fingerprint is computed from the study ID, the reproduction target, and the frozen
sections of the study's mode. Only modes that pre-register have one; `exploratory` and
`retrospective` mode studies never do.

1. Start with the line `study:<study-id>`.
2. In `replication` mode, append the line `reproduces:<value exactly as in front matter>`.
3. For each frozen section, in the fixed order of the study's mode — Question first, then the mode
   sections in the order Primary endpoint, Decision rule, Scenarios (`confirmatory`); Deliverables,
   Scenarios (`descriptive`); Success criteria, Scenarios (`methods`); Match criterion, Scenarios
   (`replication`) — append the line `## <section name>` followed by the section's normalized
   content.
4. Normalization: convert CRLF line endings to LF, strip trailing whitespace from each line, and
   remove leading and trailing blank lines from the section content.
5. Join all lines with LF, append a final LF, encode as UTF-8, and compute SHA-256.
6. The fingerprint is `sha256:` followed by the lowercase hex digest.

The fingerprint does not depend on the order of sections in the file, on non-frozen sections, or on
front matter other than the study ID and `reproduces`; it does not depend on the `mode` field, so
recording a mode never invalidates a recorded fingerprint.

## Lifecycle <!-- rq-f28e6ab8 -->

Modes that pre-register (`confirmatory`, `descriptive`, `methods`, `replication`) move through
`draft` → `preregistered` → `reported`. `exploratory` and `retrospective` mode studies move through
`draft` → `reported` directly and never have a `preregistered` block.

| Status | `preregistered` block | Frozen sections | Result | Tests |
|---|---|---|---|---|
| `draft` | absent | may change; may be empty | empty; in `retrospective` mode, hand-written result lines (tagged `post hoc`) are allowed and flagged legacy | orphan references warn |
| `preregistered` | present, fingerprint matches | non-empty, at least one scenario | empty | untested scenarios warn; orphans error |
| `reported` | present and matching in modes that pre-register; absent in `exploratory` and `retrospective` mode | non-empty, at least one scenario, in modes that pre-register | equals `SUMMARY.md` result lines; summary clean; in `retrospective` mode, hand-written result lines without a `SUMMARY.md` are flagged legacy | every scenario tested; orphans error |
| `superseded` | as before supersession | as before supersession | as before supersession | as before supersession |

In every mode, a `reported` study's `## Question` section is non-empty.

Transitions:

- `draft` → `preregistered`: only in modes that pre-register, by `tophat study preregister`, run by
  a human.
- `preregistered` → `reported`: by `tophat study report`.
- `draft` → `reported`: only in `exploratory` and `retrospective` mode, by `tophat study report`.
- `preregistered` or `reported` → `superseded`: by `tophat study supersede`.
- A `draft` study is never superseded; it is edited or deleted instead.
- A study in `exploratory` or `retrospective` mode never becomes `preregistered`.

`tophat check` validates the current state against the table; it does not reconstruct history. A
`superseded` study is validated by the rules of the status it had before supersession, which is
inferred: `preregistered` block present and Result non-empty → `reported`; block present and
Result empty → `preregistered`; block absent → `reported` (drafts are never superseded, so a
superseded study without a block was in a mode without pre-registration).

## Feature API <!-- rq-3fd30bb8 -->

### Commands <!-- rq-2dd95e98 -->

- `tophat study new <study-id> --question <question-id> [--mode <mode>] [--visibility public|private]` <!-- rq-f689a538 -->
  - Creates `studies/<study-id>/` containing `STUDY.md` (status `draft`, mode defaulting to
    `confirmatory`, `variants: [base]`, visibility defaulting to `public`, every governed section
    heading of the mode present and empty except for template guidance comments), `config/base.yaml`
    (containing `stages: []`), the `Snakefile` stub, and an empty `tests/` directory.
  - Fails without writing anything if the ID is invalid, the directory already exists, the
    question ID does not match the question ID pattern, or `--mode` is `replication` (a
    replication study is created by `tophat study replicate`, which sets its target).
- `tophat study preregister <study-id>` <!-- rq-bcb83d0a -->
  - Requires standard input and standard output to be interactive terminals; otherwise fails
    without changes, explaining that pre-registration must be done by a human.
  - Requires status `draft`, a mode that pre-registers, and that the study has no `error` diagnostics
    from the `study` rule family when evaluated as if it were `preregistered` with a matching
    fingerprint (non-empty frozen sections of the mode, at least one scenario, valid tags, variant
    configs consistent).
  - Explains in plain words what pre-registration does: the mode's frozen sections printed below
    are locked from now on, so the result is judged by the plan made before it was seen; later
    changes go into dated amendments that say whether results had been seen; the pipeline, variants
    and stages can still change. Then prints the frozen sections and asks the user to type the
    study ID to confirm. Any other input aborts without changes.
  - Says what the exploration saw before asking for confirmation, and on confirmation writes
    `EXPLORATION.md` (see `rqm/study/exploration.md`).
  - On confirmation, sets `status: preregistered` and writes `preregistered.date` (current local
    date), `preregistered.frozen_hash`, `preregistered.exploration_hash` and
    `preregistered.deciding_data_seen`. Changes nothing else in the file.
  - Prints a reminder to commit `STUDY.md` before running any answer-stage computation.
- `tophat study report <study-id>` <!-- rq-ecacda48 -->
  - Requires status `preregistered` or `reported` in any mode, or status `draft` in `exploratory` or
    `retrospective` mode.
  - A study with stages reports from `SUMMARY.md`: it must exist, parse, name this study, and have
    `dirty: false`, and every result line's variant must be listed in front matter. The summary
    must not be stale (its `keys`, and its `check` when the study has test files, equal the current
    keys; see `pipeline.summary-stale` in `rqm/pipeline/snakemake.md`), and the study must not
    depend on unreported work without an accepting amendment (see `rqm/data.md`). Replaces the
    content of
    `## Result` with the summary's result lines, verbatim and in order.
  - A `retrospective` study without stages reports its hand-written `## Result` instead: each line
    must be a result line of a listed variant with a valid trailing tag group whose tag is
    `post hoc` (see *Epistemic Tags*); `## Result` is left as written, and the reported study is
    flagged legacy by `study.result-unverified`. An `exploratory` study without stages has nothing
    to report from and fails.
  - Requires a `pass` verdict for the current key of every stage that declares `review`, in every
    listed variant (see `rqm/pipeline/snakemake.md`); otherwise it fails naming the
    `tophat stage qc` command of the first one missing.
  - Sets status `reported` if it was `preregistered` or `draft`; otherwise leaves status unchanged.
  - Changes nothing else in the file. Does not require an interactive terminal.
- `tophat study supersede <old-id> --by <new-id>` <!-- rq-f2c5cd1 --> <!-- rq-ea738ef8 -->
  - Requires both studies to exist, `<old-id>` to have status `preregistered` or `reported`, and
    `<new-id>` to differ from `<old-id>`.
  - Sets `<old-id>`'s status to `superseded` and `superseded_by: <new-id>`, and adds `<old-id>` to
    `<new-id>`'s `supersedes` list if not already present.
- `tophat study replicate <study-id> --question <question-id> --of <study-id>@<commit> [--visibility public|private]` <!-- rq-3250dbea -->
  - Creates the study exactly as `tophat study new` does with `mode: replication` and, in addition,
    writes `reproduces: <study-id>@<commit>` to the front matter.
  - Fails without writing anything under the conditions of `tophat study new`, when the target study
    does not exist, when the commit is not a 40-character lowercase hex string, or when the
    target's `SUMMARY.md` at that commit cannot be read from the project's history (its keys are
    what the reproduction compares against).
- `tophat study link <study-id> --follows-up|--reanalyses|--refutes|--extends|--derives-from <other-study-id>` <!-- rq-a8e3636b -->
  - Appends `<other-study-id>` to the named relation list in the study's front matter, creating the
    field when absent; duplicates are ignored and the list stays sorted by study ID.
  - Fails without changes when the study does not exist or cannot be parsed, the other study does
    not exist, or the other study is the study itself.
  - Works at any status; relation fields are not frozen.
- `tophat study migrate <study-id>...` <!-- rq-9bc9d54b -->
  - For each named study, in one atomic write that preserves every other byte of the file:
    - a study whose front matter has no `mode` and whose status is `draft`, `preregistered`,
      `reported` or `superseded` gains `mode: confirmatory`;
    - a study with `status: retrospective` becomes `mode: retrospective` with `status: reported`
      when its `## Result` is non-empty, and `status: draft` when it is empty.
  - Leaves a study that needs neither change untouched and says so. Fails without changes for a
    study whose `status` is `retrospective` and whose front matter already carries a `mode`, or
    whose `mode` value is invalid.

All commands that modify `STUDY.md` preserve every byte of the file outside the fields and section
they are specified to change, and write the file atomically (write to a temporary file in the same
directory, then rename).

### Types <!-- rq-dacbc76e -->

- `Study` — a parsed study: ID, front-matter fields (including mode, relation lists and <!-- rq-8071e60f -->
  `reproduces`), governed sections with their content and starting line numbers, scenarios (number,
  title, line), and amendments (date, results seen, reason, line).
- `Status` — enum `Draft`, `Preregistered`, `Reported`, `Superseded`. <!-- rq-5d9d0d62 -->
- `Mode` — enum `Confirmatory`, `Exploratory`, `Descriptive`, `Methods`, `Replication`, <!-- rq-597a5aff -->
  `Retrospective`, with a predicate for whether the mode pre-registers.
- `Relation` — enum `FollowsUp`, `Reanalyses`, `Refutes`, `Extends`, `DerivesFrom`, one variant per <!-- rq-7d5a35b9 -->
  relation field.
- `Summary` — a parsed `SUMMARY.md`: study ID, commit, dirty flag, generated timestamp, tophat <!-- rq-6764653f -->
  version, optional keys, optional check key, and result lines (variant, stage, text, tag,
  optional split, line).
- `StudyError` — error type returned by parsing and by the study commands, with at minimum: <!-- rq-dba2a6b7 -->
  - `InvalidStudyId(String)`.
  - `StudyExists(String)` — `new` on an existing directory.
  - `StudyNotFound(String)`.
  - `FrontMatter { path: String, message: String }` — missing delimiters, invalid YAML, missing or
    mistyped fields.
  - `WrongStatus { study: String, status: Status, required: Vec<Status> }`.
  - `WrongMode { study: String, mode: Mode }` — a command that does not apply to the study's mode,
    such as `preregister` on an `exploratory` or `retrospective` mode study.
  - `ModeInvalid(String)`.
  - `ReproducesInvalid(String)` — malformed target or pin, nonexistent target study, or unreadable
    target summary at the pin.
  - `LinkInvalid { study: String, relation: Relation, other: String }`.
  - `Migration(String)` — `migrate` on a study it cannot convert.
  - `NotInteractive` — `preregister` without an interactive terminal.
  - `ConfirmationMismatch` — the typed ID did not match.
  - `NotReady(Vec<Diagnostic>)` — `preregister` precondition violations.
  - `Summary(String)` — `report` with a missing, unparseable, dirty, or mismatched `SUMMARY.md`,
    or a hand-written retrospective result line that is not a valid `post hoc` result line.
  - `Io(String)`.

### Functions <!-- rq-e10b269c -->

- `parse_study(dir: &Path) -> Result<Study, StudyError>` — parses `STUDY.md` in a study directory. <!-- rq-b01ceeeb -->
- `parse_summary(path: &Path) -> Result<Summary, StudyError>` — parses a `SUMMARY.md`. <!-- rq-8da81b81 -->
- `frozen_fingerprint(study: &Study) -> String` — computes the fingerprint as specified under <!-- rq-f5a0149d -->
  *Frozen Fingerprint*.
- `study_rules(project: &Project) -> Vec<Diagnostic>` — the `study` rule family registered in <!-- rq-ce7c6815 -->
  `RULE_FAMILIES` (see `rqm/check.md`).

## Check Rules (`study` family) <!-- rq-2431d4ff -->

All rules apply to every directory directly under `studies/`. Line numbers point at the offending
front-matter field, heading, scenario, list item or result line where one exists.

| Rule | Severity | Condition |
|---|---|---|
| `study.missing-file` | error | A directory under `studies/` has no `STUDY.md` |
| `study.parse-error` | error | `STUDY.md` front matter is missing, not valid YAML, or a field has the wrong type; no further `study` rules run for that study |
| `study.id-invalid` | error | Directory name does not match the study ID pattern or length |
| `study.id-mismatch` | error | Front-matter `study` differs from the directory name |
| `study.mode-missing` | error | Front matter has no `mode` field; the hint names `tophat study migrate <study>` |
| `study.mode-invalid` | error | `mode` is not one of `confirmatory`, `exploratory`, `descriptive`, `methods`, `replication`, `retrospective` |
| `study.status-retrospective` | error | `status` has the value `retrospective` (the pre-mode form); the hint names `tophat study migrate <study>` |
| `study.field-missing` | error | A required front-matter field other than `mode` is absent (`reproduces` is `study.reproduces-missing`) |
| `study.field-invalid` | error | `title` empty, `question` malformed, `status`/`mode`/`visibility` not an allowed value (a `status` of `retrospective` is reported as `study.status-retrospective`), `variants` empty, duplicated or malformed |
| `study.field-unknown` | warning | Unknown front-matter field |
| `study.section-missing` | error | A governed section heading of the study's mode is absent |
| `study.section-duplicate` | error | A governed section heading appears more than once |
| `study.section-empty` | error | A section required non-empty by the lifecycle table is empty |
| `study.prereg-missing` | error | Status requires a `preregistered` block and it is absent or incomplete |
| `study.prereg-unexpected` | error | Status `draft` (or superseded from it), or a study in `exploratory` or `retrospective` mode, has a `preregistered` block |
| `study.frozen-modified` | error | The recomputed fingerprint differs from `preregistered.frozen_hash` |
| `study.reproduces-missing` | error | `mode: replication` without a `reproduces` field |
| `study.reproduces-unexpected` | error | `reproduces` present while `mode` is not `replication` |
| `study.reproduces-invalid` | error | `reproduces` is malformed, names a study that does not exist, or its pin is not a 40-character lowercase hex commit |
| `study.link-invalid` | error | A relation entry is malformed, names a study that does not exist, or names the study itself |
| `study.scenario-untagged` | error | A scenario lacks exactly one `@S<n>` tag |
| `study.scenario-tag-invalid` | error | A tag is `@S` followed by digits, or a checklist item starts with `S` followed by digits and a colon, but the number is not valid (e.g. `@S0`, `- S01:`) |
| `study.scenario-duplicate` | error | Two scenarios share a number |
| `study.no-scenarios` | error | Status requires at least one scenario and there are none |
| `study.scenario-untested` | warning in `preregistered`; error in `reported` | A scenario is not referenced by any test file |
| `study.test-orphan` | warning in `draft`; error otherwise | A test file references a nonexistent scenario |
| `study.config-base-missing` | error | `config/base.yaml` is absent |
| `study.config-missing` | error | A listed non-base variant has no config file |
| `study.config-unlisted` | error | A config file corresponds to no listed variant |
| `study.amendment-format` | error | An amendment line does not match the format |
| `study.amendment-date` | error | An amendment date is invalid, before the pre-registration date, or in the future |
| `study.amendment-unexpected` | error | `## Amendments` is non-empty without a `preregistered` block |
| `study.result-unexpected` | error | `## Result` is non-empty in `draft` or `preregistered`, except `draft` in `retrospective` mode |
| `study.result-missing` | error | `## Result` is empty in `reported` |
| `study.summary-missing` | error | Result must come from `SUMMARY.md` and it is absent |
| `study.summary-invalid` | error | `SUMMARY.md` cannot be parsed, names another study, or has a result line with an unlisted variant |
| `study.summary-dirty` | error | Result is copied from a `SUMMARY.md` with `dirty: true` |
| `study.result-mismatch` | error | `## Result` lines differ from `SUMMARY.md` result lines (compared verbatim after stripping trailing whitespace, in order) |
| `study.result-unverified` | legacy | A `retrospective` mode study has a non-empty Result and no `SUMMARY.md` |
| `study.tag-missing` | error | A result line in `SUMMARY.md` or `## Result` has no valid trailing tag group; the hint names `tophat study summarize` and `tophat study report` |
| `study.tag-invalid` | error | The trailing tag group of a result line holds an unknown tag, or a split name that does not match the study ID pattern |
| `study.tag-mode-invalid` | error | A result line is tagged `pre-specified` in an `exploratory` or `retrospective` mode study |
| `study.no-pre-specified-result` | error | A `reported` `confirmatory` study has no result line tagged `pre-specified` |
| `study.supersede-target-missing` | error | `superseded_by` names a study that does not exist |
| `study.supersede-backlink-missing` | error | The replacing study does not list this study in `supersedes` |
| `study.supersedes-invalid` | error | A `supersedes` entry names a study that does not exist or whose `superseded_by` is not this study |

## Assumptions <!-- rq-c5416f0d -->

- Question IDs use the format `q-` followed by 8 lowercase hex characters, matching the opaque-ID
  style of riprap. The question-tree feature owns their assignment and resolution.
- "Current date" for amendment validation and pre-registration is the local date of the machine
  running `tophat`.
- A summary written by a `tophat` version that predates epistemic tags has untagged result lines;
  those studies report `study.tag-missing` until `tophat study summarize` regenerates the summary
  (sealed outputs are unchanged, so the regenerated summary differs only by the tags) and `tophat
  study report` re-copies the lines. This residual state is deliberate: a tag states an epistemic
  fact, so only regeneration can add it truthfully.

## Out of Scope <!-- rq-87476c9b -->

- The question tree (`PROJECT.md`) and whether `question` resolves.
- Pipeline construction, stage keys and provenance, including how the summary stage computes result
  lines and whether code changed after the run that produced `SUMMARY.md`.
- How a replication study's pipeline re-runs its target and how the comparison becomes result lines
  (`rqm/pipeline/provenance.md`, `rqm/pipeline/snakemake.md`).
- Which datasets form a split and how a result line's split is derived (`rqm/data.md`).
- Stored reverse links: relations live on the relating study; the reverse direction is a query.
- Which results may be cited as evidence for a question, and with which labels
  (`rqm/question-tree.md`).
- Data, visibility-flow, and external-stage rules.
- Running scenario tests (the pipeline's check job, see `rqm/pipeline/snakemake.md`).
- Validating the content of configuration files.

## Gherkin Scenarios <!-- rq-fbdc50b4 -->

```gherkin
Feature: Study document format, lifecycle and check rules

  Background:
    Given a TopHat project
    And question "q-1a2b3c4d" exists in the question tree

  # --- tophat study new ---

  @rq-52e3a974
  Scenario: Create a new confirmatory study
    When "tophat study new x-vs-docking --question q-1a2b3c4d" is run
    Then "studies/x-vs-docking/STUDY.md" exists with status "draft", mode "confirmatory", visibility "public" and variants ["base"]
    And it contains every governed section heading of the confirmatory mode, each with a plain-language comment saying what goes there
    And "studies/x-vs-docking/config/base.yaml" exists and contains "stages: []"
    And "studies/x-vs-docking/Snakefile" equals the stub for "x-vs-docking"
    And the directory "studies/x-vs-docking/tests" exists
    And "tophat check" reports no study errors for "x-vs-docking"

  @rq-2cd827c4
  Scenario: Create a descriptive study
    When "tophat study new bench-characterisation --question q-1a2b3c4d --mode descriptive" is run
    Then its STUDY.md has mode "descriptive"
    And it contains the governed headings "Question", "Deliverables" and "Scenarios", and no "Primary endpoint" heading
    And "tophat check" reports no study errors for "bench-characterisation"

  @rq-a091b98c
  Scenario: Creating a replication study requires the replicate command
    When "tophat study new rerun-score --question q-1a2b3c4d --mode replication" is run
    Then the command fails naming "tophat study replicate"
    And no directory is created

  @rq-7fcedcf8
  Scenario: Create a private study
    When "tophat study new client-targets --question q-1a2b3c4d --visibility private" is run
    Then the STUDY.md front matter has visibility "private"

  @rq-7bc95d75
  Scenario: Reject an invalid study ID
    When "tophat study new X_vs_Docking --question q-1a2b3c4d" is run
    Then the command fails with InvalidStudyId
    And no directory is created

  @rq-36072125
  Scenario: Reject a study ID that already exists
    Given study "x-vs-docking" exists
    When "tophat study new x-vs-docking --question q-1a2b3c4d" is run
    Then the command fails with StudyExists
    And the existing study is unchanged

  @rq-bb0e22f1
  Scenario: Reject a malformed question ID
    When "tophat study new x-vs-docking --question Q1" is run
    Then the command fails without creating any directory

  # --- Front matter and structure ---

  @rq-7effa48d
  Scenario: Directory without STUDY.md
    Given the directory "studies/empty" exists without a STUDY.md
    When "tophat check" is run
    Then an error "study.missing-file" is reported for "studies/empty"

  @rq-8d5c3082
  Scenario: Invalid YAML front matter
    Given study "study-a" has front matter that is not valid YAML
    When "tophat check" is run
    Then exactly one diagnostic is reported for study "study-a", with rule "study.parse-error"

  @rq-3e058fe4
  Scenario: Study ID does not match directory
    Given the directory "studies/study-a" contains a STUDY.md with "study: study-b"
    When "tophat check" is run
    Then an error "study.id-mismatch" is reported

  @rq-38c13472
  Scenario: Invalid status value
    Given study "study-a" has "status: finished"
    When "tophat check" is run
    Then an error "study.field-invalid" is reported on the status line

  @rq-45f33b14
  Scenario: Invalid mode value
    Given study "study-a" has "mode: hybrid"
    When "tophat check" is run
    Then an error "study.mode-invalid" is reported

  @rq-bb7f2503
  Scenario: A study without a mode
    Given study "study-a" has no "mode" field
    When "tophat check" is run
    Then an error "study.mode-missing" is reported with a hint naming "tophat study migrate"

  @rq-b80d58f2
  Scenario: The pre-mode retrospective status
    Given study "old-benchmark" has "status: retrospective"
    When "tophat check" is run
    Then an error "study.status-retrospective" is reported with a hint naming "tophat study migrate"

  @rq-f2be6f14
  Scenario: Unknown front-matter field
    Given study "study-a" has a front-matter field "statsu: draft" in addition to valid fields
    When "tophat check" is run
    Then a warning "study.field-unknown" is reported

  @rq-0d2a06e5
  Scenario: Missing governed section
    Given study "study-a" has no "## Decision rule" heading
    When "tophat check" is run
    Then an error "study.section-missing" is reported

  @rq-cd497e41
  Scenario: Duplicated governed section
    Given study "study-a" has two "## Question" headings
    When "tophat check" is run
    Then an error "study.section-duplicate" is reported at the second heading

  @rq-a79ad3ba
  Scenario: Headings inside code fences are not sections
    Given study "study-a" has a fenced code block containing the line "## Result"
    And study "study-a" has status "draft"
    When "tophat check" is run
    Then neither "study.section-duplicate" nor "study.result-unexpected" is reported

  @rq-9a610110
  Scenario: Extra sections are allowed
    Given study "study-a" has a "## Background" section
    When "tophat check" is run
    Then no diagnostic mentions the "Background" section

  @rq-5702c253
  Scenario: A section of another mode is free text
    Given descriptive study "bench-characterisation" has a "## Primary endpoint" section
    When "tophat check" is run
    Then no "study.section-missing" or "study.section-empty" diagnostic is reported for that section

  # --- Scenarios and tests ---

  @rq-3c295093
  Scenario: Scenarios as a checklist
    Given study "study-a" has a Scenarios section with the list items "- S1: The endpoint is computed from all items." and "- S2: No item is skipped."
    And a test file references "th:S1" and "th:S2"
    When "tophat check" is run
    Then no "study" diagnostic is reported for its scenarios
    And the study has scenarios 1 and 2

  @rq-de619d22
  Scenario: Checklist and Gherkin numbers are unique together
    Given study "study-a" has the list item "- S1: one" and a Gherkin scenario tagged "@S1"
    When "tophat check" is run
    Then an error "study.scenario-duplicate" is reported

  @rq-e4d78a92
  Scenario: An invalid checklist number
    Given study "study-a" has the list item "- S01: one"
    When "tophat check" is run
    Then an error "study.scenario-tag-invalid" is reported

  @rq-520893f5
  Scenario: Scenario without a tag
    Given study "study-a" has a Scenario that is not preceded by an @S tag
    When "tophat check" is run
    Then an error "study.scenario-untagged" is reported at that scenario

  @rq-2b94ea79
  Scenario: Invalid scenario tag
    Given study "study-a" has a scenario tagged "@S01"
    When "tophat check" is run
    Then an error "study.scenario-tag-invalid" is reported

  @rq-67b9306b
  Scenario: Duplicate scenario numbers
    Given study "study-a" has two scenarios tagged "@S2"
    When "tophat check" is run
    Then an error "study.scenario-duplicate" is reported at the second one

  @rq-af111715
  Scenario: Gaps in scenario numbers are allowed
    Given study "study-a" has scenarios tagged "@S1" and "@S3" only
    When "tophat check" is run
    Then no scenario-numbering diagnostic is reported

  @rq-6a843b0a
  Scenario: Test reference in a Julia file counts as a test
    Given study "study-a" has status "reported" with scenario "@S1"
    And "studies/study-a/tests/test_scores.jl" contains "# th:S1"
    When "tophat check" is run
    Then no "study.scenario-untested" diagnostic is reported for S1

  @rq-3cc4a23d
  Scenario: Untested scenario in a preregistered study warns
    Given study "study-a" has status "preregistered" and scenario "@S2" that no test references
    When "tophat check" is run
    Then a warning "study.scenario-untested" is reported for S2

  @rq-1931327f
  Scenario: Untested scenario in a reported study is an error
    Given study "study-a" has status "reported" and scenario "@S2" that no test references
    When "tophat check" is run
    Then an error "study.scenario-untested" is reported for S2

  @rq-b8e989cd
  Scenario: Orphan test reference in a draft warns
    Given study "study-a" has status "draft" and no scenario S9
    And a test file references "th:S9"
    When "tophat check" is run
    Then a warning "study.test-orphan" is reported in that test file

  @rq-9197f110
  Scenario: Orphan test reference after pre-registration is an error
    Given study "study-a" has status "preregistered" and no scenario S9
    And a test file references "th:S9"
    When "tophat check" is run
    Then an error "study.test-orphan" is reported in that test file

  # --- Variants and config ---

  @rq-d4308b0e
  Scenario: Missing variant config
    Given study "study-a" lists variants ["base", "b3lyp"]
    And "studies/study-a/config/b3lyp.yaml" does not exist
    When "tophat check" is run
    Then an error "study.config-missing" is reported

  @rq-4838d3a2
  Scenario: Unlisted variant config
    Given study "study-a" lists variants ["base"]
    And "studies/study-a/config/old.yaml" exists
    When "tophat check" is run
    Then an error "study.config-unlisted" is reported for "config/old.yaml"

  @rq-f678e5d8
  Scenario: Missing base config
    Given study "study-a" has no "config/base.yaml"
    When "tophat check" is run
    Then an error "study.config-base-missing" is reported

  # --- Pre-registration ---

  @rq-9c53f79c
  Scenario: Pre-register a ready draft interactively
    Given study "study-a" has status "draft" with non-empty Question, Primary endpoint, Decision rule and one tagged scenario
    And standard input and output are interactive terminals
    When "tophat study preregister study-a" is run and the user types "study-a"
    Then the status is "preregistered"
    And "preregistered.date" is today's local date
    And "preregistered.frozen_hash" equals the frozen fingerprint of the study
    And no other byte of STUDY.md has changed

  @rq-c69454bd
  Scenario: Pre-registration explains what it locks
    Given study "study-a" is a ready draft
    When "tophat study preregister study-a" is run interactively
    Then before asking for confirmation it says that the Question, Primary endpoint, Decision rule and Scenarios will be locked, that later changes go into amendments, and that the pipeline can still change

  @rq-54c9eb81
  Scenario: Pre-registration refused without an interactive terminal
    Given study "study-a" is a ready draft
    And standard input is not an interactive terminal
    When "tophat study preregister study-a" is run
    Then the command fails with NotInteractive
    And STUDY.md is unchanged

  @rq-fdd4cac9
  Scenario: Pre-registration aborted by a wrong confirmation
    Given study "study-a" is a ready draft
    And standard input and output are interactive terminals
    When "tophat study preregister study-a" is run and the user types "study-b"
    Then the command fails with ConfirmationMismatch
    And STUDY.md is unchanged

  @rq-9334c74c
  Scenario: Pre-registration refused when the draft is not ready
    Given study "study-a" has status "draft" and an empty Decision rule section
    When "tophat study preregister study-a" is run in an interactive terminal
    Then the command fails with NotReady listing "study.section-empty"
    And the user is not asked to confirm
    And STUDY.md is unchanged

  @rq-65465008
  Scenario: Pre-registration refused for a non-draft study
    Given study "study-a" has status "reported"
    When "tophat study preregister study-a" is run in an interactive terminal
    Then the command fails with WrongStatus

  @rq-f7810e6d
  Scenario: Pre-registering a descriptive study locks its deliverables
    Given descriptive study "bench-characterisation" has status "draft" with non-empty Question and Deliverables and one tagged scenario
    When "tophat study preregister bench-characterisation" is run interactively and confirmed
    Then the status is "preregistered"
    And "preregistered.frozen_hash" equals the frozen fingerprint of Question, Deliverables and Scenarios
    When a word in the "## Deliverables" section is changed
    And "tophat check" is run
    Then an error "study.frozen-modified" is reported

  @rq-4a0634d3
  Scenario: Pre-registering a methods study locks its success criteria
    Given methods study "scoring-v2" has status "draft" with non-empty Question and Success criteria and one tagged scenario
    When "tophat study preregister scoring-v2" is run interactively and confirmed
    Then the frozen sections are Question, Success criteria and Scenarios

  @rq-5bdc928b
  Scenario: Pre-registering a replication study locks its target and match criterion
    Given study "rerun-score" has mode "replication", status "draft", non-empty Question and Match criterion, one tagged scenario, and "reproduces: x-vs-docking@<full-commit>"
    When "tophat study preregister rerun-score" is run interactively and confirmed
    Then the frozen fingerprint covers Question, Match criterion, Scenarios and "reproduces"
    When the "reproduces" field is changed to another target
    And "tophat check" is run
    Then an error "study.frozen-modified" is reported

  # --- Frozen sections ---

  @rq-a3e1bf1f
  Scenario: Editing a frozen section is detected
    Given study "study-a" is preregistered with a matching fingerprint
    When a word in the "## Primary endpoint" section is changed
    And "tophat check" is run
    Then an error "study.frozen-modified" is reported

  @rq-a046955d
  Scenario: Editing a scenario is detected
    Given study "study-a" is preregistered with a matching fingerprint
    When the Then step of scenario S1 is changed
    And "tophat check" is run
    Then an error "study.frozen-modified" is reported

  @rq-cb65dab4
  Scenario: Editing a non-frozen section is allowed
    Given study "study-a" is preregistered with a matching fingerprint
    When the "## Stages" and "## Interpretation" sections are changed
    And "tophat check" is run
    Then no "study.frozen-modified" error is reported

  @rq-2e00ed22
  Scenario: Whitespace and line-ending changes do not alter the fingerprint
    Given study "study-a" is preregistered with a matching fingerprint
    When trailing spaces are added to lines of the Question section and line endings are converted to CRLF
    And "tophat check" is run
    Then no "study.frozen-modified" error is reported

  @rq-930cacde
  Scenario: Reordering sections does not alter the fingerprint
    Given study "study-a" is preregistered with a matching fingerprint
    When the "## Scenarios" section is moved above "## Question"
    And "tophat check" is run
    Then no "study.frozen-modified" error is reported

  @rq-d4227e28
  Scenario: Renaming a pre-registered study is detected
    Given study "study-a" is preregistered with a matching fingerprint
    When its directory is renamed to "studies/study-b" and its front matter is changed to "study: study-b"
    And "tophat check" is run
    Then an error "study.frozen-modified" is reported for "studies/study-b/STUDY.md"

  @rq-ea9c4659
  Scenario: Pre-registration block on a draft
    Given study "study-a" has status "draft" and a "preregistered" block
    When "tophat check" is run
    Then an error "study.prereg-unexpected" is reported

  @rq-a97cc02c
  Scenario: Missing pre-registration block on a preregistered study
    Given study "study-a" has status "preregistered" and no "preregistered" block
    When "tophat check" is run
    Then an error "study.prereg-missing" is reported

  # --- Amendments ---

  @rq-ec84dc2b
  Scenario: Valid amendment
    Given study "study-a" was preregistered on "2026-03-01"
    And its Amendments section contains "- 2026-04-02 — results seen: no — Fixed unit conversion in the filter script"
    When "tophat check" is run
    Then no amendment diagnostic is reported

  @rq-214c0020
  Scenario: Malformed amendment
    Given study "study-a" is preregistered
    And its Amendments section contains "- changed the filter"
    When "tophat check" is run
    Then an error "study.amendment-format" is reported at that line

  @rq-68f43997
  Scenario: Amendment dated before pre-registration
    Given study "study-a" was preregistered on "2026-03-01"
    And an amendment is dated "2026-02-15"
    When "tophat check" is run
    Then an error "study.amendment-date" is reported

  @rq-70b0f2e4
  Scenario: Amendment dated in the future
    Given study "study-a" is preregistered
    And an amendment is dated one day after the current date
    When "tophat check" is run
    Then an error "study.amendment-date" is reported

  @rq-a32e7ed9
  Scenario: Amendment on a draft
    Given study "study-a" has status "draft" and a non-empty Amendments section
    When "tophat check" is run
    Then an error "study.amendment-unexpected" is reported

  # --- Reporting ---

  @rq-46e1add4
  Scenario: Report copies result lines verbatim
    Given study "study-a" is preregistered with variants ["base", "b3lyp"]
    And "studies/study-a/SUMMARY.md" names study "study-a", has "dirty: false", and result lines
      "- base/score: n=120, rho=0.61 [0.49, 0.71], p<0.001 [pre-specified]" and "- b3lyp/score: n=120, rho=0.58 [0.45, 0.69], p<0.001 [pre-specified]"
    When "tophat study report study-a" is run
    Then the Result section contains exactly those two lines in that order
    And the status is "reported"
    And no other byte of STUDY.md has changed

  @rq-330e2712
  Scenario: Report refuses a dirty summary
    Given study "study-a" is preregistered
    And SUMMARY.md has "dirty: true"
    When "tophat study report study-a" is run
    Then the command fails with a Summary error explaining the run was from uncommitted code
    And STUDY.md is unchanged

  @rq-ce0bc1c8
  Scenario: Report refuses a missing summary
    Given study "study-a" is preregistered and has no SUMMARY.md
    When "tophat study report study-a" is run
    Then the command fails with a Summary error
    And STUDY.md is unchanged

  @rq-919c4647
  Scenario: Report refuses a summary for another study
    Given study "study-a" is preregistered
    And its SUMMARY.md has "study: study-b"
    When "tophat study report study-a" is run
    Then the command fails with a Summary error

  @rq-13aae895
  Scenario: Report refuses a confirmatory draft
    Given study "study-a" has mode "confirmatory", status "draft" and a valid SUMMARY.md
    When "tophat study report study-a" is run
    Then the command fails with WrongStatus

  @rq-86197e5c
  Scenario: An exploratory study reports from a draft
    Given study "outlier-dig" has mode "exploratory", status "draft" and a clean current SUMMARY.md with the result line "- base/analyse: mean shift of 0.4 sigma [exploratory]"
    When "tophat study report outlier-dig" is run
    Then the Result section contains that line verbatim
    And the status is "reported"

  @rq-07a5604f
  Scenario: An exploratory study cannot pre-register
    Given study "outlier-dig" has mode "exploratory" and status "draft"
    When "tophat study preregister outlier-dig" is run in an interactive terminal
    Then the command fails with WrongMode

  @rq-91befa97
  Scenario: A reported exploratory study needs a non-empty question
    Given study "outlier-dig" has mode "exploratory", status "reported" and an empty Question section
    When "tophat check" is run
    Then an error "study.section-empty" is reported

  @rq-e7c71a2b
  Scenario: Re-reporting refreshes the result
    Given study "study-a" is reported
    And SUMMARY.md has been regenerated with different result lines
    When "tophat study report study-a" is run
    Then the Result section contains the new lines
    And the status remains "reported"

  @rq-d6677222
  Scenario: Hand-edited result is detected
    Given study "study-a" is reported with Result equal to its SUMMARY.md result lines
    When a number in one Result line is changed by hand
    And "tophat check" is run
    Then an error "study.result-mismatch" is reported at that line

  @rq-14d17a56
  Scenario: Reported study whose summary became dirty
    Given study "study-a" is reported
    And SUMMARY.md is regenerated with "dirty: true" and identical result lines
    When "tophat check" is run
    Then an error "study.summary-dirty" is reported

  @rq-be41ffc1
  Scenario: Result in a preregistered study
    Given study "study-a" has status "preregistered" and a non-empty Result section
    When "tophat check" is run
    Then an error "study.result-unexpected" is reported

  @rq-f2135c24
  Scenario: Empty result in a reported study
    Given study "study-a" has status "reported" and an empty Result section
    When "tophat check" is run
    Then an error "study.result-missing" is reported

  @rq-588d371f
  Scenario: Summary line with an unlisted variant
    Given study "study-a" is reported with variants ["base"]
    And SUMMARY.md contains the result line "- pbe/score: n=10, rho=0.3 [pre-specified]"
    When "tophat check" is run
    Then an error "study.summary-invalid" is reported

  # --- Epistemic tags ---

  @rq-ca33392b
  Scenario: Confirmatory result lines are tagged pre-specified by default
    Given study "study-a" is reported and its SUMMARY.md result line reads "- base/score: n=120, rho=0.61 [0.49, 0.71], p<0.001 [pre-specified]"
    When "tophat check" is run
    Then no tag diagnostic is reported for that line

  @rq-2e3e53d6
  Scenario: A confirmatory study overrides a secondary result stage to exploratory
    Given confirmatory study "study-a" has result stages "score" and "diagnose"
    And stage "diagnose" declares "epistemic: exploratory"
    When "tophat study summarize study-a" is run
    Then the line of "diagnose" ends in "[exploratory]" and the line of "score" ends in "[pre-specified]"

  @rq-c5610a75
  Scenario: A result line without a tag
    Given study "study-a" is reported and its SUMMARY.md result line reads "- base/score: n=120, rho=0.61"
    When "tophat check" is run
    Then an error "study.tag-missing" is reported naming "tophat study summarize"

  @rq-6d57beba
  Scenario: An unknown tag
    Given study "study-a" is reported and its result line ends in "[maybe]"
    When "tophat check" is run
    Then an error "study.tag-invalid" is reported

  @rq-f4564941
  Scenario: A malformed split name
    Given study "study-a" is reported and its result line ends in "[pre-specified · split:Held Out]"
    When "tophat check" is run
    Then an error "study.tag-invalid" is reported

  @rq-9dd27343
  Scenario: A valid split tag passes the syntax
    Given study "study-a" is reported and its result line ends in "[pre-specified · split:held-out]"
    When "tophat check" is run
    Then no tag diagnostic is reported

  @rq-b096fd9e
  Scenario: Pre-specified is impossible in exploratory mode
    Given study "outlier-dig" is exploratory and reported with a result line ending in "[pre-specified]"
    When "tophat check" is run
    Then an error "study.tag-mode-invalid" is reported

  @rq-d952ad20
  Scenario: An exploratory study cannot declare a pre-specified stage
    Given study "outlier-dig" is exploratory and has a result stage with "epistemic: pre-specified"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported

  @rq-43f53fc5
  Scenario: A reported confirmatory study must have a pre-specified line
    Given study "study-a" is confirmatory and reported with every result line ending in "[exploratory]"
    When "tophat check" is run
    Then an error "study.no-pre-specified-result" is reported

  # --- Retrospective mode studies ---

  @rq-c55140e8
  Scenario: Retrospective study with hand-written result is flagged as legacy
    Given study "old-benchmark" has mode "retrospective", status "reported", a non-empty Question, a hand-written Result whose lines end in "[post hoc]", and no SUMMARY.md
    When "tophat check --strict" is run
    Then a legacy diagnostic "study.result-unverified" is reported
    And no error or warning is reported for "old-benchmark"
    And the exit code is 0

  @rq-c985c1d6
  Scenario: Retrospective study with a pipeline uses strict results
    Given study "old-benchmark" has mode "retrospective", status "reported" and a SUMMARY.md
    And its Result section differs from the summary's result lines
    When "tophat check" is run
    Then an error "study.result-mismatch" is reported

  @rq-c65bf680
  Scenario: Retrospective mode has no endpoint or scenario sections
    Given study "old-benchmark" has mode "retrospective" and status "reported" with no "Primary endpoint", "Decision rule" or "Scenarios" headings
    When "tophat check" is run
    Then no "study.section-missing" or "study.no-scenarios" diagnostic is reported

  @rq-647102ae
  Scenario: A study in a mode without pre-registration cannot carry a pre-registration block
    Given study "old-benchmark" has mode "retrospective", status "reported" and a "preregistered" block
    When "tophat check" is run
    Then an error "study.prereg-unexpected" is reported

  @rq-3a79a3f7
  Scenario: Retrospective study reports from a summary
    Given study "old-benchmark" has mode "retrospective", status "draft" and a clean SUMMARY.md
    When "tophat study report old-benchmark" is run
    Then the Result section equals the summary's result lines
    And the status is "reported"

  @rq-ae6ec611
  Scenario: A retrospective study without stages reports hand-written results
    Given study "old-benchmark" has mode "retrospective", status "draft", a non-empty Question, no stages and a hand-written Result whose lines end in "[post hoc]"
    When "tophat study report old-benchmark" is run
    Then the status is "reported"
    And the Result section is unchanged
    And "tophat check --strict" reports the legacy diagnostic "study.result-unverified" and exits 0

  @rq-44085c48
  Scenario: Hand-written retrospective results must be tagged post hoc
    Given study "old-benchmark" has mode "retrospective", status "draft", no stages and a hand-written Result line ending in "[pre-specified]"
    When "tophat study report old-benchmark" is run
    Then the command fails with a Summary error naming that line
    And the status remains "draft"

  # --- Supersession ---

  @rq-78ed7708
  Scenario: Supersede a reported study
    Given study "study-a" is reported and study "study-b" exists
    When "tophat study supersede study-a --by study-b" is run
    Then study "study-a" has status "superseded" and "superseded_by: study-b"
    And study "study-b" lists "study-a" under "supersedes"
    And "tophat check" reports no supersede diagnostics

  @rq-84fe523b
  Scenario: Superseded study keeps its frozen-section check
    Given study "study-a" was preregistered, then superseded by "study-b"
    When a frozen section of study "study-a" is edited
    And "tophat check" is run
    Then an error "study.frozen-modified" is reported for study "study-a"

  @rq-a9de1b5b
  Scenario: Supersede refuses a draft
    Given study "study-a" has status "draft" and study "study-b" exists
    When "tophat study supersede study-a --by study-b" is run
    Then the command fails with WrongStatus
    And neither study is changed

  @rq-765490e4
  Scenario: Supersede refuses a missing replacement
    Given study "study-a" is reported and no study "study-b" exists
    When "tophat study supersede study-a --by study-b" is run
    Then the command fails with StudyNotFound
    And study "study-a" is unchanged

  @rq-1aa65ab4
  Scenario: Superseded-by target does not exist
    Given study "study-a" has status "superseded" and "superseded_by: gone"
    And no study "gone" exists
    When "tophat check" is run
    Then an error "study.supersede-target-missing" is reported

  @rq-ddb6cfbc
  Scenario: Missing back-link
    Given study "study-a" has status "superseded" and "superseded_by: study-b"
    And study "study-b" does not list "study-a" under "supersedes"
    When "tophat check" is run
    Then an error "study.supersede-backlink-missing" is reported for study "study-a"

  @rq-ada3b70f
  Scenario: Supersedes entry without a matching forward link
    Given study "study-b" lists "study-a" under "supersedes"
    And study "study-a" has status "reported" with no "superseded_by"
    When "tophat check" is run
    Then an error "study.supersedes-invalid" is reported for study "study-b"

  @rq-7737c032
  Scenario: Supersede an exploratory reported study
    Given study "outlier-dig" has mode "exploratory", status "reported", and study "study-b" exists
    When "tophat study supersede outlier-dig --by study-b" is run
    Then study "outlier-dig" has status "superseded" and "superseded_by: study-b"

  # --- Replication creation ---

  @rq-0725f8c7
  Scenario: Replicate creates a study with a pinned target
    Given study "x-vs-docking" is reported at commit "<full-commit>"
    When "tophat study replicate rerun-score --question q-1a2b3c4d --of x-vs-docking@<full-commit>" is run
    Then "studies/rerun-score/STUDY.md" exists with mode "replication", status "draft" and "reproduces: x-vs-docking@<full-commit>"
    And it contains the governed headings "Question" and "Match criterion"
    And "tophat check" reports no study errors for "rerun-score"

  @rq-4c09754e
  Scenario: Replicate refuses a pin that is not a commit
    When "tophat study replicate rerun-score --question q-1a2b3c4d --of x-vs-docking@latest" is run
    Then the command fails with ReproducesInvalid
    And no directory is created

  @rq-2a8b349a
  Scenario: Replicate refuses a missing target
    When "tophat study replicate rerun-score --question q-1a2b3c4d --of no-such-study@<full-commit>" is run
    Then the command fails with ReproducesInvalid
    And no directory is created

  @rq-016794d2
  Scenario: Reproduction target missing on a replication study
    Given study "rerun-score" has mode "replication" and no "reproduces" field
    When "tophat check" is run
    Then an error "study.reproduces-missing" is reported

  @rq-a779cb78
  Scenario: Reproduces outside replication mode
    Given study "study-a" has mode "confirmatory" and "reproduces: x-vs-docking@<full-commit>"
    When "tophat check" is run
    Then an error "study.reproduces-unexpected" is reported

  @rq-ac7347b1
  Scenario: Reproduces with a malformed pin
    Given study "rerun-score" has mode "replication" and "reproduces: x-vs-docking@main"
    When "tophat check" is run
    Then an error "study.reproduces-invalid" is reported

  # --- Study links ---

  @rq-9c500c8f
  Scenario: Link records a follow-up relation
    Given studies "study-a" and "study-b" exist
    When "tophat study link study-b --follows-up study-a" is run
    Then study "study-b" lists "study-a" under "follows_up"
    And no other byte of its STUDY.md changed

  @rq-c2acb5c8
  Scenario: Link ignores duplicates and keeps the list sorted
    Given studies "study-a", "study-b" and "study-c" exist
    And study "study-c" lists ["study-b"] under "follows_up"
    When "tophat study link study-c --follows-up study-a" is run
    Then study "study-c" lists ["study-a", "study-b"] under "follows_up"

  @rq-2a6c36db
  Scenario: Link refuses a missing study
    When "tophat study link study-a --refutes no-such-study" is run
    Then the command fails with LinkInvalid
    And STUDY.md is unchanged

  @rq-2dd238fe
  Scenario: Link refuses the study itself
    When "tophat study link study-a --extends study-a" is run
    Then the command fails with LinkInvalid
    And STUDY.md is unchanged

  @rq-98115a64
  Scenario: A dangling relation entry
    Given study "study-a" lists ["gone"] under "refutes"
    When "tophat check" is run
    Then an error "study.link-invalid" is reported

  @rq-6422ad44
  Scenario: Links are not frozen
    Given study "study-b" is preregistered
    When "tophat study link study-b --follows-up study-a" is run
    And "tophat check" is run
    Then no "study.frozen-modified" error is reported

  # --- Migration ---

  @rq-014c049f
  Scenario: Migrate adds the mode of an existing study
    Given study "study-a" is reported and has no "mode" field
    When "tophat study migrate study-a" is run
    Then its front matter has "mode: confirmatory"
    And every other byte of STUDY.md is unchanged
    And "tophat check" reports no study errors for "study-a"
    And no "study.frozen-modified" error is reported

  @rq-c3ccc5f9
  Scenario: Migrate converts a reported pre-mode retrospective study
    Given study "old-benchmark" has "status: retrospective", a non-empty Result and no "mode" field
    When "tophat study migrate old-benchmark" is run
    Then its front matter has "mode: retrospective" and "status: reported"

  @rq-3c9a3c47
  Scenario: Migrate converts an in-progress pre-mode retrospective study
    Given study "old-benchmark" has "status: retrospective", an empty Result and no "mode" field
    When "tophat study migrate old-benchmark" is run
    Then its front matter has "mode: retrospective" and "status: draft"

  @rq-ed56d920
  Scenario: Migrate leaves a current study untouched
    Given study "study-a" has mode "confirmatory" and status "draft"
    When "tophat study migrate study-a" is run
    Then STUDY.md is byte-for-byte unchanged
    And the command says there was nothing to migrate

  # --- File safety ---

  @rq-607bbcc0
  Scenario: Commands preserve unrelated content
    Given study "study-a" is a ready draft whose STUDY.md contains a "## Background" section, HTML comments and unusual spacing
    When it is pre-registered interactively
    Then the only differences in STUDY.md are the status line and the added "preregistered" block

  @rq-1dd3778a
  Scenario: Commands write atomically
    Given study "study-a" is preregistered with a valid clean SUMMARY.md
    And writing the temporary file in "studies/study-a" fails with a permissions error
    When "tophat study report study-a" is run
    Then the command fails with an Io error
    And STUDY.md is unchanged
```
