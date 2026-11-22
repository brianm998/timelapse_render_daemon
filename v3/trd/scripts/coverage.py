# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
"""Line coverage of the trd crates against their floors (overview §15.3).

Usage: python3 -I scripts/coverage.py <llvm-cov.json> <coverage-floors.toml>

The JSON is `cargo llvm-cov --json` output. Each crate's line coverage is the sum of the covered
and total lines of the files under crates/<crate>/src/, excluding generated code (paths under a
generated/ directory and *.pb.rs). Only trd-core and trd are measured: trd-testtools is a test
helper and trd-proto is generated. The floors file holds one `<crate> = <percent>` line per crate.

Prints a table and exits 1 when a crate is below its floor. A crate with no lines is "n/a" and
passes. Exits 2 on a usage or input error.
"""

import json
import re
import sys
import tomllib

CRATES = ("trd-core", "trd")

CRATE_RE = re.compile(r"(?:^|/)crates/([^/]+)/src/")
GENERATED_RE = re.compile(r"(?:^|/)(?:generated|Generated)/|\.pb\.rs$")


def crate_of(filename):
    """The crate a source file belongs to, or None when it is not under crates/<crate>/src/."""
    path = filename.replace("\\", "/")
    if GENERATED_RE.search(path):
        return None
    match = CRATE_RE.search(path)
    return match.group(1) if match else None


def summarise(report):
    """Sums (lines, covered) per measured crate from a parsed llvm-cov JSON document."""
    totals = {crate: [0, 0] for crate in CRATES}
    for data in report.get("data", []):
        for source in data.get("files", []):
            crate = crate_of(source.get("filename", ""))
            if crate not in totals:
                continue
            lines = source["summary"]["lines"]
            totals[crate][0] += lines["count"]
            totals[crate][1] += lines["covered"]
    return {crate: {"lines": lines, "covered": covered} for crate, (lines, covered) in totals.items()}


def check(summary, floors):
    """One row per crate: (crate, lines, covered, percent or None, floor, passed)."""
    rows = []
    for crate in CRATES:
        lines = summary[crate]["lines"]
        covered = summary[crate]["covered"]
        floor = floors[crate]
        if lines == 0:
            rows.append((crate, lines, covered, None, floor, True))
            continue
        percent = covered * 100.0 / lines
        rows.append((crate, lines, covered, percent, floor, percent >= floor))
    return rows


def render_table(rows):
    header = "crate      lines  covered  percent  floor  status"
    out = [header]
    for crate, lines, covered, percent, floor, passed in rows:
        shown = "n/a" if percent is None else f"{percent:.1f}%"
        status = "ok" if passed else "BELOW FLOOR"
        out.append(f"{crate:<10} {lines:>5}  {covered:>7}  {shown:>7}  {floor:>4}%  {status}")
    return "\n".join(out)


def load_floors(path):
    with open(path, "rb") as handle:
        floors = tomllib.load(handle)
    for crate in CRATES:
        if not isinstance(floors.get(crate), (int, float)):
            raise ValueError(f"{path}: no numeric floor for {crate}")
    return {crate: floors[crate] for crate in CRATES}


def main(argv):
    if len(argv) != 2:
        print("usage: coverage.py <llvm-cov.json> <coverage-floors.toml>", file=sys.stderr)
        return 2
    try:
        with open(argv[0], encoding="utf-8") as handle:
            report = json.load(handle)
        floors = load_floors(argv[1])
        summary = summarise(report)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"coverage.py: {error}", file=sys.stderr)
        return 2
    rows = check(summary, floors)
    print(render_table(rows))
    failed = [row[0] for row in rows if not row[5]]
    if failed:
        print(f"coverage.py: below floor: {', '.join(failed)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
