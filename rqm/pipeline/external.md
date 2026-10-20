# Feature: External (Cluster) Stages <!-- rq-750c490b -->

An **external stage** runs on a machine TopHat never touches: typically heavy compute on an HPC
cluster, using the cluster's own optimized software. TopHat's job is to make running it by hand easy
and safe. It writes a self-contained **job folder** with a clear structure and a ready `sbatch`
script; the human copies the folder to the cluster, submits it, and copies it back; the job leaves a
**receipt** recording what it was given, what ran, and the hashes of what it produced; and the next
run **ingests** the returned folder, verifies it against the receipt, and seals the stage so
downstream stages continue.

**Neither `tophat` nor an agent ever connects to a remote machine.** The project holds no host
names, remote paths or credentials; every transfer and submission is done by the human. Inputs,
parameters, the command and the outputs are verified by hash; what ran on the cluster is reported by
the receipt and labelled as executed externally.

This file specifies external stage configuration, the job folder, the job script, the receipt and
ingest. Stage configuration, keys and provenance are specified in `rqm/pipeline/provenance.md`; rule
generation in `rqm/pipeline/snakemake.md`.

## Configuration <!-- rq-fd86ea5e -->

A stage is external when it has an `external` mapping:

```yaml
  - name: md
    run: gmx mdrun -s {in.system}/topol.tpr -deffnm {out}/md -ntomp {threads}
    inputs: {system: stage:prepare}
    threads: 128
    external:
      setup:
        - module load gromacs/2024.2
      slurm:
        - --time=24:00:00
        - --nodes=1
      keep: ["*.trr", "*.xtc", "*.cpt"]
```

| Field | Required | Meaning |
|---|---|---|
| `setup` | no | Shell lines run in the job before the command (e.g. loading modules) |
| `slurm` | no | `sbatch` options for this stage, each written as an `#SBATCH` line |
| `keep` | no | Glob patterns (matched against paths relative to `out/`) of output files that stay on the cluster and are not copied back |

- `external: {}` is valid: an external stage with no setup, no extra options and nothing kept.
- An external stage cannot have `foreach`, `env` or `tool`: its software comes from the cluster.
- `code` files are placed in the job folder at their study-relative paths, so `run` can call study
  scripts exactly as local stages do.
- Placeholders: `{out}` is the job folder's `out/`; `{in.<input>}` is the input's copy in the job
  folder's `inputs/<input>/`; `{params.<key>}`, `{params_file}` and `{threads}` are as for local
  stages. `{item}`, `{item_name}` and `{tool}` are not available. All of them are relative to the job
  folder, so the folder can be placed anywhere on the cluster.
- QC gates apply as for any stage: an external stage that another stage consumes is gated unless it
  declares `qc: skip`.

## SLURM Defaults <!-- rq-964a4533 -->

`.tophat/slurm.local.yaml` (git-ignored, per machine) holds the user's own `sbatch` options, such as
their account and default partition:

```yaml
sbatch:
  - --account=e123
  - --partition=standard
  - --qos=standard
```

The job script's `#SBATCH` lines are these defaults followed by the stage's `slurm` options; when
both set the same option (the text before `=`), the stage's value is used. Nothing personal is
committed.

## Keys <!-- rq-23d21623 -->

The key text (see `rqm/pipeline/provenance.md`) of an external stage:

- has `env external` in place of an environment hash; and
- gains the line `external <canonical JSON>` directly after the `env` line, where the JSON object
  holds `setup` and `keep` (sorted keys, no whitespace).

`slurm` options and SLURM defaults are not part of the key: the resources a job is given do not
change what it computes. Local stages have no `external` line, so their keys are unaffected.

## The Job Folder <!-- rq-340771e5 -->

When an external stage is reached and has not been returned yet, `tophat` writes the job folder
`<output directory>/<study>.<stage>.<key>/` (see `rqm/pipeline/provenance.md` for the output
directory):

```text
x-vs-docking.md.sk-3f9a1c2b4d5e6f70/
  README.md          what this job is, and the steps to run it
  job.sh             the sbatch script
  stage.json         study, stage, variant, key and keep patterns
  inputs/<input>/…   copies of every input file
  inputs.sha256      one "<sha256>  <path>" line per input file
  <code files>       at their study-relative paths
  params.json        the stage's parameters
  out/               empty; the job writes its outputs here
```

- Every input is copied, so the folder is self-contained. When the copied inputs exceed 10 GB in
  total, `tophat` prints a warning with their size; the folder is still written.
- The folder is written atomically (to a temporary directory, then renamed) and only if it does not
  exist, so edits the human made to its `#SBATCH` lines are never overwritten.
- Its name is the same everywhere the folder goes, so a returned folder lands back in place.

## Job Script <!-- rq-13602cac -->

