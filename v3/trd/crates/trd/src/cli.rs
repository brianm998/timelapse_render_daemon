// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The command line of trd and trdw (spec 1 §5). `version` and `daemon` run the daemon; `scan
//! --once` runs offline; every other command except `service` is a client of the running daemon
//! over the loopback listener (`--address`, plaintext). `service` is not implemented yet (TRD-21).

#![allow(dead_code)]

use std::ffi::OsString;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, Offset, TimeZone};
use clap::builder::PossibleValuesParser;
use clap::{Args, CommandFactory, Parser, Subcommand};
use tonic::transport::Channel;
use trd_core::codecs::CodecStore;
use trd_core::config as core_config;
use trd_core::dirs::{Dirs, Env, Os};
use trd_core::paths::{Clock, SystemClock};
use trd_core::plan;
use trd_core::scan::{RealFs, ScanResult, Scanner};
use trd_proto::trd::v1 as pb;
use trd_proto::trd::v1::trd_service_client::TrdServiceClient;

use crate::status_view::{self, Style};

/// Default address of the loopback listener.
const DEFAULT_ADDRESS: &str = "127.0.0.1:50071";

/// The bar width when the daemon sends no config (the default `progress_bar_width`).
const DEFAULT_BAR_WIDTH: usize = 40;

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
    pub config: Option<PathBuf>,
    /// Codec table; default is video_codecs.json beside the config, else the built-in table.
    #[arg(long, value_name = "PATH")]
    pub codecs: Option<PathBuf>,
    /// Database, identity, thumbnails and render logs.
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,
    /// Directory of trd.log.
    #[arg(long, value_name = "PATH")]
    pub log_dir: Option<PathBuf>,
    /// Loopback port; 0 lets the OS choose.
    #[arg(long, value_name = "N", default_value_t = 50071)]
    pub port: u16,
    /// LAN TLS listener as ADDR:PORT, or `off`.
    #[arg(long, value_name = "ADDR:PORT", default_value = "0.0.0.0:50072")]
    pub lan_bind: String,
    /// mDNS instance name and ServerInfo name; default is the host name.
    #[arg(long, value_name = "NAME")]
    pub advertise_name: Option<String>,
    /// Do not advertise over mDNS.
    #[arg(long)]
    pub no_mdns: bool,
    /// Skip adoption entirely: neither its own renders nor foreign ones.
    #[arg(long)]
    pub no_adopt: bool,
    /// Re-adopt only renders recorded in its own database.
    #[arg(long)]
    pub no_adopt_foreign: bool,
    /// Skip the macOS Full Disk Access guard.
    #[arg(long)]
    pub no_fda_check: bool,
    /// Sandbox run: implies --no-adopt-foreign and --no-fda-check; sets ServerInfo.sandbox.
    #[arg(long)]
    pub sandbox: bool,
    /// Log level.
    #[arg(long, value_name = "LEVEL", default_value = "info", value_parser = PossibleValuesParser::new(["debug", "info"]))]
    pub log_level: String,
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
    let address = cli.address;
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
        Some(Command::Daemon(args)) => crate::daemon::run(args),
        Some(Command::Scan { once: true, config }) => finish(scan_once(config)),
        Some(Command::Scan { once: false, .. }) => {
            eprintln!("trd: scan needs --once; the daemon scans by itself");
            ExitCode::from(2)
        }
        Some(Command::Service { .. }) => {
            eprintln!("service: not implemented yet");
            ExitCode::from(1)
        }
        Some(other) => finish(run_client(&address, other)),
    }
}

/// Why a command stopped: the exit code it returns. The reason is printed before this is made.
struct Failed(ExitCode);

impl From<tonic::Status> for Failed {
    fn from(status: tonic::Status) -> Failed {
        fail(status.message())
    }
}

/// A failure with a reason on stderr and exit code 1.
fn fail(message: impl std::fmt::Display) -> Failed {
    eprintln!("trd: {message}");
    Failed(ExitCode::from(1))
}

