# External job

This folder is a self-contained job written by TopHat. It holds:

- `job.sh`: the sbatch script. It verifies the inputs, runs the command, and writes `receipt.txt`.
- `stage.json`: the study, stage, variant, key and keep patterns.
- `inputs/`: copies of every input file; `inputs.sha256` lists their hashes.
- `params.json`: the stage's parameters, and the study's code files at their usual paths.
- `out/`: empty; the job writes its outputs here.

## Steps

```sh
External job for stage `md` of study `external-study` (variant base), key sk-37ff266a46ae1a06.
Job folder: /data/results/external-study/md/sk-37ff266a46ae1a06/job

TopHat never connects to the cluster: you copy the folder, submit the job and copy it back. The
commands are examples; <you>@<cluster>:<directory> stands for your login and a directory of your
choice on the cluster.

# 1. Copy this folder to the cluster
rsync -a /data/results/external-study/md/sk-37ff266a46ae1a06/job/ <you>@<cluster>:<directory>/external-study.md.sk-37ff266a46ae1a06/

# 2. On the cluster, submit it (you may edit the #SBATCH lines first; change nothing else)
cd <directory>/external-study.md.sk-37ff266a46ae1a06 && sbatch job.sh

# 3. When the job has finished, copy the folder back, leaving the large files on the cluster
rsync -a --exclude='out/*.trr' \
  <you>@<cluster>:<directory>/external-study.md.sk-37ff266a46ae1a06/ /data/results/external-study/md/sk-37ff266a46ae1a06/job/

# 4. Continue the pipeline
tophat run external-study

If the job fails for a reason outside the configuration (a time limit, a node failure), delete
receipt.txt and the contents of out/ in the folder on the cluster and submit it again. If the
configuration must change, the next run writes a new job folder.
```