`job.sh` is a POSIX `sh` script needing nothing beyond standard utilities (`sha256sum` or
`shasum -a 256`, `wc`, `date`, `find`, `grep`). In order, it:

1. Starts with `#!/bin/sh` and the `#SBATCH` lines (see *SLURM Defaults*), plus
   `#SBATCH --cpus-per-task=<threads>` unless an option already sets it, and
   `#SBATCH --job-name=<stage>`.
2. Changes to its own directory and runs with `set -eu`.
3. Verifies `inputs.sha256`; on any mismatch it writes a receipt with `exit input-mismatch` and stops.
4. Runs the `setup` lines, records the start time, runs the substituted command, and records the end
   time and exit status.
5. Hashes every file under `out/` and writes `receipt.txt`, even when the command failed.
6. Exits with the command's exit status.

The human may edit the `#SBATCH` lines (time, partition, account, …). Any other change is detected
at ingest.

## Receipt <!-- rq-52dc4a39 -->

`receipt.txt` is plain text, one record per line:

```text
tophat-receipt 1
study x-vs-docking
stage md
key sk-3f9a1c2b4d5e6f70
script 7d865e959b2466918c9863afca942d0fb89d7c9ac0c99bafc3749504ded97730
host nid001234
job 8812345
start 2026-10-21T08:00:03Z
end 2026-10-21T19:42:51Z
exit 0
module gromacs/2024.2
output 9f86d081…  48211  md.edr
output 2c26b46b…  1022  md.log
kept 4e07408562bedb8b…  98234221312  md.trr
```

- `script` is the SHA-256 of `job.sh` with every line starting with `#SBATCH` removed.
- `job` is `$SLURM_JOB_ID`, or `-` outside SLURM.
- `module` lines are the output of `module list` when available, one module per line.
- `output` lines list files to copy back and `kept` lines list files matching `keep`; each has the
  SHA-256, the size in bytes, and the path relative to `out/`.

## The Human's Steps <!-- rq-9f1e97a5 -->

`README.md` in the job folder, the message printed when the pipeline stops there, and
`tophat stage instructions` all give the same steps. Commands are examples with placeholders; TopHat
never fills in a host or a remote path:

```sh
# 1. Copy this folder to the cluster
rsync -a <output directory>/x-vs-docking.md.sk-3f9a1c2b4d5e6f70 <you>@<cluster>:<directory>/

# 2. On the cluster, submit it (you may edit the #SBATCH lines first; change nothing else)
cd <directory>/x-vs-docking.md.sk-3f9a1c2b4d5e6f70 && sbatch job.sh

# 3. When the job has finished, copy the folder back, leaving the large files on the cluster
rsync -a --exclude='out/*.trr' --exclude='out/*.xtc' --exclude='out/*.cpt' \
  <you>@<cluster>:<directory>/x-vs-docking.md.sk-3f9a1c2b4d5e6f70 <output directory>/

# 4. Continue the pipeline
tophat run x-vs-docking
```

## Ingest <!-- rq-a1948f0a -->

When an external stage is reached and its job folder contains `receipt.txt`, `tophat` ingests it:

- The receipt's `study`, `stage` and `key` must match the stage's current key.
- `script` must equal the hash of the job script `tophat` generates for this key, with `#SBATCH`
  lines removed: the command, setup and checks that ran are the ones recorded.
- `exit` must be `0`. Otherwise ingest fails, quoting the exit status; to try again the human deletes
  `receipt.txt` and the contents of `out/` and resubmits.
- Every `output` file must be in `out/` with the recorded size and hash. Files matching `keep` may be
  present (copied back anyway) if they match their `kept` line. Any other file in `out/` fails ingest.
- On success the `output` files are moved from `out/` into the output directory and the stage is
  sealed (see `rqm/pipeline/provenance.md`): `outputs` are those files, and the record gains
  `"external": {"host", "job", "start", "end", "modules", "setup", "script_sha256",
  "receipt_sha256", "kept": [{"path", "size", "sha256"}]}`. The job folder stays as the record of the
  run; its outputs listing excludes it. Cleanliness is determined locally, as for any stage.

Files kept on the cluster are recorded by hash in the provenance record; lineage runs through the
returned files to the record that lists them.

## Feature API <!-- rq-c4003645 -->

### Commands <!-- rq-dd882acb -->

- `tophat stage external <study> <stage> --key <key> [--variant <variant>] [--trial]` <!-- rq-530610d9 -->
  - Used by generated rules. Without a receipt: writes the job folder if missing, prints the steps,
    and exits 3. With a receipt: ingests and seals the stage, exiting 0, or exits 1 explaining why
    ingest failed.
- `tophat stage instructions <study> <stage> [--variant <variant>]` <!-- rq-9eddd05a -->
  - Prints the steps for the stage's current key, including the job folder's path. Read-only.

