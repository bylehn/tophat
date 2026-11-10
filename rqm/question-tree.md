# Feature: Question Tree <!-- rq-3650b3f0 -->

The question tree is the top level of a TopHat project's research hierarchy. `PROJECT.md` records
the project's overarching research question broken down into sub-questions, each with a stable ID.
Studies (see `rqm/study/format.md`) attach to questions through their `question:` field. A question
is answered by citing evidence: reported studies — for their pre-specified results, or, with a
label, for their exploratory or post hoc ones — or the answers of its own sub-questions. Findings
raise follow-up questions: a question born from a study's results is recorded as **emerged**,
carries the study that produced it as its origin, and is otherwise an open question.

This file specifies the format of `PROJECT.md`, the `question` rule family evaluated by
`tophat check` (see `rqm/check.md`), and the `tophat question` commands that assign IDs, record
emerged questions and display the tree.

## PROJECT.md Format <!-- rq-ba118d75 -->

`PROJECT.md` lives at the project root. It has no front matter. A project without `PROJECT.md` has
an empty question tree.

### Title and preamble <!-- rq-44727473 -->

The first `#` heading is the project title. Text before the first question is free-form project
description and is not governed. `#` headings are never questions.

### Question headings <!-- rq-8148f10b -->

Every `##` to `######` heading outside a fenced code block is a question. Depth in the tree is the
heading level: `##` questions are top-level, and each deeper heading is a sub-question of the
nearest preceding question one level shallower.

A **stamped** question heading has the form:

```
<hashes> q-<8 lowercase hex> · <question text>
```

The separator is a middle dot `·` (U+00B7) surrounded by single spaces. The question text is
non-empty. A heading that does not start with `q-` followed by a space-free token and ` · ` is
**unstamped**. A heading that starts with `q-<token> · ` where the token is not 8 lowercase hex
characters has a **malformed ID**.

Structure rules:

- The first question is a `##` heading.
- A question's level is at most one deeper than the previous question's level (no skipped levels).
- Question IDs are unique within `PROJECT.md`.
- Descriptive sub-headings are not available inside a question; use bold text instead.

### Question body <!-- rq-76dbbbfa -->

A question's body is every line after its heading up to the next heading of any level (outside a
fenced code block) or the end of the file.

- The first non-blank line of the body is the **status line**: `status: open`, `status: emerged`,
  `status: answered`, or `status: dropped`.
- An `answered` question has:
  - an **answer line**: a line starting with `**Answer:**` followed by non-empty text; and
  - an **evidence line**: a line starting with `Evidence:` followed by a comma-separated, non-empty
    list of citations (see *Evidence*).
- A `dropped` question has a **reason line**: a line starting with `**Reason:**` followed by
  non-empty text.
- An `emerged` question has an **origin line**: a line starting with `Emerged from:` followed by
  the ID of one study of this project.
- Any other body text is free-form and not governed.

Example:

```markdown
# Method X for binding affinity

Three-year project funded by ...

## q-1a2b3c4d · Can method X improve affinity prediction for our targets?
status: open

### q-5e6f7a8b · Is X's interaction energy accurate against reference data?
status: answered

**Answer:** Yes. Mean absolute error is 0.8 kcal/mol on the benchmark set.
Evidence: x-energy-accuracy

### q-9c0d1e2f · Does X rank binders better than docking?
status: open

#### q-3b4c5d6e · Does the advantage hold for client targets?
status: dropped

**Reason:** The client withdrew the target set.

#### q-7d8e9f0a · What is going on with the outlier structures?
status: emerged

Emerged from: x-vs-docking
```

## Evidence <!-- rq-ef5ab539 -->

A citation names a study or a question. A study citation states what kind of evidence it cites,
through the same tags result lines carry (see `rqm/study/format.md`):

- `<study-id>` — cites the study's results tagged `pre-specified`.
- `<study-id> [exploratory]` or `<study-id> [post hoc]` — cites the study's results carrying that
  tag.

A question citation is a bare question ID and carries no label. Citations are checked as follows.

| Citation | Requirement | Otherwise |
|---|---|---|
| Study ID | A study directory `studies/<id>/` with `STUDY.md` exists | error |
| Study ID | The study's status is `reported` | `draft` or `preregistered`: error; `superseded`: error |
| Study ID | The study's `## Result` section carries a line matching the citation: tagged `pre-specified` when the citation is unlabelled, tagged the citation's label otherwise | error |
| Study ID | Not a `retrospective` mode study whose result lines are hand-written without a `SUMMARY.md` | legacy |
| Study ID | The study's `question` is this question or one of its descendants | warning |
| Question ID | The question exists in the tree and is a descendant of the citing question | error |
| Question ID | The cited question has status `answered` | error |

A citation that matches neither form — a malformed identifier, a label other than `exploratory` or
`post hoc`, or a label on a question citation — is an error. A cited study whose `STUDY.md` cannot
be parsed is reported by the `study` rule family and not checked further here.

**Label matching.** A cited study's tags are read from the `## Result` section of its `STUDY.md`,
so a label is never trusted that the study's own results do not carry. Exploratory and
retrospective mode studies carry no `pre-specified` result lines (see `rqm/study/format.md`), so
citing them without a label is an error: their evidence is exploratory or post hoc and is cited as
such. A `reported` study whose result lines include a line without a valid tag group is left to
the `study` rule family (`study.tag-missing`) and its citations are not label-checked.

