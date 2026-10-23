<p align="center">
  <img src="assets/logo.svg" alt="TopHat logo: a black top hat with a blue and yellow band" width="160">
</p>

<h1 align="center">TopHat</h1>

<p align="center">
  <strong>AI-assisted research where the scientist stays in charge, the agent does the legwork, and
  every result comes with its provenance.</strong>
</p>

---

TopHat is a [Copier](https://copier.readthedocs.io/) template for research projects done with an
LLM coding agent, such as Claude Code. It divides the work along a clear line:

- **You make the scientific decisions.** You ask the questions, choose what will be measured and
  what result would count as an answer, pre-register that plan, judge whether outputs are fit to
  use, and interpret what you find. The agent helps you think these through but is never allowed to
  make them for you.
- **The agent does the legwork.** It writes and runs the pipelines, and keeps the records that
  provenance needs: which question a study answers, which data and tool versions it used, which
  environment it ran in.
- **`tophat` checks the agent's work mechanically.** Every result you report can be traced, without
  trusting the LLM, to the question it answers, the plan you made before seeing it, the code and
  environment that computed it, and the data it consumed.

All of this runs in a rootless container that keeps the agent away from the rest of your machine
and your credentials (see [How safe is it?](#how-safe-is-it) for what that does and does not
protect). TopHat works in any scientific field and any language your pipeline needs (Python, R,
Julia, shell, …), and it is built for projects that live for years.

> **Status: pre-release.** TopHat works end to end on its worked example, but no `tophat` binary
> has been released yet, and the study format may still change before 1.0. See
> [Getting started](#getting-started) for running it today.

## Why "TopHat"?

When you are awarded a doctorate in Sweden, the conferment ceremony crowns you with a top hat. It
marks the moment the academic community trusts your research: you have shown, in public, that you
can ask a question, answer it honestly and defend how you got there.

TopHat is about that kind of trust. An AI agent can now write a pipeline in minutes, which makes it
easier than ever to produce results, and harder to tell which of them deserve belief. The hat
stays with the scientist: your domain expertise decides what is asked and what counts as an answer.
The agent takes on the tedious parts, including the record-keeping that usually gets skipped, under
rules it cannot quietly break.

*(Not to be confused with the TopHat RNA-seq read aligner, which is unrelated.)*

## How it works

### The scientist decides, the agent builds, `tophat` checks

| You, the scientist | The agent | The `tophat` CLI |
|---|---|---|
| Ask the questions; choose the endpoint and decision rule | Helps you phrase them; never invents them | Gives each question a stable ID |
| Pre-register the plan | Writes the plan with you | Freezes it and records the date |
| Inspect outputs and give QC verdicts | Shows you what to check | Stops the pipeline until you have |
| Run cluster jobs | Prepares the job and shows you the steps | Writes the job folders and verifies what comes back |
| Commit your work | Tells you what to commit | Records which commit each result came from |
| Write the interpretation | Applies the decision rule you chose | Copies the result lines into the study |

Skills (Markdown instructions for the agent) handle judgment. Everything that must be exactly
right, such as IDs, hashes, provenance records and validation, is done by `tophat`, a Rust CLI. Every
integrity guarantee holds with no LLM involved.

### A project is a tree of questions, answered by studies

- **`PROJECT.md`** holds the project's question, broken down into sub-questions with stable IDs.
- A **study** answers one question with one pre-registered **primary endpoint** and **decision
  rule**. Its `STUDY.md` also lists numbered **scenarios** (testable expectations of the pipeline,
  each checked by a test), the **variants** it runs under, and its **stages**.
- A study moves from `draft` to `preregistered` to `reported`. Once pre-registered, the question,
  endpoint, decision rule and scenarios are frozen; later changes are dated amendments that say
  whether results had been seen.
- An **exploration** area holds scratch work that can inspire studies but is never cited as a
  result. Work done before adopting TopHat can be written up as a clearly labelled **retrospective**
  study.

Here is the decision rule of the worked example that ships with every project:

> The median filter is better if the 95% bootstrap interval of the endpoint (2000 resamples of the
> signals, seed 1) lies entirely above 0. If the interval contains 0, the study shows no
> difference; if it lies entirely below 0, the mean filter is better.

### One command runs a study, and nothing runs twice

`tophat run <study>` runs the whole pipeline with [Snakemake](https://snakemake.github.io/). You
declare stages in a configuration file and write ordinary scripts; nobody writes Snakemake rules.

- Each stage has a **key** derived from its command, code, parameters, inputs and its locked
  [pixi](https://pixi.sh/) environment. Outputs live in a directory named by the key, so changing
  anything that matters runs into a new directory, never over an old result, and unchanged stages
  never re-run.
- **QC gates** let you inspect a stage's output and give a verdict before anything downstream uses
  it.
- **Tools** (your own or others' code) live in their own repositories and are pinned by commit, so
  different variants can use different versions side by side.
- **External stages** split a pipeline across machines: TopHat writes a self-contained job folder,
  you run it on your cluster, and on return every file is checked against the hashes the job
  recorded. TopHat never connects to a remote machine itself.
- **Data** is never modified in place. Raw datasets are registered with content hashes; studies
  hand data to each other only through named, versioned, published datasets; and lineage can be
  queried in both directions.
- A draft can be run as a **trial** to debug the pipeline, but trial results are kept apart and can
  never be reported.

`tophat check` ties it together. It fails loudly on edits to frozen sections, results from
uncommitted code, stale summaries, missing QC verdicts, scenarios without tests, private data
flowing into public studies, and more. Agents, hooks, CI and humans all use the same check.

## How safe is it?

Safe-ish. An LLM agent can be misled by text it reads (prompt injection) or simply make mistakes,
so TopHat limits what it can reach rather than trusting it to behave. All work, including the
agent, runs inside a rootless container: Podman on your machine, Apptainer on a cluster.

What the container protects:

- **The rest of your machine.** The agent sees the project directory and your configured data
  locations, and nothing else from your home directory.
- **Your credentials.** No SSH keys, Git credentials, cloud credentials or host agent configuration
  enter the container. The agent logs in separately, and that login is stored apart from your own.
- **Your Git history and remotes.** The container has no Git identity and refuses to commit. You
  review and commit on the host, so nothing the agent writes reaches a result or a remote without
  passing you.
- **Your clusters.** The agent never connects to a remote machine, transfers data or submits jobs.
  It prepares a job folder and you run it.

What it does not protect:

- **Files the agent can see.** It can change or delete files in the project and in your mounted
  data locations. Keep data backed up and commit often; `tophat check` will flag data that no longer
  matches its recorded hashes.
- **Data leaving over the network.** The container needs network access for the agent and for
  installing packages, so a misled agent could send out what it can read. Don't mount data that
  must not leave your machine without considering this.
- **Root-level escapes.** A rootless container is a strong barrier, not an absolute one.

## Getting started

### Prerequisites

- [Podman](https://podman.io/docs/installation) (or Apptainer on a cluster)
- [Copier](https://copier.readthedocs.io/) 9 or later
- A [Claude Code](https://claude.com/claude-code) account (the only supported agent so far)

### Create a project

```sh
copier copy gh:<owner>/tophat my-project
cd my-project
./th.sh       # builds and enters the dev container
claude        # start the agent inside the container
```

Copier asks for the project's name, description and author, any languages to install besides
Python, and the default GPU setting.

Until the first release, the container needs a `tophat` binary you build yourself. Run
`cargo build --release` in this repository and point `TOPHAT_DEV_BINARY` at the result before
running `./th.sh`.

### First steps in a project

The agent knows the workflow, so you can simply tell it what you want to find out. The commands
underneath are:

| Goal | Command |
|---|---|
| Where am I, and what's next? | `tophat status` |
| Start a study | `tophat study new <study> --question <question ID>` |
| Pre-register it | `tophat study preregister <study>` |
| Run (or continue) the pipeline | `tophat run <study>` |
| Give a QC verdict | `tophat stage qc <study> <stage> --variant <variant>` |
| Report the result | `tophat study report <study>` |
| Check the whole project | `tophat check` |

Each generated project includes a guide, `.tophat/managed/GUIDE.md`, that follows one study from
question to report, and the worked example `median-vs-mean` to copy from.

### Keeping a project up to date

```sh
copier update
```

Updates bring in new skills, checks and container fixes. They never overwrite your `PROJECT.md`,
studies, data or local skill extensions. The study format and stage keys are a versioned contract:
an update must not invalidate your results or make finished studies re-run.

## What TopHat is not

- A workflow engine or job scheduler. Snakemake runs pipelines; you run cluster jobs.
- A data store. Your data stays where it is; TopHat records and verifies its hashes.
- An LLM runtime. Your coding agent is the LLM; TopHat gives it structure and rules.
- A domain library, electronic lab notebook or LIMS.

## Related work

TopHat builds on ideas from riprap (agent skills that turn intent into
written, reviewable requirements before code is written), applied to research instead of software.
Other projects worth knowing:

- [preregister](https://github.com/reinthal/preregister): an agent skill that interrogates the
  researcher and freezes a pre-registration before an experiment.
- [research-methods](https://github.com/phdemotions/research-methods): Claude Code skills for
  reproducible quantitative research in business and social science.
- [showyourwork](https://github.com/showyourwork/showyourwork): Snakemake-based workflow that links
  every figure in a paper to the code that made it.
- [DVC](https://dvc.org/): content-addressed data and pipeline versioning.

## Developing TopHat

This repository contains the Copier template (`template/`), the `tophat` CLI (`src/`, Rust) and
development tooling (`xtask/`). The design is in [`rqm/ARCHITECTURE.md`](rqm/ARCHITECTURE.md), and
every behaviour is specified as a requirements file with Gherkin scenarios under [`rqm/`](rqm/).

TopHat is itself developed with riprap: all development, including the agent, happens inside a
dev container.

```sh
bash rr.sh        # Linux/macOS; rr.bat on Windows. Builds and enters the container.
cargo test        # inside the container
```

The workflow is requirements first. Change or add the requirements file under `rqm/`, then
implement it, and keep code and requirements in step. The `rr-plan` and `rr-implement` skills
guide the agent through this. A few rules:

- **Only run the agent inside the rootless container.** Never give it private keys, write access to
  remote repositories, or unreviewed pushes.
- **Keep credentials out of Git.** Install the pre-commit scanner with
  `bash rr.sh --install-git-hooks`.
- **Don't edit managed files.** Files under `.riprap/managed/` belong to riprap; project
  customization goes in `.riprap/user/`.

On a compute cluster without Podman, build an Apptainer image elsewhere with
`bash rr.sh --export-sif`, copy the repository over, and run `bash rr.sh --run-sif`.

## License

BSD 3-Clause. See [LICENSE](LICENSE).