fn finish(result: Result<(), Failed>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failed(code)) => code,
    }
}

type Client = TrdServiceClient<Channel>;

/// Runs a client command on a current-thread runtime.
fn run_client(address: &str, command: Command) -> Result<(), Failed> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| fail(format!("cannot start the runtime: {err}")))?;
    runtime.block_on(dispatch(address, command))
}

/// Connects to the daemon's loopback listener. Nothing answering is exit 2 (§5).
async fn connect(address: &str) -> Result<Client, Failed> {
    TrdServiceClient::connect(format!("http://{address}"))
        .await
        .map_err(|_| {
            eprintln!("trd is not running on {address}");
            Failed(ExitCode::from(2))
        })
}

async fn dispatch(address: &str, command: Command) -> Result<(), Failed> {
    match command {
        Command::Status { watch, plain } => status(address, watch, plain).await,
        Command::Pause { sequence, variant } => {
            let mut client = connect(address).await?;
            match variant {
                Some(variant_key) => {
                    client
                        .pause_variant(pb::VariantRef {
                            sequence_name: sequence,
                            variant_key,
                        })
                        .await?;
                }
                None => {
                    client
                        .pause_sequence(pb::SequenceRef {
                            sequence_name: sequence,
                        })
                        .await?;
                }
            }
            Ok(())
        }
        Command::Resume { sequence, variant } => {
            let mut client = connect(address).await?;
            match variant {
                Some(variant_key) => {
                    client
                        .resume_variant(pb::VariantRef {
                            sequence_name: sequence,
                            variant_key,
                        })
                        .await?;
                }
                None => {
                    client
                        .resume_sequence(pb::SequenceRef {
                            sequence_name: sequence,
                        })
                        .await?;
                }
            }
            Ok(())
        }
        Command::Render { action } => {
            let mut client = connect(address).await?;
            match action {
                RenderAction::Suspend { render_id } => {
                    client.suspend_render(pb::RenderRef { render_id }).await?;
                }
                RenderAction::Resume { render_id } => {
                    client.resume_render(pb::RenderRef { render_id }).await?;
                }
                RenderAction::Stop { render_id } => {
                    client.stop_render(pb::RenderRef { render_id }).await?;
                }
            }
            Ok(())
        }
        Command::Retry {
            sequence,
            variant_key,
        } => {
            let mut client = connect(address).await?;
            client
                .retry_variant(pb::VariantRef {
                    sequence_name: sequence,
                    variant_key,
                })
                .await?;
            Ok(())
        }
        Command::PauseAll => set_global_pause(address, true).await,
        Command::ResumeAll => set_global_pause(address, false).await,
        Command::Rescan => {
            let mut client = connect(address).await?;
            client.rescan_now(pb::RescanNowRequest {}).await?;
            Ok(())
        }
        Command::Failures {
            dismiss,
            dismiss_all,
        } => failures(address, dismiss, dismiss_all).await,
        Command::Config { action } => config(address, action).await,
        Command::Pair { json } => pair(address, json).await,
        Command::Devices { revoke } => devices(address, revoke).await,
        Command::Log {
            follow,
            level,
            limit,
        } => log(address, follow, level.as_deref(), limit).await,
        other => Err(fail(format!("{} is not a client command", other.name()))),
    }
}

async fn set_global_pause(address: &str, paused: bool) -> Result<(), Failed> {
    let mut client = connect(address).await?;
    client
        .set_global_pause(pb::SetGlobalPauseRequest { paused })
        .await?;
    Ok(())
}

async fn status(address: &str, watch: bool, plain: bool) -> Result<(), Failed> {
    let mut client = connect(address).await?;
    let config = client
        .get_config(pb::GetConfigRequest {})
        .await?
        .into_inner();
    let style = style_of(&config, plain);
    if watch && std::io::stdout().is_terminal() {
        return watch_status(&mut client, style).await;
    }
    let snapshot = client
        .get_snapshot(pb::GetSnapshotRequest {})
        .await?
        .into_inner();
    let now = now_ms();
    print_lines(&status_view::render(
        &snapshot,
        style,
        now,
        utc_offset_at(now),
    ));
    Ok(())
}

