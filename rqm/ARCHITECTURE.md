# Architecture

## Purpose

TopHat is a Copier template that scaffolds research projects in which a scientist works with an LLM
coding agent through a set of structured skills. It applies the ideas of riprap (agent skills that
turn intent into traceable, reviewable written requirements before acting) to scientific research
rather than software features: a study's plan plays the role a requirements file plays for code.

The guiding promise is **verifiable provenance**: every reported result can be traced, mechanically
and without trusting the LLM, back to the pre-registered question and endpoint it answers, the code
and tool versions that computed it, the environment it ran in, and the data it consumed.

TopHat is a community open-source project aimed at the broader scientific community. It must be
general across scientific domains, documented, and stable across template versions. Generated
projects are expected to live for years and to be updated with `copier update`.

TopHat is a standalone template. It borrows riprap's structure and conventions but has no runtime or
build dependency on riprap, and evolves independently.

## Scope and Key Features

### Hierarchy of research units

A generated project organizes work in these levels:

| Level | What it is | Executable | Lifecycle |
|---|---|---|---|
| **Project** | One repository and one overarching research question | No | Living |
| **Question tree** | The project question decomposed into sub-questions, each with a stable ID | No | Living |
| **Study** | The smallest unit that answers **one** question with **one** pre-registered primary endpoint and decision rule | Yes, end-to-end | `draft` → `preregistered` → `reported` (or `superseded`); `retrospective` for adopted past work |
| **Variant** | The same study and endpoint under a different setting (method, level of theory, tool version, data subset) | Yes | Within its study |
| **Stage** | One step of a study's pipeline, with a cached, provenance-stamped output | Yes | — |
| **Dataset** | Raw data, or a study output published for other studies, with a stable name and version | No | Immutable per version |
| **Tool** | General-purpose code used by studies, including code developed for this project's needs | In its own repo | Versioned independently |
| **Exploration** | Scratch analyses and notebooks | Yes | Never cited as a result |

- The **question tree** (`PROJECT.md`) holds only questions and links. A leaf question is answered by
  one or more studies; a parent question is answered by citing its children's results.
- A **study** is defined by its `STUDY.md`: question, primary endpoint and decision rule, numbered
  scenarios (`S<n>`, the testable expectations), variants, stages, tools pinned by version, and
  results. Each study lives in its own directory with its own pipeline.
- Change rules: a new question or new primary endpoint means a **new study** (linked both ways); a
  fix that does not change what is measured is an **amendment**; a different setting under the same
  endpoint is a **variant**.
- **Exploration** is a separate area for unregistered work. It may inform new studies but nothing in
  it may be cited as a result.

### Study lifecycle and pre-registration

- `draft`: everything may change.
- `preregistered`: set by the human (with a date) before any answer-stage result is seen. From then on
  the question, primary endpoint and scenarios are **frozen**; changes go into a dated
  **Amendments** section stating the reason and whether results had been seen.
- `reported`: the **Result** section is filled in only from pipeline output files, one line per
  variant and stage (n, effect size, uncertainty, test statistic), stating plainly when a result is
  not useful or not significant.
- `superseded`: replaced by a linked newer study.
- `retrospective`: a study written up after the fact from work done before TopHat was adopted. It
  uses the same `STUDY.md` structure and, where the work is reproducible, a pipeline, but it was
  never pre-registered. Its results may be cited and are always labelled retrospective.

Only the primary endpoint answers the question; diagnostics inform but decide nothing.

### Running a study end-to-end

Every study runs end-to-end with one command, `tophat run <study>`. Snakemake is the single
first-class workflow engine and does all execution; nobody writes Snakemake rules.

- **Stages are declared, rules are generated.** A study's stages (command, code, inputs, parameters,
  pixi environment, threads, fan-out, QC gate, result file) are declared in `config/base.yaml`, with
  one small override file per variant holding only what differs. The study's `Snakefile` is a fixed
  stub that regenerates the Snakemake rules from the configuration every time Snakemake starts; the
  generated rules are never committed and never stale, and `tophat check` rejects an edited stub.
  Because Snakemake never takes its default target from an included file, the stub itself defines
  the default target, which builds the summary.
