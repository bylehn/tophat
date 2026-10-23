<!-- TopHat-managed: how a study works. `copier update` changes this file. -->

# How a study works

TopHat makes every result you report traceable, mechanically and without trusting anyone's memory,
to the question it answers, the plan you committed to before seeing it, the code and environment
that computed it, and the data it consumed. This page follows one study from question to report and
says why each step exists. The worked example in `.tophat/managed/examples/median-vs-mean/` goes
through all of them; the agent can walk you through it as a practice study.

The path of a study, in five words: **explore, plan, lock, run, report.** Terms are explained in
the glossary at the end.

## Who does what

| You (the scientist) | The agent | `tophat` |
|---|---|---|
| Ask the questions, choose the endpoint and decision rule | Help you phrase them; never invents them | Gives questions their IDs |
| Pre-register the plan | Writes the plan with you | Freezes it and records the date |
| Answer the review questions you chose | Writes the validation checks; shows you what to review | Runs every check; stops for your verdicts |
| Run cluster jobs | Shows you the steps | Writes and checks the job folders |
| Commit, on the host | Tells you what to commit | Records which commit each result came from |
| Write the interpretation | Applies the decision rule you chose | Copies the result lines into the study |

## 1. The question

`PROJECT.md` holds the project's overarching question and its sub-questions. A study answers one
of them. `tophat question stamp` gives each question a stable ID.

## 2. Explore first

You cannot choose a good endpoint or threshold for data you do not understand yet: chosen too
early, they are guesses. So explore first. Scratch analyses and notebooks go in `exploration/`;
nothing there is ever cited as a result. You can also write a draft study and run its pipeline as
a trial (`tophat run <study> --draft`) to debug it and to learn what the data looks like.

What you see while exploring shapes the plan, which is the point. The one thing to avoid is
exploring on exactly the data that will answer the question and then pretending you had not seen
the answer. Where you can, explore on a subset (a pilot set, a few systems), and keep the rest for
the run that decides the question. A stage's `trial` field says what trial runs use instead:

```yaml
    inputs: {raw: dataset:bench@v2}
    trial:
      inputs: {raw: dataset:bench-pilot@v1}
```

Keep scratch work for a study in `exploration/<study>/`. TopHat keeps a record of the exploration
in the study's `EXPLORATION.md` (written after every trial run): the trial results, which data
they used, and the exploration files. When you lock the plan it records whether the exploration
already saw the data that decides the question. Nothing is refused for that, but such a result is
labelled "plan informed by its deciding data", the way an amendment says whether results had been
seen.

## 3. The plan, and locking it

Once you know what to measure, the study's `STUDY.md` says it: the number that answers the
question (the **primary endpoint**), how that number answers it (the **decision rule**, with every
threshold), and what must be true of the pipeline (the **scenarios**, a checklist such as
`- S1: …`, each checked by a test). Then you lock the plan, which TopHat calls pre-registering:
`tophat study preregister <study>`. From then on the question, endpoint, decision rule and
scenarios are frozen; a later change is an amendment, dated and saying whether results had been
seen. The pipeline itself can still change. Locking the plan before the deciding run is what makes
the result evidence rather than a story told afterwards.

Trial results from the draft are kept apart and can never be reported: after locking, the real run
computes everything again. Commit `EXPLORATION.md` with the locked `STUDY.md`; it is then part of
the locked record and must not change.

## 4. Stages and keys

The pipeline is a list of **stages** in `config/base.yaml`; each runs one command and writes one
output. A stage's **key** is a fingerprint of everything that determines its output: the command,
the code, the parameters, the environment and the keys of its inputs. Outputs live in a directory
named by the key, such as `results/<study>/<stage>/<key>/`, so:

- changing anything that matters gives a new key and a new directory: nothing is overwritten;
- unchanged stages are never run again;
- variants with identical settings for a stage share its output.

Each output directory has a `provenance.json` recording what produced it.

## 5. Validation and review

What code can check, code checks, every time: a stage's `validate` command (file counts, shapes,
missing values, ranges) runs right after the stage, and a failed check stops it before its output
is sealed. You are asked only for judgment: a stage with a `review` question (does this plot look
right, does this fit make sense) waits for your verdict before anything that uses it runs, and
before the study can be reported. You choose which stages get one. When the run stops there, look
at the output and answer with `tophat stage qc <study> <stage> --variant <variant>`. A `fail`
blocks everything downstream until the cause is fixed upstream, which gives the stage a new key and
asks for a new verdict.

## 6. External (cluster) stages

Heavy compute can run on a cluster. When the pipeline reaches such a stage it writes a job folder
(`<output directory>/job/`) and stops with the steps: copy the folder to the cluster, submit
`job.sh`, copy the folder back, and run the pipeline again. TopHat never connects to the cluster
itself. The job checks its inputs, and on return its receipt is checked before the outputs are
accepted.

## 7. Summary and reporting

When every stage has run, the pipeline writes `SUMMARY.md` with one result line per variant. A
result computed from uncommitted code is **dirty** and cannot be reported: commit, then run again.
`tophat study report <study>` copies the result lines into the study. The agent tells you what your
decision rule gives; you write `## Interpretation`. Finally the question in `PROJECT.md` can be
answered, citing the study.

## Commands

| Step | Command |
|---|---|
| Where am I? | `tophat status` |
| Stamp new questions | `tophat question stamp` |
| Start a study | `tophat study new <study> --question <question ID>` |
| Try the pipeline on a draft | `tophat run <study> --draft` |
| Lock the plan (pre-register) | `tophat study preregister <study>` |
| Run (or continue) the pipeline | `tophat run <study>` |
| Stage keys and output directories | `tophat pipeline keys <study> --variant <variant>` |
| Answer a review question | `tophat stage qc <study> <stage> --variant <variant>` |
| Steps for a cluster job | `tophat stage instructions <study> <stage> --variant <variant>` |
| Report | `tophat study report <study>` |
| Check the whole project | `tophat check` |

## Glossary

- **Pre-register** (lock the plan): freeze a study's question, primary endpoint, decision rule and
  scenarios before the run that answers the question, so the result is judged by the plan made in
  advance.
- **Primary endpoint**: the one number that answers the study's question, and how it is measured.
- **Decision rule**: how the endpoint's value maps to an answer, with every threshold.
- **Scenario**: one checkable expectation of the pipeline (`S1`, `S2`, …), each tested by a test
  that mentions `th:S1`.
- **Amendment**: a dated change to a locked plan, saying whether results had been seen.
- **Variant**: the same study and endpoint under a different setting (a parameter, a method, a
  data subset).
- **Stage key**: a fingerprint of everything that determines a stage's output; outputs live in a
  directory named by it.
- **Validation**: a stage's automatic check of its own output, run every time.
- **Review**: a question about a stage's output that you answer by looking, before anything uses it.
- **Trial run**: a run of a draft study, kept apart, never reportable.
