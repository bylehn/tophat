# Feature: Study Document Format and Lifecycle <!-- rq-2db7a1dd -->

A study is the smallest unit of research in a TopHat project: it answers one question with one
pre-registered primary endpoint and decision rule, and runs end-to-end with its own pipeline. Its
`STUDY.md` is the study's requirements document. This file specifies the format of `STUDY.md`, the
study lifecycle and the commands that move a study through it, the generated `SUMMARY.md` that
results are copied from, and the `study` rule family evaluated by `tophat check` (see
`rqm/check.md`).

Pipeline construction (Snakefile, stage keys, provenance), the question tree, and data rules are
specified elsewhere. This file depends on them only through the interfaces stated below.

## Study Layout <!-- rq-fc05344f -->

```
studies/<study-id>/
  STUDY.md          the study document (this file's subject)
  SUMMARY.md        written by `tophat study summarize`; committed
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
| `question` | string | always | ID of the question in the project question tree this study answers; matches `^q-[0-9a-f]{8}$` |
| `status` | enum | always | `draft`, `preregistered`, `reported`, `superseded`, or `retrospective` |
| `visibility` | enum | always | `public` or `private` |
| `variants` | list of strings | always | Variant names; non-empty; each matches the study ID pattern; no duplicates |
| `preregistered` | mapping | see *Lifecycle* | `date` (`YYYY-MM-DD`) and `frozen_hash` (`sha256:` followed by 64 lowercase hex characters) |
| `superseded_by` | string | iff `status: superseded` | Study ID of the replacing study |
| `supersedes` | list of strings | optional | Study IDs this study replaces |

Unknown front-matter fields are a warning (they may be typos of known fields).

Whether the `question` ID exists in the question tree is checked by the question-tree feature, not
by this rule family.

### Body Sections <!-- rq-e9d4388a -->

Sections are `##` headings. Headings inside fenced code blocks are not headings. Section names are
matched exactly (case-sensitive). The governed sections are:

| Section | Frozen | Content |
|---|---|---|
| `## Question` | yes | The question, in prose |
| `## Primary endpoint` | yes | The single quantity that answers the question, and how it is measured |
| `## Decision rule` | yes | How the endpoint's value maps to an answer, including any thresholds |
| `## Scenarios` | yes | Testable expectations, as Gherkin (see *Scenarios*) |
| `## Variants` | no | Prose describing each variant listed in front matter |
| `## Stages` | no | Prose describing the pipeline stages; the machine-readable definition is in `config/` |
| `## Amendments` | no | Dated changes after pre-registration (see *Amendments*) |
| `## Result` | no | Result lines copied from `SUMMARY.md` by `tophat study report` |
| `## Interpretation` | no | Free human interpretation of the result |

- Every governed section heading must be present in every status, in any order; a section may be
  empty where the lifecycle allows.
- A governed heading must not appear more than once.
- Any other `##` sections (e.g. `## Background`) are allowed and not governed.
- A section's content is every line after its heading up to the next `##` heading outside a fenced
  code block, or the end of the file. `###` and deeper headings are part of the section.
- A section is **empty** if its content contains only whitespace and HTML comments.

### Scenarios <!-- rq-04595081 -->

- The `## Scenarios` section contains one or more ` ```gherkin ` fenced blocks.
- Each `Scenario:` (or `Scenario Outline:`) is immediately preceded, on its own line, by exactly one
  scenario tag of the form `@S<n>`, where `<n>` is a positive integer without leading zeros. Other
  Gherkin tags may appear on the same line. A tag is a scenario tag only if it is `@S` followed by
  digits only; tags such as `@Slow` or `@Smoke` are ordinary tags.
- Scenario numbers are unique within the study. Gaps are allowed (a removed draft scenario's number
  need not be reused).

Example:

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
- A body containing a `## Results` section whose content is one or more **result lines**, each a
  top-level list item `- <variant>/<stage>: <text>`, where `<variant>` is a listed variant and
  `<stage>` matches the study ID pattern.

## Frozen Fingerprint <!-- rq-08c7718c -->

