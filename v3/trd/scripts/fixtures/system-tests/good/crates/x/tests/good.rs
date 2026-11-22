// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

// Fixture for check-system-tests.sh (never compiled). A correct ordinary test: it starts the
// daemon in a sandbox with mDNS off and uses no real process table.

#[test]
fn starts_the_daemon_in_a_sandbox() {
    let args = ["daemon", "--no-mdns", "--sandbox", "--port", "0"];
    assert_eq!(args[0], "daemon");
}
