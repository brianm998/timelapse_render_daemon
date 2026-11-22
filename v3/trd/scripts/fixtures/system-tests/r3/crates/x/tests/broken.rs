// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

// Fixture for check-system-tests.sh (never compiled). Rule R3: an ordinary test starts the daemon
// without --no-mdns and without --sandbox or --no-adopt.

#[test]
fn starts_the_daemon() {
    let args = ["daemon", "--port", "0"];
    assert_eq!(args[0], "daemon");
}
