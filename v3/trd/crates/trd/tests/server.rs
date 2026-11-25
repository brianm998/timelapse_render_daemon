// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of the trd.v1 gRPC server on loopback (TRD-17, spec 1 §5, §7).
//!
//! Each test starts `trd daemon` with `--port 0 --no-adopt --no-fda-check --lan-bind off
//! --no-mdns`, temporary data, log and config folders, and the fake tools copied into a temporary
//! tools folder. It reads the port from the `TRD_LISTENING` line and calls the daemon with the
//! tonic client of trd-proto. No test binds 50071 or 50072, reads the owner's data, or signals a
//! process it did not start. The only waits are for a state the daemon reaches, under a hang guard.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;
use tonic::transport::Channel;
use tonic::{Code, Status};
use trd_core::config::{Config, MaxProcesses, RenderType};
use trd_proto::trd::v1 as pb;
use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
use trd_testtools::{fake_bin_dir, make_tools_dir};

/// Stops a test that waits for a state that never comes. It is not a timing assertion.
const HANG_GUARD: Duration = Duration::from_secs(60);
/// The pause between two looks at the daemon while a test waits.
const POLL: Duration = Duration::from_millis(50);

type Client = TrdServiceClient<Channel>;

/// The temporary folders of one test. Removed when the test ends.
struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::Builder::new()
            .prefix("trd-server-test-")
            .tempdir()
            .expect("a temporary directory");
        make_tools_dir(&fake_bin_dir(), &root.path().join("tools")).expect("the fake tools");
        Fixture { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn config_path(&self) -> PathBuf {
        self.path("config/config.json")
    }

    /// Writes config.json (creating its folder) from `config`.
    fn write_config(&self, config: &Config) {
        let path = self.config_path();
        fs::create_dir_all(path.parent().expect("a folder")).expect("the config folder");
        fs::write(path, config.to_json_pretty()).expect("config.json");
    }
}

/// A running `trd daemon` child. Killed when dropped unless it was stopped first.
struct Daemon {
    child: Child,
    _stdout: BufReader<ChildStdout>,
    port: u16,
}

