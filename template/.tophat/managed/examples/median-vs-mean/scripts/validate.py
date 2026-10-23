"""Mechanical checks of the outputs of stages `generate` and `score` (their `validate` commands).

    validate.py signals <dir> --signals N --length L   N signal files of L rows, finite values
    validate.py rmse <dir>                              one rmse.tsv with two finite, non-negative values

Exits non-zero, saying what is wrong, when a check fails; the stage is then not sealed.
"""

import argparse
import math
import sys
from pathlib import Path


def numbers(line, path):
    try:
        values = [float(x) for x in line.split("\t")]
    except ValueError:
        sys.exit(f"{path}: non-numeric value in {line!r}")
    if not all(math.isfinite(v) for v in values):
        sys.exit(f"{path}: non-finite value in {line!r}")
    return values


def signals(directory, count, length):
    files = sorted(Path(directory).glob("*.tsv"))
    if len(files) != count:
        sys.exit(f"{directory}: expected {count} signal files, found {len(files)}")
    for path in files:
        lines = path.read_text().splitlines()
        if lines[0] != "clean\tnoisy" or len(lines) != length + 1:
            sys.exit(f"{path}: expected a header and {length} rows, found {len(lines) - 1}")
        for line in lines[1:]:
            if len(numbers(line, path)) != 2:
                sys.exit(f"{path}: expected 2 columns in {line!r}")


def rmse(directory):
    path = Path(directory, "rmse.tsv")
    if not path.is_file():
        sys.exit(f"{path} is missing")
    lines = path.read_text().splitlines()
    if len(lines) != 2 or lines[0] != "mean_rmse\tmedian_rmse":
        sys.exit(f"{path}: expected a header and one row")
    values = numbers(lines[1], path)
    if len(values) != 2 or any(v < 0 for v in values):
        sys.exit(f"{path}: expected two non-negative values, found {lines[1]!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("what", choices=["signals", "rmse"])
    parser.add_argument("directory", type=Path)
    parser.add_argument("--signals", type=int)
    parser.add_argument("--length", type=int)
    args = parser.parse_args()
    if args.what == "signals":
        if args.signals is None or args.length is None:
            parser.error("signals needs --signals and --length")
        signals(args.directory, args.signals, args.length)
    else:
        rmse(args.directory)


if __name__ == "__main__":
    main()
