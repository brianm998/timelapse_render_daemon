// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The live mDNS test (TRD-19, spec 1 §8.4; overview §15.2). It starts `trd daemon` with mDNS on,
//! browses `_trd._tcp.local.` with an mDNS browser of its own, and checks the advertised instance
//! and its TXT records against `GetServerInfo`. Then it stops the daemon and checks that the service
//! is withdrawn. It multicasts for real, so it runs only as a system test on the GitHub Linux runner
//! (`trd-live` in v3-system-tests.yml). It is Unix only: it stops `trd` with SIGTERM, which the
//! daemon handles, and the Windows runner does not run it.
//!
//! The daemon's data, log and config folders are in a temporary folder, its listeners use port 0 and
//! loopback, and no test signals a process it did not start. The waits are for states the daemon
//! reaches, under hang guards.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent};
use trd_proto::trd::v1 as pb;
use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
use trd_testtools::{fake_bin_dir, make_tools_dir, system_test};

/// The service type the daemon advertises (§8.4).
const SERVICE_TYPE: &str = "_trd._tcp.local.";
/// How long the registration may take to show up, and how long the daemon may take to stop. These
/// only guard against a hang; the withdrawal itself must be seen within 30 s (TRD-19 A3).
const APPEAR: Duration = Duration::from_secs(60);
const WITHDRAW: Duration = Duration::from_secs(30);

/// A `trd daemon` child that is killed when the test ends before it was stopped.
struct Daemon {
    child: Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The port that follows `prefix` in the daemon's `TRD_LISTENING` line.
fn port_after(line: &str, prefix: &str) -> u16 {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(prefix))
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("no {prefix}PORT in {line:?}"))
}

/// Waits for the first event of `events` that `wanted` accepts. Panics when none comes in `limit`.
fn wait_for(
    events: &Receiver<ServiceEvent>,
    limit: Duration,
    wanted: impl Fn(&ServiceEvent) -> bool,
) -> ServiceEvent {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(event) if wanted(&event) => return event,
            Ok(_) => {}
            Err(err) => panic!("no matching mDNS event within {limit:?}: {err}"),
        }
    }
}

system_test!(
    TRD_MDNS_TEST,
    async fn advertised_service_matches_server_info() {
        let root = tempfile::Builder::new()
            .prefix("trd-mdns-live-")
            .tempdir()
            .expect("a temporary directory");
        make_tools_dir(&fake_bin_dir(), &root.path().join("tools")).expect("the fake tools");

        // The advertised name has a dot and a space, so the instance name must be sanitized (§8.4).
        let pid = std::process::id();
        let advertised = format!("trd live.test {pid}");
        let instance = format!("trd-live-test-{pid}");
        let fullname = format!("{instance}.{SERVICE_TYPE}");

        let mut child = Command::new(env!("CARGO_BIN_EXE_trd"))
            .arg("daemon")
            .args(["--sandbox", "--port", "0", "--lan-bind", "127.0.0.1:0"])
            .arg("--advertise-name")
            .arg(&advertised)
            .arg("--data-dir")
            .arg(root.path().join("data"))
            .arg("--log-dir")
            .arg(root.path().join("logs"))
            .arg("--config")
            .arg(root.path().join("config/config.json"))
            .env("TRD_FFMPEG_DIR", root.path().join("tools"))
            .env("TRD_EXIFTOOL_DIR", root.path().join("tools"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("trd daemon starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut daemon = Daemon { child };
        let mut listening = String::new();
        stdout
            .read_line(&mut listening)
            .expect("the listening line");
        let loopback = port_after(&listening, "loopback=127.0.0.1:");
        let lan_port = port_after(&listening, "lan=127.0.0.1:");

        // What the daemon says about itself, over loopback.
        let mut client = TrdServiceClient::connect(format!("http://127.0.0.1:{loopback}"))
            .await
            .expect("the loopback listener accepts");
        let info = client
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .expect("GetServerInfo")
            .into_inner();
        assert_eq!(info.name, advertised);
        assert_eq!(info.lan_port, u32::from(lan_port));

        // What the network says: the service resolves under the sanitized instance name.
        let browser = ServiceDaemon::new().expect("an mDNS browser starts");
        let events = browser.browse(SERVICE_TYPE).expect("the browse starts");
        let resolved = match wait_for(&events, APPEAR, |event| {
            matches!(event, ServiceEvent::ServiceResolved(found)
            if found.get_fullname().eq_ignore_ascii_case(&fullname))
        }) {
            ServiceEvent::ServiceResolved(found) => found,
            other => panic!("not a resolved service: {other:?}"),
        };
        assert_eq!(
            u32::from(resolved.get_port()),
            info.lan_port,
            "the SRV port"
        );
        let txt = |key: &str| resolved.get_property_val_str(key).map(str::to_owned);
        assert_eq!(txt("id"), Some(info.server_id.clone()), "TXT id");
        assert_eq!(txt("name"), Some(info.name.clone()), "TXT name");
        assert!(!info.cert_fingerprint_sha256.is_empty());
        assert_eq!(
            txt("fp"),
            Some(info.cert_fingerprint_sha256.clone()),
            "TXT fp"
        );
        assert_eq!(txt("port"), Some(info.lan_port.to_string()), "TXT port");
        assert_eq!(txt("ver"), Some(info.version.clone()), "TXT ver");
        assert_eq!(txt("api"), Some("1".to_owned()), "TXT api");

        // Stop the daemon the way the service manager does: SIGTERM, which it handles.
        let pid = libc::pid_t::try_from(daemon.child.id()).expect("a pid fits");
        // SAFETY: the signal goes to this test's own daemon child, which it started.
        assert_eq!(
            unsafe { libc::kill(pid, libc::SIGTERM) },
            0,
            "SIGTERM is sent"
        );
        let stopped = Instant::now();
        let status = loop {
            if let Some(status) = daemon.child.try_wait().expect("the daemon's status") {
                break status;
            }
            assert!(
                stopped.elapsed() < APPEAR,
                "the daemon did not stop in {APPEAR:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(status.success(), "the daemon stops cleanly: {status}");

        // The goodbye packets withdraw the service: the browser sees it removed.
        let removed = wait_for(&events, WITHDRAW, |event| {
            matches!(event, ServiceEvent::ServiceRemoved(_, removed)
            if removed.eq_ignore_ascii_case(&fullname))
        });
        assert!(matches!(removed, ServiceEvent::ServiceRemoved(..)));
        let _ = browser.shutdown();
    }
);
