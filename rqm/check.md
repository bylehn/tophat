# Feature: Project Integrity Check (`tophat check`) <!-- rq-10af2996 -->

`tophat check` is the integrity gate of a TopHat project. It reads the project's committed files,
evaluates every registered rule, and reports problems as diagnostics. Agents, humans, pre-commit
hooks and CI all run the same command and get the same answer; no LLM is involved.

This file specifies the check command itself: project discovery, the diagnostic model, severities,
output formats, exit codes, and how rule families plug in. The rules themselves are specified in the
requirements file of the feature they belong to (e.g. study rules in `rqm/study/format.md`).

## Behaviour <!-- rq-b620446f -->

### Project discovery <!-- rq-abbd00cd -->

The project root is the nearest ancestor of the current working directory (including the directory
itself) that contains a `.tophat/` directory. All paths in diagnostics are relative to the project
root and use `/` as the separator.

### Read-only and offline <!-- rq-39ebb522 -->

`tophat check` never modifies any file, never makes network requests, and never runs tests,
pipelines or external commands. It is safe to run at any time and in any state of the working tree.

### Rule evaluation <!-- rq-e54fa7fd -->

- Every registered rule family runs on every check. A failure inside one rule family (for example an
  unreadable file) is reported as a diagnostic and does not stop other rules or other files from
  being checked.
- A file that cannot be read or parsed produces exactly one error diagnostic for that file from the
  rule family that owns the file type; rules that depend on that file's content are skipped for that
  file only.
- Cross-references between files (for example one study naming another) are always resolved against
  the whole project, even when the check is scoped.

### Scope <!-- rq-cf68a3b8 -->

`tophat check [PATH...]` limits the **reported** diagnostics to those whose `path` lies at or below
one of the given paths. Given paths are interpreted relative to the current working directory and
must lie inside the project. With no paths, the whole project is in scope.

### Severities <!-- rq-192a5ff9 -->

| Severity | Meaning | Fails the check |
|---|---|---|
| `error` | The project violates an integrity guarantee | Yes |
| `warning` | Something is incomplete or suspicious but not yet a violation (e.g. a scenario without a test in a study that is still being built) | Only with `--strict` |
| `legacy` | A tolerated gap in a partially adopted project (declared datasets, retrospective results without a pipeline, unmapped legacy paths) | Never |

Legacy diagnostics are never suppressed: they always appear in the output so gaps stay visible.

### Rule identifiers <!-- rq-9b9f7ee6 -->

Each diagnostic carries a rule identifier of the form `<family>.<rule>`, lowercase, using `a-z`,
`0-9` and `-` (e.g. `study.frozen-modified`). Identifiers are stable across releases: a rule may be
removed, but its identifier is never reused for a different meaning. The `check` family is reserved
for diagnostics about the check run itself.

### Output <!-- rq-b757d2fc -->

**Human-readable (default)**, on standard output:

- Errors and warnings first, grouped by path, then a separate `Legacy` section listing legacy
  diagnostics.
- Each diagnostic on one line as `<path>[:<line>]: <severity>[<rule>]: <message>`, followed by an
  indented `hint: <hint>` line when a hint exists.
- A final summary line `<E> error(s), <W> warning(s), <L> legacy item(s)`.

**JSON (`--json`)**, on standard output, a single object:

```json
{
  "format_version": 1,
  "tophat_version": "0.1.0",
  "diagnostics": [
    {
      "rule": "study.frozen-modified",
      "severity": "error",
      "path": "studies/x-vs-docking/STUDY.md",
      "line": 12,
      "message": "Frozen sections changed after pre-registration",
      "hint": "Revert the change and record it under Amendments, or start a new study"
    }
  ],
  "summary": { "errors": 1, "warnings": 0, "legacy": 0 }
}
```

`line` and `hint` are omitted when absent. `format_version` changes only on incompatible changes to
this structure.

**Ordering.** Diagnostics are sorted by path, then line (absent lines first), then rule identifier,
then message, in both formats. Two runs on an unchanged project produce byte-identical output.

### Exit codes <!-- rq-35e01b00 -->

| Code | Meaning |
|---|---|
| `0` | The check ran and found no failing diagnostics |
| `1` | The check ran and found at least one failing diagnostic |
| `2` | The check could not run: not inside a TopHat project, invalid arguments, a scope path outside the project, or an internal error |

On exit code 2 a message is written to standard error and nothing is written to standard output
(not even with `--json`).

## Feature API <!-- rq-c35dd468 -->

### Command <!-- rq-cf46eca9 -->

- `tophat check [--json] [--strict] [PATH...]` <!-- rq-1bce095c -->
  - Discovers the project root, runs all rule families, filters diagnostics to the scope, and prints
    them in the selected format.
  - `--strict` makes `warning` diagnostics fail the check. `legacy` diagnostics never fail it.
  - Exits with the codes defined above.

### Types <!-- rq-96dcd148 -->

- `Severity` — enum with variants `Error`, `Warning`, `Legacy`. <!-- rq-fdb74980 -->
- `Diagnostic` — one finding: <!-- rq-84ef0512 -->
  - `rule: &'static str` — the rule identifier.
  - `severity: Severity`.
  - `path: String` — project-relative path, `/`-separated.
  - `line: Option<u32>` — 1-based line number, when the finding has a location within the file.
  - `message: String` — one sentence stating what is wrong.
  - `hint: Option<String>` — one sentence stating how to fix it.
