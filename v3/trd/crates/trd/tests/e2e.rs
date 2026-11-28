// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! End-to-end acceptance of `trd` with the real ffmpeg, ffprobe and exiftool (TRD-GATE, spec 1 §12
//! and §16). Every test returns at once unless `TRD_E2E=1` is set. With it set, a tool that does
//! not resolve (§6.15) fails the test rather than skipping it, as `trd-core/tests/xmp.rs` does.
//!
//! Each test builds its own sandbox tree (§12) in a temporary folder with `sandbox.py init --dir`,
//! and starts `trd daemon --sandbox --no-mdns --port 0 --lan-bind 127.0.0.1:0` on it. No test binds
//! 50071, 50072, 50081 or 50082, reads the owner's data, or signals a process it did not start: a
//! test signals only the fake ffmpeg it starts itself, its own daemon, and the renders of its own
//! daemon. The waits poll a state the daemon reaches, under a hang guard. They are not timing
//! assertions.
//!
//! A sandbox render can end in under a second, so the daemon under test runs [`Gate`] as its
//! ffmpeg: the renders a test has to control are held until it releases them, and then the real
//! ffmpeg renders them.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use trd_core::tools::{ResolvedTool, Tools};
use trd_proto::trd::v1 as pb;
use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
use trd_testtools::{fake_bin_dir, make_tools_dir};

type Client = TrdServiceClient<Channel>;

/// Stops a test that waits for a state that never comes. It is not a timing assertion.
const HANG_GUARD: Duration = Duration::from_secs(120);
/// How often a settled state is polled.
const POLL: Duration = Duration::from_millis(100);
/// `sandbox.py`, from the crate directory (crates/trd) up to v3/trd.
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sandbox/sandbox.py");

/// The sandbox sequences (§12), by their export folder names.
const SEQ_A: &str = "LRT_2026_01_01-sandbox-a";
const SEQ_B: &str = "LRT_2026_01_01-sandbox-b";
const SEQ_G: &str = "LRT_2026_01_01-sandbox-g-merge";
const SEQ_F: &str = "LRT_2026_01_01-sandbox-f";
/// Sequence C is named by the substring the daemon's rows carry.
const SEQ_C: &str = "sandbox-c";

/// The variant keys of the sandbox's rendering list (§6.5): `<codec>_<res>_<frame_rate>`.
const VARIANTS: [&str; 3] = ["ProResMQ_1080x720_30", "h264_720w_30", "ProResUHQ_ori_30"];
/// The variant that UpdateConfig adds: its key is `h264_3840w_30`.
const ADDED_KEY: &str = "h264_3840w_30";

/// Sequence B's three renders: variant key, output file name (§6.5) and the size ffprobe reports.
const B_RENDERS: [(&str, &str, u32, u32); 3] = [
    (
        "ProResMQ_1080x720_30",
        "2026_01_01-sandbox-b_ProRes-422_Rec.709F_720p_30_MQ.mov",
        1080,
        720,
    ),
    (
        "h264_720w_30",
        "2026_01_01-sandbox-b_H264-420_Rec.709F_720w_30_HQ.mov",
        720,
        480,
    ),
    (
        "ProResUHQ_ori_30",
        "2026_01_01-sandbox-b_ProRes-444_Rec.709F_OriRes_30_UHQ.mov",
        320,
        213,
    ),
];

/// The sequences that must never show in a snapshot before UpdateConfig. After it, D gets the new
/// variant too, so only E stays hidden.
const HIDDEN_BEFORE_UPDATE: [&str; 2] = ["sandbox-d", "sandbox-e"];
const HIDDEN_AFTER_UPDATE: [&str; 1] = ["sandbox-e"];

/// The XMP values the sandbox frames carry (§12): Make, Model and ISO, in that order.
const XMP_VALUES: [&str; 3] = ["SandboxCam", "SC-1", "1600"];

/// The gate's pattern for every render of sequence A: its export folder and outputs carry it.
const HOLD_A: &str = "sandbox-a";
/// The gate's pattern for every render of the added variant: its output names carry it.
const HOLD_ADDED: &str = "_3840w_";

/// The stand-in `ffmpeg` of [`Gate`]. Only a render (it has `-progress`) is ever held: while a file
/// in `hold/` names a part of its arguments, it marks `held/<name>` and waits. A release moves the
/// file to `released/`; when it is gone otherwise (the test's folder is being removed), or after
/// 12000 polls (at least ten minutes), the script fails instead. Then it becomes the real ffmpeg,
/// with the same pid, start time and arguments, so the daemon controls and adopts it as before.
const GATE_SCRIPT: &str = r#"#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
render=no
for arg in "$@"; do
  if [ "$arg" = "-progress" ]; then render=yes; fi
