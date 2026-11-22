# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
"""Tests of scripts/coverage.py against the recorded llvm-cov fixture (overview §15.3).

Run with: python3 -I scripts/test_coverage.py (from v3/trd). `python3 -I -m unittest <path>` cannot
import this module by its dotted name, because -I keeps the working directory off sys.path. The
checker is loaded by path for the same reason. Temporary files go to the system temporary
directory only.
"""

import contextlib
import importlib.util
import io
import json
import os
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "fixtures", "llvm-cov.json")

_spec = importlib.util.spec_from_file_location("coverage", os.path.join(HERE, "coverage.py"))
coverage = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(coverage)


def file_entry(name, count, covered):
    return {
        "filename": name,
        "summary": {"lines": {"count": count, "covered": covered, "percent": 0.0}},
    }


class CoverageTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.mkdtemp(prefix="trd-coverage-test.")
        self.addCleanup(shutil.rmtree, self.work, True)

    def write(self, name, text):
        path = os.path.join(self.work, name)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(text)
        return path

    def run_main(self, report_path, floors_text):
        floors_path = self.write("coverage-floors.toml", floors_text)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = coverage.main([report_path, floors_path])
        return code, out.getvalue(), err.getvalue()

    def test_summary_sums_lines_per_crate_from_the_fixture(self):
        with open(FIXTURE, encoding="utf-8") as handle:
            summary = coverage.summarise(json.load(handle))
        self.assertEqual(summary["trd-core"], {"lines": 30, "covered": 18})
        self.assertEqual(summary["trd"], {"lines": 4, "covered": 4})

    def test_generated_code_and_test_helpers_are_excluded(self):
        self.assertIsNone(coverage.crate_of("/c/v3/trd/crates/trd/src/generated/proto.rs"))
        self.assertIsNone(coverage.crate_of("/c/v3/trd/crates/trd-proto/src/x.pb.rs"))
        self.assertIsNone(coverage.crate_of("/c/v3/trd/crates/trd/tests/cli.rs"))
        # trd-testtools is named, but summarise() measures only trd-core and trd.
        self.assertEqual(coverage.crate_of("/c/v3/trd/crates/trd-testtools/src/lib.rs"), "trd-testtools")
        self.assertEqual(coverage.crate_of("/c/v3/trd/crates/trd-core/src/a.rs"), "trd-core")
        self.assertEqual(coverage.crate_of("C:\\c\\v3\\trd\\crates\\trd\\src\\main.rs"), "trd")

    def test_fixture_passes_the_zero_floors(self):
        code, out, _ = self.run_main(FIXTURE, "trd-core = 0\ntrd = 0\n")
        self.assertEqual(code, 0)
        self.assertIn("trd-core", out)
        self.assertIn("60.0%", out)
        self.assertIn("100.0%", out)

    def test_floor_above_coverage_exits_one_and_names_the_crate(self):
        code, out, err = self.run_main(FIXTURE, "trd-core = 61\ntrd = 0\n")
        self.assertEqual(code, 1)
        self.assertIn("BELOW FLOOR", out)
        self.assertIn("trd-core", err)

    def test_floor_equal_to_coverage_passes(self):
        code, _, _ = self.run_main(FIXTURE, "trd-core = 60\ntrd = 100\n")
        self.assertEqual(code, 0)

    def test_crate_without_lines_is_not_applicable_and_passes(self):
        report = {
            "data": [
                {
                    "files": [
                        file_entry("/c/v3/trd/crates/trd-core/src/a.rs", 10, 5),
                    ]
                }
            ]
        }
        path = self.write("report.json", json.dumps(report))
        code, out, _ = self.run_main(path, "trd-core = 50\ntrd = 90\n")
        self.assertEqual(code, 0)
        self.assertIn("n/a", out)

    def test_missing_floor_is_a_usage_error(self):
        code, _, err = self.run_main(FIXTURE, "trd-core = 0\n")
        self.assertEqual(code, 2)
        self.assertIn("trd", err)

    def test_wrong_argument_count_is_a_usage_error(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = coverage.main([FIXTURE])
        self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main()