impl Daemon {
    /// Starts the daemon for `fixture` with `env` added to its environment, and waits for the
    /// `TRD_LISTENING` line.
    fn start(fixture: &Fixture, env: &[(&str, String)]) -> Daemon {
        let mut command = Command::new(env!("CARGO_BIN_EXE_trd"));
        command
            .arg("daemon")
            .args(["--port", "0"])
            .arg("--data-dir")
            .arg(fixture.path("data"))
            .arg("--log-dir")
            .arg(fixture.path("logs"))
            .arg("--config")
            .arg(fixture.config_path())
            .args([
                "--no-adopt",
                "--no-fda-check",
                "--lan-bind",
                "off",
                "--no-mdns",
            ])
            .env("TRD_FFMPEG_DIR", fixture.path("tools"))
            .env("TRD_EXIFTOOL_DIR", fixture.path("tools"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("trd daemon starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = String::new();
        stdout.read_line(&mut line).expect("the listening line");
        let port = line
            .trim_end()
            .strip_prefix("TRD_LISTENING loopback=127.0.0.1:")
            .and_then(|rest| rest.strip_suffix(" lan=off"))
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or_else(|| panic!("unexpected first line: {line:?}"));
        Daemon {
            child,
            _stdout: stdout,
            port,
        }
    }

    async fn client(&self) -> Client {
        TrdServiceClient::connect(format!("http://127.0.0.1:{}", self.port))
            .await
            .expect("the loopback listener accepts")
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends SIGTERM and waits for the process to exit.
    #[cfg(unix)]
    fn terminate(&mut self) -> ExitStatus {
        let pid = i32::try_from(self.pid()).expect("a pid fits in i32");
        // SAFETY: the pid is the daemon this test started and still owns; SIGTERM only asks it to stop.
        assert_eq!(
            unsafe { libc::kill(pid, libc::SIGTERM) },
            0,
            "SIGTERM is sent"
        );
        self.wait()
    }

    /// Waits for the process to exit, under the hang guard.
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + HANG_GUARD;
        loop {
            if let Some(status) = self.child.try_wait().expect("the child can be polled") {
                return status;
            }
            assert!(Instant::now() < deadline, "the daemon did not exit");
            std::thread::sleep(POLL);
        }
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

/// A config that names no sequence: the export and source folders are empty.
fn empty_config(fixture: &Fixture) -> Config {
    Config {
        export_paths: vec![fixture.path("export").to_string_lossy().into_owned()],
        source_paths: vec![fixture.path("source").to_string_lossy().into_owned()],
        rendering: vec![pro_res()],
        image_name_prefix: "LRT_".to_owned(),
        image_sequence_dirname_prefix: "LRT_".to_owned(),
        raw_types: vec!["tif".to_owned()],
        image_extensions: vec!["tif".to_owned()],
        max_processes: MaxProcesses {
            ffmpeg: 1,
            exiftool: 1,
        },
        max_cuncurrent_renders: 1,
        master_videos: Vec::new(),
        ..Config::default()
    }
}

fn pro_res() -> RenderType {
    RenderType {
        codec: "ProResUHQ".to_owned(),
        frame_rate: 25,
        resolution: Some(vec![1280, 720]),
    }
}

/// The proto form of [`empty_config`].
fn proto_config(fixture: &Fixture) -> pb::Config {
    pb::Config {
        export_paths: vec![fixture.path("export").to_string_lossy().into_owned()],
        source_paths: vec![fixture.path("source").to_string_lossy().into_owned()],
        rendering: vec![pb::RenderVariant {
            codec: "ProResUHQ".to_owned(),
            frame_rate: 25,
            resolution: vec![1280, 720],
            ..Default::default()
        }],
        image_name_prefix: "LRT_".to_owned(),
        image_sequence_dirname_prefix: "LRT_".to_owned(),
        raw_types: vec!["tif".to_owned()],
        image_extensions: vec!["tif".to_owned()],
        max_processes: Some(pb::MaxProcesses {
            ffmpeg: 1,
            exiftool: 1,
        }),
        max_concurrent_renders: 1,
        // A client sends the whole config it read; zero here would be refused by validation.
        scan_interval_seconds: 2.0,
        recent_render_history_days: 30,
        ..Default::default()
    }
}

/// The expected code of a failed call.
fn code_of<T: std::fmt::Debug>(result: Result<T, Status>) -> Code {
    result.expect_err("the call fails").code()
}

/// Makes one sequence `shot` (three raw frames, three exported frames) and returns the release file
/// that holds the fake render until it exists.
fn one_sequence(fixture: &Fixture) -> PathBuf {
    let raw = fixture.path("source/shot");
    let export = fixture.path("export/LRT_shot");
    fs::create_dir_all(&raw).expect("the raw folder");
    fs::create_dir_all(&export).expect("the export folder");
    for index in 1..=3 {
        fs::write(raw.join(format!("raw_{index:04}.tif")), b"x").expect("a raw frame");
        fs::write(export.join(format!("LRT_{index:04}.tif")), b"x").expect("an exported frame");
    }
    fixture.path("release")
}

/// Polls `GetSnapshot` until `found` returns a value, under the hang guard.
async fn poll_snapshot<T>(
    client: &mut Client,
    mut found: impl FnMut(&pb::Snapshot) -> Option<T>,
) -> T {
    let deadline = Instant::now() + HANG_GUARD;
    loop {
        let snapshot = client
            .get_snapshot(pb::GetSnapshotRequest {})
            .await
            .expect("GetSnapshot")
            .into_inner();
        if let Some(value) = found(&snapshot) {
            return value;
        }
        assert!(Instant::now() < deadline, "the state was not reached");
        tokio::time::sleep(POLL).await;
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn server_info_and_device_stubs() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    let info = client
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .expect("GetServerInfo")
        .into_inner();
    assert_eq!(info.api_version, 1);
    assert!(!info.server_id.is_empty());
    assert!(
        info.caller_is_local,
        "the loopback listener marks its callers local"
    );
    assert!(!info.sandbox, "no --sandbox was passed");
    assert_eq!(info.lan_port, 0, "the LAN listener is off");
    let tools: Vec<&str> = info.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tools, ["ffmpeg", "ffprobe", "exiftool"]);
    assert!(
        info.tools.iter().all(|t| t.available),
        "the fake tools resolve"
    );

    // Pairing arrives with TRD-18.
    assert_eq!(
        code_of(client.pair(pb::PairRequest::default()).await),
        Code::Unimplemented
    );
    assert_eq!(
        code_of(client.start_pairing(pb::StartPairingRequest {}).await),
        Code::Unimplemented
    );

    let devices = client
        .list_paired_devices(pb::ListPairedDevicesRequest {})
        .await
        .expect("ListPairedDevices")
        .into_inner();
    assert!(devices.devices.is_empty());
    assert_eq!(
        code_of(
            client
                .revoke_device(pb::RevokeDeviceRequest {
                    device_id: "no-such-device".to_owned()
                })
                .await
        ),
        Code::NotFound
    );
}

/// A2: `WatchSnapshots` sends the current snapshot at once, and then at most one message per
/// revision, in increasing order. It counts messages per revision, not time.
#[tokio::test(flavor = "multi_thread")]
async fn watch_rate() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    let before = client
        .get_snapshot(pb::GetSnapshotRequest {})
        .await
        .expect("GetSnapshot")
        .into_inner()
        .revision;
    let mut stream = client
        .watch_snapshots(pb::WatchSnapshotsRequest { max_per_second: 10 })
        .await
        .expect("WatchSnapshots")
        .into_inner();
    let first = stream
        .message()
        .await
        .expect("the stream answers")
        .expect("the current snapshot comes at once");
    assert!(
        first.revision >= before,
        "the first message is the current snapshot"
    );

    // Four changes publish four revisions; the stream must end on the last one.
    for paused in [true, false, true, false] {
        client
            .set_global_pause(pb::SetGlobalPauseRequest { paused })
            .await
            .expect("SetGlobalPause");
    }
    let last = client
        .get_snapshot(pb::GetSnapshotRequest {})
        .await
        .expect("GetSnapshot")
        .into_inner()
        .revision;

    let mut seen = vec![first.revision];
    let deadline = Instant::now() + HANG_GUARD;
    while *seen.last().expect("one revision") < last {
        assert!(
            Instant::now() < deadline,
            "the newest revision was not sent"
        );
        let message = stream
            .message()
            .await
            .expect("the stream answers")
            .expect("the stream stays open");
        seen.push(message.revision);
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] < pair[1]),
        "each revision is sent at most once, in order: {seen:?}"
    );
    assert!(seen.len() as u64 <= last - first.revision + 1);
}

/// A3: an `UpdateConfig` with a stale revision fails with FAILED_PRECONDITION and leaves the file as
/// it was. The revision of an existing file must be quoted to change it.
#[tokio::test(flavor = "multi_thread")]
async fn update_config_stale_revision() {
    let fixture = Fixture::new();
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    let missing = client
        .get_config(pb::GetConfigRequest {})
        .await
        .expect("GetConfig")
        .into_inner();
    assert!(!missing.exists);
    assert!(missing.revision.is_empty());
    assert_eq!(
        missing.path_source,
        pb::ConfigPathSource::ConfigPathFlag as i32
    );

    // An empty revision creates the file.
    let created = client
        .update_config(pb::UpdateConfigRequest {
            config: Some(proto_config(&fixture)),
            expected_revision: String::new(),
        })
        .await
        .expect("UpdateConfig creates the file")
        .into_inner();
    assert!(created.exists);
    assert_eq!(created.revision.len(), 64, "a SHA-256 in hex");
    let bytes_before = fs::read(fixture.config_path()).expect("config.json");

    let mut edited = proto_config(&fixture);
    edited.image_name_prefix = "other_".to_owned();
    let stale = client
        .update_config(pb::UpdateConfigRequest {
            config: Some(edited.clone()),
            expected_revision: "0".repeat(64),
        })
        .await;
    assert_eq!(code_of(stale), Code::FailedPrecondition);
    assert_eq!(
        fs::read(fixture.config_path()).expect("config.json"),
        bytes_before,
        "a refused edit writes nothing"
    );

    // An empty revision no longer means "create": the file exists.
    assert_eq!(
        code_of(
            client
                .update_config(pb::UpdateConfigRequest {
                    config: Some(edited.clone()),
                    expected_revision: String::new(),
                })
                .await
        ),
        Code::FailedPrecondition
    );

    let updated = client
        .update_config(pb::UpdateConfigRequest {
            config: Some(edited.clone()),
            expected_revision: created.revision,
        })
        .await
        .expect("UpdateConfig with the current revision")
        .into_inner();
    assert_eq!(
        updated.config.expect("the config").image_name_prefix,
        "other_"
    );

    let report = client
        .validate_config(pb::ValidateConfigRequest {
            config: Some(proto_config(&fixture)),
            codecs: Vec::new(),
        })
        .await
        .expect("ValidateConfig")
        .into_inner();
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.severity == pb::IssueSeverity::IssueError as i32),
        "a valid config has no error: {:?}",
        report.issues
    );
    assert_eq!(report.rendering.len(), 1);
    assert!(report.rendering[0].example_filename.ends_with(".mov"));
}

