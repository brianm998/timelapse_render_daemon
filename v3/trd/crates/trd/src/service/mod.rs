// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Per-user autostart: one module per platform (§10).

pub mod linux;
pub mod macos;
pub mod windows;