## Emerged Questions <!-- rq-7e178b97 -->

A question born from a study's results is recorded as **emerged**: it carries the study whose
results raised it as its origin, and is otherwise an open question. It needs neither an answer nor
evidence; a study may address it through the study's `question` field like any other question; and
it is answered or dropped like an open question, its origin line remaining in the body as history.

- The origin names one study of this project, whatever its status: a follow-up seen in a draft's
  trial results is recorded as faithfully as one seen in a reported result. The origin says where
  a question came from, not what may be claimed from it; what may be claimed is governed by
  *Evidence*.
- An origin line is required while the status is `emerged` and, wherever present at any status,
  names an existing study.
- Emerged questions and their origin lines are written by `tophat question emerge`, never by hand.
  `tophat check` validates their structure the way it validates answers, which are written by
  editing `PROJECT.md` directly.

## Study Links <!-- rq-a147f8cf -->

Every study whose `STUDY.md` parses and has a well-formed `question` field must name a question that
exists in the tree. The diagnostic is reported on the study's `STUDY.md`. A study may name any
question, at any depth, including one with sub-questions; an `emerged` question is nameable like
any other. The link from an emerged question to the study that produced it is its origin line (see
*Emerged Questions*); links between studies live in study front matter and are specified in
`rqm/study/format.md`.

## Check Rules (`question` family) <!-- rq-ceffcdab -->

Line numbers point at the offending heading, status line, answer line, evidence line, origin line
or reason line in `PROJECT.md`, or at the `question` field of a study.

| Rule | Severity | Condition |
|---|---|---|
| `question.parse-error` | error | `PROJECT.md` exists but cannot be read as UTF-8 text; no further `question` rules run on it, and study links are not checked |
| `question.unstamped` | error | A question heading has no ID |
| `question.id-invalid` | error | A question heading has a malformed ID |
| `question.text-empty` | error | A stamped question heading has no question text |
| `question.duplicate-id` | error | An ID is used by more than one question (reported at every occurrence after the first) |
| `question.level-skip` | error | The first question is not `##`, or a question is more than one level deeper than the previous question |
| `question.status-missing` | error | The body's first non-blank line is not a status line |
| `question.status-invalid` | error | The status line's value is not `open`, `emerged`, `answered` or `dropped` |
| `question.answer-missing` | error | An `answered` question has no answer line with text |
| `question.evidence-missing` | error | An `answered` question has no evidence line, or it cites nothing |
| `question.evidence-invalid` | error | A citation is malformed — a label other than `exploratory` or `post hoc`, or a label on a question citation — names a study or question that does not exist, or names a question that is not a descendant |
| `question.evidence-unreported` | error | A cited study is `draft` or `preregistered` |
| `question.evidence-superseded` | error | A cited study is `superseded` |
| `question.evidence-tag-mismatch` | error | A citation names a `reported` study whose `## Result` carries no line with the citation's tag (`pre-specified` when unlabelled); the hint names the tags the study's result lines do carry. Not checked for a study with a result line lacking a valid tag group |
| `question.evidence-retrospective` | legacy | A cited `reported` study is in `retrospective` mode and has no `SUMMARY.md` (its result lines are hand-written and unverified) |
| `question.evidence-unrelated` | warning | A cited study's `question` is not this question or one of its descendants |
| `question.evidence-unanswered` | error | A cited question is `open`, `emerged` or `dropped` |
| `question.reason-missing` | error | A `dropped` question has no reason line with text |
| `question.origin-missing` | error | An `emerged` question has no origin line with text |
| `question.origin-invalid` | error | An origin line's value is not an existing study's ID |
| `question.unknown-reference` | error | A study's `question` names an ID that is not in the tree (reported on the study's `STUDY.md`) |

## Feature API <!-- rq-4e7bac77 -->

### Commands <!-- rq-148e9ae7 -->

- `tophat question stamp` <!-- rq-1513da11 -->
  - Assigns a fresh random ID to every unstamped question heading, rewriting it as
    `<hashes> q-<id> · <original heading text>`. A fresh ID is unique among all IDs already in
    `PROJECT.md` and those assigned in the same run.
  - For every question (stamped before or by this run) whose body has no status line, inserts
    `status: open` as the line immediately after the heading.
  - Leaves headings with a malformed ID and duplicated IDs unchanged, lists them, and exits
    non-zero after writing the other changes.
  - Preserves every other byte of `PROJECT.md` and writes it atomically. Prints one line per
    stamped question (`q-<id> <text>`). With nothing to do, leaves the file untouched.
  - Fails without changes when `PROJECT.md` does not exist.
- `tophat question emerge "<text>" --from <study-id> [--under <question-id>]` <!-- rq-7449b1fb -->
  - Records a question born from the results of the study `<study-id>`, as specified under
    *Emerged Questions*. Fails without changes when:
    - `PROJECT.md` does not exist (`ProjectFileMissing`);
    - the text is empty;
    - the study does not exist or its `STUDY.md` cannot be parsed;
    - the parent question — `<question-id>` when given, otherwise the study's `question` — is not
      a stamped question in the tree;
    - the parent question is at level 6, where no deeper heading exists.
  - On success, inserts one question as the parent's last sub-question: the heading
    `<hashes> q-<id> · <text>` at one level deeper than the parent, immediately followed by the
    lines `status: emerged` and `Emerged from: <study-id>` and one blank line, placed immediately
    before the next heading outside fenced code blocks at a level at most the parent's, or at the
    end of the file. The ID is fresh, unique among all IDs already in `PROJECT.md`. Inserted
    lines use the file's line endings; every other byte of `PROJECT.md` is preserved and the file
    is written atomically.
  - Prints `q-<id> <text>`.
