# Worked example: median-vs-mean

A complete small study, kept here as a model for planning and building studies. It is never part
of the project's results. To run it in this project:

1. Add a question to `PROJECT.md` that the study answers, and stamp it: `tophat question stamp`.
2. Copy this directory to `studies/median-vs-mean/` and set `question:` in its `STUDY.md` to the
   stamped ID.
3. Commit (on the host), then pre-register it yourself: `tophat study preregister median-vs-mean`.
4. Run it: `tophat run median-vs-mean`. The `filter` stage is external: the run stops and shows the
   steps to run its job folder on a cluster (any machine with a POSIX shell works).

Its scenario tests run with `pixi run python -m unittest discover -s tests` from the study
directory.
