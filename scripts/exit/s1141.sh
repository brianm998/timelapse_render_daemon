#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1141: the .mason/** impact entry is the same in .mason/mason.toml and in
# v3/1-trd-core-daemon.md §11.3, and the workflow, SPDX and script suites pass locally. t0-mason is
# not run here (it runs on the owner's Mac and on nodes with mason installed).
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/suite.sh t0-workflows
bash v3/scripts/suite.sh t0-spdx
bash v3/scripts/suite.sh t0-scripts
toml_line=$(grep -F '".mason/**" =' .mason/mason.toml)
spec_line=$(grep -F '".mason/**" =' v3/1-trd-core-daemon.md)
if [ "$toml_line" != "$spec_line" ]; then
  echo "s1141: the .mason/** impact entry differs between .mason/mason.toml and v3/1-trd-core-daemon.md" >&2
  echo "  toml: $toml_line" >&2
  echo "  spec: $spec_line" >&2
  exit 1
fi
echo "s1141: .mason/** impact entry matches in .mason/mason.toml and v3/1-trd-core-daemon.md"
