"""Computes the primary endpoint (stage `analyse`): the mean over signals of RMSE(mean filter) −
RMSE(median filter), with a 95% percentile bootstrap interval over signals, and writes the single
result line to <out>/result.txt."""

import argparse
import random
from pathlib import Path


def differences(scores_dir):
    """RMSE(mean) − RMSE(median) of each signal, from <scores>/items/<signal>/rmse.tsv."""
    diffs = []
    for path in sorted(Path(scores_dir, "items").glob("*/rmse.tsv")):
        mean_rmse, median_rmse = (float(x) for x in path.read_text().splitlines()[1].split("\t"))
        diffs.append(mean_rmse - median_rmse)
    if not diffs:
        raise SystemExit(f"no items/*/rmse.tsv under {scores_dir}")
    return diffs


def endpoint(diffs, resamples, seed):
    """The mean difference and its 95% percentile bootstrap interval."""
    rng = random.Random(seed)
    n = len(diffs)
    means = sorted(sum(rng.choices(diffs, k=n)) / n for _ in range(resamples))
    low = means[int(0.025 * (resamples - 1))]
    high = means[int(0.975 * (resamples - 1))]
    return sum(diffs) / n, low, high


def result_line(diffs, resamples, seed):
    estimate, low, high = endpoint(diffs, resamples, seed)
    return (
        f"n={len(diffs)} mean RMSE(mean) - RMSE(median) = {estimate:.4f}, "
        f"95% bootstrap interval [{low:.4f}, {high:.4f}]"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scores", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--resamples", type=int, required=True)
    parser.add_argument("--seed", type=int, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    line = result_line(differences(args.scores), args.resamples, args.seed)
    (args.out / "result.txt").write_text(line + "\n")


if __name__ == "__main__":
    main()