- **Stage keys and outputs.** Each stage has a key derived from its command, its pixi environment
  (hashed across all platforms in `pixi.lock`, so keys agree between machines), the contents of its
  code files, its parameters, and its inputs (upstream stage keys and dataset content hashes).
  Outputs live in a directory named by the key, so changing anything that matters runs into a new
  directory and never overwrites a result; unchanged stages are never re-run; and variants with
  identical settings for a stage share one output.
- **Any language.** Stage commands may call scripts in any language available in the project
  environment (e.g. Python, R, Julia, bash). Scripts take input and output paths as arguments and
  never hard-code paths. Users write configuration and scripts only.
- **Fan-out.** A stage may run once per item of an input (e.g. one job per structure), in parallel,
  with per-item provenance; a failed item is retried alone.
- **External stages** — compute that must run elsewhere (e.g. MD simulations on an HPC cluster)
  — are described in *External stages and clusters* below. The agent never hides steps in
  agent-writable scripts that a human is asked to run.
- **QC gates** — a stage declared with `qc: true` requires a human verdict (`pass` or `fail`) on its
  output before any stage that consumes it runs. Verdicts are given only through an interactive
  command, belong to one stage key (any change upstream needs a new verdict), and are committed with
  the study. Each consuming job checks the verdict before doing anything, so a missing verdict stops
  the run with the exact command to give one, and a `fail` blocks everything downstream.
- **Summary.** The default target writes `SUMMARY.md` from the result files of sealed stages, with
  the key behind each result line. `STUDY.md` is never an input of any rule, so editing results or
  status never re-runs analysis.
- **Committed code.** A stage's provenance record is *clean* when every file that determined it (its
  code, the study configuration, the pixi files) is committed unchanged; the summary is *dirty* if any
  record it derives from is not. A dirty summary cannot be reported, and a summary whose keys no
  longer match the configuration is *stale*: it cannot be reported and must be re-run.
- **Trial runs.** A draft study can be run only as a trial (`tophat run --draft`): trial outputs live
  apart, skip QC gates, never produce a summary, and can never be published or reported. After
  pre-registration the real run computes everything again, so results seen before the plan was frozen
  can never be reused as reported results.

### Day-to-day workflow

`tophat status` shows, for every study, its state, pipeline progress per variant, and the single next
action, marked when only a human may take it (pre-registration and QC verdicts). The next action is
computed from committed and local files by a fixed priority order: fix check errors, write the plan,
pre-register, add stages, configure the results root, give or fix QC verdicts, run, commit and re-run
if dirty, report, answer the question in `PROJECT.md`. `tophat run` refuses to start a run that
cannot produce reportable results (check errors in the study's scope, a draft without `--draft`) and
ends by printing the next action. Skills use the same information, in machine-readable form, to
decide what to do next.

### External stages and clusters

The typical pattern is: prepare locally, run heavy compute on a cluster, analyze locally. An external
stage splits a study's pipeline across machines without breaking provenance.

1. **Export.** When the pipeline reaches an external stage, `tophat` writes a job bundle: the input
   files with their hashes, parameters, the stage key, the list of expected outputs, and a generated
   job script (e.g. for SLURM). The pipeline then stops and prints the short, readable commands the
   human runs to transfer the bundle and submit the job.
2. **Run.** The job runs on the cluster with the cluster's own software (e.g. an optimized MD engine
   with MPI), not inside the TopHat container. At the end, the generated job script writes a
   **receipt**: the stage key it was given, the execution environment as reported by the job (host,
   loaded modules, software versions, GPUs, start and end times, exit status), and content hashes of
   every output. The receipt is produced with plain POSIX shell and standard hashing utilities, so
   nothing needs to be installed on the cluster.
