#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1296: the SPDX headers are present, the tests DSK-12 names exist, the
# Kotlin suite passes with the contract tests skipped, and the contract tests pass against a trd
# built from this checkout.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
python3 -I v3/scripts/check-test-names.py --ticket DSK-12
bash v3/scripts/suite.sh kotlin-test
bash v3/clients/kotlin/scripts/contract-test.sh
