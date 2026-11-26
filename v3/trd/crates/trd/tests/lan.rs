// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of the LAN listener: pinned TLS, bearer tokens, pairing and revocation (TRD-18, spec 1 §8).
//!
//! Each test starts `trd daemon` with `--port 0 --lan-bind 127.0.0.1:0 --no-adopt --no-fda-check
//! --no-mdns`, temporary data, log and config folders, and the fake tools in a temporary folder. The
//! LAN address is loopback: no test binds a non-loopback address, reads the owner's data, binds
//! 50071 or 50072, or signals a process it did not start. The only waits are for a state the daemon
//! reaches, under a hang guard.

mod lan {
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, ChildStdout, Command, Stdio};
    use std::time::Duration;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use tokio_stream::StreamExt;
    use tonic::metadata::MetadataValue;
    use tonic::transport::{Certificate, Channel, ClientTlsConfig};
    use tonic::{Code, Request, Status};
    use trd_proto::trd::v1 as pb;
    use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
    use trd_testtools::{fake_bin_dir, make_tools_dir};

    /// Stops a test that waits for a state that never comes. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);

    type Client = TrdServiceClient<Channel>;

    /// The temporary folders of one test. Removed when the test ends.
    struct Fixture {
        root: TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::Builder::new()
                .prefix("trd-lan-test-")
                .tempdir()
                .expect("a temporary directory");
            make_tools_dir(&fake_bin_dir(), &root.path().join("tools")).expect("the fake tools");
            Fixture { root }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.path().join(relative)
        }
    }

    /// A running `trd daemon` child with a LAN listener. Killed when dropped.
    struct Daemon {
        child: Child,
        #[allow(dead_code)]
        stdout: BufReader<ChildStdout>,
        port: u16,
        lan_port: u16,
    }

    impl Daemon {
        /// Starts the daemon for `fixture` and waits for its `TRD_LISTENING` line.
        fn start(fixture: &Fixture) -> Daemon {
            let mut child = Command::new(env!("CARGO_BIN_EXE_trd"))
                .arg("daemon")
                .args(["--port", "0"])
                .args(["--lan-bind", "127.0.0.1:0"])
                .arg("--data-dir")
                .arg(fixture.path("data"))
                .arg("--log-dir")
                .arg(fixture.path("logs"))
                .arg("--config")
                .arg(fixture.path("config/config.json"))
                .args(["--no-adopt", "--no-fda-check", "--no-mdns"])
                .env("TRD_FFMPEG_DIR", fixture.path("tools"))
                .env("TRD_EXIFTOOL_DIR", fixture.path("tools"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("trd daemon starts");
            let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
            let mut line = String::new();
            stdout.read_line(&mut line).expect("the listening line");
            let fields: Vec<&str> = line.split_whitespace().collect();
            let port = fields
                .iter()
                .find_map(|f| f.strip_prefix("loopback=127.0.0.1:"))
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or_else(|| panic!("no loopback port in {line:?}"));
            let lan_port = fields
                .iter()
                .find_map(|f| f.strip_prefix("lan=127.0.0.1:"))
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or_else(|| panic!("no LAN port in {line:?}"));
            Daemon {
                child,
                stdout,
                port,
                lan_port,
            }
        }

        /// The loopback client: plaintext, every RPC allowed.
        async fn loopback(&self) -> Client {
            TrdServiceClient::connect(format!("http://127.0.0.1:{}", self.port))
                .await
                .expect("the loopback listener accepts")
        }

        /// The LAN certificate as the identity files hold it.
        fn certificate_pem(fixture: &Fixture) -> String {
            fs::read_to_string(fixture.path("data/identity/cert.pem")).expect("cert.pem")
        }

        /// A LAN client pinned to `pem`: the server's certificate must be that one, for localhost.
        async fn lan(&self, pem: &str) -> Client {
            let channel = Channel::from_shared(format!("https://127.0.0.1:{}", self.lan_port))
                .expect("a URI")
                .tls_config(
                    ClientTlsConfig::new()
                        .ca_certificate(Certificate::from_pem(pem))
                        .domain_name("localhost"),
                )
                .expect("the TLS settings")
                .connect()
                .await
                .expect("the LAN listener accepts the pinned certificate");
            TrdServiceClient::new(channel)
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    /// The lowercase hex SHA-256 of the DER inside a PEM certificate: the fingerprint (§8.1).
    fn fingerprint_of_pem(pem: &str) -> String {
        let body: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let der = STANDARD.decode(body).expect("the PEM body is base64");
        hex::encode(Sha256::digest(der))
    }

    /// A request carrying `token` as its bearer token.
    fn with_token<T>(message: T, token: &str) -> Request<T> {
        let mut request = Request::new(message);
        let value: MetadataValue<tonic::metadata::Ascii> =
            format!("Bearer {token}").parse().expect("an ASCII header");
        request.metadata_mut().insert("authorization", value);
        request
    }

    /// Issues a code on loopback and returns it.
    async fn start_pairing(daemon: &Daemon) -> pb::StartPairingResponse {
        daemon
            .loopback()
            .await
            .start_pairing(pb::StartPairingRequest {})
            .await
            .expect("loopback starts pairing")
            .into_inner()
    }

    fn pair_request(code: &str) -> pb::PairRequest {
        pb::PairRequest {
            code: code.to_owned(),
            device_name: "Phone".to_owned(),
            platform: pb::DevicePlatform::Ios as i32,
        }
    }

    fn code_of<T: std::fmt::Debug>(result: Result<T, Status>) -> Code {
        result.expect_err("the call fails").code()
    }

    /// Pairs a LAN device with the code of `StartPairing` and returns its token.
    async fn pair_device(daemon: &Daemon, fixture: &Fixture, code: &str) -> (String, String) {
        let mut lan = daemon.lan(&Daemon::certificate_pem(fixture)).await;
        let paired = lan
            .pair(pair_request(code))
            .await
            .expect("a correct code pairs")
            .into_inner();
        (paired.token, paired.device_id)
    }

    /// A1: a client pinned to the fingerprint pairs with a code from `StartPairing` and then
    /// streams snapshots with its token. The loopback `ServerInfo` reports the LAN port and the
    /// fingerprint of the certificate the LAN listener presents.
    #[tokio::test(flavor = "multi_thread")]
    async fn pair_then_stream() {
        let fixture = Fixture::new();
        let daemon = Daemon::start(&fixture);
        let pem = Daemon::certificate_pem(&fixture);

        let info = daemon
            .loopback()
            .await
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .expect("loopback ServerInfo")
            .into_inner();
        assert_eq!(info.lan_port, u32::from(daemon.lan_port));
        assert_eq!(info.cert_fingerprint_sha256, fingerprint_of_pem(&pem));
        assert!(info.caller_is_local);
        assert!(!info.caller_is_paired);

        let issued = start_pairing(&daemon).await;
        assert_eq!(issued.code.len(), 6);
        assert_eq!(issued.cert_fingerprint_sha256, info.cert_fingerprint_sha256);
        assert_eq!(issued.lan_port, u32::from(daemon.lan_port));

        // The pinned client reads ServerInfo without a token, then pairs with the code.
        let mut lan = daemon.lan(&pem).await;
        let open = lan
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .expect("ServerInfo needs no token")
            .into_inner();
        assert!(!open.caller_is_local);
        assert!(!open.caller_is_paired);
        assert_eq!(open.server_id, info.server_id);

        let paired = lan
            .pair(pair_request(&issued.code))
            .await
            .expect("the correct code pairs")
            .into_inner();
        assert!(!paired.token.is_empty());
        assert!(!paired.device_id.is_empty());
        assert_eq!(paired.server_id, info.server_id);

        let bearer = lan
            .get_server_info(with_token(pb::GetServerInfoRequest {}, &paired.token))
            .await
            .expect("the token is accepted")
            .into_inner();
        assert!(bearer.caller_is_paired);

        let mut stream = lan
            .watch_snapshots(with_token(
                pb::WatchSnapshotsRequest::default(),
                &paired.token,
            ))
            .await
            .expect("WatchSnapshots with the token")
            .into_inner();
        let first = tokio::time::timeout(HANG_GUARD, stream.next())
            .await
            .expect("a snapshot arrives")
            .expect("the stream has an item")
            .expect("the snapshot is not an error");
        assert!(first.revision > 0);

        let devices = daemon
            .loopback()
            .await
            .list_paired_devices(pb::ListPairedDevicesRequest {})
            .await
            .expect("loopback lists the devices")
            .into_inner();
        assert_eq!(devices.devices.len(), 1);
        assert_eq!(devices.devices[0].device_id, paired.device_id);
        assert_eq!(devices.devices[0].name, "Phone");
    }

    /// A2: a LAN call without a token fails UNAUTHENTICATED; a paired device's token is refused the
    /// loopback-only calls with PERMISSION_DENIED. GetServerInfo and Pair need no token.
    #[tokio::test(flavor = "multi_thread")]
    async fn lan_refusals() {
        let fixture = Fixture::new();
        let daemon = Daemon::start(&fixture);
        let pem = Daemon::certificate_pem(&fixture);
        let issued = start_pairing(&daemon).await;

        let mut lan = daemon.lan(&pem).await;
        assert_eq!(
            code_of(lan.get_snapshot(pb::GetSnapshotRequest {}).await),
            Code::Unauthenticated
        );
        assert_eq!(
            code_of(
                lan.list_paired_devices(pb::ListPairedDevicesRequest {})
                    .await
            ),
            Code::Unauthenticated
        );
        assert_eq!(
            code_of(lan.start_pairing(pb::StartPairingRequest {}).await),
            Code::Unauthenticated
        );
        let wrong = if issued.code == "000000" {
            "111111"
        } else {
            "000000"
        };
        assert_eq!(
            code_of(lan.pair(pair_request(wrong)).await),
            Code::Unauthenticated,
            "a wrong code is refused, but Pair itself is open"
        );

        let (token, _) = pair_device(&daemon, &fixture, &issued.code).await;
        assert_eq!(
            code_of(
                lan.list_paired_devices(with_token(pb::ListPairedDevicesRequest {}, &token))
                    .await
            ),
            Code::PermissionDenied
        );
        assert_eq!(
            code_of(
                lan.start_pairing(with_token(pb::StartPairingRequest {}, &token))
                    .await
            ),
            Code::PermissionDenied
        );
        assert_eq!(
            code_of(
                lan.revoke_device(with_token(
                    pb::RevokeDeviceRequest {
                        device_id: "any".to_owned()
                    },
                    &token
                ))
                .await
            ),
            Code::PermissionDenied
        );
        assert_eq!(
            code_of(
                lan.get_snapshot(with_token(pb::GetSnapshotRequest {}, "not-a-token"))
                    .await
            ),
            Code::Unauthenticated,
            "an unknown token is no token"
        );
    }

    /// A3: five wrong codes invalidate the code: the correct one is then refused, and a new code
    /// from `StartPairing` works.
    #[tokio::test(flavor = "multi_thread")]
    async fn attempt_limit() {
        let fixture = Fixture::new();
        let daemon = Daemon::start(&fixture);
        let pem = Daemon::certificate_pem(&fixture);
        let issued = start_pairing(&daemon).await;
        let wrong = if issued.code == "000000" {
            "111111"
        } else {
            "000000"
        };

        let mut lan = daemon.lan(&pem).await;
        for _ in 0..4 {
            assert_eq!(
                code_of(lan.pair(pair_request(wrong)).await),
                Code::Unauthenticated
            );
        }
        assert_eq!(
            code_of(lan.pair(pair_request(wrong)).await),
            Code::ResourceExhausted,
            "the fifth wrong code"
        );
        assert_eq!(
            code_of(lan.pair(pair_request(&issued.code)).await),
            Code::Unauthenticated,
            "the invalidated code is refused"
        );

        let reissued = start_pairing(&daemon).await;
        lan.pair(pair_request(&reissued.code))
            .await
            .expect("a new code pairs");
    }

    /// A4: revoking a device ends its open LAN stream with UNAUTHENTICATED, and its token is refused
    /// afterwards.
    #[tokio::test(flavor = "multi_thread")]
    async fn revoke_ends_stream() {
        let fixture = Fixture::new();
        let daemon = Daemon::start(&fixture);
        let pem = Daemon::certificate_pem(&fixture);
        let issued = start_pairing(&daemon).await;
        let (token, device_id) = pair_device(&daemon, &fixture, &issued.code).await;

        let mut lan = daemon.lan(&pem).await;
        let mut stream = lan
            .watch_snapshots(with_token(pb::WatchSnapshotsRequest::default(), &token))
            .await
            .expect("the stream opens")
            .into_inner();
        tokio::time::timeout(HANG_GUARD, stream.next())
            .await
            .expect("the first snapshot arrives")
            .expect("an item")
            .expect("a snapshot");

        daemon
            .loopback()
            .await
            .revoke_device(pb::RevokeDeviceRequest {
                device_id: device_id.clone(),
            })
            .await
            .expect("loopback revokes the device");

        let ended = tokio::time::timeout(HANG_GUARD, stream.next())
            .await
            .expect("the revoke ends the stream")
            .expect("an error item");
        assert_eq!(ended.expect_err("revoked").code(), Code::Unauthenticated);
        assert!(stream.next().await.is_none(), "the stream is over");

        assert_eq!(
            code_of(
                lan.get_snapshot(with_token(pb::GetSnapshotRequest {}, &token))
                    .await
            ),
            Code::Unauthenticated,
            "the revoked token is refused"
        );
        let devices = daemon
            .loopback()
            .await
            .list_paired_devices(pb::ListPairedDevicesRequest {})
            .await
            .expect("loopback lists the devices")
            .into_inner();
        assert!(
            devices.devices.is_empty(),
            "a revoked device leaves the list"
        );
    }

    /// A5: the identity files and the fingerprint are the same after a restart, and the key is
    /// private.
    #[tokio::test(flavor = "multi_thread")]
    async fn stable_identity() {
        let fixture = Fixture::new();
        let read =
            |name: &str| fs::read(fixture.path(&format!("data/identity/{name}"))).expect(name);

        let first = Daemon::start(&fixture);
        let fingerprint = first
            .loopback()
            .await
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .expect("ServerInfo")
            .into_inner()
            .cert_fingerprint_sha256;
        let before = (read("cert.pem"), read("key.pem"), read("fingerprint.txt"));
        assert_eq!(fingerprint, String::from_utf8_lossy(&before.2).trim());
        assert_eq!(
            fingerprint,
            fingerprint_of_pem(&String::from_utf8_lossy(&before.0))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(fixture.path("data/identity/key.pem"))
                .expect("key.pem")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the key is private");
        }
        drop(first);

        let second = Daemon::start(&fixture);
        let after = (read("cert.pem"), read("key.pem"), read("fingerprint.txt"));
        assert_eq!(before, after, "the identity files are unchanged");
        let again = second
            .loopback()
            .await
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .expect("ServerInfo")
            .into_inner()
            .cert_fingerprint_sha256;
        assert_eq!(
            again, fingerprint,
            "the fingerprint is stable across restarts"
        );
    }
}
