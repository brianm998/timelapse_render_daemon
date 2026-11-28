#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Exit test for stage s1324 (task 1330): SetConfigPath reaches the running engine.
set -euo pipefail
cd "$(dirname "$0")/../.."
bash v3/scripts/check-spdx.sh
(cd v3/trd && cargo test -p trd-core --test engine --locked)
(cd v3/trd && cargo test -p trd --test server --locked)
