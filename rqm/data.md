# Feature: Data Roots, Datasets and Lineage <!-- rq-5737320e -->

Data in a TopHat project is never modified in place. Every file is either **raw** (registered once,
then frozen), **declared** (existing data whose history TopHat did not observe), or **derived**
(produced by a study stage; see `rqm/pipeline/provenance.md`). Studies hand data to each other only
through **published datasets**: named, numbered versions of a stage's output. This file specifies
where data lives (data roots), dataset manifests, the commands that register, publish, verify and
retire datasets, the lineage queries, and the `data` rule family evaluated by `tophat check`.

## Data Roots <!-- rq-985dbe20 -->

Data bytes live outside git under named **data roots**. Manifests in git refer to files by root name
and a path relative to the root, so the same project works on machines that keep the data in
different places.

### Declaring roots <!-- rq-e4355ed1 -->

`.tophat/roots.yaml` (committed) declares the root names of the project. Each project chooses its
own names; only `results` has a fixed meaning. For example, a project with instrument data and an
HPC work filesystem might declare:

```yaml
roots:
  raw:
    description: Instrument data as delivered
  results:
    description: Stage outputs
  archer:
    description: ARCHER2 work filesystem
```

- Root names match `^[a-z0-9]+(-[a-z0-9]+)*$`.
- The root `results` is always declared; it holds stage outputs.
- A missing `.tophat/roots.yaml` declares only `results`.

### Locating roots on a machine <!-- rq-7ec07143 -->

A root's location on the current machine is, in order of precedence:

1. The environment variable `TOPHAT_ROOT_<NAME>`, where `<NAME>` is the root name upper-cased with
   `-` replaced by `_` (e.g. `TOPHAT_ROOT_RESULTS`).
2. The entry for the root in `.tophat/roots.local.yaml`, a git-ignored file of the form
   `roots: { <name>: <absolute path> }`.

A root with neither is **unconfigured** on this machine, except `results`, which then defaults to
the git-ignored directory `results/` in the project root (created when the first output is
written), so a new project needs no configuration. Configuring `results` puts stage outputs
elsewhere, for example on a data disk. Locations are absolute filesystem paths; a relative path is
an error. Object-store URIs are not supported.

## Datasets <!-- rq-01f47f40 -->

### Manifests <!-- rq-011facdf -->

Each dataset version has a committed manifest at `datasets/<name>/v<n>.yaml`. Dataset names match
the study ID pattern and are 3 to 64 characters; versions are positive integers numbered from 1
without gaps per name, and are cited as `<name>@v<n>`.

```yaml
dataset: benchmark-set
version: 2
kind: raw                # raw | declared | published
visibility: public       # public | private
root: raw
path: benchmark/2024-05  # directory relative to the root
created: "2026-10-20T09:12:00Z"
origin:                  # raw: source, date, license; declared: statement
  source: "PDBbind refined set, filtered"
  date: "2024-05-02"
  license: "CC BY 4.0"
produced_by:             # published only
  study: clean-structures
  stage: clean
  key: sk-3f9a1c2b4d5e6f70
files:
  - path: 1abc.pdb
    size: 48211
    sha256: 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
content_hash: "sha256:..."
retired:                 # present only after `tophat data retire`
  date: "2026-11-02"
  reason: "Protonation states were wrong for 12 structures"
```

- `files` lists every regular file below the dataset directory, recursively, sorted by `path`
  (relative, `/`-separated). Symbolic links are not followed and are an error at registration.
- `content_hash` is `sha256:` followed by the SHA-256 of the text formed by one line
  `<sha256>  <path>\n` per file, in `files` order.
- `origin` is required for `raw` (`source` required; `date` and `license` optional) and `declared`
  (`statement` required). `produced_by` is required for `published` and absent otherwise.
- A manifest is immutable once written, except that `tophat data retire` adds `retired`.

### Kinds <!-- rq-4c85d6f4 -->

