#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1169: the trd checks pass and the CLI tests pass, including the
# Windows-sensitive usage-name test.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/trd/scripts/ci.sh
cd v3/trd
cargo test -p trd --test cli -- --nocapture