/// `UpdateCodecs` writes the table beside the config, `SetConfigPath` moves the path, and a relative
/// path is refused.
#[tokio::test(flavor = "multi_thread")]
async fn codecs_and_config_path() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    let codec = pb::VideoCodec {
        name: "Probe".to_owned(),
        ffmpeg: Some(pb::CodecFfmpeg {
            codec: "prores_ks".to_owned(),
            pixel_format: "yuv422p10le".to_owned(),
            color_range: "tv".to_owned(),
            ..Default::default()
        }),
        filename: Some(pb::CodecFilename {
            pixel_format: "422".to_owned(),
            codec: "ProRes".to_owned(),
            quality: "HQ".to_owned(),
        }),
    };
    let written = client
        .update_codecs(pb::UpdateCodecsRequest {
            codecs: vec![codec.clone()],
            expected_revision: String::new(),
        })
        .await
        .expect("UpdateCodecs")
        .into_inner();
    assert!(written.codecs.iter().any(|c| c.name == "Probe"));
    assert!(written.codecs_path.ends_with("video_codecs.json"));
    assert_eq!(written.codecs_revision.len(), 64);
    assert_eq!(
        code_of(
            client
                .update_codecs(pb::UpdateCodecsRequest {
                    codecs: vec![codec],
                    expected_revision: String::new(),
                })
                .await
        ),
        Code::FailedPrecondition,
        "the table exists now, so an empty revision is stale"
    );

    assert_eq!(
        code_of(
            client
                .set_config_path(pb::SetConfigPathRequest {
                    path: "relative.json".to_owned(),
                    create_if_missing: false,
                })
                .await
        ),
        Code::InvalidArgument
    );

    let moved = fixture.path("moved/config.json");
    let state = client
        .set_config_path(pb::SetConfigPathRequest {
            path: moved.to_string_lossy().into_owned(),
            create_if_missing: true,
        })
        .await
        .expect("SetConfigPath")
        .into_inner();
    assert_eq!(state.path, moved.to_string_lossy());
    assert_eq!(
        state.path_source,
        pb::ConfigPathSource::ConfigPathSetting as i32
    );
    assert!(state.exists, "create_if_missing writes the file");
    assert!(Path::new(&moved).is_file());
}