/// The view's style: colour only when the config wants it, stdout is a terminal and `--plain` is
/// not given (§5, Perl `log_in_color`).
fn style_of(config: &pb::ConfigState, plain: bool) -> Style {
    let (bar_width, log_in_color) = match config.config.as_ref() {
        Some(config) => (config.progress_bar_width as usize, config.log_in_color),
        None => (DEFAULT_BAR_WIDTH, true),
    };
    Style {
        bar_width,
        color: !plain && log_in_color && std::io::stdout().is_terminal(),
    }
}

/// `status --watch`: redraws on the alternate screen as snapshots arrive, at most twice a second
/// (the server limits the stream), until Ctrl-C or the daemon stops.
async fn watch_status(client: &mut Client, style: Style) -> Result<(), Failed> {
    let mut stream = client
        .watch_snapshots(pb::WatchSnapshotsRequest { max_per_second: 2 })
        .await?
        .into_inner();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen);
    let result = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break Ok(()),
            message = stream.message() => match message {
                Ok(Some(snapshot)) => {
                    let now = now_ms();
                    let lines = status_view::render(&snapshot, style, now, utc_offset_at(now));
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::cursor::MoveTo(0, 0),
                        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
                    );
                    print_lines(&lines);
                }
                Ok(None) => break Ok(()),
                Err(status) => break Err(Failed::from(status)),
            },
        }
    };
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
    result
}

async fn failures(address: &str, dismiss: Option<String>, dismiss_all: bool) -> Result<(), Failed> {
    let mut client = connect(address).await?;
    if dismiss_all {
        client
            .dismiss_all_failures(pb::DismissAllFailuresRequest {})
            .await?;
        return Ok(());
    }
    if let Some(failure_id) = dismiss {
        client
            .dismiss_failure(pb::FailureRef { failure_id })
            .await?;
        return Ok(());
    }
    let snapshot = client
        .get_snapshot(pb::GetSnapshotRequest {})
        .await?
        .into_inner();
    if snapshot.failures.is_empty() {
        println!("no failures");
    }
    for failure in &snapshot.failures {
        let file = if failure.output_filename.is_empty() {
            &failure.sequence_name
        } else {
            &failure.output_filename
        };
        println!(
            "{}  {}  {}  {}",
            failure.id, failure.kind_label, file, failure.summary
        );
    }
    Ok(())
}

async fn config(address: &str, action: ConfigAction) -> Result<(), Failed> {
    match action {
        ConfigAction::Show => {
            let mut client = connect(address).await?;
            let state = client
                .get_config(pb::GetConfigRequest {})
                .await?
                .into_inner();
            if state.raw_json.is_empty() {
                println!("no config file at {}", state.path);
            } else {
                println!("{}", state.raw_json.trim_end());
            }
            Ok(())
        }
        ConfigAction::Path => {
            let mut client = connect(address).await?;
            let state = client
                .get_config(pb::GetConfigRequest {})
                .await?
                .into_inner();
            println!("{}", state.path);
            Ok(())
        }
        ConfigAction::Validate { file } => {
            let path = match file {
                Some(file) => file,
                None => {
                    let mut client = connect(address).await?;
                    let state = client
                        .get_config(pb::GetConfigRequest {})
                        .await?
                        .into_inner();
                    PathBuf::from(state.path)
                }
            };
            validate_file(&path)
        }
        ConfigAction::SetPath { path } => {
            let path = std::path::absolute(&path)
                .map_err(|err| fail(format!("cannot resolve {}: {err}", path.display())))?;
            let mut client = connect(address).await?;
            client
                .set_config_path(pb::SetConfigPathRequest {
                    path: path.to_string_lossy().into_owned(),
                    create_if_missing: false,
                })
                .await?;
            println!("config path set to {}", path.display());
            Ok(())
        }
    }
}

