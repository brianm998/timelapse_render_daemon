#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1312: the SPDX headers are present, the system-test rules hold, and the
# daemon's checks and tests pass. The live service test is not run: it installs a real service.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/trd/scripts/check-system-tests.sh
bash v3/scripts/suite.sh trd-check
bash v3/scripts/suite.sh trd-test
