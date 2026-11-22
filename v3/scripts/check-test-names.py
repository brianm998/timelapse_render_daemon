#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
"""Proves that every test the v3 specs name exists under that name.

Usage: python3 -I v3/scripts/check-test-names.py [--spec N]... [--ticket ID]... [--completed]

It reads v3/<N>-*.md for N in 1-4. A ticket's text runs from its `### <ID>` heading to the next
`---` line, `## <n>.` heading or `### ` ticket heading. Names come from the `check:` clause of
each `- A<n>:` line under the ticket's `## Acceptance criteria`; for a selected spec (`--spec N`,
or every completed spec with `--completed`) also from the "Automated tests" column of that spec's
test-plan table. Three kinds of name are recognised:

  * a Rust test function: a backticked [a-z][a-z0-9_]* token in a list introduced by the word
    `test` or `tests`;
  * a test class: a backticked [A-Z][A-Za-z0-9]*Tests? token, optionally followed by .<member>;
  * a file: each word of a backticked token that ends in .rs .kt .swift .sh or .py and contains
    no `*`, `<` or `$`.

Prints `MISSING <ticket ID or "test plan N">: <name> (<kind>)` for every absent name and exits 1,
or `TEST-NAMES-OK <number of names checked>` and exits 0 (also when nothing is selected).
V3_ROOT is the directory that plays the role of v3/ (default: the real one); the repository root
is its parent.
"""

import argparse
import glob
import os
import re
import sys

COMPLETED_MARKERS = {
    1: "v3/trd/scripts/ci-media.sh",
    2: ".github/workflows/v3-desktop-build.yml",
    3: ".github/workflows/v3-ios-build.yml",
    4: ".github/workflows/v3-android-build.yml",
}

KIND_RUST = "rust test"
KIND_CLASS = "test class"
KIND_FILE = "file"

FILE_EXTENSIONS = (".rs", ".kt", ".swift", ".sh", ".py")
TEST_DIR_NAMES = ("test", "androidTest", "testFixtures")

TICKET_RE = re.compile(r"^### ([A-Z][A-Z0-9]*-[A-Z0-9]+)\b")
TOKEN_RE = r"`[a-z][a-z0-9_]*`"
RUST_LIST_RE = re.compile(
    r"\btests?\b\s*:?\s*(" + TOKEN_RE + r"(?:\s*(?:,\s*and|,\s*or|,|and|or)\s*" + TOKEN_RE + r")*)"
)
BACKTICK_RE = re.compile(r"`([^`\n]+)`")
CLASS_RE = re.compile(r"^([A-Z][A-Za-z0-9]*Tests?)(?:\.[A-Za-z_][A-Za-z0-9_]*)?$")
RUST_TEST_ATTR_RE = re.compile(r"^\s*#\[\s*(?:tokio::test|test)\b")
RUST_FN_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
SYSTEM_TEST_RE = re.compile(
    r"system_test!\(\s*[A-Za-z_][A-Za-z0-9_]*\s*,\s*(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\("
)


def read_lines(path):
    with open(path, encoding="utf-8", errors="replace") as handle:
        return handle.read().split("\n")


def gitignore_dir_patterns(v3_root):
    """Directory patterns of v3/.gitignore as compiled regexes over paths relative to v3/."""
    patterns = []
    path = os.path.join(v3_root, ".gitignore")
    if not os.path.isfile(path):
        return patterns
    for raw in read_lines(path):
        line = raw.strip()
        if not line or line.startswith("#") or not line.endswith("/"):
            continue
        body = line.rstrip("/")
        anchored = "/" in body
        body = body.lstrip("/")
        regex = ""
        i = 0
        while i < len(body):
            if body.startswith("**/", i):
                regex += "(?:.*/)?"
                i += 3
            elif body.startswith("/**", i) and i + 3 == len(body):
                regex += "(?:/.*)?"
                i += 3
            elif body[i] == "*":
                regex += "[^/]*"
                i += 1
            elif body[i] == "?":
                regex += "[^/]"
                i += 1
            else:
                regex += re.escape(body[i])
                i += 1
        if not anchored:
            regex = "(?:.*/)?" + regex
        patterns.append(re.compile("^" + regex + "$"))
    return patterns