The frozen fingerprint is computed from the study ID and the four frozen sections:

1. Start with the line `study:<study-id>`.
2. For each frozen section in the fixed order Question, Primary endpoint, Decision rule, Scenarios,
   append the line `## <section name>` followed by the section's normalized content.
3. Normalization: convert CRLF line endings to LF, strip trailing whitespace from each line, and
   remove leading and trailing blank lines from the section content.
4. Join all lines with LF, append a final LF, encode as UTF-8, and compute SHA-256.
5. The fingerprint is `sha256:` followed by the lowercase hex digest.

The fingerprint does not depend on the order of sections in the file, on non-frozen sections, or on
front matter other than the study ID.

## Lifecycle <!-- rq-f28e6ab8 -->

| Status | `preregistered` block | Frozen sections | Result | Tests |
|---|---|---|---|---|
| `draft` | absent | may change; may be empty | empty | orphan references warn |
| `preregistered` | present, fingerprint matches | non-empty, at least one scenario | empty | untested scenarios warn; orphans error |
| `reported` | present, fingerprint matches | non-empty, at least one scenario | equals `SUMMARY.md` result lines; summary clean | every scenario tested; orphans error |
| `retrospective` | absent | Question and Primary endpoint non-empty; others may be empty | from `SUMMARY.md` if it exists, otherwise hand-written and flagged legacy | orphans error |
| `superseded` | as before supersession | as before supersession | as before supersession | as before supersession |

Transitions:

- `draft` → `preregistered`: only by `tophat study preregister`, run by a human.
- `preregistered` → `reported`: by `tophat study report`.
- `preregistered`, `reported` or `retrospective` → `superseded`: by `tophat study supersede`.
- A `draft` study is never superseded; it is edited or deleted instead.
- A `retrospective` study never becomes `preregistered` or `reported`.

`tophat check` validates the current state against the table; it does not reconstruct history. A
`superseded` study is validated by the rules of the status it had before supersession, which is
inferred: `preregistered` block present and Result non-empty → `reported`; block present and
Result empty → `preregistered`; block absent → `retrospective`.

## Feature API <!-- rq-3fd30bb8 -->

### Commands <!-- rq-2dd95e98 -->

- `tophat study new <study-id> --question <question-id> [--visibility public|private]` <!-- rq-f689a538 -->
  - Creates `studies/<study-id>/` containing `STUDY.md` (status `draft`, `variants: [base]`,
    visibility defaulting to `public`, every governed section heading present and empty except for
    template guidance comments), `config/base.yaml` (containing `stages: []`), the `Snakefile`
    stub, and an empty `tests/` directory.
  - Fails without writing anything if the ID is invalid, the directory already exists, or the
    question ID does not match the question ID pattern.
- `tophat study preregister <study-id>` <!-- rq-bcb83d0a -->
  - Requires standard input and standard output to be interactive terminals; otherwise fails
    without changes, explaining that pre-registration must be done by a human.
  - Requires status `draft` and that the study has no `error` diagnostics from the `study` rule
    family when evaluated as if it were `preregistered` with a matching fingerprint (non-empty
    frozen sections, at least one scenario, valid tags, variant configs consistent).
  - Prints the frozen sections, then asks the user to type the study ID to confirm. Any other input
    aborts without changes.
  - On confirmation, sets `status: preregistered` and writes `preregistered.date` (current local
    date) and `preregistered.frozen_hash`. Changes nothing else in the file.
  - Prints a reminder to commit `STUDY.md` before running any answer-stage computation.
- `tophat study report <study-id>` <!-- rq-ecacda48 -->
  - Requires status `preregistered` or `reported`, or status `retrospective` with a `SUMMARY.md`.
  - Requires `SUMMARY.md` to exist, parse, name this study, and have `dirty: false`; every result
    line's variant must be listed in front matter.
  - Requires the summary not to be stale (its `keys`, when present, equal the current stage keys;
    see `pipeline.summary-stale` in `rqm/pipeline/snakemake.md`), and the study not to depend on
    unreported work without an accepting amendment (see `rqm/data.md`).
  - Replaces the content of `## Result` with the summary's result lines, verbatim and in order.
  - Sets status `reported` if it was `preregistered`; otherwise leaves status unchanged.
  - Changes nothing else in the file. Does not require an interactive terminal.