### Types <!-- rq-b499b7e1 -->

- `Receipt` — a parsed `receipt.txt`. <!-- rq-10f972ce -->
- `ExternalError` — error type of the external commands, with at minimum: <!-- rq-57a8f3ae -->
  - `Waiting(String)` — the job folder is ready and has not been returned (carries the steps).
  - `ReceiptInvalid(String)`, `KeyMismatch { expected: String, actual: String }`.
  - `ScriptModified`, `JobFailed { exit: String }`.
  - `OutputMismatch(Vec<String>)`, `Io(String)`.

### Functions <!-- rq-5e395d39 -->

- `parse_receipt(text: &str) -> Result<Receipt, String>` — parses and validates a receipt. <!-- rq-d39d6009 -->
- `job_script(stage: &ResolvedStage, …) -> String` — the job script text. <!-- rq-d966a5fb -->
- `write_job_folder(root: &Path, request: &SealRequest) -> Result<PathBuf, ExternalError>` — writes <!-- rq-3de08c05 -->
  the job folder.
- `ingest(root: &Path, request: &SealRequest) -> Result<PathBuf, ExternalError>` — verifies a <!-- rq-0ab3c68d -->
  returned job folder and seals the stage.

## Check Rules (`pipeline` family, external) <!-- rq-3898af16 -->

| Rule | Severity | Condition |
|---|---|---|
| `pipeline.config-invalid` | error | An `external` value is not a mapping or has an unknown key, or an external stage has `foreach`, `env` or `tool`, or uses an unavailable placeholder |
| `pipeline.slurm-defaults-invalid` | error | `.tophat/slurm.local.yaml` cannot be parsed or `sbatch` is not a list of options starting with `--` |

## Assumptions <!-- rq-f5aed182 -->

- The cluster is a Linux system with SLURM and a POSIX shell.
- The development container holds no SSH keys or cluster credentials, so even an agent with shell
  access inside it cannot reach the cluster; agent skills additionally never run transfer or
  submission commands.
- `.tophat/slurm.local.yaml` is git-ignored in every TopHat project; the project template lists it.

## Out of Scope <!-- rq-9a642ead -->

- Connecting to, transferring data to or from, or submitting jobs on a remote machine.
- Inputs read in place on the cluster instead of copied.
- Batch systems other than SLURM, job arrays, and fan-out external stages.
- Verifying files kept on the cluster from another machine.

## Gherkin Scenarios <!-- rq-2891bce5 -->

