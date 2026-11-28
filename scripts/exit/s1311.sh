#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1311 (DSK-13): SPDX headers, actionlint, every test spec 2 names, the script
# tests, the Kotlin build with its coverage floors, and the kotlin-test suite. The package builds,
# notarization and the system tests (A1-A3, A5) need GitHub and are not run here.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/scripts/check-workflows.sh
python3 -I v3/scripts/check-test-names.py --spec 2
bash v3/scripts/test-scripts.sh
bash v3/clients/kotlin/scripts/ci-desktop.sh
bash v3/scripts/suite.sh kotlin-test