class Tree:
    """The files under v3/, outside the build directories v3/.gitignore lists."""

    def __init__(self, v3_root):
        self.v3_root = v3_root
        self.prefix = os.path.basename(os.path.normpath(v3_root))
        self.files = []  # paths relative to the repository root, with '/' separators
        self._contents = {}
        patterns = gitignore_dir_patterns(v3_root)
        for dirpath, dirnames, filenames in os.walk(v3_root):
            rel_dir = os.path.relpath(dirpath, v3_root)
            rel_dir = "" if rel_dir == "." else rel_dir.replace(os.sep, "/")
            kept = []
            for name in dirnames:
                rel = (rel_dir + "/" + name) if rel_dir else name
                if any(p.match(rel) for p in patterns):
                    continue
                kept.append(name)
            dirnames[:] = sorted(kept)
            for name in sorted(filenames):
                rel = (rel_dir + "/" + name) if rel_dir else name
                self.files.append(self.prefix + "/" + rel)
        self.by_basename = {}
        for path in self.files:
            self.by_basename.setdefault(path.rsplit("/", 1)[-1], []).append(path)
        self._rust_tests = None
        self._classes = None

    def abs_path(self, repo_rel):
        return os.path.join(os.path.dirname(os.path.normpath(self.v3_root)), *repo_rel.split("/"))

    def read(self, repo_rel):
        if repo_rel not in self._contents:
            with open(self.abs_path(repo_rel), encoding="utf-8", errors="replace") as handle:
                self._contents[repo_rel] = handle.read()
        return self._contents[repo_rel]

    def has_file(self, word):
        word = word[2:] if word.startswith("./") else word
        for path in self.by_basename.get(word.rsplit("/", 1)[-1], []):
            if path == word or path.endswith("/" + word):
                return True
        return False

    def rust_tests(self):
        if self._rust_tests is None:
            names = set()
            trd_prefix = self.prefix + "/trd/"
            for path in self.files:
                if not (path.endswith(".rs") and path.startswith(trd_prefix)):
                    continue
                lines = self.read(path).split("\n")
                budget = 0
                for line in lines:
                    for match in SYSTEM_TEST_RE.finditer(line):
                        names.add(match.group(1))
                    if RUST_TEST_ATTR_RE.search(line):
                        budget = 3
                        continue
                    if budget > 0:
                        budget -= 1
                        match = RUST_FN_RE.search(line)
                        if match:
                            names.add(match.group(1))
                            budget = 0
            self._rust_tests = names
        return self._rust_tests

    def test_classes(self):
        if self._classes is None:
            names = set()
            decl = re.compile(r"\b(?:class|struct|object)\s+([A-Za-z_][A-Za-z0-9_]*)\b")
            for path in self.files:
                if not path.endswith((".kt", ".swift")):
                    continue
                dirs = path.split("/")[:-1]
                if not any(d in TEST_DIR_NAMES or d.endswith("Tests") for d in dirs):
                    continue
                for match in decl.finditer(self.read(path)):
                    names.add(match.group(1))
            self._classes = names
        return self._classes


def split_tickets(lines):
    """Returns [(ticket id, [lines])] in file order."""
    tickets = []
    current = None
    in_fence = False
    for line in lines:
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
        if not in_fence:
            heading = TICKET_RE.match(line)
            if heading:
                current = (heading.group(1), [])
                tickets.append(current)
                continue
            if current is not None and (
                line.strip() == "---" or re.match(r"^## \d+\.", line) or line.startswith("### ")
            ):
                current = None
                continue
        if current is not None:
            current[1].append(line)
    return tickets


def acceptance_checks(ticket_lines):
    """The `check:` clause text of each `- A<n>:` line under `## Acceptance criteria`."""
    clauses = []
    in_section = False
    item = None

    def flush():
        if item is not None:
            text = " ".join(part.strip() for part in item)
            match = re.search(r"—\s*check:\s*(.*)$", text) or re.search(r"\bcheck:\s*(.*)$", text)
            if match:
                clauses.append(match.group(1))

    for line in ticket_lines:
        if line.startswith("## "):
            flush()
            item = None
            in_section = line.strip() == "## Acceptance criteria"
            continue
        if not in_section:
            continue
        if re.match(r"^- A\d+:", line):
            flush()
            item = [line]
        elif item is not None and line.strip() and not line.startswith("- "):
            item.append(line)
        elif item is not None:
            flush()
            item = None
    flush()
    return clauses


