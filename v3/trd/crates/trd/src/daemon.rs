// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! `trd daemon` (spec 1 §5, §6.13): the start-up that wires the engine to the gRPC server, and the
//! serving loop that ends on SIGTERM or SIGINT (Ctrl-C and console close on Windows).
//!
//! Stopping never signals a render: the engine drops its handles and the server ends its streams,
//! so a render that is running keeps running (§6.11, overview §10 rule 2).

use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;
use tracing::Level;

use trd_core::adopt::AdoptPolicy;
use trd_core::dirs::{Dirs, Env, Os};
use trd_core::engine::{Engine, EngineOptions, MacDiskAccess};
use trd_core::logging;
use trd_core::paths::{SystemClock, resolve_config_path};
use trd_core::process::SysinfoProcessTable;
use trd_core::scan::RealFs;
use trd_core::store::Store;
use trd_core::thumbs::{Runner, Thumbnailer, run_ffmpeg};
use trd_core::tools::Tools;
use trd_proto::trd::v1::trd_service_server::TrdServiceServer;

use crate::cli::DaemonArgs;
use crate::server::{ConfigSlot, PathSource, Server, Shared, SharedTools, mark_loopback, now_ms};

/// How long the loopback listener waits before it binds again after a failure.
const BIND_RETRY: Duration = Duration::from_secs(30);
/// How long the engine gets to end after the server stops.
const ENGINE_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// What the daemon needs from the command line and the platform, apart from the listener.
pub struct Settings {
    pub dirs: Dirs,
    /// `--config`, which overrides the stored setting for this run.
    pub config: Option<PathBuf>,
    /// `--codecs`.
    pub codecs: Option<PathBuf>,
    pub advertise_name: Option<String>,
    pub policy: AdoptPolicy,
    /// Whether the macOS Full Disk Access guard runs.
    pub fda_check: bool,
    pub sandbox: bool,
    pub home: PathBuf,
    pub os: Os,
}

/// Runs `trd daemon` until it is told to stop. Returns the process exit code.
pub fn run(args: DaemonArgs) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("trd: cannot start the runtime: {err}");
            return ExitCode::from(1);
        }
    };
    let code = runtime.block_on(serve(args));
    runtime.shutdown_timeout(Duration::from_secs(5));
    code
}

async fn serve(args: DaemonArgs) -> ExitCode {
    let dirs = Dirs::for_os(Os::current(), &Env::system())
        .with_overrides(args.data_dir.clone(), args.log_dir.clone());
    let level = if args.log_level == "debug" {
        Level::DEBUG
    } else {
        Level::INFO
    };
    // Held until the process ends: it flushes the log file when dropped.
    let _log = match logging::init(&dirs, level, false) {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!(
                "trd: cannot start logging in {}: {err}",
                dirs.logs.display()
            );
            return ExitCode::from(1);
        }
    };

    let (stop_tx, stop_rx) = watch::channel(false);
    if args.lan_bind != "off" {
        tracing::warn!(
            lan_bind = %args.lan_bind,
            "the LAN listener is not built yet (TRD-18); --lan-bind is ignored"
        );
    }
    let settings = Settings {
        dirs: dirs.clone(),
        config: args.config.clone(),
        codecs: args.codecs.clone(),
        advertise_name: args.advertise_name.clone(),
        policy: AdoptPolicy::from_flags(args.no_adopt, args.no_adopt_foreign, args.sandbox),
        fda_check: !(args.no_fda_check || args.sandbox),
        sandbox: args.sandbox,
        home: Env::system().home,
        os: Os::current(),
    };
    let (shared, engine) = match assemble(settings, stop_rx.clone()) {
        Ok(parts) => parts,
        Err(message) => {
            tracing::error!(error = %message, "trd cannot start");
            eprintln!("trd: {message}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "trd daemon starting");
    let engine_task = tokio::spawn(engine.run());
    tokio::spawn(async move {
        wait_for_stop_signal().await;
        tracing::info!("stop requested; ending the streams and the server");
        let _ = stop_tx.send(true);
    });

    let Some(listener) = bind_retrying(args.port, stop_rx.clone()).await else {
        stop_engine(&shared, engine_task).await;
        return ExitCode::SUCCESS;
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(err) => {
            tracing::error!(error = %err, "the listener has no address");
            stop_engine(&shared, engine_task).await;
            return ExitCode::from(1);
        }
    };
    // The one line the clients and the tests wait for. It is flushed before anything else runs.
    println!("TRD_LISTENING loopback=127.0.0.1:{port} lan=off");
    let _ = std::io::stdout().flush();
    tracing::info!(port, "the loopback listener is serving");

    let service = TrdServiceServer::with_interceptor(Server::new(shared.clone()), mark_loopback);
    let mut stop = stop_rx;
    let served = tonic::transport::Server::builder()
        .add_service(service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
            let _ = stop.wait_for(|stopped| *stopped).await;
        })
        .await;
    let code = match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "the gRPC server stopped with an error");
            ExitCode::from(1)
        }
    };
    stop_engine(&shared, engine_task).await;
    code
}

