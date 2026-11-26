// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The trd command line (spec 1 §5); every module of the binary is declared here.

mod auth;
mod cli;
mod daemon;
mod mdns;
mod pairing;
mod server;
mod service;
mod status_view;
mod tls;

fn main() -> std::process::ExitCode {
    cli::run(std::env::args_os().collect(), None)
}