def table_cells(lines, heading_re, column_title):
    """Cell texts of one column of the first table under the heading."""
    cells = []
    in_section = False
    column = None
    for line in lines:
        if line.startswith("## "):
            in_section = bool(heading_re.match(line))
            column = None
            continue
        if not in_section or not line.startswith("|"):
            continue
        parts = [part.strip() for part in line.strip().strip("|").split("|")]
        if column is None:
            for index, title in enumerate(parts):
                if title.lower() == column_title.lower():
                    column = index
            continue
        if set("".join(parts)) <= set("-: "):
            continue
        if column < len(parts):
            cells.append(parts[column])
    return cells


def names_in(text):
    """[(name, kind)] named by one clause or table cell, without duplicates."""
    found = []

    def add(name, kind):
        if (name, kind) not in found:
            found.append((name, kind))

    for match in RUST_LIST_RE.finditer(text):
        for token in re.findall(r"`([a-z][a-z0-9_]*)`", match.group(1)):
            add(token, KIND_RUST)
    for match in BACKTICK_RE.finditer(text):
        token = match.group(1).strip()
        klass = CLASS_RE.match(token)
        if klass:
            add(klass.group(1), KIND_CLASS)
        for word in token.split():
            word = word.strip(",;()\"'")
            if (
                word.endswith(FILE_EXTENSIONS)
                and not any(ch in word for ch in "*<$")
            ):
                add(word, KIND_FILE)
    return found


def exists(tree, name, kind):
    if kind == KIND_RUST:
        return name in tree.rust_tests()
    if kind == KIND_CLASS:
        return name in tree.test_classes()
    return tree.has_file(name)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--spec", action="append", type=int, default=[], metavar="N")
    parser.add_argument("--ticket", action="append", default=[], metavar="ID")
    parser.add_argument("--completed", action="store_true")
    args = parser.parse_args(argv)

    here = os.path.dirname(os.path.abspath(__file__))
    v3_root = os.environ.get("V3_ROOT") or os.path.dirname(here)
    v3_root = os.path.abspath(v3_root)
    repo_root = os.path.dirname(v3_root)
    tree = Tree(v3_root)

    spec_lines = {}
    for number in (1, 2, 3, 4):
        matches = sorted(glob.glob(os.path.join(v3_root, "%d-*.md" % number)))
        if matches:
            spec_lines[number] = read_lines(matches[0])

    specs = list(args.spec)
    if args.completed:
        for number, marker in sorted(COMPLETED_MARKERS.items()):
            done = os.path.exists(os.path.join(repo_root, *marker.split("/")))
            if done and number in spec_lines and number not in specs:
                specs.append(number)
    for number in specs:
        if number not in spec_lines:
            print("check-test-names: no spec file v3/%d-*.md" % number, file=sys.stderr)
            return 2

    # (origin, name, kind) in a stable order, without duplicates.
    wanted = []

    def want(origin, items):
        for name, kind in items:
            entry = (origin, name, kind)
            if entry not in wanted:
                wanted.append(entry)

    def want_ticket(ticket_id, ticket_lines):
        for clause in acceptance_checks(ticket_lines):
            want(ticket_id, names_in(clause))

    for number in sorted(set(specs)):
        for ticket_id, ticket_lines in split_tickets(spec_lines[number]):
            want_ticket(ticket_id, ticket_lines)
        heading = re.compile(r"^## \d+\. Test plan\b")
        for cell in table_cells(spec_lines[number], heading, "Automated tests"):
            want("test plan %d" % number, names_in(cell))

    for ticket_id in args.ticket:
        found = False
        for number in sorted(spec_lines):
            for tid, ticket_lines in split_tickets(spec_lines[number]):
                if tid == ticket_id:
                    want_ticket(tid, ticket_lines)
                    found = True
        if not found:
            print("check-test-names: ticket %s not found in the specs" % ticket_id, file=sys.stderr)
            return 2

    missing = 0
    for origin, name, kind in wanted:
        if not exists(tree, name, kind):
            print("MISSING %s: %s (%s)" % (origin, name, kind))
            missing += 1
    if missing:
        return 1
    print("TEST-NAMES-OK %d" % len(wanted))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
