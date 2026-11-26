// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! trdw: the Windows-subsystem build of trd, which runs `trd daemon` with no console (spec 1 §5).

#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../auth.rs"]
mod auth;
#[path = "../cli.rs"]
mod cli;
#[path = "../daemon.rs"]
mod daemon;
#[path = "../mdns.rs"]
mod mdns;
#[path = "../pairing.rs"]
mod pairing;
#[path = "../server.rs"]
mod server;
#[path = "../service/mod.rs"]
mod service;
#[path = "../tls.rs"]
mod tls;

fn main() -> std::process::ExitCode {
    cli::run(std::env::args_os().collect(), Some("daemon"))
}
