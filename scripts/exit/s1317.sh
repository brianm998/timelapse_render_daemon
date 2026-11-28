#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1317 (task 1347): SPDX headers, the build-script recovery wrapper's tests,
# and ci.sh using the wrapper for clippy and the test build.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
bash v3/trd/scripts/test-recover-build-script.sh
grep -q 'recover-build-script.sh cargo clippy' v3/trd/scripts/ci.sh
grep -q 'recover-build-script.sh cargo test --workspace --locked --no-run' v3/trd/scripts/ci.sh
