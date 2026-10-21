"""Writes seeded synthetic step signals, one tab-separated file per signal (stage `generate`).

Each file has a header line and one row per sample with the columns `clean` and `noisy`.
"""

import argparse
import random
from pathlib import Path


def signal(rng, length, noise):
    """A clean step signal (0 then 1) and its noisy version."""
    clean = [0.0 if i < length // 2 else 1.0 for i in range(length)]
    if noise == "gaussian":
        noisy = [c + rng.gauss(0.0, 0.3) for c in clean]
    elif noise == "impulse":
        noisy = [rng.choice((-3.0, 3.0)) if rng.random() < 0.1 else c for c in clean]
    else:
        raise SystemExit(f"unknown noise {noise!r}: use gaussian or impulse")
    return clean, noisy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--signals", type=int, required=True)
    parser.add_argument("--length", type=int, required=True)
    parser.add_argument("--noise", required=True)
    parser.add_argument("--seed", type=int, required=True)
    args = parser.parse_args()
    if args.signals < 1 or args.length < 2:
        raise SystemExit("--signals must be at least 1 and --length at least 2")
    rng = random.Random(args.seed)
    args.out.mkdir(parents=True, exist_ok=True)
    for n in range(1, args.signals + 1):
        clean, noisy = signal(rng, args.length, args.noise)
        rows = ["clean\tnoisy"] + [f"{c:.6f}\t{x:.6f}" for c, x in zip(clean, noisy)]
        (args.out / f"sig-{n:03d}.tsv").write_text("\n".join(rows) + "\n")


if __name__ == "__main__":
    main()