- `tophat study supersede <old-id> --by <new-id>` <!-- rq-9f2c5cd1 -->
  - Requires both studies to exist, `<old-id>` to have status `preregistered`, `reported` or
    `retrospective`, and `<new-id>` to differ from `<old-id>`.
  - Sets `<old-id>`'s status to `superseded` and `superseded_by: <new-id>`, and adds `<old-id>` to
    `<new-id>`'s `supersedes` list if not already present.

All commands that modify `STUDY.md` preserve every byte of the file outside the fields and section
they are specified to change, and write the file atomically (write to a temporary file in the same
directory, then rename).

### Types <!-- rq-dacbc76e -->

- `Study` — a parsed study: ID, front-matter fields, governed sections with their content and <!-- rq-8071e60f -->
  starting line numbers, scenarios (number, title, line), and amendments (date, results seen,
  reason, line).
- `Status` — enum `Draft`, `Preregistered`, `Reported`, `Superseded`, `Retrospective`. <!-- rq-5e9d0d62 -->
- `Summary` — a parsed `SUMMARY.md`: study ID, commit, dirty flag, generated timestamp, tophat <!-- rq-6764653f -->
  version, optional keys, and result lines (variant, stage, text, line).
- `StudyError` — error type returned by parsing and by the study commands, with at minimum: <!-- rq-dba2a6b7 -->
  - `InvalidStudyId(String)`.
  - `StudyExists(String)` — `new` on an existing directory.
  - `StudyNotFound(String)`.
  - `FrontMatter { path: String, message: String }` — missing delimiters, invalid YAML, missing or
    mistyped fields.
  - `WrongStatus { study: String, status: Status, required: Vec<Status> }`.
  - `NotInteractive` — `preregister` without an interactive terminal.
  - `ConfirmationMismatch` — the typed ID did not match.
  - `NotReady(Vec<Diagnostic>)` — `preregister` precondition violations.
  - `Summary(String)` — `report` with a missing, unparseable, dirty, or mismatched `SUMMARY.md`.
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
| `study.field-missing` | error | A required front-matter field is absent |
| `study.field-invalid` | error | `title` empty, `question` malformed, `status`/`visibility` not an allowed value, `variants` empty, duplicated or malformed |
| `study.field-unknown` | warning | Unknown front-matter field |
| `study.section-missing` | error | A governed section heading is absent |
| `study.section-duplicate` | error | A governed section heading appears more than once |
| `study.section-empty` | error | A section required non-empty by the lifecycle table is empty |
| `study.prereg-missing` | error | Status requires a `preregistered` block and it is absent or incomplete |
| `study.prereg-unexpected` | error | Status `draft` or `retrospective` (or superseded from it) has a `preregistered` block |
| `study.frozen-modified` | error | The recomputed fingerprint differs from `preregistered.frozen_hash` |
| `study.scenario-untagged` | error | A scenario lacks exactly one `@S<n>` tag |
| `study.scenario-tag-invalid` | error | A tag is `@S` followed by digits but is not a valid scenario tag (e.g. `@S0`, `@S01`) |
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
| `study.result-unexpected` | error | `## Result` is non-empty in `draft` or `preregistered` |
| `study.result-missing` | error | `## Result` is empty in `reported` |
| `study.summary-missing` | error | Result must come from `SUMMARY.md` and it is absent |
| `study.summary-invalid` | error | `SUMMARY.md` cannot be parsed, names another study, or has a result line with an unlisted variant |
| `study.summary-dirty` | error | Result is copied from a `SUMMARY.md` with `dirty: true` |
| `study.result-mismatch` | error | `## Result` lines differ from `SUMMARY.md` result lines (compared verbatim after stripping trailing whitespace, in order) |
| `study.result-unverified` | legacy | A `retrospective` study has a non-empty Result and no `SUMMARY.md` |
| `study.supersede-target-missing` | error | `superseded_by` names a study that does not exist |
| `study.supersede-backlink-missing` | error | The replacing study does not list this study in `supersedes` |
| `study.supersedes-invalid` | error | A `supersedes` entry names a study that does not exist or whose `superseded_by` is not this study |