| Kind | Created by | Lineage |
|---|---|---|
| `raw` | `tophat data add` | Starts here |
| `declared` | `tophat data declare` | Stops here; marked *declared, not verified* |
| `published` | `tophat data publish` | Continues into the producing stage's provenance |

### Referencing datasets <!-- rq-f1d5b29a -->

Stage inputs reference a dataset version as `dataset:<name>@v<n>` (see
`rqm/pipeline/provenance.md`). A reference always names an exact version; there is no "latest".

## Verification <!-- rq-3301d594 -->

### Fast check <!-- rq-84e20fdc -->

`tophat check` does not re-hash data. For every dataset file in a configured root it checks that
the file exists and has the recorded size. It also consults the local hash cache: if the cache
holds an entry for the file with the same size but a different modification time, the file is
reported as modified since it was last verified.

### Hash cache <!-- rq-731766dd -->

`.tophat/cache/hashes.json` (git-ignored, per machine) records, for each verified file (keyed by
root name and path), its size, modification time (nanoseconds), SHA-256, and when it was verified.
It is written by `tophat data verify`, `tophat data add`, `tophat data declare` and
`tophat data publish`, and by `tophat stage seal` for stage outputs. Deleting it is always safe.

### Full verification <!-- rq-597bb615 -->

`tophat data verify` re-hashes files and compares them to their manifests. It streams files (never
loading a whole file into memory) and hashes several files in parallel.

## Producer Maturity and Visibility <!-- rq-5512a963 -->

- A study consuming a published dataset whose producing study is not `reported` is depending on
  unreported work. While the consuming study is `draft` or `preregistered` this is a warning. Once
  the consuming study is `reported` it is an error, unless the consuming study's Amendments contain
  an amendment whose reason includes `accept <name>@v<n>` for that dataset version.
- `tophat study report` refuses (with a `Summary` error naming the dataset) under the same condition
  that makes this an error.
- A `public` study must not consume a `private` dataset. A published dataset takes the visibility of
  its producing study.

## Feature API <!-- rq-6dc26c5f -->

### Commands <!-- rq-78e4b035 -->

- `tophat data add <name> --root <root> --path <path> --source <text> [--date <YYYY-MM-DD>] [--license <text>] [--visibility public|private]` <!-- rq-af5ed3df -->
  - Registers the directory `<root>/<path>` as the next version of raw dataset `<name>`: hashes
    every file, writes `datasets/<name>/v<n>.yaml`, and updates the hash cache. Prints the new
    reference (`<name>@v<n>`).
  - Fails without writing anything if the name is invalid, the root is undeclared or unconfigured,
    the path is absolute or escapes the root, the directory does not exist or contains no files, or
    it contains a symbolic link.
- `tophat data declare <name> --root <root> --path <path> --statement <text> [--visibility public|private]` <!-- rq-d89c8a6b -->
  - As `add`, but writes a `declared` manifest whose `origin.statement` records what is known about
    the data's history.
- `tophat data publish <study> --stage <stage> [--variant <variant>] --as <name>` <!-- rq-4fc6c2a5 -->
  - Publishes the output of `<stage>` for `<variant>` (default `base`) as the next version of
    `<name>`. The output is the non-trial stage output directory for the stage's current key (see
    `rqm/pipeline/provenance.md`); trial outputs cannot be published.
  - Requires a sealed output (`provenance.json` present) whose record is `clean`, and re-hashes the
    output files to confirm they match the provenance record.
  - Writes a `published` manifest with `root: results`, the output directory as `path`, the files
    from the provenance record, `produced_by` (study, stage, key) and the study's visibility.
  - Fails without writing if the study, stage or variant is unknown, the output is not sealed, the
    record is not clean, or any file differs from the record.
- `tophat data verify [<name>[@v<n>]]` <!-- rq-33c3cc6a -->
  - Re-hashes the files of the named dataset version, all versions of a name, or (with no argument)
    every dataset whose root is configured. Updates the hash cache for files that match.
  - Prints one line per mismatching, missing or extra file and a summary. Exits 0 if everything
    matches, 1 otherwise. Datasets in unconfigured roots are listed as skipped.
