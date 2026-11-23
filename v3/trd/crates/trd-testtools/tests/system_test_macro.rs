// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The system-test macro marks its test ignored, so an ordinary run never reaches its guard.

use trd_testtools::system_test;

// The switch is never set, so if the test were not ignored its guard would fail the run.
system_test!(TRD_TESTTOOLS_NEVER_SET, async fn sample_system_test() {});
