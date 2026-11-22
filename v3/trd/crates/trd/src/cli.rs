// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The command line of trd and trdw (spec 1 §5). Only `version` works yet; every other command
//! parses and then prints "not implemented yet". The fields are read by the tickets that
//! implement each command, so they are allowed to be unread until then.

#![allow(dead_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::builder::PossibleValuesParser;
use clap::{Args, CommandFactory, Parser, Subcommand};

/// Default address of the loopback listener.
const DEFAULT_ADDRESS: &str = "127.0.0.1:50071";

#[derive(Parser, Debug)]
#[command(
    name = "trd",
    version,
    about = "The timelapse render daemon and its command line"
)]
pub struct Cli {
    /// Address of the running daemon for client commands (plaintext loopback).
    #[arg(long, global = true, value_name = "ADDR", default_value = DEFAULT_ADDRESS)]
    address: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the daemon (what the LaunchAgent, unit and task run).
    Daemon(DaemonArgs),
    /// Print the Activity view as text.
    Status {
        /// Redraw twice a second on the alternate screen until Ctrl-C.
        #[arg(long)]
        watch: bool,
        /// Print without colour or cursor control.
        #[arg(long)]
        plain: bool,
    },
    /// Pause a sequence, or one variant of it.
    Pause {
        sequence: String,
        #[arg(long, value_name = "KEY")]
        variant: Option<String>,
    },
    /// Resume a sequence, or one variant of it.
    Resume {
        sequence: String,
        #[arg(long, value_name = "KEY")]
        variant: Option<String>,
    },
    /// Per-render controls.
    Render {
        #[command(subcommand)]
        action: RenderAction,
    },
    /// Retry a variant.
    Retry {
        sequence: String,
        variant_key: String,
    },
    /// Pause every sequence.
    PauseAll,
    /// Resume every sequence.
    ResumeAll,
    /// Scan now.
    Rescan,
    /// List failures, or dismiss some.
    Failures {
        #[arg(long, value_name = "ID", conflicts_with = "dismiss_all")]
        dismiss: Option<String>,
        #[arg(long)]
        dismiss_all: bool,
    },
    /// Show, locate, validate or move the config file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Issue a pairing code on loopback.
    Pair {
        /// Print one JSON object for scripts.
        #[arg(long)]
        json: bool,
    },
    /// List paired devices, or revoke one.
    Devices {
        #[arg(long, value_name = "ID")]
        revoke: Option<String>,
    },
    /// Print the log, newest last.
    Log {
        /// Keep printing new lines.
        #[arg(short = 'f', long)]
        follow: bool,
        #[arg(long, value_name = "LEVEL", value_parser = PossibleValuesParser::new(["info", "warn", "error"]))]
        level: Option<String>,
        #[arg(long, value_name = "N")]
        limit: Option<u32>,
    },
    /// Offline dry run of the scan, without a daemon.
    Scan {
        /// Scan once and print the result.
        #[arg(long)]
        once: bool,
        #[arg(long, value_name = "PATH")]
        config: Option<PathBuf>,
    },
    /// Manage the per-user autostart.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Print the version and platform.
    Version,
}

#[derive(Args, Debug)]
pub struct DaemonArgs {
    /// Config file; overrides the stored setting for this run.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Codec table; default is video_codecs.json beside the config, else the built-in table.
    #[arg(long, value_name = "PATH")]
    codecs: Option<PathBuf>,
    /// Database, identity, thumbnails and render logs.
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,
    /// Directory of trd.log.
    #[arg(long, value_name = "PATH")]
    log_dir: Option<PathBuf>,
    /// Loopback port; 0 lets the OS choose.
    #[arg(long, value_name = "N", default_value_t = 50071)]
    port: u16,
    /// LAN TLS listener as ADDR:PORT, or `off`.
    #[arg(long, value_name = "ADDR:PORT", default_value = "0.0.0.0:50072")]
    lan_bind: String,
    /// mDNS instance name and ServerInfo name; default is the host name.
    #[arg(long, value_name = "NAME")]
    advertise_name: Option<String>,
    /// Do not advertise over mDNS.
    #[arg(long)]
    no_mdns: bool,
    /// Skip adoption entirely: neither its own renders nor foreign ones.
    #[arg(long)]
    no_adopt: bool,
    /// Re-adopt only renders recorded in its own database.
    #[arg(long)]
    no_adopt_foreign: bool,
    /// Skip the macOS Full Disk Access guard.
    #[arg(long)]
    no_fda_check: bool,
    /// Sandbox run: implies --no-adopt-foreign and --no-fda-check; sets ServerInfo.sandbox.
    #[arg(long)]
    sandbox: bool,
    /// Log level.
    #[arg(long, value_name = "LEVEL", default_value = "info", value_parser = PossibleValuesParser::new(["debug", "info"]))]
    log_level: String,
}