- `tophat data retire <name>@v<n> --reason <text>` <!-- rq-a7709a28 -->
  - Adds `retired` (today's local date and the reason) to the manifest. Retiring an already retired
    version fails.
- `tophat data lineage <target> [--json]` <!-- rq-1a97b91b -->
  - `<target>` is a dataset reference `<name>@v<n>`, or a stage output `<study>/<stage>@<key>`.
  - Prints the tree of everything the target was derived from: a published dataset leads to its
    producing stage output; a stage output leads to its inputs as recorded in its
    `provenance.json`; raw datasets are leaves; declared datasets are leaves marked
    `[declared, not verified]`; outputs of a study whose plan was informed by its deciding data are
    marked `[plan informed by its deciding data]` (see `rqm/study/exploration.md`); outputs whose
    provenance cannot be read (unconfigured root or
    missing record) are leaves marked `[unavailable]`.
- `tophat data used-by <name>[@v<n>] [--json]` <!-- rq-23b73104 -->
  - Lists every study, variant and stage whose configuration references the dataset (or the given
    version), and every published dataset whose producing stage did.
- `tophat data roots` <!-- rq-531eebf2 -->
  - Prints one line per declared root, sorted by name: `<name>`, a tab, and its location on this
    machine (after `TOPHAT_ROOT_<NAME>` overrides), or `-` when unconfigured. Read-only; used by the
    launcher to mount the roots into the container (see `rqm/template/container.md`).
  - Exits 1 with the problem when `.tophat/roots.yaml` or `.tophat/roots.local.yaml` is invalid.

### Types <!-- rq-45fb5dee -->

- `DatasetKind` — enum `Raw`, `Declared`, `Published`. <!-- rq-eab2ab92 -->
- `DatasetRef` — a parsed `<name>@v<n>`: `name: String`, `version: u32`. <!-- rq-0f8bd6f3 -->
- `Manifest` — a parsed manifest: name, version, kind, visibility, root, path, created, origin, <!-- rq-b2fecc6c -->
  produced by, files (path, size, sha256), content hash, retired.
- `Roots` — declared root names and their resolved locations on this machine. <!-- rq-bb98136e -->
- `DataError` — error type of the data commands, with at minimum: <!-- rq-bd19d141 -->
  - `InvalidName(String)`, `InvalidReference(String)`.
  - `UnknownRoot(String)`, `UnconfiguredRoot(String)`.
  - `PathOutsideRoot(String)`, `NoFiles(String)`, `SymbolicLink(String)`.
  - `DatasetNotFound(String)`, `AlreadyRetired(String)`.
  - `NotSealed(String)`, `NotClean(String)`, `OutputMismatch(Vec<String>)`.
  - `Manifest { path: String, message: String }`.
  - `Io(String)`.

### Functions <!-- rq-7babf9d1 -->

- `load_roots(root: &Path) -> Result<Roots, DataError>` — reads the declared roots and resolves <!-- rq-2960fd26 -->
  their locations from the environment and `.tophat/roots.local.yaml`.
- `parse_manifest(text: &str) -> Result<Manifest, String>` — parses and validates a manifest. <!-- rq-adfff14c -->
- `content_hash(files: &[(String, String)]) -> String` — the `content_hash` of `(path, sha256)` <!-- rq-da8648ca -->
  pairs sorted by path.
- `hash_file(path: &Path) -> io::Result<(u64, String)>` — size and SHA-256 of a file, streamed. <!-- rq-6f389d0b -->
- `data_rules(project: &Project) -> Vec<Diagnostic>` — the `data` rule family registered in <!-- rq-b2eccf77 -->
  `RULE_FAMILIES`.

## Check Rules (`data` family) <!-- rq-5b6f6e50 -->

Manifest diagnostics are reported on the manifest; reference diagnostics on the study configuration
file that contains the reference, at the reference's line.

| Rule | Severity | Condition |
|---|---|---|
| `data.roots-invalid` | error | `.tophat/roots.yaml` or `.tophat/roots.local.yaml` cannot be parsed, a name is malformed, or a local location is relative |
| `data.manifest-invalid` | error | A manifest cannot be parsed, a required field is missing or malformed, `kind`-specific fields are missing or present wrongly, or `content_hash` does not match `files` |
| `data.version-mismatch` | error | A manifest's `dataset`/`version` does not match its file name `datasets/<name>/v<n>.yaml` |
| `data.version-gap` | error | The versions of a name are not numbered 1, 2, … without gaps |
| `data.root-unknown` | error | A manifest names an undeclared root |
| `data.root-unconfigured` | warning | A manifest's root is unconfigured on this machine (reported once per root) |
| `data.file-missing` | error | A manifest file does not exist in its configured root |
| `data.file-changed` | error | A manifest file's size differs from the recorded size |
| `data.unverified` | warning | The hash cache shows a manifest file was modified since it was last verified |
| `data.reference-unknown` | error | A stage input references a dataset version that does not exist |
| `data.reference-retired` | error | A stage input references a retired dataset version |
| `data.newer-version` | warning | A `draft` or `preregistered` study references a version older than the newest non-retired version |
| `data.producer-unreported` | warning or error | A study consumes a published dataset whose producing study is not `reported` (error once the consumer is `reported` without an accepting amendment) |
| `data.visibility-leak` | error | A `public` study consumes a `private` dataset |
| `data.declared-input` | legacy | A study consumes a declared dataset |

## Assumptions <!-- rq-e95264a8 -->

- `.tophat/roots.local.yaml` and `.tophat/cache/` are git-ignored in every TopHat project; the
  project template's `.gitignore` lists them.
- Hashing uses SHA-256 throughout.
- The fast check trusts file sizes and the local cache; integrity guarantees that require content
  hashes come from `tophat data verify`, which `publish` performs implicitly for what it publishes.

## Out of Scope <!-- rq-b70e5d89 -->

- Object-store roots (`s3://` and similar).
- Datasets of other TopHat projects and private companion repositories.
- Copying, moving or deleting data, and warning before deletion (retention).
- Upgrading a declared dataset to verified.

## Gherkin Scenarios <!-- rq-3460a0ee -->

```gherkin
Feature: Data roots, datasets and lineage

  Background:
    Given a TopHat project
    And ".tophat/roots.yaml" declares roots "raw" and "results"
    And root "raw" is configured in ".tophat/roots.local.yaml" at a temporary directory
    And root "results" is configured at another temporary directory

  # --- Roots ---

  @rq-49765877
  Scenario: Environment variable overrides the local roots file
    Given TOPHAT_ROOT_RAW is set to "/data/elsewhere"
    When the roots are loaded
    Then root "raw" is located at "/data/elsewhere"

  @rq-bc2ad0ff
  Scenario: Missing roots file declares only results
    Given ".tophat/roots.yaml" does not exist
    When the roots are loaded
    Then the only declared root is "results"

  @rq-b6cac057
  Scenario: Relative local location is rejected
    Given ".tophat/roots.local.yaml" locates root "raw" at "data/raw"
    When "tophat check" is run
    Then an error "data.roots-invalid" is reported

  @rq-b4146a69
  Scenario: List the roots and their locations
    Given TOPHAT_ROOT_RAW is set to "/data/elsewhere"
    When "tophat data roots" is run
    Then it prints "raw\t/data/elsewhere" and "results\t<the results directory>", in that order
    And the exit code is 0

  @rq-f3dab8f0
  Scenario: Listing invalid roots fails
    Given ".tophat/roots.local.yaml" locates root "raw" at "data/raw"
    When "tophat data roots" is run
    Then the exit code is 1 and the message names ".tophat/roots.local.yaml"

  @rq-61aa0c3d
  Scenario: The results root defaults to the project's results folder
    Given neither TOPHAT_ROOT_RESULTS nor ".tophat/roots.local.yaml" locates root "results"
    When the roots are loaded
    Then root "results" is located at "<project root>/results"

  # --- Registering raw and declared data ---

  @rq-88716093
  Scenario: Add a raw dataset
    Given the directory "benchmark/2024-05" in root "raw" contains files "1abc.pdb" and "sub/2xyz.pdb"
    When "tophat data add benchmark-set --root raw --path benchmark/2024-05 --source PDBbind" is run
    Then "datasets/benchmark-set/v1.yaml" is written with kind "raw" and visibility "public"
    And its files are "1abc.pdb" and "sub/2xyz.pdb" with their sizes and SHA-256 hashes
    And its content_hash matches its files
    And the command prints "benchmark-set@v1"
    And the hash cache contains both files

  @rq-622fce95
  Scenario: Adding again creates the next version
    Given "benchmark-set@v1" exists
    When "tophat data add benchmark-set --root raw --path benchmark/2024-06 --source PDBbind" is run
    Then "datasets/benchmark-set/v2.yaml" is written
    And "datasets/benchmark-set/v1.yaml" is unchanged

  @rq-71bac1e4
  Scenario: Adding from an unconfigured root fails
    Given root "archer" is declared but not configured
    When "tophat data add traj --root archer --path runs --source MD" is run
    Then the command fails with UnconfiguredRoot
    And no manifest is written

  @rq-872d0d8b
  Scenario: Adding a path that escapes the root fails
    When "tophat data add x-set --root raw --path ../outside --source x" is run
    Then the command fails with PathOutsideRoot

  @rq-44c48945
  Scenario: Adding an empty directory fails
    Given the directory "empty" in root "raw" contains no files
    When "tophat data add x-set --root raw --path empty --source x" is run
    Then the command fails with NoFiles

  @rq-701626f2
  Scenario: Adding a directory with a symbolic link fails
    Given the directory "linked" in root "raw" contains a symbolic link
    When "tophat data add x-set --root raw --path linked --source x" is run
    Then the command fails with SymbolicLink
    And no manifest is written

  @rq-ce1e3132
  Scenario: Declare data of unknown history
    Given the directory "old-processed" in root "raw" contains files
    When "tophat data declare old-set --root raw --path old-processed --statement 'Processed in 2023 by unknown scripts'" is run
    Then "datasets/old-set/v1.yaml" is written with kind "declared" and that statement

  # --- Publishing ---

  @rq-dbb8600d
  Scenario: Publish a clean stage output
    Given study "clean-structures" has a sealed, clean output for stage "clean" in variant "base"
    When "tophat data publish clean-structures --stage clean --as clean-set" is run
    Then "datasets/clean-set/v1.yaml" is written with kind "published" and root "results"
    And its produced_by names study "clean-structures", stage "clean" and the stage key
    And its files equal the outputs in the stage's provenance record
    And its visibility equals the study's visibility

  @rq-510dffe3
  Scenario: Publishing an unsealed output fails
    Given study "clean-structures" has no sealed output for stage "clean"
    When "tophat data publish clean-structures --stage clean --as clean-set" is run
    Then the command fails with NotSealed

  @rq-1abe7472
  Scenario: Publishing a dirty output fails
    Given study "clean-structures" has a sealed output for stage "clean" whose record is not clean
    When "tophat data publish clean-structures --stage clean --as clean-set" is run
    Then the command fails with NotClean

  @rq-205358e8
  Scenario: Publishing an output modified after sealing fails
    Given study "clean-structures" has a sealed, clean output for stage "clean"
    And one output file was modified after sealing
    When "tophat data publish clean-structures --stage clean --as clean-set" is run
    Then the command fails with OutputMismatch naming that file
    And no manifest is written

  # --- Manifests in the check ---

  @rq-79aca0da
  Scenario: Valid manifests pass
    Given "benchmark-set@v1" was added and its files are unchanged
    When "tophat check" is run
    Then no "data" diagnostic is reported

  @rq-1690899a
  Scenario: Hand-edited content hash is detected
    Given the content_hash of "benchmark-set@v1" was edited by hand
    When "tophat check" is run
    Then an error "data.manifest-invalid" is reported for "datasets/benchmark-set/v1.yaml"

  @rq-007499c3
  Scenario: Manifest file name disagrees with its contents
    Given "datasets/benchmark-set/v2.yaml" contains "version: 3"
    When "tophat check" is run
    Then an error "data.version-mismatch" is reported

  @rq-2ba558eb
  Scenario: Version gap
    Given manifests "v1" and "v3" exist for "benchmark-set" but not "v2"
    When "tophat check" is run
    Then an error "data.version-gap" is reported

  @rq-95cf00a8
  Scenario: Missing data file
    Given a file of "benchmark-set@v1" was deleted from root "raw"
    When "tophat check" is run
    Then an error "data.file-missing" is reported naming that file

  @rq-2e2aead6
  Scenario: Changed file size
    Given a file of "benchmark-set@v1" was appended to
    When "tophat check" is run
    Then an error "data.file-changed" is reported

  @rq-7d9f7e52
  Scenario: Same-size modification is flagged by the cache
    Given a file of "benchmark-set@v1" was overwritten with different content of the same size
    When "tophat check" is run
    Then a warning "data.unverified" is reported for that file
    And "tophat data verify benchmark-set@v1" exits 1 naming that file

  @rq-b418ac06
  Scenario: Unconfigured root is a warning, not an error
    Given "traj-set@v1" lives in root "archer"
    And root "archer" is declared but not configured on this machine
    When "tophat check" is run
    Then exactly one warning "data.root-unconfigured" is reported for root "archer"
    And no "data.file-missing" diagnostic is reported for "traj-set@v1"

  @rq-2446b713
  Scenario: Fresh clone without a hash cache
    Given "benchmark-set@v1" exists and ".tophat/cache/hashes.json" does not exist
    When "tophat check" is run
    Then no "data.unverified" diagnostic is reported

  # --- Verification ---

  @rq-43c471b8
  Scenario: Verify a matching dataset
    Given "benchmark-set@v1" files are unchanged
    When "tophat data verify benchmark-set@v1" is run
    Then the exit code is 0
    And the hash cache is updated for its files

  @rq-973f9875
  Scenario: Verify reports extra files
    Given a new file was added to the directory of "benchmark-set@v1"
    When "tophat data verify benchmark-set@v1" is run
    Then the output names the extra file
    And the exit code is 1

  @rq-b6d6ddcc
  Scenario: Verify skips unconfigured roots
    Given "traj-set@v1" lives in an unconfigured root
    When "tophat data verify" is run
    Then the output lists "traj-set@v1" as skipped

  # --- Retiring and references ---

  @rq-98b54f50
  Scenario: Retire a dataset version
    When "tophat data retire benchmark-set@v1 --reason 'Wrong protonation'" is run
    Then the manifest gains "retired" with today's date and the reason
    And no other byte of the manifest changes

  @rq-c3395d20
  Scenario: Retiring twice fails
    Given "benchmark-set@v1" is retired
    When "tophat data retire benchmark-set@v1 --reason again" is run
    Then the command fails with AlreadyRetired

  @rq-c25e83b5
  Scenario: Referencing a retired version is an error
    Given "benchmark-set@v1" is retired
    And study "x-vs-docking" has a stage input "dataset:benchmark-set@v1"
    When "tophat check" is run
    Then an error "data.reference-retired" is reported on the study's config file at that input

  @rq-dbcd9057
  Scenario: Referencing an unknown version is an error
    Given study "x-vs-docking" has a stage input "dataset:benchmark-set@v9"
    When "tophat check" is run
    Then an error "data.reference-unknown" is reported

  @rq-0a389452
  Scenario: Newer version available for a draft
    Given "benchmark-set@v1" and "benchmark-set@v2" exist
    And draft study "x-vs-docking" references "dataset:benchmark-set@v1"
    When "tophat check" is run
    Then a warning "data.newer-version" is reported

  @rq-f19caed4
  Scenario: Reported study pinned to an older version is not warned
    Given "benchmark-set@v1" and "benchmark-set@v2" exist
    And reported study "x-vs-docking" references "dataset:benchmark-set@v1"
    When "tophat check" is run
    Then no "data.newer-version" diagnostic is reported

  # --- Maturity and visibility ---

  @rq-fed9f56b
  Scenario: Consuming unreported work while still in draft warns
    Given "clean-set@v1" was published by study "clean-structures" with status "preregistered"
    And draft study "x-vs-docking" references "dataset:clean-set@v1"
    When "tophat check" is run
    Then a warning "data.producer-unreported" is reported

  @rq-228b4cbc
  Scenario: A reported study cannot rest on unreported work
    Given "clean-set@v1" was published by study "clean-structures" with status "preregistered"
    And reported study "x-vs-docking" references "dataset:clean-set@v1"
    When "tophat check" is run
    Then an error "data.producer-unreported" is reported

  @rq-0ca6fca2
  Scenario: An amendment accepts the unreported dependency
    Given "clean-set@v1" was published by study "clean-structures" with status "preregistered"
    And reported study "x-vs-docking" references "dataset:clean-set@v1"
    And its Amendments contain "- 2026-10-01 — results seen: no — accept clean-set@v1 before its study is reported"
    When "tophat check" is run
    Then the "data.producer-unreported" diagnostic is a warning

  @rq-d682f5a4
  Scenario: Report refuses while depending on unreported work
    Given "clean-set@v1" was published by study "clean-structures" with status "preregistered"
    And preregistered study "x-vs-docking" references "dataset:clean-set@v1" and has a clean SUMMARY.md
    When "tophat study report x-vs-docking" is run
    Then the command fails with a Summary error naming "clean-set@v1"

  @rq-5dee52f5
  Scenario: Public study consuming private data
    Given "client-set@v1" is private
    And public study "x-vs-docking" references "dataset:client-set@v1"
    When "tophat check" is run
    Then an error "data.visibility-leak" is reported

  @rq-e4c5a373
  Scenario: Consuming declared data is flagged as legacy
    Given "old-set@v1" is declared
    And study "x-vs-docking" references "dataset:old-set@v1"
    When "tophat check --strict" is run
    Then a legacy diagnostic "data.declared-input" is reported
    And the exit code is not affected by it

  # --- Lineage ---

  @rq-7d7e180b
  Scenario: Lineage of a published dataset reaches raw data
    Given "benchmark-set@v1" is raw
    And study "clean-structures" stage "prepare" consumed "dataset:benchmark-set@v1"
    And stage "clean" consumed stage "prepare"
    And "clean-set@v1" was published from stage "clean"
    When "tophat data lineage clean-set@v1" is run
    Then the output shows "clean-set@v1" derived from stage "clean", from stage "prepare", from "benchmark-set@v1"
    And "benchmark-set@v1" is a leaf

  @rq-1eaa34af
  Scenario: Lineage stops at declared data
    Given stage "prepare" of study "x-vs-docking" consumed "dataset:old-set@v1", which is declared
    When "tophat data lineage x-vs-docking/prepare@<key>" is run
    Then "old-set@v1" is shown as a leaf marked "[declared, not verified]"

  @rq-6c036885
  Scenario: Lineage through outputs that are not on this machine
    Given "clean-set@v1" was published from a stage whose output is not in this machine's results root
    When "tophat data lineage clean-set@v1" is run
    Then the producing stage output is shown marked "[unavailable]"
    And the exit code is 0

  @rq-8ccc06c7
  Scenario: Used-by lists consumers
    Given studies "x-vs-docking" and "x-energy" reference "dataset:benchmark-set@v1" in stage "prepare"
    And "clean-set@v1" was published from a stage that consumed "benchmark-set@v1"
    When "tophat data used-by benchmark-set@v1" is run
    Then the output lists both studies with their variant and stage
    And it lists "clean-set@v1"
```