- `tophat question tree [--json]` <!-- rq-5d07156c -->
  - Prints the tree read-only: one line per question, indented two spaces per level below `##`, as
    `q-<id> [<status>] <text>`, followed by one indented line per attached study as
    `study <study-id> [<study status>]` (sorted by study ID), then, when the question's body has
    an origin line, the line `emerged from <study-id> [<study status>]`; these lines precede the
    question's sub-questions. Studies are listed under the question their `question` field names.
    A question without a valid ID shows `(no id)` in place of the ID; a missing or invalid status
    shows as `?`; a study or origin study whose status cannot be read shows `?`.
  - With `--json`, prints `{"questions": [...]}` where each question object has `id`, `text`,
    `status`, `level`, `parent` (ID or `null`), `line`, `studies` (a list of
    `{"id", "status"}`), and `origin` — `null`, or `{"id", "status"}` when the question's body has
    an origin line naming an existing study (`status` is `null` when that study's status cannot
    be read) — in file order. Unstamped questions appear with `id: null`.
  - Exits 0. It does not validate; `tophat check` does.

### Types <!-- rq-563ae19a -->

- `QuestionStatus` — enum `Open`, `Emerged`, `Answered`, `Dropped`. <!-- rq-f448ed00 -->
- `Citation` — one entry of an evidence line: `target: String` (a study ID or a question ID) and <!-- rq-ddd0dee1 -->
  `label: Option<String>` — `Some("exploratory")` or `Some("post hoc")` on a labelled study
  citation, `None` otherwise.
- `Question` — a question parsed from `PROJECT.md`: <!-- rq-e956ef6a -->
  - `id: Option<String>` — `None` when unstamped or malformed.
  - `text: String`, `level: u8` (2–6), `heading_line: u32`.
  - `parent: Option<usize>` — index of the parent question in the tree.
  - `status: Option<(String, u32)>` — raw status value and its line, when a status line exists.
  - `origin: Option<(String, u32)>` — raw origin value and its line, when an origin line exists.
  - `answer: Option<(String, u32)>`, `reason: Option<(String, u32)>`.
  - `evidence: Option<(Vec<Citation>, u32)>` — citations and the evidence line.
- `QuestionTree` — the questions of `PROJECT.md` in file order, plus the problems found while <!-- rq-dcafcef0 -->
  parsing the structure (unstamped and malformed headings, level skips).
- `QuestionError` — error type of the commands, with at minimum: <!-- rq-7bde5706 -->
  - `ProjectFileMissing` — `PROJECT.md` does not exist (`stamp`, `emerge`).
  - `Unresolved(Vec<String>)` — `stamp` left malformed or duplicated IDs unchanged.
  - `TextInvalid` — `emerge` with an empty question text.
  - `OriginStudyInvalid(String)` — `emerge`'s `--from` study does not exist or its `STUDY.md`
    cannot be parsed.
  - `ParentInvalid(String)` — `emerge`'s parent question (`--under`, or the study's `question`) is
    not a stamped question in the tree.
  - `ParentTooDeep` — `emerge`'s parent question is at level 6.
  - `Io(String)`.

### Functions <!-- rq-d4c65f91 -->

- `parse_project(text: &str) -> QuestionTree` — parses `PROJECT.md` text; never fails, recording <!-- rq-0e66003f -->
  structural problems in the tree.
- `question_rules(project: &Project) -> Vec<Diagnostic>` — the `question` rule family registered <!-- rq-cc797bfb -->
  in `RULE_FAMILIES`.
- `stamp(root: &Path) -> Result<Vec<(String, String)>, QuestionError>` — performs <!-- rq-ef3d43f8 -->
  `tophat question stamp`, returning the `(id, text)` pairs it assigned.
- `emerge(root: &Path, text: &str, from: &str, under: Option<&str>) -> Result<(String, String), QuestionError>` — <!-- rq-a88520bf -->
  performs `tophat question emerge`, returning the `(id, text)` pair it assigned.

## Assumptions <!-- rq-2b3be8cc -->

- Every `##`–`######` heading is a question. This keeps the tree unambiguous at the cost of not
  allowing descriptive sub-headings inside a question.
- A project may have more than one top-level question; nothing requires a single root.
- Question IDs are random rather than derived from position or text, so reorganizing the tree never
  changes them. Randomness need not be cryptographic; uniqueness is checked explicitly.
- A study attached to a `dropped` question is not flagged.
- An emerged question differs from an open one only in carrying its origin line: every rule that
  applies to an open question applies to it, `tophat status` counts it under `emerged`
  (see `rqm/workflow.md`), a study may address it, and it may be answered or dropped.
- An origin line names one study at any status; an origin records where a question came from, not
  what may be claimed from it.