#[derive(Subcommand, Debug)]
enum RenderAction {
    /// Suspend a running render.
    Suspend { render_id: String },
    /// Resume a suspended render.
    Resume { render_id: String },
    /// Stop a render.
    Stop { render_id: String },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Show the effective config.
    Show,
    /// Print the config path.
    Path,
    /// Validate a config file offline; the current one when none is named.
    Validate { file: Option<PathBuf> },
    /// Change the config path the daemon uses.
    SetPath { path: PathBuf },
}

#[derive(Subcommand, Debug)]
enum ServiceAction {
    /// Install the autostart.
    Install(ServiceArgs),
    /// Remove the autostart.
    Uninstall(ServiceArgs),
    /// Show the autostart state.
    Status(ServiceArgs),
    /// Restart the autostart service.
    Restart(ServiceArgs),
}

#[derive(Args, Debug)]
pub struct ServiceArgs {
    /// Print what would be done, change nothing.
    #[arg(long)]
    dry_run: bool,
    /// Config file the service starts with.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// trd binary the service starts.
    #[arg(long, value_name = "PATH")]
    binary: Option<PathBuf>,
}

impl Command {
    /// The name printed by "not implemented yet".
    fn name(&self) -> &'static str {
        match self {
            Command::Daemon(_) => "daemon",
            Command::Status { .. } => "status",
            Command::Pause { .. } => "pause",
            Command::Resume { .. } => "resume",
            Command::Render { .. } => "render",
            Command::Retry { .. } => "retry",
            Command::PauseAll => "pause-all",
            Command::ResumeAll => "resume-all",
            Command::Rescan => "rescan",
            Command::Failures { .. } => "failures",
            Command::Config { .. } => "config",
            Command::Pair { .. } => "pair",
            Command::Devices { .. } => "devices",
            Command::Log { .. } => "log",
            Command::Scan { .. } => "scan",
            Command::Service { .. } => "service",
            Command::Version => "version",
        }
    }
}

/// `<os>-<arch>` names used by the packages (spec 1 §9.4 and the release table).
pub fn platform() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => "macos-universal".to_string(),
        ("linux", "x86_64") => "linux-x64".to_string(),
        ("linux", "aarch64") => "linux-arm64".to_string(),
        ("windows", "x86_64") => "windows-x64".to_string(),
        (os, arch) => format!("{os}-{arch}"),
    }
}

/// Inserts `default` as the subcommand when the arguments name none (trdw runs `daemon`).
fn with_default_subcommand(mut args: Vec<OsString>, default: Option<&str>) -> Vec<OsString> {
    let Some(default) = default else {
        return args;
    };
    if args.is_empty() {
        args.push(OsString::from("trdw"));
    }
    let names_subcommand = args
        .get(1)
        .and_then(|arg| arg.to_str())
        .is_some_and(|first| {
            matches!(first, "-h" | "--help" | "-V" | "--version")
                || Cli::command()
                    .get_subcommands()
                    .any(|sub| sub.get_name() == first)
        });
    if !names_subcommand {
        args.insert(1, OsString::from(default));
    }
    args
}

/// Runs trd or trdw with the given arguments (the first is the program name). `default_subcommand`
/// is used when the arguments name no subcommand.
pub fn run(args: Vec<OsString>, default_subcommand: Option<&str>) -> ExitCode {
    let args = with_default_subcommand(args, default_subcommand);
    let cli = Cli::parse_from(args);
    match cli.command {
        None => {
            // No arguments: help, and success.
            let _ = Cli::command().print_help();
            println!();
            ExitCode::SUCCESS
        }
        Some(Command::Version) => {
            println!("trd {} ({})", env!("CARGO_PKG_VERSION"), platform());
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("{}: not implemented yet", other.name());
            ExitCode::from(1)
        }
    }
}
