// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! `trd daemon` (spec 1 §5, §6.13): the start-up that wires the engine to the gRPC server, and the
//! serving loop that ends on SIGTERM or SIGINT (Ctrl-C and console close on Windows).
//!
//! Stopping never signals a render: the engine drops its handles and the server ends its streams,
//! so a render that is running keeps running (§6.11, overview §10 rule 2).

use std::env;
use std::future::Future;
use std::io::{self, Write};
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
use trd_core::process::{ProcessTable, SysinfoProcessTable};
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
    let settings = settings_from(&args, dirs.clone(), Env::system().home, Os::current());
    let table = Box::new(SysinfoProcessTable::new());
    let (shared, engine) = match assemble(settings, table, stop_rx.clone()) {
        Ok(parts) => parts,
        Err(message) => {
            tracing::error!(error = %message, "trd cannot start");
            eprintln!("trd: {message}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "trd daemon starting");
    // The handlers are installed here, before anything can tell a caller that the daemon is up: a
    // signal sent right after TRD_LISTENING must stop the daemon cleanly, not kill it.
    let signalled = match stop_signal() {
        Ok(signalled) => signalled,
        Err(err) => {
            tracing::error!(error = %err, "the stop signals cannot be handled");
            eprintln!("trd: cannot handle the stop signals: {err}");
            return ExitCode::from(1);
        }
    };
    let engine_task = tokio::spawn(engine.run());
    tokio::spawn(async move {
        signalled.await;
        tracing::info!("stop requested; ending the streams and the server");
        let _ = stop_tx.send(true);
    });

    let Some(listener) = bind_retrying(args.port, BIND_RETRY, stop_rx.clone()).await else {
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

/// The settings the `trd daemon` flags give (§5). `--sandbox` implies `--no-adopt-foreign` and
/// `--no-fda-check`.
fn settings_from(args: &DaemonArgs, dirs: Dirs, home: PathBuf, os: Os) -> Settings {
    Settings {
        dirs,
        config: args.config.clone(),
        codecs: args.codecs.clone(),
        advertise_name: args.advertise_name.clone(),
        policy: AdoptPolicy::from_flags(args.no_adopt, args.no_adopt_foreign, args.sandbox),
        fda_check: !(args.no_fda_check || args.sandbox),
        sandbox: args.sandbox,
        home,
        os,
    }
}

/// `ServerInfo.name`: `--advertise-name` when it is given and not empty, else the host name.
fn server_name(advertise: Option<String>) -> String {
    advertise
        .filter(|name| !name.is_empty())
        .or_else(sysinfo::System::host_name)
        .unwrap_or_else(|| "trd".to_owned())
}

/// Builds the engine and the shared state of the server from `settings`. Opens the database,
/// resolves the config path and the codec table, and resolves the tools. Nothing is scanned until
/// the engine runs. The caller supplies the process table the engine reads: the daemon passes the
/// operating system's, in-process tests a fake one.
pub fn assemble(
    settings: Settings,
    table: Box<dyn ProcessTable + Send>,
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

    // One thumbnailer: the engine and the server share its slot gate (MAX_CONCURRENT_THUMBNAILS).
    let thumbs = Thumbnailer::new(&settings.dirs.data, thumbnail_runner(shared_tools.clone()));
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
        table,
        policy: settings.policy,
        thumbs: thumbs.share(),
        env: Vec::new(),
        history_path,
    });

    let server_id = store
        .server_id()
        .map_err(|err| format!("cannot read the server identity: {err}"))?;
    let server_name = server_name(settings.advertise_name);
    let shared = Arc::new(Shared {
        handle,
        store,
        thumbs,
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

/// Binds the loopback listener on `port`. A failed bind is logged and tried again after `delay`
/// ([`BIND_RETRY`] in the daemon). Returns `None` when a stop is requested before a bind succeeds.
async fn bind_retrying(
    port: u16,
    delay: Duration,
    mut stop: watch::Receiver<bool>,
) -> Option<TcpListener> {
    loop {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Some(listener),
            Err(err) => tracing::warn!(
                port,
                error = %err,
                retry_seconds = delay.as_secs(),
                "the loopback listener cannot bind; trying again later"
            ),
        }
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
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

/// Installs the SIGTERM and SIGINT handlers now and returns a future that resolves on the first of
/// them. A signal that arrives before the future is first polled is kept, not lost.
#[cfg(unix)]
fn stop_signal() -> io::Result<impl Future<Output = ()> + Send> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

/// Installs the Ctrl-C and console-close handlers now and returns a future that resolves on the
/// first of them. An event that arrives before the future is first polled is kept, not lost.
#[cfg(windows)]
fn stop_signal() -> io::Result<impl Future<Output = ()> + Send> {
    use tokio::signal::windows::{ctrl_c, ctrl_close};
    let mut interrupt = ctrl_c()?;
    let mut close = ctrl_close()?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = close.recv() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use std::future::poll_fn;
    use std::pin::pin;
    use std::task::Poll;

    use clap::Parser;

    use super::*;

    /// Stops a test that waits for a state that never comes. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);

    #[derive(Parser)]
    struct Flags {
        #[command(flatten)]
        daemon: DaemonArgs,
    }

    fn settings(flags: &[&str]) -> Settings {
        let args = Flags::try_parse_from(["daemon"].iter().chain(flags))
            .expect("the flags parse")
            .daemon;
        let dirs = Dirs {
            config_default: PathBuf::from("config.json"),
            data: PathBuf::from("data"),
            logs: PathBuf::from("logs"),
        };
        settings_from(&args, dirs, PathBuf::from("home"), Os::current())
    }

    /// `--sandbox` implies `--no-adopt-foreign` and `--no-fda-check`.
    #[test]
    fn sandbox_implies_own_renders_only_and_no_disk_access_check() {
        let plain = settings(&[]);
        assert!(plain.policy == AdoptPolicy::All && plain.fda_check && !plain.sandbox);
        let sandbox = settings(&["--sandbox"]);
        assert!(sandbox.policy == AdoptPolicy::OwnOnly);
        assert!(!sandbox.fda_check && sandbox.sandbox);
        assert!(settings(&["--no-adopt"]).policy == AdoptPolicy::None);
        assert!(settings(&["--no-adopt-foreign"]).policy == AdoptPolicy::OwnOnly);
        assert!(!settings(&["--no-fda-check"]).fda_check);
    }

    #[test]
    fn server_name_is_the_advertise_name_or_the_host_name() {
        assert_eq!(server_name(Some("Studio".to_owned())), "Studio");
        let host = server_name(None);
        assert!(!host.is_empty());
        assert_eq!(server_name(Some(String::new())), host);
    }

    #[test]
    fn history_path_comes_from_the_config() {
        let dir = tempfile::Builder::new()
            .prefix("trd-daemon-unit-")
            .tempdir()
            .expect("a temporary directory");
        let config = dir.path().join("config.json");
        assert_eq!(history_path_from_config(&config), None, "no file");
        std::fs::write(&config, r#"{"render_history_path": ""}"#).expect("config.json");
        assert_eq!(history_path_from_config(&config), None, "empty");
        std::fs::write(&config, r#"{"render_history_path": "history.json"}"#).expect("config.json");
        assert_eq!(
            history_path_from_config(&config),
            Some(PathBuf::from("history.json"))
        );
    }

    /// A listener on a port that something else holds, and that port.
    fn busy_port() -> (std::net::TcpListener, u16) {
        let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
        let port = holder.local_addr().expect("an address").port();
        (holder, port)
    }

    #[tokio::test]
    async fn bind_retrying_retries_a_busy_port() {
        assert_eq!(BIND_RETRY, Duration::from_secs(30), "§7 retries every 30 s");
        let (holder, port) = busy_port();
        let (_stop, stop_rx) = watch::channel(false);
        let mut bound = pin!(bind_retrying(port, Duration::from_millis(5), stop_rx));
        // One poll makes the first attempt, which fails; the future now waits to try again.
        let first = poll_fn(|cx| Poll::Ready(bound.as_mut().poll(cx))).await;
        assert!(first.is_pending(), "the port is busy");
        drop(holder);
        let listener = tokio::time::timeout(HANG_GUARD, bound)
            .await
            .expect("a later attempt binds")
            .expect("no stop was requested");
        assert_eq!(listener.local_addr().expect("an address").port(), port);
    }

    #[tokio::test]
    async fn bind_retrying_returns_none_when_stopped() {
        let (_holder, port) = busy_port();
        let (stop, stop_rx) = watch::channel(false);
        // The delay is far beyond the hang guard: only the stop can end the wait.
        let mut bound = pin!(bind_retrying(port, Duration::from_secs(3600), stop_rx));
        let first = poll_fn(|cx| Poll::Ready(bound.as_mut().poll(cx))).await;
        assert!(first.is_pending(), "the port is busy");
        stop.send(true).expect("the receiver lives");
        let ended = tokio::time::timeout(HANG_GUARD, bound)
            .await
            .expect("the stop ends the wait");
        assert!(ended.is_none());
    }

    /// The handlers exist once `stop_signal` returns: a SIGTERM raised before the future is ever
    /// polled neither kills the process nor gets lost.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_signals_are_installed_before_the_wait() {
        for number in [libc::SIGTERM, libc::SIGINT] {
            let signalled = stop_signal().expect("the handlers");
            // SAFETY: raise signals only this test process, which has just installed a handler.
            assert_eq!(unsafe { libc::raise(number) }, 0);
            tokio::time::timeout(HANG_GUARD, signalled)
                .await
                .expect("the signal is delivered to the waiting future");
        }
    }
}
