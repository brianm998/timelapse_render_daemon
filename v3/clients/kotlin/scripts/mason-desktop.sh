#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The `kotlin-test` mason suite: runs the tests and merges the JUnit XML reports into
# build/mason-desktop-results.xml (one <testsuites> root). Exits with Gradle's status.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p build
rm -f build/mason-desktop-results.xml
rc=0
./gradlew --no-daemon --console=plain -Djava.awt.headless=true \
  :proto:build :core:test :desktop:test || rc=$?
python3 -I - build/mason-desktop-results.xml proto core desktop <<'PY' || { [ "$rc" -ne 0 ] || rc=1; }
import glob, sys
import xml.etree.ElementTree as ET

out, modules = sys.argv[1], sys.argv[2:]
root = ET.Element("testsuites")
for m in modules:
    for f in sorted(glob.glob(m + "/build/test-results/test/TEST-*.xml")):
        root.append(ET.parse(f).getroot())
if len(root) == 0:
    sys.exit("no test results found")
ET.ElementTree(root).write(out, encoding="utf-8", xml_declaration=True)
PY
exit "$rc"