/// Builds the engine and the shared state of the server from `settings`. Opens the database,
/// resolves the config path and the codec table, and resolves the tools. Nothing is scanned until
/// the engine runs.
pub fn assemble(
    settings: Settings,
    shutdown: watch::Receiver<bool>,
) -> Result<(Arc<Shared>, Engine), String> {
    let database = settings.dirs.database();
    let store = Store::open(&database)
        .map_err(|err| format!("cannot open the database {}: {err}", database.display()))?;
    let stored = store
        .config_path()
        .map_err(|err| format!("cannot read the config path setting: {err}"))?;
    let source = if settings.config.is_some() {
        PathSource::Flag
    } else if stored.is_some() {
        PathSource::Setting
    } else {
        PathSource::Default
    };
    let config_path = resolve_config_path(
        settings.config.as_deref(),
        stored.as_deref(),
        &settings.dirs.config_default,
    );
    let slot = ConfigSlot::open(config_path.clone(), source, settings.codecs.clone());
    let codec_table = slot.codec_table();
    let history_path = history_path_from_config(&config_path);

    let shared_tools: SharedTools = Arc::new(Mutex::new(resolve_tools()));
    let resolve: Box<dyn FnMut() -> Tools + Send> = {
        let tools = shared_tools.clone();
        Box::new(move || {
            let resolved = resolve_tools();
            *tools.lock().unwrap_or_else(PoisonError::into_inner) = resolved.clone();
            resolved
        })
    };

    let (engine, handle) = Engine::new(EngineOptions {
        store: store.clone(),
        config_path,
        data_dir: settings.dirs.data.clone(),
        codecs: codec_table,
        resolve_tools: resolve,
        disk_access: Box::new(MacDiskAccess::new(
            settings.fda_check,
            settings.home.clone(),
        )),
        scan_fs: Arc::new(RealFs),
        clock: Arc::new(SystemClock),
        now_ms: Arc::new(now_ms),
        table: Box::new(SysinfoProcessTable::new()),
        policy: settings.policy,
        thumbs: Thumbnailer::new(&settings.dirs.data, thumbnail_runner(shared_tools.clone())),
        env: Vec::new(),
        history_path,
    });

    let server_id = store
        .server_id()
        .map_err(|err| format!("cannot read the server identity: {err}"))?;
    let server_name = settings
        .advertise_name
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(sysinfo::System::host_name)
        .unwrap_or_else(|| "trd".to_owned());
    let shared = Arc::new(Shared {
        handle,
        store,
        thumbs: Thumbnailer::new(&settings.dirs.data, thumbnail_runner(shared_tools.clone())),
        tools: shared_tools,
        config: Mutex::new(slot),
        server_id,
        server_name,
        sandbox: settings.sandbox,
        shutdown,
        home: settings.home,
        os: settings.os,
    });
    Ok((shared, engine))
}

/// The tools resolved from the environment and the running executable (§6.15).
fn resolve_tools() -> Tools {
    let exe = env::current_exe().unwrap_or_default();
    Tools::resolve(|name: &str| env::var_os(name), &exe)
}

/// The thumbnailer's ffmpeg: the one the daemon resolved last, read at each call.
fn thumbnail_runner(tools: SharedTools) -> Runner {
    Box::new(move |args: &[String]| {
        let command = tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ffmpeg
            .command
            .clone();
        match command {
            Some(command) => run_ffmpeg(&command, args),
            None => Err("ffmpeg is not available".to_owned()),
        }
    })
}

/// The Swift history file named by the config's `render_history_path` (§6.17). The Swift default
/// location is not read here: a test must never read the owner's Application Support folder.
fn history_path_from_config(config_path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("render_history_path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Binds the loopback listener on `port`. A failed bind is logged and tried again after
/// [`BIND_RETRY`]. Returns `None` when a stop is requested before a bind succeeds.
async fn bind_retrying(port: u16, mut stop: watch::Receiver<bool>) -> Option<TcpListener> {
    loop {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Some(listener),
            Err(err) => tracing::warn!(
                port,
                error = %err,
                "the loopback listener cannot bind; trying again in 30 seconds"
            ),
        }
        tokio::select! {
            () = tokio::time::sleep(BIND_RETRY) => {}
            _ = stop.wait_for(|stopped| *stopped) => return None,
        }
    }
}

/// Stops the engine: it drops its handles and signals no process. Waits a little for it to end.
async fn stop_engine(shared: &Shared, engine: JoinHandle<()>) {
    shared.handle.shutdown().await;
    if tokio::time::timeout(ENGINE_STOP_TIMEOUT, engine)
        .await
        .is_err()
    {
        tracing::warn!("the engine did not stop in time; exiting anyway");
    }
    tracing::info!("trd daemon stopped");
}

/// Resolves on SIGTERM or SIGINT.
#[cfg(unix)]
async fn wait_for_stop_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("a SIGTERM handler");
    let mut interrupt = signal(SignalKind::interrupt()).expect("a SIGINT handler");
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
}

/// Resolves on Ctrl-C or the console closing.
#[cfg(windows)]
async fn wait_for_stop_signal() {
    let mut close = tokio::signal::windows::ctrl_close().expect("a console close handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = close.recv() => {}
    }
}