3. **Ingest.** Back in the project, `tophat` checks that the receipt's stage key matches the expected
   one (inputs and parameters did not change since export) and that the returned outputs match the
   receipt's hashes. Mismatches, such as stale outputs, the wrong directory copied back, or inputs
   edited after export, fail the stage. On success the outputs become stage outputs with provenance
   and proceed to their QC gate and downstream stages.

Provenance for an external stage is explicit about its limits: inputs, parameters and outputs are
**verified** by hash, while what ran on the cluster is **reported** by the receipt, because the
cluster environment is outside TopHat's control. Provenance records label such stages as executed
externally.

Large outputs (e.g. trajectories) need not return to the local machine. The cluster's storage is
configured as a data root, outputs there are recorded by the hashes in the receipt (computed on the
cluster), and a reduction step at the end of the job produces the smaller files that are transferred
back for local analysis. Lineage runs through the reduced files to the full outputs that remain on
the cluster.

TopHat never transfers data or submits jobs itself; the human runs the printed commands.

### Data and provenance

Data is **never modified in place**. Every piece of data is either raw or derived, and every derived
file has exactly one producer. "Modifying" data always means a study stage producing a new derived
version.

- **Raw datasets** are registered once (`tophat data add`) with a stable ID, a manifest of files and
  content hashes, and origin metadata (source or instrument, date, license). After registration they
  are immutable.
- **Stage outputs** carry a machine-readable provenance record: producing study and stage, stage
  key, the command as executed, project commit and clean/dirty state, environment hash, code hashes,
  parameters, inputs, content hashes of outputs, host and platform. Stage outputs are internal to
  their study.
- **Published datasets** are the only handoff between studies. A study explicitly promotes a stage
  output to a named, versioned derived dataset (e.g. `clean-structures@v3`), whose manifest links
  back to the producing stage's provenance. Another study consumes it by pinning that name and
  version, never by reaching into the producing study's results by path. Intermediate outputs not
  published stay private to their study, so restructuring a study's stages cannot break consumers.
- **Pinning, never following.** Inputs are always pinned versions, never moving pointers (such as
  "latest" or "current"). When a producer publishes a new version, consumers keep their pinned
  version until someone deliberately bumps it, recorded as an amendment or a new variant.
  `tophat check` reports that a newer version is available (for studies not yet reported), and
  reports an error when the pinned version has been **retired** because of a defect.
- **Maturity.** A study may consume datasets published by studies that are not yet `reported`.
  `tophat check` flags the dependency on unreported work, and the consuming study cannot be marked
  `reported` until its producers are, unless the human explicitly accepts the dependency in an
  amendment.
- **Declared datasets** cover data whose history TopHat did not observe, typically processed data
  that existed before adoption. They are registered with an ID and hashes like any dataset, plus a
  human-written statement of their origin. Lineage stops at them and is marked *declared, not
  verified*; studies may consume them, and every result depending on one is labelled as such.
- **Lineage in both directions** is a CLI query: from any result or dataset back to the raw data it
  came from, and from any dataset forward to every study that consumed it. Which study ran on which
  data is always answerable from the committed manifests alone.

**Storage.** Data bytes live outside git, under one or more named **data roots** (e.g. a local
directory or a cluster filesystem path). Root names are declared in the project; each machine says
where they are in a git-ignored file or an environment variable, so the same project works on a
laptop and on a cluster with different locations, and a root that is not present on a machine is a
warning rather than an error. The root `results` holds stage outputs. Committed manifests refer to
files relative to a named root. TopHat verifies presence and hashes but never copies, moves, or
deletes data, and does not require a data-versioning tool.

**Verification.** Re-hashing terabytes on every check is impractical, so verification has two
levels. `tophat check` is fast: it confirms that every manifest file exists with its recorded size,
and uses a per-machine hash cache to flag files modified since they were last verified. Full
re-hashing is explicit (`tophat data verify`) and is performed implicitly by commands that create
integrity claims, such as publishing a dataset.

