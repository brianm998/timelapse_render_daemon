// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

// Fixture for check-system-tests.sh (never compiled). Rule R1: a *_live.rs file declares a plain
// #[test] instead of system_test!.

#[test]
fn installs_for_real() {
    assert!(true);
}