done
if [ "$render" = yes ]; then
  for hold in "$TRD_E2E_GATE_DIR"/hold/*; do
    [ -e "$hold" ] || continue
    name=${hold##*/}
    case "$*" in
      *"$name"*) ;;
      *) continue ;;
    esac
    : > "$TRD_E2E_GATE_DIR/held/$name"
    polls=0
    while [ -e "$hold" ]; do
      polls=$((polls + 1))
      [ "$polls" -le 12000 ] || exit 1
      sleep 0.05
    done
    [ -e "$TRD_E2E_GATE_DIR/released/$name" ] || exit 1
  done
fi
exec "$TRD_E2E_REAL_FFMPEG" "$@"
"#;

/// Whether the real-tool tests run. The default run returns from each test at once.
fn e2e_enabled() -> bool {
    std::env::var_os("TRD_E2E").is_some()
}

/// The tools as the daemon resolves them (§6.15). Under `TRD_E2E` a missing tool fails the test.
fn real_tools() -> Tools {
    let exe = std::env::current_exe().expect("the test executable's path");
    let tools = Tools::resolve(|key| std::env::var_os(key), &exe);
    for (name, tool) in [
        ("ffmpeg", &tools.ffmpeg),
        ("ffprobe", &tools.ffprobe),
        ("exiftool", &tools.exiftool),
    ] {
        assert!(
            tool.command.is_some(),
            "TRD_E2E is set but {name} is not available: {:?}",
            tool.info
        );
    }
    tools
}