## Assumptions <!-- rq-c5416f0d -->

- Question IDs use the format `q-` followed by 8 lowercase hex characters, matching the opaque-ID
  style of riprap. The question-tree feature owns their assignment and resolution.
- "Current date" for amendment validation and pre-registration is the local date of the machine
  running `tophat`.

## Out of Scope <!-- rq-87476c9b -->

- The question tree (`PROJECT.md`) and whether `question` resolves.
- Pipeline construction, stage keys and provenance, including how the summary stage computes result
  lines and whether code changed after the run that produced `SUMMARY.md`.
- Data, visibility-flow, and external-stage rules.
- Running scenario tests (the pipeline's `check` stage does this).
- Validating the content of configuration files.

## Gherkin Scenarios <!-- rq-fbdc50b4 -->

```gherkin
Feature: Study document format, lifecycle and check rules

  Background:
    Given a TopHat project
    And question "q-1a2b3c4d" exists in the question tree

  # --- tophat study new ---

  @rq-52e3a974
  Scenario: Create a new study
    When "tophat study new x-vs-docking --question q-1a2b3c4d" is run
    Then "studies/x-vs-docking/STUDY.md" exists with status "draft", visibility "public" and variants ["base"]
    And it contains every governed section heading
    And "studies/x-vs-docking/config/base.yaml" exists and contains "stages: []"
    And "studies/x-vs-docking/Snakefile" equals the stub for "x-vs-docking"
    And the directory "studies/x-vs-docking/tests" exists
    And "tophat check" reports no study errors for "x-vs-docking"

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

  # --- Scenarios and tests ---

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
      "- base/score: n=120, rho=0.61 [0.49, 0.71], p<0.001" and "- b3lyp/score: n=120, rho=0.58 [0.45, 0.69], p<0.001"
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
  Scenario: Report refuses a draft
    Given study "study-a" has status "draft" and a valid SUMMARY.md
    When "tophat study report study-a" is run
    Then the command fails with WrongStatus

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
    And SUMMARY.md contains the result line "- pbe/score: n=10, rho=0.3"
    When "tophat check" is run
    Then an error "study.summary-invalid" is reported

  # --- Retrospective studies ---

  @rq-c55140e8
  Scenario: Retrospective study with hand-written result is flagged as legacy
    Given study "old-benchmark" has status "retrospective", non-empty Question and Primary endpoint, a non-empty Result, and no SUMMARY.md
    When "tophat check --strict" is run
    Then a legacy diagnostic "study.result-unverified" is reported
    And no error or warning is reported for "old-benchmark"
    And the exit code is 0

  @rq-c985c1d6
  Scenario: Retrospective study with a pipeline uses strict results
    Given study "old-benchmark" has status "retrospective" and a SUMMARY.md
    And its Result section differs from the summary's result lines
    When "tophat check" is run
    Then an error "study.result-mismatch" is reported

  @rq-c65bf680
  Scenario: Retrospective study may have empty decision rule and scenarios
    Given study "old-benchmark" has status "retrospective" with empty Decision rule and Scenarios sections
    When "tophat check" is run
    Then no "study.section-empty" or "study.no-scenarios" diagnostic is reported

  @rq-647102ae
  Scenario: Retrospective study cannot carry a pre-registration block
    Given study "old-benchmark" has status "retrospective" and a "preregistered" block
    When "tophat check" is run
    Then an error "study.prereg-unexpected" is reported

  @rq-3a79a3f7
  Scenario: Retrospective study can be reported from a summary
    Given study "old-benchmark" has status "retrospective" and a clean SUMMARY.md
    When "tophat study report old-benchmark" is run
    Then the Result section equals the summary's result lines
    And the status remains "retrospective"

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