**Visibility.** Every dataset and study carries a visibility label, `public` or `private` (e.g.
client or patient data). `tophat check` enforces that private data never flows into a public study
or published public dataset. By default a project is a single repository. A project that wants to
open-source its public part pairs it with a **private companion** TopHat repository, which holds the
private studies and may consume the public repository's published datasets, but never the reverse.

**Other projects.** A project may consume datasets published by another TopHat project. It pins that
project like a tool (`other-project@commit`) and references its datasets as
`other-project:name@version`. `tophat` reads the pinned project's committed manifests, so lineage
continues across the project boundary to the original raw data. The data bytes must be reachable
through one of the consuming project's data roots. A private companion repository uses the same
mechanism to consume its public counterpart.

### Adopting existing projects

TopHat can be added to research that already exists. Past work is labelled honestly rather than
rejected or presented as verified.

- **Two adoption paths.** *In place*: the template is applied to the existing repository (on a
  branch); template files are namespaced so collisions are rare, and Copier reports conflicts rather
  than overwriting user files. *Wrap*: a fresh TopHat project is generated, the old repository is
  pinned as a tool, and its data is registered. An adoption skill (`th-adopt`) inventories the
  existing repository and recommends a path.
- **Mapping.** The adoption skill proposes, and the human approves, how existing content maps onto
  TopHat: reusable code to tools, raw data to raw datasets, processed data of unknown history to
  declared datasets, past analyses to retrospective studies, existing environments into pixi
  (e.g. from `environment.yml`, `requirements.txt`, or a Julia `Project.toml`), and existing
  pipelines into study stage configurations where practical. Content not yet mapped is listed as
  legacy.
- **Gradual adoption.** New studies receive the full checks from the start. Legacy items (declared
  datasets, retrospective studies, unmapped legacy paths) pass `tophat check` but are reported in a
  summary, so a project can migrate incrementally without hiding gaps.

### Non-Goals

- Executing or scheduling compute itself; Snakemake runs pipelines and external stages run where the
  human runs them.
- Workflow engines other than Snakemake as first-class integrations.
- Storing, versioning, or transferring data.
- Calling LLM APIs or running an agent loop; the user's coding agent is the LLM runtime.
- Domain-specific science libraries (simulation, statistics, plotting).
- Developing tools inside the research project; tool development follows its own repository and
  process (ideally a riprap project).
- A plugin system with runtime loading of third-party code.
- Electronic lab notebook, LIMS, or publication-management features.

## Architectural Decisions

### Repository components

This repository is one Cargo workspace plus the template sources:

1. **Template** — the Copier template that generates research projects: agent skills, the dev
   container and launcher, the pixi environment, the `PROJECT.md` question tree, study templates and
   a worked example study, CI, and hooks.
2. **`tophat` CLI** (Rust) — a prebuilt binary shipped into every generated project's container. It
   owns everything that must be exactly right: IDs, stage keys, hashing, provenance records,
   validation of `PROJECT.md` and `STUDY.md` structure and status rules (frozen sections, amendment
   format, scenario-to-test traceability), lineage queries, `tophat check` (the integrity gate used
   by agents, humans, hooks and CI alike), and `tophat status` and `tophat run`.
3. **Generated Snakemake rules** — `tophat` generates each study's rules from its configuration. The
   **interface is the CLI**: generated rules hold no integrity logic of their own; every step that
   matters (preparing a job's output directory, substituting the command, checking QC verdicts,
   sealing outputs, writing the summary) is a `tophat` command the rule calls. Guarantees therefore
   live in one place and remain usable from any future engine. Generated rules call `tophat` with
   argument lists rather than shell strings, so commands are never re-interpreted by Snakemake.
4. **Template tooling** (Rust, same workspace) — development-only tooling that renders the template,
   lints skills, and runs the test suites below. Not shipped to generated projects.

### Division of labour between skills and code

