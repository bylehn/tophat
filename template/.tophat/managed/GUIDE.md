<!-- TopHat-managed: how a study works. `copier update` changes this file. -->

# How a study works

TopHat makes every result you report traceable, mechanically and without trusting anyone's memory,
to the question it answers, the plan you committed to before seeing it, the code and environment
that computed it, and the data it consumed. This page follows one study from question to report and
says why each step exists. The worked example in `.tophat/managed/examples/median-vs-mean/` goes
through all of them.

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

## 2. The plan, and pre-registration

A study's `STUDY.md` says what will be measured (the **primary endpoint**), how its value answers
the question (the **decision rule**, with every threshold), and what must be true of the pipeline
(the **scenarios**, each checked by a test). You pre-register it with
`tophat study preregister <study>` before any result is seen. From then on the question, endpoint,
decision rule and scenarios are frozen; a later change is an amendment, dated and saying whether
results had been seen. This is what makes a result evidence rather than a story told afterwards.

A draft can be run as a trial (`tophat run <study> --draft`) to debug the pipeline. Trial results
are kept apart and can never be reported.

## 3. Stages and keys

The pipeline is a list of **stages** in `config/base.yaml`; each runs one command and writes one
output. A stage's **key** is a fingerprint of everything that determines its output: the command,
the code, the parameters, the environment and the keys of its inputs. Outputs live in a directory
named by the key, such as `results/<study>/<stage>/<key>/`, so:

- changing anything that matters gives a new key and a new directory: nothing is overwritten;
- unchanged stages are never run again;
- variants with identical settings for a stage share its output.

Each output directory has a `provenance.json` recording what produced it.

## 4. Validation and review

What code can check, code checks, every time: a stage's `validate` command (file counts, shapes,
missing values, ranges) runs right after the stage, and a failed check stops it before its output
is sealed. You are asked only for judgment: a stage with a `review` question (does this plot look
right, does this fit make sense) waits for your verdict before anything that uses it runs, and
before the study can be reported. You choose which stages get one. When the run stops there, look
at the output and answer with `tophat stage qc <study> <stage> --variant <variant>`. A `fail`
blocks everything downstream until the cause is fixed upstream, which gives the stage a new key and
asks for a new verdict.

## 5. External (cluster) stages

Heavy compute can run on a cluster. When the pipeline reaches such a stage it writes a job folder
(`<output directory>/job/`) and stops with the steps: copy the folder to the cluster, submit
`job.sh`, copy the folder back, and run the pipeline again. TopHat never connects to the cluster
itself. The job checks its inputs, and on return its receipt is checked before the outputs are
accepted.

## 6. Summary and reporting

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
| Pre-register | `tophat study preregister <study>` |
| Run (or continue) the pipeline | `tophat run <study>` |
| Stage keys and output directories | `tophat pipeline keys <study> --variant <variant>` |
| Answer a review question | `tophat stage qc <study> <stage> --variant <variant>` |
| Steps for a cluster job | `tophat stage instructions <study> <stage> --variant <variant>` |
| Report | `tophat study report <study>` |
| Check the whole project | `tophat check` |