- `Project` — a read-only view of the project used by rule families: the project root and accessors <!-- rq-d94a6490 -->
  for the project's files. Rule families read files only through it.
- `CheckError` — failure to run the check at all, with at minimum: <!-- rq-c8046ff3 -->
  - `NotInProject` — no ancestor directory contains `.tophat/`.
  - `ScopeOutsideProject(String)` — a given path is not inside the project.
  - `Io(String)` — a failure reading project structure that prevents any check from running.

### Functions <!-- rq-db030abe -->

- `run_check(root: &Path, scope: &[PathBuf]) -> Result<Vec<Diagnostic>, CheckError>` <!-- rq-6236fe4c -->
  - Runs every registered rule family against the project at `root` and returns the diagnostics in
    scope, sorted as specified under *Ordering*.
- `RULE_FAMILIES: &[fn(&Project) -> Vec<Diagnostic>]` — the static list of rule families. Adding a <!-- rq-9b44b88f -->
  family means adding a function to this list; there is no runtime plugin mechanism.

## Out of Scope <!-- rq-de80fc6b -->

- The individual rules (specified by each feature's requirements file).
- Automatically fixing problems.
- Running tests, pipelines or hash verification of external data roots (specified with the data and
  provenance features).
- Configuration files that disable rules or change severities.

## Gherkin Scenarios <!-- rq-c112cac7 -->

```gherkin
Feature: tophat check command

  # --- Project discovery ---

  @rq-23b830b6
  Scenario: Check runs from the project root
    Given a TopHat project with no problems
    And the current directory is the project root
    When "tophat check" is run
    Then the exit code is 0
    And the output ends with "0 error(s), 0 warning(s), 0 legacy item(s)"

  @rq-8889d0ed
  Scenario: Check runs from a subdirectory of the project
    Given a TopHat project with no problems
    And the current directory is "studies/x-vs-docking" inside the project
    When "tophat check" is run
    Then the exit code is 0

  @rq-580c8189
  Scenario: Check outside any TopHat project
    Given the current directory and none of its ancestors contain a ".tophat" directory
    When "tophat check" is run
    Then the exit code is 2
    And standard error explains that no TopHat project was found
    And standard output is empty

  # --- Severities and exit codes ---

  @rq-b17553a6
  Scenario: An error fails the check
    Given a project with one error diagnostic and no others
    When "tophat check" is run
    Then the exit code is 1

  @rq-0bdbef43
  Scenario: A warning does not fail the check by default
    Given a project with one warning diagnostic and no others
    When "tophat check" is run
    Then the warning is printed
    And the exit code is 0

  @rq-74adaff3
  Scenario: A warning fails the check in strict mode
    Given a project with one warning diagnostic and no others
    When "tophat check --strict" is run
    Then the exit code is 1

  @rq-4151372b
  Scenario: Legacy items never fail the check, even in strict mode
    Given a project with one legacy diagnostic and no others
    When "tophat check --strict" is run
    Then the legacy diagnostic is printed in the "Legacy" section
    And the exit code is 0

  # --- Robustness ---

  @rq-a8be8316
  Scenario: An unparseable file does not stop other files being checked
    Given a project with two studies
    And the first study's STUDY.md has invalid YAML front matter
    And the second study has a frozen-section violation
    When "tophat check" is run
    Then exactly one parse error is reported for the first study's STUDY.md
    And the frozen-section error is reported for the second study
    And the exit code is 1

  @rq-9bca6bbf
  Scenario: Check does not modify the working tree
    Given a project with errors, warnings and legacy items
    When "tophat check" is run
    Then no file in the project is created, modified or deleted

  # --- Scope ---

  @rq-e57fc88d
  Scenario: Scoped check reports only diagnostics under the given path
    Given study "a" has an error diagnostic
    And study "b" has an error diagnostic
    When "tophat check studies/a" is run
    Then only the diagnostic for study "a" is printed
    And the exit code is 1

  @rq-170877ff
  Scenario: Scoped check still resolves cross-references project-wide
    Given study "a" is superseded by study "b"
    And study "b" lists "a" under supersedes
    When "tophat check studies/a" is run
    Then no supersede-link diagnostic is reported for study "a"

  @rq-451aca52
  Scenario: Scope path outside the project
    When "tophat check /tmp" is run inside a project
    Then the exit code is 2
    And standard error explains that the path is outside the project

  # --- Output ---

  @rq-d47ba6b9
  Scenario: JSON output structure
    Given a project with one error diagnostic that has a line and a hint
    When "tophat check --json" is run
    Then standard output is a single JSON object
    And it has "format_version" equal to 1
    And its "diagnostics" array has one entry with keys "rule", "severity", "path", "line", "message", "hint"
    And its "summary" is {"errors": 1, "warnings": 0, "legacy": 0}

  @rq-f94476f9
  Scenario: JSON omits absent optional fields
    Given a project with one diagnostic that has no line and no hint
    When "tophat check --json" is run
    Then the diagnostic object has no "line" key and no "hint" key

  @rq-68a6dd72
  Scenario: Output is deterministic
    Given a project with diagnostics in several files
    When "tophat check --json" is run twice without changes to the project
    Then both outputs are byte-identical
    And diagnostics are sorted by path, then line, then rule, then message

  @rq-11b56000
  Scenario: Human output separates legacy items
    Given a project with one error and one legacy diagnostic
    When "tophat check" is run
    Then the error appears before a "Legacy" section
    And the legacy diagnostic appears in the "Legacy" section
    And paths are shown relative to the project root with "/" separators
```