- Citation labels mirror the epistemic tags of result lines (`rqm/study/format.md`); an unlabelled
  citation cites a study's `pre-specified` results. `tophat check` reads the cited study's result
  lines, so a label is never trusted that the study's own results do not carry.

## Out of Scope <!-- rq-7aba8e5c -->

- Questions in other TopHat projects, and private questions in a companion repository.
- Creating questions with a command, other than emerged questions: ordinary questions are written
  as Markdown headings and stamped.
- Moving or deleting questions with a command.
- Study-to-study relation fields and the commands that write them (`rqm/study/format.md`);
  addressing an emerged question with a new study records no `follows_up` relation by itself.
- Any rendering of `PROJECT.md` beyond `tophat question tree`.

## Gherkin Scenarios <!-- rq-e746f1bb -->

```gherkin
Feature: Question tree

  Background:
    Given a TopHat project

  # --- Structure ---

  @rq-e75cad86
  Scenario: A well-formed tree passes the check
    Given PROJECT.md contains the example tree from this specification
    And study "x-energy-accuracy" is reported with question "q-5e6f7a8b" and a result line tagged "pre-specified"
    And study "x-vs-docking" is a draft with question "q-9c0d1e2f"
    When "tophat check" is run
    Then no "question" diagnostic is reported

  @rq-5ef3ab11
  Scenario: A project without PROJECT.md has an empty tree
    Given PROJECT.md does not exist
    And there are no studies
    When "tophat check" is run
    Then no "question" diagnostic is reported

  @rq-c18830cc
  Scenario: Unstamped question heading
    Given PROJECT.md contains the heading "## Can X improve affinity prediction?"
    When "tophat check" is run
    Then an error "question.unstamped" is reported at that heading

  @rq-cfed6415
  Scenario: Malformed question ID
    Given PROJECT.md contains the heading "## q-12 · Can X improve affinity prediction?"
    When "tophat check" is run
    Then an error "question.id-invalid" is reported at that heading

  @rq-17514314
  Scenario: Stamped heading without text
    Given PROJECT.md contains the heading "## q-1a2b3c4d · "
    When "tophat check" is run
    Then an error "question.text-empty" is reported

  @rq-60a2fa37
  Scenario: Duplicate question IDs
    Given PROJECT.md contains two questions with ID "q-1a2b3c4d"
    When "tophat check" is run
    Then an error "question.duplicate-id" is reported at the second heading only

  @rq-1a1c0293
  Scenario: First question is not a level-two heading
    Given the first question heading in PROJECT.md is "### q-1a2b3c4d · Sub-question first?"
    When "tophat check" is run
    Then an error "question.level-skip" is reported at that heading

  @rq-a4669866
  Scenario: Skipped heading level
    Given PROJECT.md has a "##" question followed directly by a "####" question
    When "tophat check" is run
    Then an error "question.level-skip" is reported at the "####" heading

  @rq-55f9341a
  Scenario: Returning to a shallower level is allowed
    Given PROJECT.md has questions at levels "##", "###", "####", then "##"
    When "tophat check" is run
    Then no "question.level-skip" diagnostic is reported

  @rq-1dd39a2c
  Scenario: Headings in code fences are not questions
    Given PROJECT.md contains a fenced code block with the line "## Not a question"
    When "tophat check" is run
    Then no "question.unstamped" diagnostic is reported

  @rq-1f183c28
  Scenario: Parent is the nearest shallower question
    Given PROJECT.md has "## q-1a2b3c4d · A?", "### q-5e6f7a8b · B?", "#### q-3b4c5d6e · C?", "### q-9c0d1e2f · D?"
    When the tree is parsed
    Then the parent of "q-3b4c5d6e" is "q-5e6f7a8b"
    And the parent of "q-9c0d1e2f" is "q-1a2b3c4d"
    And "q-1a2b3c4d" has no parent

  # --- Status, answers and reasons ---

  @rq-86db826d
  Scenario: Missing status line
    Given a stamped question whose first body line is "Some description"
    When "tophat check" is run
    Then an error "question.status-missing" is reported at the question heading

  @rq-a85bdde8
  Scenario: Invalid status value
    Given a stamped question with the status line "status: done"
    When "tophat check" is run
    Then an error "question.status-invalid" is reported at the status line

  @rq-0e435215
  Scenario: Answered question without an answer
    Given a stamped question with status "answered" and an evidence line but no answer line
    When "tophat check" is run
    Then an error "question.answer-missing" is reported

  @rq-ea89d221
  Scenario: Answered question without evidence
    Given a stamped question with status "answered" and an answer line but no evidence line
    When "tophat check" is run
    Then an error "question.evidence-missing" is reported

  @rq-b358b21a
  Scenario: Empty evidence list
    Given an answered question with the line "Evidence: "
    When "tophat check" is run
    Then an error "question.evidence-missing" is reported

  @rq-9d3f67c8
  Scenario: Dropped question without a reason
    Given a stamped question with status "dropped" and no reason line
    When "tophat check" is run
    Then an error "question.reason-missing" is reported

  @rq-427db370
  Scenario: Open question needs neither answer nor evidence
    Given a stamped question with status "open" and no other body lines
    When "tophat check" is run
    Then no "question" diagnostic is reported for that question

  # --- Emerged questions ---

  @rq-f36436b8
  Scenario: An emerged question needs neither answer nor evidence
    Given a stamped question with status "emerged" and the origin line "Emerged from: x-vs-docking"
    And study "x-vs-docking" exists
    When "tophat check" is run
    Then no "question" diagnostic is reported for that question

  @rq-e75df1d9
  Scenario: Emerged question without an origin
    Given a stamped question with status "emerged" and no origin line
    When "tophat check" is run
    Then an error "question.origin-missing" is reported at the question heading

  @rq-1b7579af
  Scenario: Emerged question naming a study that does not exist
    Given a stamped question with status "emerged" and the origin line "Emerged from: no-such-study"
    When "tophat check" is run
    Then an error "question.origin-invalid" is reported at the origin line

  @rq-273c5387
  Scenario: An emerged question is answered like an open one
    Given question "q-9c0d1e2f" is answered with "Evidence: outlier-dig [exploratory]"
    And its body carries the origin line "Emerged from: x-vs-docking"
    And exploratory study "outlier-dig" is reported with question "q-9c0d1e2f" and the result line "- base/analyse: rho=0.42 [exploratory]"
    And study "x-vs-docking" exists
    When "tophat check" is run
    Then no "question" diagnostic is reported for "q-9c0d1e2f"

  @rq-a5a6c56e
  Scenario: An emerged question is dropped like an open one
    Given question "q-9c0d1e2f" is dropped with a reason
    And its body carries the origin line "Emerged from: x-vs-docking"
    And study "x-vs-docking" exists
    When "tophat check" is run
    Then no "question" diagnostic is reported for "q-9c0d1e2f"

  @rq-ea31a640
  Scenario: An answered question keeps a valid origin line
    Given question "q-9c0d1e2f" is answered with "Evidence: x-energy-accuracy"
    And its body carries the origin line "Emerged from: x-vs-docking"
    And study "x-energy-accuracy" is reported with question "q-9c0d1e2f" and a result line tagged "pre-specified"
    And study "x-vs-docking" exists
    When "tophat check" is run
    Then no "question" diagnostic is reported for "q-9c0d1e2f"

  # --- Evidence ---

  @rq-9cadbb01
  Scenario: Citing a reported study about this question
    Given question "q-5e6f7a8b" is answered with "Evidence: x-energy-accuracy"
    And study "x-energy-accuracy" is reported with question "q-5e6f7a8b" and a result line tagged "pre-specified"
    When "tophat check" is run
    Then no "question.evidence" diagnostic is reported

  @rq-cffe1a61
  Scenario: Citing a reported study about a descendant question
    Given question "q-1a2b3c4d" is answered with "Evidence: x-energy-accuracy"
    And "q-5e6f7a8b" is a sub-question of "q-1a2b3c4d"
    And study "x-energy-accuracy" is reported with question "q-5e6f7a8b" and a result line tagged "pre-specified"
    When "tophat check" is run
    Then no "question.evidence-unrelated" diagnostic is reported

  @rq-fd01ec6f
  Scenario: Citing a study that does not exist
    Given question "q-5e6f7a8b" is answered with "Evidence: no-such-study"
    When "tophat check" is run
    Then an error "question.evidence-invalid" is reported at the evidence line

  @rq-544bf01b
  Scenario: Citing a malformed identifier
    Given question "q-5e6f7a8b" is answered with "Evidence: X_Study"
    When "tophat check" is run
    Then an error "question.evidence-invalid" is reported

  @rq-c7e4a9d5
  Scenario: Citing a study that is only pre-registered
    Given question "q-5e6f7a8b" is answered with "Evidence: x-energy-accuracy"
    And study "x-energy-accuracy" is preregistered with question "q-5e6f7a8b"
    When "tophat check" is run
    Then an error "question.evidence-unreported" is reported

  @rq-384cc812
  Scenario: Citing a draft study
    Given question "q-5e6f7a8b" is answered with "Evidence: x-energy-accuracy"
    And study "x-energy-accuracy" is a draft with question "q-5e6f7a8b"
    When "tophat check" is run
    Then an error "question.evidence-unreported" is reported

  @rq-9f4d2081
  Scenario: Citing a superseded study
    Given question "q-5e6f7a8b" is answered with "Evidence: x-energy-accuracy"
    And study "x-energy-accuracy" is superseded by study "x-energy-accuracy-v2"
    When "tophat check" is run
    Then an error "question.evidence-superseded" is reported
    And its hint names "x-energy-accuracy-v2"

  @rq-d47955a2
  Scenario: Citing an exploratory study with its label
    Given question "q-9c0d1e2f" is answered with "Evidence: outlier-dig [exploratory]"
    And exploratory study "outlier-dig" is reported with question "q-9c0d1e2f" and the result line "- base/analyse: rho=0.42 [exploratory]"
    When "tophat check" is run
    Then no "question.evidence" diagnostic is reported

  @rq-94b72fce
  Scenario: Citing an exploratory study without a label
    Given question "q-9c0d1e2f" is answered with "Evidence: outlier-dig"
    And exploratory study "outlier-dig" is reported with question "q-9c0d1e2f" and the result line "- base/analyse: rho=0.42 [exploratory]"
    When "tophat check" is run
    Then an error "question.evidence-tag-mismatch" is reported at the evidence line
    And its hint names "exploratory"

  @rq-9e04442c
  Scenario: Citing a confirmatory study's exploratory result
    Given question "q-9c0d1e2f" is answered with "Evidence: x-vs-docking [exploratory]"
    And study "x-vs-docking" is reported with question "q-9c0d1e2f" and the result lines "- base/score: n=120, rho=0.61 [pre-specified]" and "- pbe/analyse: rho=0.42 [exploratory]"
    When "tophat check" is run
    Then no "question.evidence" diagnostic is reported

  @rq-9447b114
  Scenario: A label the study has no result line for
    Given question "q-9c0d1e2f" is answered with "Evidence: x-vs-docking [post hoc]"
    And study "x-vs-docking" is reported with question "q-9c0d1e2f" and the result line "- base/score: n=120, rho=0.61 [pre-specified]"
    When "tophat check" is run
    Then an error "question.evidence-tag-mismatch" is reported
    And its hint names "pre-specified"

  @rq-f5e9098c
  Scenario: A label that is not a citation label
    Given question "q-9c0d1e2f" is answered with "Evidence: x-vs-docking [pre-specified]"
    And study "x-vs-docking" is reported with question "q-9c0d1e2f" and the result line "- base/score: n=120, rho=0.61 [pre-specified]"
    When "tophat check" is run
    Then an error "question.evidence-invalid" is reported at the evidence line

  @rq-c333e981
  Scenario: A question citation carries no label
    Given question "q-1a2b3c4d" is answered with "Evidence: q-5e6f7a8b [exploratory]"
    And "q-5e6f7a8b" is an answered sub-question of "q-1a2b3c4d" with valid evidence
    When "tophat check" is run
    Then an error "question.evidence-invalid" is reported

  @rq-df111cbc
  Scenario: Citing a retrospective study with a verified summary
    Given question "q-9c0d1e2f" is answered with "Evidence: old-benchmark [post hoc]"
    And retrospective study "old-benchmark" is reported with question "q-9c0d1e2f" and the result line "- base/score: n=60, MAE=0.9 kcal/mol [post hoc]" from its SUMMARY.md
    When "tophat check" is run
    Then no "question.evidence-retrospective" diagnostic is reported

  @rq-db0d11c1
  Scenario: Citing a retrospective study without a summary is flagged as legacy
    Given question "q-9c0d1e2f" is answered with "Evidence: old-benchmark [post hoc]"
    And retrospective study "old-benchmark" is reported with question "q-9c0d1e2f" and hand-written result lines without a SUMMARY.md
    When "tophat check --strict" is run
    Then a legacy diagnostic "question.evidence-retrospective" is reported
    And no "question" error or warning is reported

  @rq-a7910365
  Scenario: An untagged result line is left to the study rules
    Given question "q-9c0d1e2f" is answered with "Evidence: x-vs-docking"
    And the reported study "x-vs-docking" has a result line without a tag group
    When "tophat check" is run
    Then an error "study.tag-missing" is reported for "x-vs-docking"
    And no "question.evidence-tag-mismatch" is reported

  @rq-e895c8ff
  Scenario: Citing a study about an unrelated question
    Given questions "q-5e6f7a8b" and "q-9c0d1e2f" are siblings
    And question "q-5e6f7a8b" is answered with "Evidence: x-vs-docking"
    And study "x-vs-docking" is reported with question "q-9c0d1e2f" and a result line tagged "pre-specified"
    When "tophat check" is run
    Then a warning "question.evidence-unrelated" is reported

  @rq-01b7434d
  Scenario: Citing an answered sub-question
    Given question "q-1a2b3c4d" is answered with "Evidence: q-5e6f7a8b"
    And "q-5e6f7a8b" is an answered sub-question of "q-1a2b3c4d" with valid evidence
    When "tophat check" is run
    Then no "question.evidence" diagnostic is reported for "q-1a2b3c4d"

  @rq-4d76640c
  Scenario: Citing an open sub-question
    Given question "q-1a2b3c4d" is answered with "Evidence: q-9c0d1e2f"
    And "q-9c0d1e2f" is an open sub-question of "q-1a2b3c4d"
    When "tophat check" is run
    Then an error "question.evidence-unanswered" is reported

  @rq-6dc70288
  Scenario: Citing an emerged sub-question
    Given question "q-1a2b3c4d" is answered with "Evidence: q-9c0d1e2f"
    And "q-9c0d1e2f" is an emerged sub-question of "q-1a2b3c4d"
    When "tophat check" is run
    Then an error "question.evidence-unanswered" is reported

  @rq-503474ab
  Scenario: Citing a question that is not a descendant
    Given questions "q-5e6f7a8b" and "q-9c0d1e2f" are siblings
    And question "q-5e6f7a8b" is answered with "Evidence: q-9c0d1e2f"
    When "tophat check" is run
    Then an error "question.evidence-invalid" is reported

  @rq-13deda96
  Scenario: Citing a question that does not exist
    Given question "q-1a2b3c4d" is answered with "Evidence: q-deadbeef"
    And no question "q-deadbeef" exists
    When "tophat check" is run
    Then an error "question.evidence-invalid" is reported

  @rq-f6dec5d4
  Scenario: Mixed citations are each checked
    Given question "q-1a2b3c4d" is answered with "Evidence: q-5e6f7a8b, x-vs-docking , no-such-study"
    And "q-5e6f7a8b" is an answered sub-question with valid evidence
    And study "x-vs-docking" is reported with a question below "q-1a2b3c4d" and a result line tagged "pre-specified"
    When "tophat check" is run
    Then exactly one "question.evidence-invalid" error is reported, naming "no-such-study"

  # --- Study links ---

  @rq-fe8b6a81
  Scenario: Study names a question that does not exist
    Given study "x-vs-docking" has "question: q-deadbeef"
    And no question "q-deadbeef" exists in PROJECT.md
    When "tophat check" is run
    Then an error "question.unknown-reference" is reported on "studies/x-vs-docking/STUDY.md" at the question line

  @rq-6ef37d58
  Scenario: Study names a question when PROJECT.md is missing
    Given PROJECT.md does not exist
    And study "x-vs-docking" has "question: q-1a2b3c4d"
    When "tophat check" is run
    Then an error "question.unknown-reference" is reported for "x-vs-docking"

  @rq-dd539d53
  Scenario: Study names a question with sub-questions
    Given study "x-vs-docking" has "question: q-1a2b3c4d"
    And "q-1a2b3c4d" has sub-questions
    When "tophat check" is run
    Then no "question.unknown-reference" diagnostic is reported

  @rq-91713e05
  Scenario: Study with a malformed question field is left to the study rules
    Given study "x-vs-docking" has "question: Q1"
    When "tophat check" is run
    Then "study.field-invalid" is reported for the question field
    And no "question.unknown-reference" diagnostic is reported

  @rq-944a6f44
  Scenario: A study may address an emerged question
    Given study "outlier-dig" has "question: q-7d8e9f0a"
    And question "q-7d8e9f0a" is emerged with the origin line "Emerged from: x-vs-docking"
    And study "x-vs-docking" exists
    When "tophat check" is run
    Then no "question.unknown-reference" diagnostic is reported

  # --- tophat question stamp ---

  @rq-940d0a4b
  Scenario: Stamp assigns IDs and status lines
    Given PROJECT.md contains "## Can X improve affinity prediction?" followed by "Why it matters."
    When "tophat question stamp" is run
    Then the heading becomes "## q-<8 hex> · Can X improve affinity prediction?"
    And the line after the heading is "status: open"
    And "Why it matters." follows unchanged
    And the command prints the new ID and the question text
    And "tophat check" reports no "question" diagnostic

  @rq-2c9834f8
  Scenario: Stamp keeps existing IDs and status lines
    Given PROJECT.md contains "## q-1a2b3c4d · A?" followed by "status: answered"
    When "tophat question stamp" is run
    Then PROJECT.md is unchanged

  @rq-dc4c3c26
  Scenario: Stamp adds a missing status line to a stamped question
    Given PROJECT.md contains "## q-1a2b3c4d · A?" with no status line
    When "tophat question stamp" is run
    Then the line after the heading is "status: open"
    And the ID is unchanged

  @rq-ba94061b
  Scenario: Stamp produces unique IDs
    Given PROJECT.md contains 50 unstamped question headings
    When "tophat question stamp" is run
    Then all 50 questions have distinct IDs matching "q-[0-9a-f]{8}"

  @rq-d19b87ac
  Scenario: Stamp leaves malformed IDs for the user
    Given PROJECT.md contains "## q-12 · A?" and the unstamped heading "## B?"
    When "tophat question stamp" is run
    Then "## B?" is stamped
    And "## q-12 · A?" is unchanged
    And the command reports "q-12" and exits non-zero

  @rq-24fdff0d
  Scenario: Stamp leaves duplicated IDs for the user
    Given PROJECT.md contains two questions with ID "q-1a2b3c4d"
    When "tophat question stamp" is run
    Then both headings are unchanged
    And the command reports "q-1a2b3c4d" and exits non-zero

  @rq-a9a0b08d
  Scenario: Stamp preserves unrelated content
    Given PROJECT.md has a title, a preamble with HTML comments, CRLF line endings and a fenced code block
    When "tophat question stamp" is run
    Then the only differences are the rewritten headings and inserted status lines
    And inserted lines use CRLF line endings

  @rq-6106e112
  Scenario: Stamp ignores headings in code fences
    Given PROJECT.md contains a fenced code block with the line "## Not a question"
    When "tophat question stamp" is run
    Then that line is unchanged

  @rq-7eba13df
  Scenario: Stamp without PROJECT.md
    Given PROJECT.md does not exist
    When "tophat question stamp" is run
    Then the command fails with ProjectFileMissing
    And no file is created

  # --- tophat question emerge ---

  @rq-4044efe1
  Scenario: Emerge records a question born from a study
    Given PROJECT.md contains the example tree from this specification
    And study "x-vs-docking" is a draft with question "q-9c0d1e2f"
    When "tophat question emerge 'What causes the outlier structures?' --from x-vs-docking" is run
    Then PROJECT.md gains the question heading "#### q-<8 hex> · What causes the outlier structures?" as the last question, a sub-question of "q-9c0d1e2f"
    And the lines after the heading are "status: emerged" and "Emerged from: x-vs-docking"
    And the command prints the new ID and the question text
    And "tophat check" reports no "question" diagnostic

  @rq-6c48cc09
  Scenario: Emerge places the question as the parent's last sub-question
    Given PROJECT.md has the questions "## q-1a2b3c4d · A?", "### q-5e6f7a8b · B?" and "## q-9c0d1e2f · C?" with status lines
    And study "x-vs-docking" is a draft with question "q-1a2b3c4d"
    When "tophat question emerge 'Why did B behave oddly?' --from x-vs-docking" is run
    Then the new question's heading appears after the body of "q-5e6f7a8b" and before the heading of "q-9c0d1e2f"

  @rq-7853bb0a
  Scenario: Emerge with an explicit parent
    Given PROJECT.md contains the example tree from this specification
    And study "x-vs-docking" is a draft with question "q-9c0d1e2f"
    When "tophat question emerge 'Is the reference data reliable?' --from x-vs-docking --under q-1a2b3c4d" is run
    Then the new question is a sub-question of "q-1a2b3c4d" at level 3

  @rq-9ff67f40
  Scenario: Emerge from a study whose question is not in the tree
    Given PROJECT.md contains the example tree from this specification
    And study "x-vs-docking" is a draft with question "q-deadbeef"
    When "tophat question emerge 'A?' --from x-vs-docking" is run
    Then the command fails with ParentInvalid
    And PROJECT.md is unchanged

  @rq-1176e768
  Scenario: Emerge from a study that does not exist
    Given PROJECT.md contains the example tree from this specification
    When "tophat question emerge 'A?' --from no-such-study" is run
    Then the command fails with OriginStudyInvalid
    And PROJECT.md is unchanged

  @rq-9c8407f2
  Scenario: Emerge at the maximum depth
    Given PROJECT.md has questions at levels "##" through "######", each a child of the previous
    And study "x-vs-docking" is a draft with the deepest question
    When "tophat question emerge 'A?' --from x-vs-docking" is run
    Then the command fails with ParentTooDeep
    And PROJECT.md is unchanged

  @rq-8848775c
  Scenario: Emerge with an empty text
    Given PROJECT.md contains the example tree from this specification
    And study "x-vs-docking" is a draft with question "q-9c0d1e2f"
    When "tophat question emerge '' --from x-vs-docking" is run
    Then the command fails with TextInvalid
    And PROJECT.md is unchanged

  @rq-6248ce74
  Scenario: Emerge preserves the rest of the file
    Given PROJECT.md has a title, a preamble with HTML comments, CRLF line endings and a fenced code block
    And study "x-vs-docking" is a draft with a question in PROJECT.md
    When "tophat question emerge 'A?' --from x-vs-docking" is run
    Then the only difference is the inserted question block
    And the inserted lines use CRLF line endings

  @rq-c7e4904d
  Scenario: Emerged questions get distinct IDs
    Given PROJECT.md contains the example tree from this specification
    And study "x-vs-docking" is a draft with question "q-9c0d1e2f"
    When "tophat question emerge 'A?' --from x-vs-docking" is run twice
    Then PROJECT.md contains two new questions with distinct IDs matching "q-[0-9a-f]{8}"

  @rq-cdad3c4b
  Scenario: Emerge without PROJECT.md
    Given PROJECT.md does not exist
    When "tophat question emerge 'A?' --from x-vs-docking" is run
    Then the command fails with ProjectFileMissing
    And no file is created

  # --- tophat question tree ---

  @rq-c62bd382
  Scenario: Tree shows questions and attached studies
    Given PROJECT.md contains the example tree from this specification
    And study "x-energy-accuracy" is reported with question "q-5e6f7a8b"
    And study "x-vs-docking" is a draft with question "q-9c0d1e2f"
    When "tophat question tree" is run
    Then the output is
      """
      q-1a2b3c4d [open] Can method X improve affinity prediction for our targets?
        q-5e6f7a8b [answered] Is X's interaction energy accurate against reference data?
          study x-energy-accuracy [reported]
        q-9c0d1e2f [open] Does X rank binders better than docking?
          study x-vs-docking [draft]
          q-3b4c5d6e [dropped] Does the advantage hold for client targets?
          q-7d8e9f0a [emerged] What is going on with the outlier structures?
            emerged from x-vs-docking [draft]
      """
    And the exit code is 0

  @rq-41dc6a2d
  Scenario: Tree as JSON
    Given PROJECT.md contains the example tree from this specification
    When "tophat question tree --json" is run
    Then the output is a JSON object with a "questions" array of 5 entries in file order
    And the entry for "q-3b4c5d6e" has "level" 4 and "parent" "q-9c0d1e2f"
    And the entry for "q-7d8e9f0a" has "parent" "q-9c0d1e2f" and "origin" with "id" "x-vs-docking"
    And the entry for "q-1a2b3c4d" has "parent" null and "origin" null

  @rq-7d741e69
  Scenario: Tree does not validate
    Given PROJECT.md contains an unstamped heading "## B?" with no status line
    When "tophat question tree" is run
    Then the output contains a line for "B?" without an ID
    And the exit code is 0

  @rq-e03f0826
  Scenario: Tree shows an origin line for a missing study
    Given an emerged question with the origin line "Emerged from: no-such-study"
    When "tophat question tree" is run
    Then the question's line is followed by "emerged from no-such-study [?]"
    And the exit code is 0
```