/// `config validate`: the file's issues, then exit 1 when any is an error. The codec table is the
/// one beside the file, else the built-in table; no daemon is used.
fn validate_file(path: &Path) -> Result<(), Failed> {
    let text = fs::read_to_string(path)
        .map_err(|err| fail(format!("cannot read {}: {err}", path.display())))?;
    let codecs = CodecStore::open(None, path.to_path_buf()).codecs().clone();
    let loaded = core_config::load_text(&text, &codecs, SystemClock.year())
        .map_err(|message| fail(format!("{}: {message}", path.display())))?;
    for issue in &loaded.issues {
        let severity = if issue.is_error() { "error" } else { "warning" };
        let field = if issue.field.is_empty() {
            "(whole file)"
        } else {
            issue.field.as_str()
        };
        println!("{severity}: {field}: {}", issue.message);
    }
    if loaded.issues.iter().any(core_config::ConfigIssue::is_error) {
        return Err(Failed(ExitCode::from(1)));
    }
    println!("{} is valid", path.display());
    Ok(())
}

async fn pair(address: &str, json: bool) -> Result<(), Failed> {
    let mut client = connect(address).await?;
    let issued = client
        .start_pairing(pb::StartPairingRequest {})
        .await?
        .into_inner();
    let fingerprint_prefix: String = issued.cert_fingerprint_sha256.chars().take(16).collect();
    if json {
        let object = serde_json::json!({
            "code": issued.code,
            "expires_at_ms": issued.expires_at_ms,
            "server_name": issued.server_name,
            "lan_addresses": issued.lan_addresses,
            "lan_port": issued.lan_port,
            "fingerprint": issued.cert_fingerprint_sha256,
            "fingerprint_prefix": fingerprint_prefix,
        });
        println!("{object}");
        return Ok(());
    }
    let lan_addresses = if issued.lan_addresses.is_empty() {
        "none".to_owned()
    } else {
        issued.lan_addresses.join(", ")
    };
    let lan_port = if issued.lan_port == 0 {
        "off".to_owned()
    } else {
        issued.lan_port.to_string()
    };
    println!("Pairing code: {}", issued.code);
    println!(
        "Expires: {} local time",
        status_view::clock_time(issued.expires_at_ms, utc_offset_at(issued.expires_at_ms))
    );
    println!("Server: {}", issued.server_name);
    println!("LAN addresses: {lan_addresses}");
    println!("LAN port: {lan_port}");
    println!("Fingerprint: {fingerprint_prefix}");
    Ok(())
}

async fn devices(address: &str, revoke: Option<String>) -> Result<(), Failed> {
    let mut client = connect(address).await?;
    if let Some(device_id) = revoke {
        client
            .revoke_device(pb::RevokeDeviceRequest { device_id })
            .await?;
        return Ok(());
    }
    let listed = client
        .list_paired_devices(pb::ListPairedDevicesRequest {})
        .await?
        .into_inner();
    if listed.devices.is_empty() {
        println!("no paired devices");
    }
    for device in &listed.devices {
        let platform = match device.platform() {
            pb::DevicePlatform::Ios => "ios",
            pb::DevicePlatform::Android => "android",
            pb::DevicePlatform::Desktop => "desktop",
            pb::DevicePlatform::Unspecified => "unknown",
        };
        let paired = utc_offset_at(device.paired_at_ms);
        let seen = utc_offset_at(device.last_seen_ms);
        println!(
            "{}  {}  {platform}  paired {}  last seen {}",
            device.device_id,
            device.name,
            status_view::date_time(device.paired_at_ms, paired),
            status_view::date_time(device.last_seen_ms, seen),
        );
    }
    Ok(())
}

