"""Scenario tests of study median-vs-mean. Run from the study directory:

    pixi run python -m unittest discover -s tests
"""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

STUDY = Path(__file__).resolve().parents[1]


def load(script):
    spec = importlib.util.spec_from_file_location(script, STUDY / "scripts" / f"{script}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def generate(out, seed=1, signals=3):
    subprocess.run(
        [sys.executable, STUDY / "scripts/generate.py", "--out", out, "--signals", str(signals),
         "--length", "20", "--noise", "gaussian", "--seed", str(seed)],
        check=True,
    )


class Scenarios(unittest.TestCase):
    def test_s1_endpoint_is_the_mean_rmse_difference(self):  # th:S1
        analyse = load("analyse")
        with tempfile.TemporaryDirectory() as tmp:
            for name, (m, d) in {"a": (0.5, 0.2), "b": (0.4, 0.3)}.items():
                item = Path(tmp, "items", name)
                item.mkdir(parents=True)
                (item / "rmse.tsv").write_text(f"mean_rmse\tmedian_rmse\n{m}\t{d}\n")
            diffs = analyse.differences(tmp)
        self.assertEqual(len(diffs), 2)
        estimate, low, high = analyse.endpoint(diffs, resamples=200, seed=1)
        self.assertAlmostEqual(estimate, 0.2)
        self.assertLessEqual(low, estimate)
        self.assertGreaterEqual(high, estimate)
        line = analyse.result_line(diffs, 200, 1)
        self.assertTrue(line.startswith("n=2 mean RMSE(mean) - RMSE(median) = 0.2000"), line)

    def test_s2_signals_are_reproducible(self):  # th:S2
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            generate(a)
            generate(b)
            files = sorted(p.name for p in Path(a).iterdir())
            self.assertEqual(files, ["sig-001.tsv", "sig-002.tsv", "sig-003.tsv"])
            for name in files:
                self.assertEqual(Path(a, name).read_bytes(), Path(b, name).read_bytes())

    def test_s3_filter_keeps_every_sample(self):  # th:S3
        with tempfile.TemporaryDirectory() as signals, tempfile.TemporaryDirectory() as out:
            generate(signals, signals=1)
            subprocess.run(["sh", STUDY / "scripts/filter.sh", signals, out, "5"], check=True)
            rows = Path(out, "sig-001.tsv").read_text().splitlines()
            self.assertEqual(rows[0], "clean\tmean\tmedian")
            self.assertEqual(len(rows), 21)
            self.assertTrue(all(len(r.split("\t")) == 3 for r in rows))


if __name__ == "__main__":
    unittest.main()
