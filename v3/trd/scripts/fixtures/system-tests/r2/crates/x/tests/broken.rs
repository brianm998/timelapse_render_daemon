// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

// Fixture for check-system-tests.sh (never compiled). Rule R2: an ordinary test uses the real
// process table.

#[test]
fn lists_real_processes() {
    let table = SysinfoProcessTable::new();
    assert!(table.is_empty());
}
