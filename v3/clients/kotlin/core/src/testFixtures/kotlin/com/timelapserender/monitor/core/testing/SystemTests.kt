// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.testing

/** Gate for system tests that touch the real network (overview §15.2). */
object SystemTests {
    /** True only when `CI` is `true` and the test's own [switch] variable is `1`. */
    fun allowed(env: (String) -> String?, switch: String): Boolean =
        env("CI") == "true" && env(switch) == "1"
}