/// `ListDirectory`: the roots, the folder rules (directories first, dot entries hidden unless
/// asked for), and the error codes.
#[tokio::test(flavor = "multi_thread")]
async fn list_directory_rules() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    let roots = client
        .list_directory(pb::ListDirectoryRequest::default())
        .await
        .expect("ListDirectory of the roots")
        .into_inner();
    assert!(roots.entries.is_empty());
    assert!(
        roots.roots.iter().any(|root| root == "/"),
        "{:?}",
        roots.roots
    );
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().into_owned();
        assert!(roots.roots.contains(&home), "the home folder is a root");
    }

    let dir = fixture.path("browse");
    fs::create_dir_all(dir.join("visible")).expect("a folder");
    fs::create_dir_all(dir.join(".hidden")).expect("a hidden folder");
    fs::write(dir.join("note.txt"), b"x").expect("a file");
    fs::write(dir.join(".secret"), b"x").expect("a hidden file");
    let dir_text = dir.to_string_lossy().into_owned();

    let names = |entries: &[pb::DirEntry]| -> Vec<String> {
        entries.iter().map(|e| e.name.clone()).collect()
    };
    let folders_only = client
        .list_directory(pb::ListDirectoryRequest {
            path: dir_text.clone(),
            include_files: false,
            show_hidden: false,
        })
        .await
        .expect("ListDirectory")
        .into_inner();
    assert_eq!(names(&folders_only.entries), ["visible"]);
    assert_eq!(folders_only.path, dir_text);
    assert_eq!(
        folders_only.parent,
        fixture.root.path().to_string_lossy(),
        "the parent of the folder"
    );

    let with_files = client
        .list_directory(pb::ListDirectoryRequest {
            path: dir_text.clone(),
            include_files: true,
            show_hidden: false,
        })
        .await
        .expect("ListDirectory with files")
        .into_inner();
    assert_eq!(names(&with_files.entries), ["visible", "note.txt"]);
    assert!(with_files.entries[0].is_dir && !with_files.entries[1].is_dir);

    let everything = client
        .list_directory(pb::ListDirectoryRequest {
            path: dir_text,
            include_files: true,
            show_hidden: true,
        })
        .await
        .expect("ListDirectory showing hidden entries")
        .into_inner();
    assert_eq!(
        names(&everything.entries),
        [".hidden", "visible", ".secret", "note.txt"]
    );

    assert_eq!(
        code_of(
            client
                .list_directory(pb::ListDirectoryRequest {
                    path: "relative".to_owned(),
                    ..Default::default()
                })
                .await
        ),
        Code::InvalidArgument
    );
    assert_eq!(
        code_of(
            client
                .list_directory(pb::ListDirectoryRequest {
                    path: fixture.path("absent").to_string_lossy().into_owned(),
                    ..Default::default()
                })
                .await
        ),
        Code::NotFound
    );
}