/// The Python 3 interpreter: `PYTHON` when set, else the first of `python3` and `python` that is
/// Python 3.
fn python() -> OsString {
    if let Some(chosen) = std::env::var_os("PYTHON") {
        return chosen;
    }
    for candidate in ["python3", "python"] {
        let is_python3 = Command::new(candidate)
            .args([
                "-c",
                "import sys; sys.exit(0 if sys.version_info[0] == 3 else 1)",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if is_python3 {
            return OsString::from(candidate);
        }
    }
    OsString::from("python3")
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

/// The temporary folder of one test, removed when the test ends.
struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::Builder::new()
            .prefix("trd-e2e-")
            .tempdir()
            .expect("a temporary directory");
        Fixture { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    /// Builds the sandbox tree with the real tools (§12) and returns its folder.
    fn build_tree(&self) -> PathBuf {
        let tree = self.path("tree");
        let output = Command::new(python())
            .arg("-I")
            .arg(SCRIPT)
            .arg("init")
            .arg("--dir")
            .arg(&tree)
            .env_remove("TRD_FAKE_FFMPEG")
            .env_remove("TRD_FAKE_FFPROBE")
            .stdin(Stdio::null())
            .output()
            .expect("the Python 3 interpreter runs sandbox.py");
        assert!(
            output.status.success(),
            "sandbox.py init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        tree
    }
}

/// Holds chosen renders until the test releases them. Its folder has `tools/`, a `TRD_FFMPEG_DIR`
/// whose ffmpeg is [`GATE_SCRIPT`] and whose ffprobe is the real one, and the `hold/`, `held/` and
/// `released/` folders the script uses. Unix only when run; it builds on every platform.
struct Gate {
    root: PathBuf,
    real_ffmpeg: OsString,
}

impl Gate {
    fn new(root: PathBuf, real_ffmpeg: &OsStr, real_ffprobe: &OsStr) -> Gate {
        for folder in ["tools", "hold", "held", "released"] {
            fs::create_dir_all(root.join(folder)).expect("a folder of the gate");
        }
        // A child writes the script: this process, which spawns from several threads, never holds
        // a write descriptor to a file that is about to be executed (ETXTBSY on Linux).
        let mut writer = Command::new("sh")
            .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
            .arg(root.join("tools").join("ffmpeg"))
            .stdin(Stdio::piped())
            .spawn()
            .expect("sh starts");
        writer
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(GATE_SCRIPT.as_bytes())
            .expect("the gate script is sent");
        assert!(
            writer.wait().expect("sh ends").success(),
            "the gate script is written"
        );
        let linked = Command::new("ln")
            .arg("-s")
            .arg(real_ffprobe)
            .arg(root.join("tools").join("ffprobe"))
            .status()
            .expect("ln runs");
        assert!(linked.success(), "ffprobe is linked beside the gate");
        Gate {
            root,
            real_ffmpeg: real_ffmpeg.to_owned(),
        }
    }

    /// Makes `command` use the gate as its ffmpeg.
    fn apply(&self, command: &mut Command) {
        command
            .env("TRD_FFMPEG_DIR", self.root.join("tools"))
            .env("TRD_E2E_REAL_FFMPEG", &self.real_ffmpeg)
            .env("TRD_E2E_GATE_DIR", &self.root);
    }

    /// Holds every render started from now on whose arguments contain `pattern`.
    fn hold(&self, pattern: &str) {
        fs::write(self.root.join("hold").join(pattern), "").expect("the hold file");
    }

    /// Lets the renders held for `pattern` go on to the real ffmpeg.
    fn release(&self, pattern: &str) {
        fs::rename(
            self.root.join("hold").join(pattern),
            self.root.join("released").join(pattern),
        )
        .expect("the pattern is held");
    }
}

/// `sandbox.py drip` at 50 frames a second, started in the background. Killed when dropped.
struct Drip {
    child: Child,
}

impl Drip {
    fn start(tree: &Path) -> Drip {
        let child = Command::new(python())
            .arg("-I")
            .arg(SCRIPT)
            .arg("drip")
            .arg("--dir")
            .arg(tree)
            .args(["--fps", "50"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("sandbox.py drip starts");
        Drip { child }
    }

    /// Waits until the drip ends, under the hang guard, and returns its status.
    async fn finish(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("the drip can be polled") {
                return status;
            }
            assert!(
                started.elapsed() < HANG_GUARD,
                "sandbox.py drip did not finish within {HANG_GUARD:?}"
            );
            tokio::time::sleep(POLL).await;
        }
    }
}

impl Drop for Drip {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// On Linux, makes this test process the parent of renders whose daemon is killed. A render lives
/// in its own process group; when the daemon dies while the render is stopped, init would adopt it,
/// the group would count as orphaned and the kernel would send it SIGHUP and SIGCONT, killing
/// ffmpeg before the new daemon can adopt it. Under this process the group is not orphaned.
fn keep_orphans_in_session() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: prctl with PR_SET_CHILD_SUBREAPER only sets a flag on this process.
        unsafe {
            libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
        }
    }
}

/// A running `trd daemon --sandbox` on a tree. Killed when dropped.
struct Daemon {
    child: Child,
    #[allow(dead_code)]
    stdout: BufReader<ChildStdout>,
    port: u16,
    lan_port: u16,
}

impl Daemon {
    /// Starts the daemon on `tree` with the same flags every time, and waits for `TRD_LISTENING`.
    /// With a `gate`, the daemon's ffmpeg is the gate's.
    fn start(tree: &Path, gate: Option<&Gate>) -> Daemon {
        keep_orphans_in_session();
        let mut command = Command::new(env!("CARGO_BIN_EXE_trd"));
        command
            .arg("daemon")
            .arg("--sandbox")
            .arg("--no-mdns")
            .args(["--port", "0"])
            .args(["--lan-bind", "127.0.0.1:0"])
            .arg("--config")
            .arg(tree.join("config.json"))
            .arg("--data-dir")
            .arg(tree.join("data"))
            .arg("--log-dir")
            .arg(tree.join("logs"))
            .env_remove("TRD_FAKE_FFMPEG")
            .env_remove("TRD_FAKE_FFPROBE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(gate) = gate {
            gate.apply(&mut command);
        }
        let mut child = command.spawn().expect("trd daemon starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = String::new();
        stdout.read_line(&mut line).expect("the listening line");
        let fields: Vec<&str> = line.split_whitespace().collect();
        let port = fields
            .iter()
            .find_map(|field| field.strip_prefix("loopback=127.0.0.1:"))
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or_else(|| panic!("no loopback port in {line:?}"));
        let lan_port = fields
            .iter()
            .find_map(|field| field.strip_prefix("lan=127.0.0.1:"))
            .and_then(|port| port.parse::<u16>().ok())
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

    /// Kills the daemon, as a crash would. Its renders are separate processes and keep running.
    fn crash(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// Runs a tool with `args` and returns its standard output. A failing tool fails the test.
fn tool_text(tool: &ResolvedTool, args: &[&str]) -> String {
    let command = tool
        .command
        .as_ref()
        .unwrap_or_else(|| panic!("a tool is missing: {:?}", tool.info));
    let output = command
        .command()
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the tool runs");
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The width, height and display aspect ratio that ffprobe reports for the first video stream.
fn probe(tools: &Tools, file: &Path) -> (u32, u32, String) {
    let text = tool_text(
        &tools.ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,display_aspect_ratio",
            "-of",
            "default=noprint_wrappers=1",
            utf8(file),
        ],
    );
    let field = |name: &str| -> String {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("ffprobe reports no {name} for {file:?}: {text}"))
            .to_owned()
    };
    (
        field("width").parse().expect("a width"),
        field("height").parse().expect("a height"),
        field("display_aspect_ratio"),
    )
}

/// The XMP Make, Model and ISO that exiftool reads from a render, in that order.
fn xmp_values(tools: &Tools, file: &Path) -> Vec<String> {
    tool_text(
        &tools.exiftool,
        &[
            "-s3",
            "-XMP-tiff:Make",
            "-XMP-tiff:Model",
            "-XMP-exif:ISO",
            utf8(file),
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

/// The `paused_variants` entries of config.json, as (sequence, variant key) pairs.
fn paused_in(tree: &Path) -> Vec<(String, String)> {
    let text = fs::read_to_string(tree.join("config.json")).expect("config.json");
    let config: serde_json::Value = serde_json::from_str(&text).expect("config.json is JSON");
    config["paused_variants"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item["sequence_name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        item["variant_key"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fails when a snapshot names one of `hidden`: §12 says those sequences are never shown.
fn assert_hidden(snapshot: &pb::Snapshot, hidden: &[&str]) {
    if hidden.is_empty() {
        return;
    }
    let text = format!("{snapshot:?}");
    for name in hidden {
        assert!(
            !text.contains(name),
            "the snapshot mentions {name}: {snapshot:?}"
        );
    }
}

/// Polls GetSnapshot until `check` accepts a snapshot, under the hang guard. A snapshot that names
/// one of `hidden` fails at once.
async fn settle<T>(
    client: &mut Client,
    what: &str,
    every: Duration,
    hidden: &[&str],
    mut check: impl FnMut(&pb::Snapshot) -> Option<T>,
) -> T {
    let started = Instant::now();
    loop {
        let snapshot = snapshot_of(client).await;
        assert_hidden(&snapshot, hidden);
        if let Some(found) = check(&snapshot) {
            return found;
        }
        assert!(
            started.elapsed() < HANG_GUARD,
            "{what} was not reached within {HANG_GUARD:?}: {}",
            summary_of(&snapshot)
        );
        tokio::time::sleep(every).await;
    }
}

/// A short account of a snapshot for a failed wait: the statuses of its rows and its failures.
fn summary_of(snapshot: &pb::Snapshot) -> String {
    let mut text = String::new();
    for card in &snapshot.in_progress {
        text.push_str(&format!("\n  {} stage={:?}", card.name, card.stage));
        for variant in &card.variants {
            text.push_str(&format!(
                "\n    {} {:?} id={} {}",
                variant.variant_key, variant.status, variant.render_id, variant.message
            ));
        }
    }
    for failure in &snapshot.failures {
        text.push_str(&format!(
            "\n  failure {} {}: {}",
            failure.sequence_name, failure.variant_key, failure.summary
        ));
    }
    format!(
        "{} rendered, {} in progress, {} failures{text}",
        snapshot.recently_rendered_total,
        snapshot.in_progress.len(),
        snapshot.failures.len()
    )
}

async fn snapshot_of(client: &mut Client) -> pb::Snapshot {
    client
        .get_snapshot(pb::GetSnapshotRequest {})
        .await
        .expect("GetSnapshot answers")
        .into_inner()
}

/// The variant row of `sequence` (matched by a substring of its export folder) for `key`.
fn row(snapshot: &pb::Snapshot, sequence: &str, key: &str) -> Option<pb::VariantRow> {
    snapshot
        .in_progress
        .iter()
        .filter(|card| card.name.contains(sequence))
        .flat_map(|card| card.variants.iter())
        .find(|candidate| candidate.variant_key == key)
        .cloned()
}

/// The variant row whose render has `render_id`, in any sequence.
fn row_by_id(snapshot: &pb::Snapshot, render_id: &str) -> Option<pb::VariantRow> {
    snapshot
        .in_progress
        .iter()
        .flat_map(|card| card.variants.iter())
        .find(|candidate| candidate.render_id == render_id)
        .cloned()
}

fn is(row: &pb::VariantRow, status: pb::VariantStatus) -> bool {
    row.status == status as i32
}

/// The finished render of `sequence` for `key`, from the newest renders the snapshot lists.
fn rendered<'a>(
    snapshot: &'a pb::Snapshot,
    sequence: &str,
    key: &str,
) -> Option<&'a pb::RenderedVideo> {
    snapshot
        .recently_rendered
        .iter()
        .find(|video| video.sequence_name.contains(sequence) && video.variant_key == key)
}

/// B's three renders carry the §6.5 names, the sizes ffprobe reports, DAR 3:2 for the original
/// size, and the XMP values of the frames.
fn check_sequence_b(tools: &Tools, snapshot: &pb::Snapshot) {
    for (key, name, width, height) in B_RENDERS {
        let video = rendered(snapshot, SEQ_B, key)
            .unwrap_or_else(|| panic!("B has no {key} render: {snapshot:?}"));
        assert_eq!(video.output_filename, name, "the output name of {key}");
        let file = Path::new(&video.output_path);
        assert!(file.is_file(), "{} is on disk", video.output_path);
        let (probed_width, probed_height, dar) = probe(tools, file);
        assert_eq!(
            (probed_width, probed_height),
            (width, height),
            "the size of {name}"
        );
        if key == "ProResUHQ_ori_30" {
            // ProRes stores a rounded sample aspect ratio, so ffprobe may print 254:169 for 3:2.
            let (num, den) = dar.split_once(':').expect("DAR is num:den");
            let ratio = num.parse::<f64>().expect("DAR numerator")
                / den.parse::<f64>().expect("DAR denominator");
            assert!(
                (ratio - 1.5).abs() < 0.01,
                "the original-size render keeps DAR 3:2, got {dar}"
            );
        }
        assert_eq!(
            xmp_values(tools, file),
            XMP_VALUES,
            "the XMP of {name} (Make, Model, ISO)"
        );
    }
}

/// Sequence F renders from its backup sidecar, whose ImageSize count is its 16 frames.
fn check_sequence_f(snapshot: &pb::Snapshot) {
    for key in VARIANTS {
        let video =
            rendered(snapshot, SEQ_F, key).unwrap_or_else(|| panic!("F has no {key} render"));
        assert_eq!(video.frame_count, 16, "F renders the backup's 16 frames");
    }
}

/// Sequence C renders all 24 frames although frame 12 is truncated (the accepted real behaviour,
/// §12): it shows no failure card, and the log of each of its renders records the decode error.
async fn check_sequence_c(client: &mut Client, snapshot: &pb::Snapshot) {
    assert!(
        !snapshot
            .failures
            .iter()
            .any(|failure| failure.sequence_name.contains(SEQ_C)),
        "C shows no failure card: {:?}",
        snapshot.failures
    );
    for key in VARIANTS {
        let video = rendered(snapshot, SEQ_C, key)
            .unwrap_or_else(|| panic!("C has no {key} render: {snapshot:?}"));
        assert_eq!(video.frame_count, 24, "C renders all 24 frames");
        let details = client
            .get_render_details(pb::GetRenderDetailsRequest {
                target: Some(pb::get_render_details_request::Target::RenderedVideoId(
                    video.id.clone(),
                )),
            })
            .await
            .expect("the render of C has details")
            .into_inner();
        assert!(
            details.log_tail.iter().any(|line| {
                let line = line.to_ascii_lowercase();
                ["error", "invalid", "premature", "truncat", "corrupt"]
                    .iter()
                    .any(|word| line.contains(word))
            }),
            "the log tail of the {key} render of C records the decode error: {:?}",
            details.log_tail
        );
    }
}

/// The history lists the newest render first, and each B render has a thumbnail.
async fn check_history(client: &mut Client) {
    let snapshot = snapshot_of(client).await;
    assert!(
        snapshot
            .recently_rendered
            .windows(2)
            .all(|pair| pair[0].rendered_at_ms >= pair[1].rendered_at_ms),
        "recently_rendered is newest first: {:?}",
        snapshot
            .recently_rendered
            .iter()
            .map(|video| video.rendered_at_ms)
            .collect::<Vec<_>>()
    );
    for (key, ..) in B_RENDERS {
        let video = rendered(&snapshot, SEQ_B, key).expect("the render is listed");
        let thumbnail = client
            .get_thumbnail(pb::GetThumbnailRequest {
                rendered_video_id: video.id.clone(),
            })
            .await
            .expect("a thumbnail is made")
            .into_inner();
        assert!(
            !thumbnail.jpeg.is_empty() && thumbnail.width > 0 && thumbnail.height > 0,
            "the thumbnail of {key} has a picture"
        );
    }
    let after = snapshot_of(client).await;
    for (key, ..) in B_RENDERS {
        let video = rendered(&after, SEQ_B, key).expect("the render is still listed");
        assert!(
            video.has_thumbnail,
            "the listed {key} render has a thumbnail"
        );
    }
}

/// The real daemon with the real tools: sequences B, C, D, E, F and G (§12), the A drip, the
/// controls, UpdateConfig, LAN pairing and re-adoption (§16 row "the whole daemon with real tools").
#[tokio::test(flavor = "multi_thread")]
async fn e2e_daemon_with_real_tools() {
    if !e2e_enabled() {
        return;
    }
    let tools = real_tools();
    let fixture = Fixture::new();
    let tree = fixture.build_tree();
    let program = |tool: &ResolvedTool| tool.command.as_ref().expect("resolved").program.clone();
    let gate = Gate::new(
        fixture.path("gate"),
        &program(&tools.ffmpeg),
        &program(&tools.ffprobe),
    );
    gate.hold(HOLD_A);
    let mut drip = Drip::start(&tree);
    let daemon = Daemon::start(&tree, Some(&gate));
    let mut client = daemon.loopback().await;

    // After the drip ends, A renders. The first of its renders that runs is suspended, then
    // resumed, and its status follows each control. The gate holds that render until the controls
    // are done, so it cannot end before them, and A's other variants wait behind it.
    let drip_status = drip.finish().await;
    assert!(
        drip_status.success(),
        "sandbox.py drip failed: {drip_status:?}"
    );
    let (first_key, first_id) = settle(
        &mut client,
        "a render of A is running",
        POLL,
        &HIDDEN_BEFORE_UPDATE,
        |s| {
            VARIANTS.iter().find_map(|key| {
                row(s, SEQ_A, key)
                    .filter(|r| is(r, pb::VariantStatus::VariantRunning) && !r.render_id.is_empty())
                    .map(|r| (key.to_string(), r.render_id))
            })
        },
    )
    .await;
    client
        .suspend_render(pb::RenderRef {
            render_id: first_id.clone(),
        })
        .await
        .expect("SuspendRender stops the running render");
    // While the first render is suspended nothing of A can finish before the check below, so the
    // StopRender target is chosen in the same snapshot: any other variant of A that is not done.
    let (stop_key, stop_id) = settle(
        &mut client,
        "A's render shows as suspended and another variant of A is waiting",
        POLL,
        &HIDDEN_BEFORE_UPDATE,
        |s| {
            row(s, SEQ_A, &first_key).filter(|r| is(r, pb::VariantStatus::VariantSuspended))?;
            VARIANTS
                .iter()
                .filter(|key| **key != first_key)
                .find_map(|key| {
                    row(s, SEQ_A, key)
                        .filter(|r| {
                            (is(r, pb::VariantStatus::VariantQueued)
                                || is(r, pb::VariantStatus::VariantRunning))
                                && !r.render_id.is_empty()
                        })
                        .map(|r| (key.to_string(), r.render_id))
                })
        },
    )
    .await;
    // StopRender ends another variant of A: it is paused in config.json. RetryVariant clears the
    // pause and renders it.
    client
        .stop_render(pb::RenderRef { render_id: stop_id })
        .await
        .expect("StopRender ends a waiting render");
    assert!(
        paused_in(&tree).contains(&(SEQ_A.to_owned(), stop_key.clone())),
        "StopRender pauses {stop_key} in config.json: {:?}",
        paused_in(&tree)
    );
    client
        .retry_variant(pb::VariantRef {
            sequence_name: SEQ_A.to_owned(),
            variant_key: stop_key.clone(),
        })
        .await
        .expect("RetryVariant renders the paused variant");
    assert!(
        !paused_in(&tree).contains(&(SEQ_A.to_owned(), stop_key.clone())),
        "RetryVariant clears the pause of {stop_key}"
    );
    client
        .resume_render(pb::RenderRef {
            render_id: first_id,
        })
        .await
        .expect("ResumeRender continues the suspended render");
    settle(
        &mut client,
        "A's render runs again",
        POLL,
        &HIDDEN_BEFORE_UPDATE,
        |s| {
            (!row(s, SEQ_A, &first_key)
                .is_some_and(|r| is(&r, pb::VariantStatus::VariantSuspended)))
            .then_some(())
        },
    )
    .await;
    gate.release(HOLD_A);

    settle(
        &mut client,
        "A has all three renders",
        POLL,
        &HIDDEN_BEFORE_UPDATE,
        |s| {
            VARIANTS
                .iter()
                .all(|key| rendered(s, SEQ_A, key).is_some())
                .then_some(())
        },
    )
    .await;

    // B renders all three variants, C renders all 24 frames, F renders from its backup and G
    // renders, and G's export folder is deleted after its renders. D and E never show.
    settle(
        &mut client,
        "B, C, F and G settle",
        POLL,
        &HIDDEN_BEFORE_UPDATE,
        |s| {
            let ready = VARIANTS.iter().all(|key| {
                rendered(s, SEQ_B, key).is_some()
                    && rendered(s, SEQ_C, key).is_some()
                    && rendered(s, SEQ_F, key).is_some()
                    && rendered(s, SEQ_G, key).is_some()
            }) && !tree.join(SEQ_G).exists();
            ready.then_some(())
        },
    )
    .await;
    let snapshot = snapshot_of(&mut client).await;
    check_sequence_b(&tools, &snapshot);
    check_sequence_f(&snapshot);
    check_sequence_c(&mut client, &snapshot).await;
    check_history(&mut client).await;

    // UpdateConfig adds a variant. Renders of it start at once, and the gate holds them, so a render
    // of it is suspended while it runs and the daemon is killed. A new daemon with the same flags and
    // data folder adopts that suspended render from its own database, shows it still suspended, and
    // ResumeRender and the gate's release finish it.
    gate.hold(HOLD_ADDED);
    let state = client
        .get_config(pb::GetConfigRequest {})
        .await
        .expect("GetConfig answers")
        .into_inner();
    let mut config = state.config.clone().expect("the config has its settings");
    config.rendering.push(pb::RenderVariant {
        codec: "h264".to_owned(),
        frame_rate: 30,
        resolution: vec![3840],
        variant_key: String::new(),
        example_filename: String::new(),
    });
    client
        .update_config(pb::UpdateConfigRequest {
            config: Some(config),
            expected_revision: state.revision.clone(),
        })
        .await
        .expect("UpdateConfig accepts the added variant");

    let (run_id, run_sequence) = settle(
        &mut client,
        "a render of the added variant is running",
        POLL,
        &HIDDEN_AFTER_UPDATE,
        |s| {
            s.in_progress.iter().find_map(|card| {
                card.variants
                    .iter()
                    .find(|r| {
                        r.variant_key == ADDED_KEY
                            && is(r, pb::VariantStatus::VariantRunning)
                            && !r.render_id.is_empty()
                    })
                    .map(|r| (r.render_id.clone(), card.name.clone()))
            })
        },
    )
    .await;
    client
        .suspend_render(pb::RenderRef {
            render_id: run_id.clone(),
        })
        .await
        .expect("SuspendRender stops the render before the crash");
    drop(client);
    daemon.crash();

    let daemon = Daemon::start(&tree, Some(&gate));
    let mut client = daemon.loopback().await;
    settle(
        &mut client,
        "the suspended render is adopted",
        POLL,
        &HIDDEN_AFTER_UPDATE,
        |s| {
            row_by_id(s, &run_id)
                .filter(|r| is(r, pb::VariantStatus::VariantSuspended))
                .map(|_| ())
        },
    )
    .await;
    client
        .resume_render(pb::RenderRef {
            render_id: run_id.clone(),
        })
        .await
        .expect("ResumeRender finishes the adopted render");
    gate.release(HOLD_ADDED);
    settle(
        &mut client,
        "the adopted render is finished",
        POLL,
        &HIDDEN_AFTER_UPDATE,
        |s| rendered(s, &run_sequence, ADDED_KEY).map(|_| ()),
    )
    .await;
    settle(
        &mut client,
        "every sequence has the added variant",
        POLL,
        &HIDDEN_AFTER_UPDATE,
        |s| {
            [SEQ_B, SEQ_F, SEQ_A]
                .iter()
                .all(|seq| rendered(s, seq, ADDED_KEY).is_some())
                .then_some(())
        },
    )
    .await;

    // Pairing over the LAN listener: the pinned fingerprint is the certificate's, a code pairs,
    // and WatchSnapshots answers with the token.
    let issued = client
        .start_pairing(pb::StartPairingRequest {})
        .await
        .expect("loopback starts pairing")
        .into_inner();
    let pem = fs::read_to_string(tree.join("data/identity/cert.pem")).expect("the LAN certificate");
    assert_eq!(
        fingerprint_of_pem(&pem),
        issued.cert_fingerprint_sha256,
        "the pinned fingerprint is the certificate's"
    );
    assert_eq!(issued.lan_port, u32::from(daemon.lan_port));
    let mut lan = daemon.lan(&pem).await;
    let paired = lan
        .pair(pb::PairRequest {
            code: issued.code.clone(),
            device_name: "E2E".to_owned(),
            platform: pb::DevicePlatform::Desktop as i32,
        })
        .await
        .expect("the correct code pairs")
        .into_inner();
    let mut stream = lan
        .watch_snapshots(with_token(
            pb::WatchSnapshotsRequest { max_per_second: 0 },
            &paired.token,
        ))
        .await
        .expect("WatchSnapshots with the token")
        .into_inner();
    let first = stream
        .message()
        .await
        .expect("the stream answers")
        .expect("the stream sends a snapshot");
    assert!(
        !first.recently_rendered.is_empty(),
        "the first LAN snapshot lists the renders"
    );
    drop(stream);
    drop(lan);
}

/// A foreign render (a fake ffmpeg the test starts itself, with a render's arguments) is never
/// adopted by a `--sandbox` daemon, so it never shows in a snapshot (§6.11, §12, A7).
#[tokio::test(flavor = "multi_thread")]
async fn e2e_ignores_foreign_renders() {
    if !e2e_enabled() {
        return;
    }
    let _tools = real_tools();
    let fixture = Fixture::new();
    let tree = fixture.build_tree();

    // The test's own stand-in ffmpeg: named ffmpeg, with the arguments of a render of sequence B. It
    // waits for a file that the test never creates, so it is still running when the daemon scans.
    let stand_in = fixture.path("foreign-tools");
    make_tools_dir(&fake_bin_dir(), &stand_in).expect("the stand-in tools");
    let never_created = fixture.path("never-created");
    let behaviour = serde_json::json!({ "wait_for_file": utf8(&never_created) }).to_string();
    let mut foreign =
        Command::new(stand_in.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX)))
            .args(["-nostdin", "-hide_banner", "-nostats", "-y", "-progress"])
            .arg(fixture.path("foreign-progress.txt"))
            .args(["-r", "30", "-i"])
            .arg(tree.join("export").join(SEQ_B).join("LRT_%05d.tif"))
            .arg(
                fixture
                    .path("foreign")
                    .join("2026_01_01-sandbox-b_foreign.mov"),
            )
            .env("TRD_FAKE_FFMPEG", behaviour)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the stand-in ffmpeg starts");

    let daemon = Daemon::start(&tree, None);
    let mut client = daemon.loopback().await;

    // Two scans after the stand-in started, so the daemon has looked at it at least twice.
    let mut scans: Vec<i64> = Vec::new();
    settle(&mut client, "the daemon scans twice", POLL, &[], |s| {
        if s.last_scan_completed_ms > 0 && !scans.contains(&s.last_scan_completed_ms) {
            scans.push(s.last_scan_completed_ms);
        }
        (scans.len() >= 2).then_some(())
    })
    .await;

    let snapshot = snapshot_of(&mut client).await;
    assert!(
        !format!("{snapshot:?}").contains("_foreign.mov"),
        "the snapshot never lists the foreign render: {snapshot:?}"
    );
    assert!(
        snapshot
            .in_progress
            .iter()
            .flat_map(|card| card.variants.iter())
            .all(|variant| !variant.adopted),
        "the daemon adopted nothing: {snapshot:?}"
    );
    assert!(
        matches!(foreign.try_wait(), Ok(None)),
        "the stand-in ffmpeg is still running when the scans are checked"
    );
    let _ = foreign.kill();
    let _ = foreign.wait();
}

/// The gate of the real-tool test holds only a render whose arguments name a held pattern, and
/// after the release runs its target with the arguments unchanged. `echo` stands in for ffmpeg, so
/// this runs without the real tools and without a daemon.
#[cfg(unix)]
#[test]
fn e2e_gate_holds_render_until_released() {
    let fixture = Fixture::new();
    let echo = OsStr::new("/bin/echo");
    let gate = Gate::new(fixture.path("gate"), echo, echo);
    gate.hold("held-name");
    let ffmpeg = |args: &[&str]| {
        let mut command = Command::new(gate.root.join("tools").join("ffmpeg"));
        gate.apply(&mut command);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        command
    };
    let marker = gate.root.join("held").join("held-name");

    // Not a render, and a render that names no held pattern: both pass straight through.
    for args in [["-i", "held-name.mov"], ["-progress", "other-name.mov"]] {
        let output = ffmpeg(&args).output().expect("the gate runs");
        assert!(output.status.success(), "{args:?}: {:?}", output.status);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n", args.join(" "))
        );
    }
    assert!(!marker.exists(), "neither call was held");

    let args = ["-progress", "progress.txt", "two words/held-name.mov"];
    let mut held = ffmpeg(&args).spawn().expect("the gate runs");
    let started = Instant::now();
    while !marker.exists() {
        assert!(
            started.elapsed() < HANG_GUARD,
            "the render was not held within {HANG_GUARD:?}"
        );
        std::thread::sleep(POLL);
    }
    assert!(
        matches!(held.try_wait(), Ok(None)),
        "the held render is still running"
    );
    gate.release("held-name");
    let output = held.wait_with_output().expect("the released render ends");
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{}\n", args.join(" "))
    );
}
