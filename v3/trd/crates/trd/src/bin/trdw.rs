// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! trdw: the Windows-subsystem build of trd, which runs `trd daemon` with no console (spec 1 §5).

#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../cli.rs"]
mod cli;

fn main() -> std::process::ExitCode {
    cli::run(std::env::args_os().collect(), Some("daemon"))
}