/// Every control answers OK on a valid target and maps its refusals: an unknown id is NOT_FOUND.
#[tokio::test(flavor = "multi_thread")]
async fn controls_map_errors() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    for paused in [true, false] {
        client
            .set_global_pause(pb::SetGlobalPauseRequest { paused })
            .await
            .expect("SetGlobalPause");
    }
    let paused_now = poll_snapshot(&mut client, |s| Some(s.global_pause)).await;
    assert!(!paused_now, "the last pause request resumed");

    let sequence = |name: &str| pb::SequenceRef {
        sequence_name: name.to_owned(),
    };
    let variant = |name: &str| pb::VariantRef {
        sequence_name: name.to_owned(),
        variant_key: "ProResUHQ_25_1280x720".to_owned(),
    };
    let render = |id: &str| pb::RenderRef {
        render_id: id.to_owned(),
    };
    assert_eq!(
        code_of(client.pause_sequence(sequence("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.resume_sequence(sequence("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.pause_variant(variant("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.resume_variant(variant("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.retry_variant(variant("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.suspend_render(render("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.resume_render(render("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(client.stop_render(render("nope")).await),
        Code::NotFound
    );
    assert_eq!(
        code_of(
            client
                .dismiss_failure(pb::FailureRef {
                    failure_id: "nope".to_owned()
                })
                .await
        ),
        Code::NotFound
    );

    client
        .dismiss_all_failures(pb::DismissAllFailuresRequest {})
        .await
        .expect("DismissAllFailures with none to dismiss");
    client
        .rescan_now(pb::RescanNowRequest {})
        .await
        .expect("RescanNow");
}

/// The history and detail reads answer NOT_FOUND for unknown ids, and an empty history is empty.
#[tokio::test(flavor = "multi_thread")]
async fn history_and_details() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    let page = client
        .list_rendered_videos(pb::ListRenderedVideosRequest::default())
        .await
        .expect("ListRenderedVideos")
        .into_inner();
    assert!(page.videos.is_empty());
    assert!(!page.more);
    assert_eq!(
        code_of(
            client
                .list_rendered_videos(pb::ListRenderedVideosRequest {
                    before_id: "nope".to_owned(),
                    ..Default::default()
                })
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code_of(
            client
                .get_thumbnail(pb::GetThumbnailRequest {
                    rendered_video_id: "nope".to_owned()
                })
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code_of(
            client
                .get_render_details(pb::GetRenderDetailsRequest {
                    target: Some(pb::get_render_details_request::Target::RenderId(
                        "nope".to_owned()
                    )),
                })
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code_of(
            client
                .get_render_details(pb::GetRenderDetailsRequest {
                    target: Some(pb::get_render_details_request::Target::RenderedVideoId(
                        "nope".to_owned()
                    )),
                })
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code_of(
            client
                .get_render_details(pb::GetRenderDetailsRequest {
                    target: Some(pb::get_render_details_request::Target::FailureId(
                        "nope".to_owned()
                    )),
                })
                .await
        ),
        Code::NotFound
    );
    assert_eq!(
        code_of(
            client
                .get_render_details(pb::GetRenderDetailsRequest { target: None })
                .await
        ),
        Code::InvalidArgument
    );
}

/// `ListLog` pages newest first by `before_seq` with no repeats, and `min_level` filters.
/// `WatchLog` sends the stored rows after `after_seq` and then the live ones, once each.
#[tokio::test(flavor = "multi_thread")]
async fn log_paging_and_watch() {
    let fixture = Fixture::new();
    fixture.write_config(&empty_config(&fixture));
    let daemon = Daemon::start(&fixture, &[]);
    let mut client = daemon.client().await;

    // Each edit writes an INFO row, and the engine writes one when it reloads the config.
    let mut revision = client
        .get_config(pb::GetConfigRequest {})
        .await
        .expect("GetConfig")
        .into_inner()
        .revision;
    for index in 0..3 {
        let mut config = proto_config(&fixture);
        config.image_name_prefix = format!("prefix{index}_");
        revision = client
            .update_config(pb::UpdateConfigRequest {
                config: Some(config),
                expected_revision: revision,
            })
            .await
            .expect("UpdateConfig")
            .into_inner()
            .revision;
    }

    let mut seqs = Vec::new();
    let mut before = 0;
    loop {
        let page = client
            .list_log(pb::ListLogRequest {
                before_seq: before,
                limit: 2,
                min_level: 0,
            })
            .await
            .expect("ListLog")
            .into_inner();
        assert!(page.entries.len() <= 2);
        seqs.extend(page.entries.iter().map(|e| e.seq));
        match page.entries.last() {
            Some(last) if page.more => before = last.seq,
            _ => break,
        }
    }
    assert!(seqs.len() >= 3, "the edits left rows: {seqs:?}");
    assert!(
        seqs.windows(2).all(|pair| pair[0] > pair[1]),
        "newest first, no repeats: {seqs:?}"
    );
    let newest = seqs[0];

    let warnings = client
        .list_log(pb::ListLogRequest {
            before_seq: 0,
            limit: 0,
            min_level: pb::LogLevel::LogWarn as i32,
        })
        .await
        .expect("ListLog at WARN")
        .into_inner();
    assert!(
        warnings
            .entries
            .iter()
            .all(|e| e.level == pb::LogLevel::LogWarn as i32
                || e.level == pb::LogLevel::LogError as i32),
        "only WARN and ERROR rows at WARN"
    );

    // The backlog: every stored row after 0, oldest first, up to the newest one.
    let mut stream = client
        .watch_log(pb::WatchLogRequest {
            after_seq: 0,
            min_level: 0,
        })
        .await
        .expect("WatchLog")
        .into_inner();
    let mut watched = Vec::new();
    let deadline = Instant::now() + HANG_GUARD;
    while watched.last().copied().unwrap_or(0) < newest {
        assert!(Instant::now() < deadline, "the backlog did not arrive");
        let entry = stream
            .message()
            .await
            .expect("WatchLog answers")
            .expect("the stream stays open");
        watched.push(entry.seq);
    }
    assert!(
        watched.windows(2).all(|pair| pair[0] < pair[1]),
        "oldest first, no repeats: {watched:?}"
    );

    // A live row: one more edit, and the stream sends it once.
    client
        .update_config(pb::UpdateConfigRequest {
            config: Some(proto_config(&fixture)),
            expected_revision: revision,
        })
        .await
        .expect("UpdateConfig");
    let deadline = Instant::now() + HANG_GUARD;
    loop {
        assert!(Instant::now() < deadline, "the live row did not arrive");
        let entry = stream
            .message()
            .await
            .expect("WatchLog answers")
            .expect("the stream stays open");
        assert!(
            entry.seq > *watched.last().expect("some rows"),
            "no row is sent twice"
        );
        watched.push(entry.seq);
        if entry.message.contains("updated over the API") {
            break;
        }
    }
    assert!(watched.windows(2).all(|pair| pair[0] < pair[1]));
}

/// SIGTERM ends the streams with UNAVAILABLE, leaves a running render alive (A4), and exits 0.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_keeps_renders() {
    let fixture = Fixture::new();
    let release = one_sequence(&fixture);
    fixture.write_config(&Config {
        export_paths: vec![fixture.path("export").to_string_lossy().into_owned()],
        source_paths: vec![fixture.path("source").to_string_lossy().into_owned()],
        rendering: vec![pro_res()],
        image_name_prefix: "LRT_".to_owned(),
        image_sequence_dirname_prefix: "LRT_".to_owned(),
        raw_types: vec!["tif".to_owned()],
        image_extensions: vec!["tif".to_owned()],
        max_processes: MaxProcesses {
            ffmpeg: 1,
            exiftool: 1,
        },
        max_cuncurrent_renders: 1,
        master_videos: Vec::new(),
        ..Config::default()
    });
    // The histogram comes from the fake exiftool; the render is the fake ffmpeg, held until `release`.
    let rows = json!({
        "rows": {
            "raw_0001.tif": {"Model": "Model-A"},
            "raw_0002.tif": {"Model": "Model-A"},
            "raw_0003.tif": {"Model": "Model-A"}
        }
    });
    let mut daemon = Daemon::start(
        &fixture,
        &[
            ("TRD_FAKE_EXIFTOOL", rows.to_string()),
            (
                "TRD_FAKE_FFMPEG",
                json!({"wait_for_file": release.to_string_lossy()}).to_string(),
            ),
        ],
    );
    let mut client = daemon.client().await;

    let render_id = poll_snapshot(&mut client, |snapshot| {
        snapshot
            .in_progress
            .iter()
            .flat_map(|card| card.variants.iter())
            .find(|row| row.status == pb::VariantStatus::VariantRunning as i32)
            .map(|row| row.render_id.clone())
    })
    .await;
    let details = client
        .get_render_details(pb::GetRenderDetailsRequest {
            target: Some(pb::get_render_details_request::Target::RenderId(
                render_id.clone(),
            )),
        })
        .await
        .expect("GetRenderDetails of the running render")
        .into_inner();
    let render_pid = details.pid;
    assert!(render_pid > 0, "a running render has a pid");
    assert_eq!(details.status, pb::VariantStatus::VariantRunning as i32);

    // A control that does not apply to a running render: FAILED_PRECONDITION.
    assert_eq!(
        code_of(
            client
                .resume_render(pb::RenderRef {
                    render_id: render_id.clone()
                })
                .await
        ),
        Code::FailedPrecondition
    );

    let mut snapshots = client
        .watch_snapshots(pb::WatchSnapshotsRequest { max_per_second: 4 })
        .await
        .expect("WatchSnapshots")
        .into_inner();
    let mut logs = client
        .watch_log(pb::WatchLogRequest {
            after_seq: 0,
            min_level: 0,
        })
        .await
        .expect("WatchLog")
        .into_inner();
    snapshots
        .message()
        .await
        .expect("the snapshot stream answers")
        .expect("the current snapshot");

    let status = daemon.terminate();
    assert!(
        status.success(),
        "the daemon exits 0 after SIGTERM: {status:?}"
    );

    // Both streams end with UNAVAILABLE, not with a silent end.
    let end_of_snapshots = loop {
        match snapshots.message().await {
            Ok(Some(_)) => continue,
            other => break other,
        }
    };
    assert_eq!(code_of(end_of_snapshots), Code::Unavailable);
    let end_of_logs = loop {
        match logs.message().await {
            Ok(Some(_)) => continue,
            other => break other,
        }
    };
    assert_eq!(code_of(end_of_logs), Code::Unavailable);

    // The render the daemon started is still alive; it was not signalled.
    let pid = render_pid;
    // SAFETY: signal 0 only checks that the pid exists; the render is the one this test's daemon started.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(
        alive,
        "the render {render_pid} is still running after the daemon stopped"
    );
    // SAFETY: the pid is the fake ffmpeg this test's daemon started; it is ended here, after the check.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}