```gherkin
Feature: External (cluster) stages

  Background:
    Given a TopHat project in a git repository with root "results" configured
    And preregistered study "x-vs-docking" has stages "prepare" (qc: skip), "md" (external, keep ["*.trr"]) and "analyse" (result)
    And "prepare" is sealed

  # --- Configuration and keys ---

  @rq-69a74980
  Scenario: An external stage cannot fan out
    Given stage "md" also has "foreach: system"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported at stage "md"

  @rq-51d60cdc
  Scenario: An external stage cannot have a pixi environment
    Given stage "md" also has "env: default"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" is reported

  @rq-d8643ac5
  Scenario: Connection details are not configuration
    Given stage "md" has "external: {remote: archer}"
    When "tophat check" is run
    Then an error "pipeline.config-invalid" naming "remote" is reported

  @rq-254e43a2
  Scenario: The key covers setup and keep, not resources
    Given the key of "md" is recorded
    When the stage's "slurm" options or ".tophat/slurm.local.yaml" change
    Then the key of "md" is unchanged
    When a "setup" line changes
    Then the key of "md" changes

  @rq-deeead81
  Scenario: Local stages keep their keys
    Given a study without external stages and its keys
    When this feature is introduced
    Then those keys are unchanged

  # --- Job folder and steps ---

  @rq-843afa31
  Scenario: Reaching an external stage writes the job folder and waits
    When "tophat stage external x-vs-docking md --key <key>" is run
    Then "<output directory>/x-vs-docking.md.<key>/" contains "README.md", "job.sh", "stage.json", "inputs.sha256", "params.json", an empty "out/" and the copied inputs under "inputs/system/"
    And the command prints the steps, naming that folder and "sbatch job.sh"
    And the exit code is 3

  @rq-dc6ab3b5
  Scenario: The steps contain no connection details
    Given the job folder has been written
    Then "README.md" and the printed steps contain "<you>@<cluster>:<directory>"
    And the project contains no host name or remote path

  @rq-3132a729
  Scenario: Return steps leave kept files on the cluster
    When the steps are printed
    Then the rsync that copies the folder back has "--exclude='out/*.trr'"

  @rq-96163724
  Scenario: An existing job folder is never overwritten
    Given the job folder has been written and its "#SBATCH --time" line was edited
    When "tophat stage external" is run again for the same key
    Then the edited line is unchanged

  @rq-172dd12a
  Scenario: SLURM defaults and stage options are merged
    Given ".tophat/slurm.local.yaml" has "--account=e123" and "--time=01:00:00"
    And stage "md" has slurm option "--time=24:00:00"
    When the job folder is written
    Then "job.sh" has "#SBATCH --account=e123" and "#SBATCH --time=24:00:00" and no "#SBATCH --time=01:00:00"

  @rq-3f18bffc
  Scenario: Large inputs are copied with a warning
    Given the inputs of "md" total more than 10 GB
    When the job folder is written
    Then a warning states their total size
    And the inputs are copied

  @rq-c8a251ef
  Scenario: Code files travel with the job folder
    Given stage "md" lists code "scripts/run-md.sh"
    When the job folder is written
    Then it contains "scripts/run-md.sh"

  # --- Job script and receipt (run locally with POSIX tools) ---

  @rq-52e5c665
  Scenario: The job script writes a receipt
    Given the job folder has been copied to a directory standing in for the cluster
    And "md" runs "sh -c 'echo energy > {out}/md.edr; head -c 1000 /dev/zero > {out}/md.trr'"
    When "sh job.sh" is run there with only POSIX tools on PATH
    Then "receipt.txt" has "key <key>", a "script" line, "exit 0", one "output" line for "md.edr" and one "kept" line for "md.trr"

  @rq-bfa261cf
  Scenario: The job refuses changed inputs
    Given a copied input in the job folder was modified
    When "sh job.sh" is run
    Then "receipt.txt" has "exit input-mismatch"
    And the command was not run

  @rq-1b912496
  Scenario: The receipt is written when the command fails
    Given "md" runs a command that exits 7
    When "sh job.sh" is run
    Then "receipt.txt" has "exit 7"
    And the script exits 7

  @rq-82d88391
  Scenario: Receipts are parsed strictly
    Given a receipt whose "output" line has a malformed hash
    When it is parsed
    Then parsing fails naming that line

  # --- Ingest ---

  @rq-afe4a64f
  Scenario: Ingest a returned job folder
    Given the job ran and its folder was copied back without the kept files
    When "tophat stage external x-vs-docking md --key <key>" is run
    Then "md.edr" is moved from the job folder's "out/" into the output directory
    And the stage is sealed with outputs "md.edr" (and not "md.trr")
    And its provenance record has "external" with host, job, start, end, setup, script_sha256, receipt_sha256 and a "kept" entry for "md.trr"
    And the exit code is 0

  @rq-add5fac2
  Scenario: Edited SBATCH lines are accepted
    Given the "#SBATCH --time" line of "job.sh" was edited before submission
    When the returned folder is ingested
    Then the stage is sealed

  @rq-3727b411
  Scenario: Any other edit to the job script is refused
    Given the command line in "job.sh" was edited before submission
    When the returned folder is ingested
    Then ingest fails with ScriptModified
    And no provenance.json is written

  @rq-1d95d762
  Scenario: A receipt for another key is refused
    Given the returned "receipt.txt" has a different key
    When the folder is ingested
    Then ingest fails with KeyMismatch

  @rq-c0f8402d
  Scenario: A failed job is refused
    Given the returned "receipt.txt" has "exit 7"
    When the folder is ingested
    Then ingest fails with JobFailed quoting "7"

  @rq-d571140f
  Scenario: A changed returned file is refused
    Given a returned file differs from its "output" line
    When the folder is ingested
    Then ingest fails with OutputMismatch naming it

  @rq-42c672ac
  Scenario: An unexpected returned file is refused
    Given "out/" contains a file not listed in the receipt
    When the folder is ingested
    Then ingest fails with OutputMismatch naming it

  @rq-4865c985
  Scenario: Kept files copied back anyway are accepted
    Given "out/md.trr" was copied back and matches its "kept" line
    When the folder is ingested
    Then the stage is sealed and "md.trr" is not among its outputs

  # --- End to end (requires Snakemake) ---

  @rq-5cf6fbfe
  Scenario: The pipeline stops at an external stage and resumes after the return
    Given Snakemake is available
    When "tophat run x-vs-docking" is run
    Then it stops with the steps for "md"
    And "tophat status x-vs-docking" shows that the external job is waiting
    When the job folder is copied to a stand-in directory, "sh job.sh" is run there, and the folder is copied back without kept files
    And a pass verdict is given for "md"
    And "tophat run x-vs-docking" is run again
    Then "md" is ingested and "analyse" is sealed

  # --- SLURM defaults ---

  @rq-f9c4d1f4
  Scenario: Invalid SLURM defaults
    Given ".tophat/slurm.local.yaml" has "sbatch: account=e123"
    When "tophat check" is run
    Then an error "pipeline.slurm-defaults-invalid" is reported
```