Skills (Markdown instructions) handle judgment: refining questions, designing studies, eliciting the
endpoint and decision rule from the human, writing pipelines, interpreting results. Code handles
anything deterministic or integrity-critical. Skills call the CLI for those operations; the agent
never invents an ID, computes a hash, or writes a result number by hand. Every integrity guarantee
holds with no LLM involved.

Decisions that belong to the scientist — the question, primary endpoint, decision rule, thresholds,
pre-registration, QC verdicts — are always elicited from the human and never invented by the agent.

Skills operate in explicit modes, following the study skill pattern: a **plan** mode that writes only
the study document, and a **build** mode that implements the pipeline from a pre-registered study
and never edits its frozen sections.

### Agent neutrality

Canonical skills are written agent-neutrally in a managed directory. Each supported agent (Claude
Code, Codex, OpenCode, …) gets thin adapters that point at the canonical skill. Skills must not depend
on features unique to one agent.

### Languages and environments

A TopHat project is not a software package and has no single project language.

- **pixi** manages the project environment from conda-forge and PyPI, with a committed lockfile. It
  covers interpreters (Python, R, Julia), command-line tools, and GPU runtime libraries.
- Copier offers a multi-select of languages to preinstall. A Python interpreter is always installed
  because Snakemake runs on it, but Python is infrastructure, not a language users are expected to
  write: Snakefiles are generated, so a project may be, for example, pure Julia.
- Each stage runs in a named pixi environment, and that environment's hash is part of the stage key.
  A tool may bring its own locked environment, pinned with the tool's version.

### Tools

Tools are separate repositories with their own requirements, tests and release process. A study
pins `tool@commit` for every stage the tool computes.

`tophat` manages a git-ignored tool cache: it checks out each pinned `name@commit` into its own
directory, so different variants can use different versions of the same tool side by side. A tool's
environment is its own pixi lockfile at that commit; for a tool without one, the project supplies a
committed lockfile for it. The tool commit and the environment lock hash are part of the stage key.
Git submodules are not used, because they allow only one version of a tool at a time. Data shipped inside a tool repository is read from the pinned checkout so the commit fixes
the input. Studies never live inside tool repositories, and tool code is never changed from within a
study; it goes through the tool's own process.

### Container and execution environment

All work happens inside a dev container (Podman locally, Apptainer on clusters). TopHat owns its
launcher.

- **GPU passthrough**: the launcher detects NVIDIA or AMD GPUs on the host and passes them through
  (NVIDIA via CDI on Podman and `--nv` on Apptainer; AMD via device nodes on Podman and `--rocm` on
  Apptainer). A user setting can force GPU on or off. Without a usable GPU the launcher falls back to
  CPU with a clear message. The host supplies the driver; GPU runtime libraries come from the pixi
  environment, not the base image.
- The `tophat` CLI itself is single-node and CPU-only. Hashing streams large files and may be
  parallelized across files.

### Language, dependencies, and distribution of TopHat's own code

- The CLI and tooling are written in Rust (edition 2021).
- Dependencies are kept small and well established. Avoid heavy frameworks or dependencies for small
  tasks.
- The `tophat` binary is released as static Linux binaries (GitHub releases with checksums). The
  container image downloads the version pinned by the template and verifies its checksum; `copier
  update` changes the pin. Generated projects do not need a Rust toolchain.
- `tophat` is needed only inside the container, where Snakemake runs and calls it. External stages
  on clusters depend only on POSIX shell and standard hashing utilities.
- Snakemake runs each job in its own process, so commands that generated rules call may run
  concurrently; files they share are written atomically through per-process temporary files and
  left untouched when unchanged.
- Every provenance record stores the `tophat` version that produced it, and released versions are
  kept available indefinitely so old projects can rebuild their environments.
- The `PROJECT.md` / `STUDY.md` structure, provenance record format, and stage-key derivation are a
  **versioned, stable contract**. Changes must be backward compatible or ship with a migration the CLI
  applies, because `copier update` brings existing projects onto new template versions. Stage keys
  must not change under an update unless their inputs did, or every study would re-run.

### Failure behaviour