async fn log(
    address: &str,
    follow: bool,
    level: Option<&str>,
    limit: Option<u32>,
) -> Result<(), Failed> {
    let mut client = connect(address).await?;
    let level = match level {
        Some("warn") => pb::LogLevel::LogWarn,
        Some("error") => pb::LogLevel::LogError,
        Some("info") => pb::LogLevel::LogInfo,
        _ => pb::LogLevel::Unspecified,
    };
    let min_level = level as i32;
    let listed = client
        .list_log(pb::ListLogRequest {
            before_seq: 0,
            limit: limit.unwrap_or(0),
            min_level,
        })
        .await?
        .into_inner();
    // The server sends the newest first; the view prints the newest last.
    let mut last_seq = 0;
    for entry in listed.entries.iter().rev() {
        println!(
            "{}",
            status_view::log_line(entry, utc_offset_at(entry.at_ms))
        );
        last_seq = last_seq.max(entry.seq);
    }
    if !follow {
        return Ok(());
    }
    let mut stream = client
        .watch_log(pb::WatchLogRequest {
            after_seq: last_seq,
            min_level,
        })
        .await?
        .into_inner();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            message = stream.message() => match message? {
                Some(entry) => {
                    println!("{}", status_view::log_line(&entry, utc_offset_at(entry.at_ms)));
                }
                None => return Ok(()),
            },
        }
    }
}

/// `scan --once`: the scanner and the output naming, in-process, with no daemon and no database.
/// Lists the raw and export sequences, and for each complete export the output files that would
/// render (the ones that already exist are skipped). Writes nothing.
fn scan_once(config_path: Option<PathBuf>) -> Result<(), Failed> {
    let path =
        config_path.unwrap_or_else(|| Dirs::for_os(Os::current(), &Env::system()).config_default);
    let text = fs::read_to_string(&path)
        .map_err(|err| fail(format!("cannot read {}: {err}", path.display())))?;
    let codecs = CodecStore::open(None, path.clone()).codecs().clone();
    let year = SystemClock.year();
    let loaded = core_config::load_text(&text, &codecs, year)
        .map_err(|message| fail(format!("{}: {message}", path.display())))?;
    for issue in &loaded.issues {
        eprintln!("config {}: {}", issue.field, issue.message);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| fail(format!("cannot start the runtime: {err}")))?;
    let scanner = Scanner::new(Arc::new(RealFs), Arc::new(SystemClock));
    let result = runtime.block_on(scanner.scan(&loaded.config));
    let config = &loaded.config;
    print_scan(&result, config, &codecs, year);
    Ok(())
}

fn print_scan(
    result: &ScanResult,
    config: &core_config::Config,
    codecs: &trd_core::codecs::VideoCodecs,
    year: i32,
) {
    println!("raw sequences: {}", result.raw.len());
    for raw in &result.raw {
        let backup = if raw.from_backup { "  (backup)" } else { "" };
        println!(
            "  {}  {} frames  {}{backup}",
            raw.name, raw.frames, raw.parent
        );
    }
    println!("export sequences: {}", result.exports.len());
    let mut skipped = 0usize;
    for export in &result.exports {
        let complete = export.frames == export.raw_frames;
        let state = if complete { "complete" } else { "in progress" };
        println!(
            "  {}  {} of {} frames  {state}  {}",
            export.name, export.frames, export.raw_frames, export.path
        );
        if !complete {
            continue;
        }
        let root = Path::new(&export.path)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default();
        for render in &config.rendering {
            if !codecs.contains_key(&render.codec) {
                println!("    unknown codec {}: not rendered", render.codec);
                continue;
            }
            let filename = plan::output_filename(config, codecs, render, &export.name);
            if plan::already_rendered(config, &root, &filename, &[], year) {
                skipped += 1;
                continue;
            }
            println!("    would render {root}/{filename}");
        }
    }
    if skipped > 0 {
        println!("already rendered, skipped: {skipped}");
    }
    for problem in &result.unreachable {
        eprintln!("unreachable {}: {}", problem.path, problem.reason);
    }
}

/// Writes lines to stdout, ignoring a closed pipe.
fn print_lines(lines: &[String]) {
    let mut out = std::io::stdout().lock();
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();
}

fn print_raw(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// The local UTC offset in seconds at `ms` (0 when the zone cannot be resolved).
fn utc_offset_at(ms: i64) -> i32 {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or(0, |time| time.offset().fix().local_minus_utc())
}
