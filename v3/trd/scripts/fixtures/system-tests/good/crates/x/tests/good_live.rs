// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

// Fixture for check-system-tests.sh (never compiled). A correct system test: declared only with
// system_test!, so it is ignored unless both switches are set.

system_test!(real_service_install, fn real_service_install() {
    assert!(true);
});