TopHat never assumes the happy path. Missing or changed data, dirty commits behind reported results,
missing QC verdicts, edits to frozen sections, scenarios without tests (or the reverse), and dangling
links produce explicit, actionable diagnostics and a non-zero exit code from `tophat check`. They are
never silently skipped. Legacy items in a partially adopted project are the one tolerated exception:
they do not fail the check, but are always listed in its output.

## Extensibility

Extension is through files, not plugins:

- **Per-skill local extensions** — each skill reads a project-owned `local.md` that survives
  `copier update` and can add or override instructions, e.g. domain conventions or the project's
  rule for new study vs. amendment vs. variant.
- **Project schemas** — projects may declare additional structured fields for studies and stages
  (e.g. a wet-lab sample sheet) in a schema file that the CLI validates.
- **Study templates** — projects may add their own `STUDY.md` templates and worked example studies
  for recurring study types.
- **Copier answers** select optional components at generation time (languages, agents, GPU defaults).

## Testing Strategy

Testing establishes that TopHat itself works:

1. **Template render tests** (required CI gate) — render the template across a matrix of Copier
   answers (languages, agents; including a single-language non-Python project such as Julia-only)
   and verify each generated project builds, its environment solves, and
   it passes `tophat check` from its initial state. GPU detection logic is unit-tested against
   simulated host configurations, since CI has no GPU.
2. **CLI conformance suite** (required CI gate) — golden example projects, some valid and some
   deliberately broken (edited frozen sections, scenario without a test, result from a dirty commit,
   changed input hash, missing QC verdict, dangling question link, consuming another study's
   unpublished output, a pinned dataset superseded for a defect, reporting a study whose producers
   are unreported, private data flowing into a public study, an external-stage receipt
   with a mismatched stage key or output hash). Every CLI version must accept the
   valid ones and report the expected diagnostic for each broken one. This suite is the executable
   definition of the provenance guarantees.
3. **End-to-end example study** (required CI gate) — a small toy study, with scripts in several
   preinstalled languages, an external stage and a QC gate, is run end-to-end with Snakemake in a
   generated project. The external stage's job script is executed outside the container with only
   POSIX tools available, and the export–receipt–ingest cycle is exercised including stale and
   mismatched returns. The test verifies the outputs, provenance, cache-key behaviour (a changed tool
   version re-runs exactly the downstream stages) and summary.
4. **Adoption tests** (required CI gate) — the template is applied in place to fixture repositories
   representing typical existing projects (scripts and notebooks, conda or Julia environments,
   existing Snakemake or Make pipelines, processed data). The tests verify that no user file is
   overwritten, that the result passes `tophat check` with legacy items reported, and that
   declared datasets and retrospective studies are labelled in lineage.
5. **Skill scenario evals** (periodic or manual, not a CI gate) — scripted agent sessions on toy
   questions, checking that the study documents and pipelines the agent produces pass `tophat check`
   and that the agent does not invent endpoints or edit frozen sections.

Unit and integration tests for the CLI sit alongside these, one test per Gherkin scenario of each
requirements file. Tests that need real Snakemake run only when it is available, so the CI image must
provide Snakemake and pixi for the end-to-end tests to count as a gate.

## Open Questions

- Exact set and names of `th-` skills and how plan, build, QC and reporting are split between them.
- The detailed design of tool stages (the tool cache and tool environments) and external stages; the
  stage configuration reserves `tool:` and `external:` for them.
- Whether scenario tests run as part of the pipeline (e.g. a generated check stage before the
  summary) or separately.
- Object-store data roots (e.g. `s3://`), which are not supported yet.
- How a declared dataset can later be upgraded to verified (e.g. a study that re-derives it from raw
  data and shows matching hashes).
- Retention: how users are warned before deleting data that a pinned dataset version still needs.
- Whether notebooks in exploration get any tooling beyond being excluded from citation.
- Which batch schedulers external-stage job scripts support beyond SLURM, and how job arrays and
  multi-job stages map onto a single receipt.
