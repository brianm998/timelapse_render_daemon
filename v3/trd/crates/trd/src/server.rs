// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The trd.v1 gRPC server on loopback and LAN (§7).
//!
//! [`Server`] implements every `TrdService` method over [`Shared`], the state the daemon wires
//! together in `daemon.rs`: the engine handle, the store, the thumbnailer, the tools, and the
//! config file with its codec table. Each request carries a [`Listener`] extension that the
//! listener's interceptor sets, so the device RPCs can refuse anything that did not arrive on the
//! loopback listener.
//!
//! Errors are gRPC statuses named by §7: `NOT_FOUND` for unknown ids, `FAILED_PRECONDITION` for a
//! stale revision or a control that does not apply, `INVALID_ARGUMENT` for a config with ERROR
//! issues, `PERMISSION_DENIED` for a device call on the LAN, and `UNAVAILABLE` once the daemon is
//! shutting down.

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use trd_core::codecs::{self, CodecStore, VideoCodecs};
use trd_core::config::{
    self as core_config, Config as CoreConfig, ConfigError, ConfigStore, MaxProcesses,
    PausedVariant, RenderType, Status as ConfigLoadStatus,
};
use trd_core::controls::ControlError;
use trd_core::diagnosis::FailureKind as DiagnosedKind;
use trd_core::dirs::Os;
use trd_core::engine::{Control, EngineHandle};
use trd_core::paths::{Clock, SystemClock};
use trd_core::problems::{Problem, Severity as ProblemSeverity};
use trd_core::snapshot::{
    Phase as SnapPhase, SequenceCard, Snapshot, Stage as SnapStage, VariantRow,
    VariantStatus as SnapVariantStatus,
};
use trd_core::store::{
    Device, Failure, JobStatus, LogFields, LogLevel, LogRow, RenderJob, RenderedVideo, Store,
    log_event,
};
use trd_core::thumbs::{Thumbnailer, ensure_thumbnail};
use trd_core::tools::{ToolProblem, Tools};
use trd_proto::trd::v1 as pb;
use trd_proto::trd::v1::trd_service_server::TrdService;

use crate::auth::{Caller, Revocations, UntilRevoked, authorize, revocation_of, until_revoked};
use crate::pairing::{Pairing, Refusal, new_code, new_device};
use crate::tls::Identity;

/// The tools as the daemon last resolved them. The engine writes them, the thumbnailer and the
/// server read them.
pub type SharedTools = std::sync::Arc<Mutex<Tools>>;

/// The listener a request came through. The interceptor of each listener sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listener {
    Loopback,
    Lan,
}

/// Where the config file's path came from (§6.1, `ConfigState.path_source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    Flag,
    Setting,
    Default,
}

/// The config file as the server reads and edits it: its store, the codec table beside it, and
/// where the path came from. Locked only for short synchronous work, never across an await.
pub struct ConfigSlot {
    store: ConfigStore,
    codecs: CodecStore,
    source: PathSource,
    explicit_codecs: Option<PathBuf>,
}

impl ConfigSlot {
    /// The slot for the config at `path`, with the codec table resolved as the daemon resolves it.
    pub fn open(path: PathBuf, source: PathSource, explicit_codecs: Option<PathBuf>) -> ConfigSlot {
        ConfigSlot {
            store: ConfigStore::new(path.clone(), std::sync::Arc::new(SystemClock)),
            codecs: CodecStore::open(explicit_codecs.clone(), path),
            source,
            explicit_codecs,
        }
    }

    /// The codec table in use, as the engine starts with it.
    pub fn codec_table(&self) -> VideoCodecs {
        self.codecs.codecs().clone()
    }

    /// Re-reads the config and the codec table when their files changed.
    fn reload(&mut self) {
        self.codecs.reload_if_changed();
        let ConfigSlot { store, codecs, .. } = self;
        store.reload_if_changed(codecs.codecs());
    }
}

/// Everything the gRPC methods read and write.
pub struct Shared {
    pub handle: EngineHandle,
    pub store: Store,
    pub thumbs: Thumbnailer,
    pub tools: SharedTools,
    pub config: Mutex<ConfigSlot>,
    /// Serialises `SetConfigPath`, so the server and the engine end on the same path.
    pub config_switch: tokio::sync::Mutex<()>,
    pub server_id: String,
    pub server_name: String,
    pub sandbox: bool,
    pub shutdown: watch::Receiver<bool>,
    pub home: PathBuf,
    pub os: Os,
    /// The LAN listener's certificate, when `--lan-bind` is not `off` (§8.1).
    pub identity: Option<Identity>,
    /// The non-loopback IPv4 addresses `StartPairing` reports (§8.3).
    pub lan_addresses: Vec<String>,
    /// The port the LAN listener is bound to; 0 while it is off or not bound (§7 `ServerInfo`).
    pub lan_port: AtomicU16,
    /// The one pairing code that can be valid (§8.3).
    pub pairing: Mutex<Pairing>,
    /// The revocation signal of each device, for the streams it has open (§8.3).
    pub revocations: Revocations,
    /// The problems the LAN listener raises, read by the engine at each snapshot (§8.2).
    pub external_problems: Arc<Mutex<Vec<Problem>>>,
}

impl Shared {
    fn lock_config(&self) -> MutexGuard<'_, ConfigSlot> {
        self.config.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn log(&self, level: LogLevel, subject: &str, message: &str, fields: &LogFields) {
        if let Err(err) = log_event(&self.store, now_ms(), level, subject, message, fields) {
            tracing::warn!(error = %err, "a log row could not be written");
        }
    }

    /// The proto snapshot, with each failure card filled from its stored row.
    fn snapshot_message(&self, snap: &Snapshot) -> pb::Snapshot {
        let failures: HashMap<String, Failure> = self
            .store
            .failures()
            .map(|rows| rows.into_iter().map(|row| (row.id.clone(), row)).collect())
            .unwrap_or_default();
        snapshot_to_proto(snap, &failures)
    }
}

/// The gRPC service. Cheap to clone: it holds the shared state.
#[derive(Clone)]
pub struct Server {
    shared: std::sync::Arc<Shared>,
}

impl Server {
    pub fn new(shared: std::sync::Arc<Shared>) -> Server {
        Server { shared }
    }
}

/// The interceptor of the loopback listener: every request is marked as loopback.
pub fn mark_loopback(mut request: Request<()>) -> Result<Request<()>, Status> {
    request.extensions_mut().insert(Listener::Loopback);
    Ok(request)
}

/// The listener of a request. A request without a marker counts as LAN, the stricter answer.
fn listener_of<T>(request: &Request<T>) -> Listener {
    request
        .extensions()
        .get::<Listener>()
        .copied()
        .unwrap_or(Listener::Lan)
}

/// `PERMISSION_DENIED` unless the request came through the loopback listener (§7).
fn require_loopback<T>(request: &Request<T>) -> Result<(), Status> {
    match listener_of(request) {
        Listener::Loopback => Ok(()),
        Listener::Lan => Err(Status::permission_denied(
            "device calls are only served on the loopback listener",
        )),
    }
}

/// The certificate fingerprint the LAN listener serves, or nothing while it is off.
fn fingerprint(shared: &Shared) -> String {
    shared
        .identity
        .as_ref()
        .map(|identity| identity.fingerprint.clone())
        .unwrap_or_default()
}

/// The status of a refused `Pair` (§8.3).
fn refusal_status(refusal: Refusal) -> Status {
    match refusal {
        Refusal::NoCode => Status::unauthenticated(
            "there is no valid pairing code; StartPairing issues one on loopback",
        ),
        Refusal::Wrong => Status::unauthenticated("the pairing code is wrong"),
        Refusal::TooManyWrong => Status::resource_exhausted(
            "too many wrong codes; the code is invalid and StartPairing issues a new one",
        ),
    }
}

fn shutting_down() -> Status {
    Status::unavailable("trd is shutting down")
}

/// Ends when the daemon starts shutting down (or its sender is gone).
async fn wait_shutdown(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stopped| *stopped).await;
}

/// How long a stream stopped by shutdown waits to deliver its UNAVAILABLE. A client that does not
/// read must not hold the daemon's shutdown, so the wait is bounded.
const SHUTDOWN_NOTICE_WAIT: Duration = Duration::from_secs(1);

/// Sends `item` unless shutdown starts first. False when the stream should end: shutdown started,
/// or the client has gone.
async fn send_unless_stopped<T>(
    tx: &mpsc::Sender<Result<T, Status>>,
    item: Result<T, Status>,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        sent = tx.send(item) => sent.is_ok(),
        () = wait_shutdown(shutdown) => false,
    }
}

/// Ends a stream that shutdown stopped with UNAVAILABLE. Does nothing when shutdown has not started.
async fn end_for_shutdown<T>(
    tx: &mpsc::Sender<Result<T, Status>>,
    shutdown: &watch::Receiver<bool>,
) {
    if *shutdown.borrow() {
        let _ = tokio::time::timeout(SHUTDOWN_NOTICE_WAIT, tx.send(Err(shutting_down()))).await;
    }
}

fn control_status(err: ControlError) -> Status {
    match err {
        ControlError::NotFound(message) => Status::not_found(message),
        ControlError::FailedPrecondition(message) => Status::failed_precondition(message),
        ControlError::Internal(message) => Status::internal(message),
    }
}

fn internal(err: impl std::fmt::Display) -> Status {
    Status::internal(err.to_string())
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn u32_of(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

// ── mapping: snapshot ────────────────────────────────────────────────────────

fn phase_to_proto(phase: SnapPhase) -> pb::Phase {
    match phase {
        SnapPhase::Starting => pb::Phase::Starting,
        SnapPhase::NoConfig => pb::Phase::NoConfig,
        SnapPhase::WaitingForDiskAccess => pb::Phase::WaitingForDiskAccess,
        SnapPhase::Idle => pb::Phase::Idle,
        SnapPhase::Exporting => pb::Phase::Exporting,
        SnapPhase::Rendering => pb::Phase::Rendering,
        SnapPhase::Paused => pb::Phase::Paused,
    }
}

fn stage_to_proto(stage: SnapStage) -> pb::Stage {
    match stage {
        SnapStage::Exporting => pb::Stage::Exporting,
        SnapStage::CountMismatch => pb::Stage::CountMismatch,
        SnapStage::CollectingMetadata => pb::Stage::CollectingMetadata,
        SnapStage::Queued => pb::Stage::Queued,
        SnapStage::Rendering => pb::Stage::Rendering,
        SnapStage::Blocked => pb::Stage::Blocked,
        SnapStage::Paused => pb::Stage::Paused,
    }
}

fn variant_status_to_proto(status: SnapVariantStatus) -> pb::VariantStatus {
    match status {
        SnapVariantStatus::Waiting => pb::VariantStatus::VariantWaiting,
        SnapVariantStatus::AlreadyRendered => pb::VariantStatus::VariantAlreadyRendered,
        SnapVariantStatus::Queued => pb::VariantStatus::VariantQueued,
        SnapVariantStatus::Running => pb::VariantStatus::VariantRunning,
        SnapVariantStatus::Suspended => pb::VariantStatus::VariantSuspended,
        SnapVariantStatus::Done => pb::VariantStatus::VariantDone,
        SnapVariantStatus::Failed => pb::VariantStatus::VariantFailed,
        SnapVariantStatus::Paused => pb::VariantStatus::VariantPaused,
        SnapVariantStatus::UnknownCodec => pb::VariantStatus::VariantUnknownCodec,
    }
}

/// The label of a failure kind, by its proto name (the labels of `diagnosis`).
fn kind_label(code: &str) -> &'static str {
    const KINDS: [DiagnosedKind; 12] = [
        DiagnosedKind::InputFramesNotFound,
        DiagnosedKind::InputUnreadable,
        DiagnosedKind::SourceFrameUnreadable,
        DiagnosedKind::FfmpegRejectedArguments,
        DiagnosedKind::OutputNotWritable,
        DiagnosedKind::DiskFull,
        DiagnosedKind::TerminatedBySignal,
        DiagnosedKind::ProducedNoOutput,
        DiagnosedKind::StructuralCorruption,
        DiagnosedKind::LaunchFailed,
        DiagnosedKind::NotQueueable,
        DiagnosedKind::Unknown,
    ];
    KINDS
        .into_iter()
        .find(|kind| kind.code() == code)
        .map_or("render failed", DiagnosedKind::label)
}

fn failure_kind_to_proto(code: &str) -> pb::FailureKind {
    pb::FailureKind::from_str_name(code).unwrap_or(pb::FailureKind::Unspecified)
}

fn problem_to_proto(problem: &trd_core::problems::Problem) -> pb::Problem {
    pb::Problem {
        id: problem.id.clone(),
        severity: match problem.severity {
            ProblemSeverity::Warning => pb::Severity::Warning,
            ProblemSeverity::Error => pb::Severity::Error,
        } as i32,
        kind: pb::ProblemKind::from_str_name(problem.kind).unwrap_or(pb::ProblemKind::Unspecified)
            as i32,
        title: problem.title.clone(),
        detail: problem.detail.clone(),
        fix: problem.fix.clone(),
        paths: problem.paths.clone(),
        since_ms: problem.since_ms,
    }
}

fn failure_to_proto(
    card: &trd_core::snapshot::FailureCard,
    row: Option<&Failure>,
) -> pb::FailureCard {
    let tool_output: Vec<String> = row
        .and_then(|row| serde_json::from_str::<Vec<String>>(&row.tool_output_json).ok())
        .unwrap_or_default();
    let tail_start = tool_output.len().saturating_sub(20);
    pb::FailureCard {
        id: card.id.clone(),
        first_seen_ms: card.first_seen_ms,
        last_seen_ms: card.last_seen_ms,
        sequence_name: card.sequence_name.clone(),
        export_path: row.map(|r| r.export_path.clone()).unwrap_or_default(),
        output_filename: card.output_filename.clone(),
        output_path: card.output_path.clone(),
        variant_key: card.variant_key.clone(),
        kind: failure_kind_to_proto(&card.kind) as i32,
        kind_label: kind_label(&card.kind).to_owned(),
        summary: card.summary.clone(),
        detail: card.detail.clone(),
        has_exit_code: row.is_some_and(|r| r.exit_code.is_some()),
        exit_code: row.and_then(|r| r.exit_code).unwrap_or(0) as i32,
        command_line: row.map(|r| r.command_line.clone()).unwrap_or_default(),
        tool_output: tool_output[tail_start..].to_vec(),
        occurrences: u32_of(card.occurrences),
        variant_auto_paused: row.is_some_and(|r| r.variant_auto_paused),
        render_id: row.and_then(|r| r.render_id.clone()).unwrap_or_default(),
    }
}

fn variant_to_proto(row: &VariantRow) -> pb::VariantRow {
    pb::VariantRow {
        variant_key: row.variant_key.clone(),
        codec: row.codec.clone(),
        resolution_label: row.resolution_label.clone(),
        frame_rate: row.frame_rate,
        output_filename: row.output_filename.clone(),
        output_path: row.output_path.clone(),
        status: variant_status_to_proto(row.status) as i32,
        render_id: row.render_id.clone(),
        frame: u32_of(row.frame as i64),
        total_frames: u32_of(row.total_frames as i64),
        fps: row.fps,
        eta_seconds: row.eta_seconds,
        progress_known: row.progress_known,
        adopted: row.adopted,
        size_bytes: row.size_bytes,
        message: row.message.clone(),
        failure_id: row.failure_id.clone(),
    }
}

fn card_to_proto(card: &SequenceCard) -> pb::SequenceCard {
    pb::SequenceCard {
        id: card.id.clone(),
        name: card.name.clone(),
        export_path: card.export_path.clone(),
        raw_sequence_name: card.raw_sequence_name.clone(),
        raw_sequence_location: card.raw_sequence_location.clone(),
        raw_from_backup: card.raw_from_backup,
        expected_frames: card.expected_frames,
        exported_frames: card.exported_frames,
        stage: stage_to_proto(card.stage) as i32,
        paused: card.paused,
        export_frames_per_minute: card.export_frames_per_minute,
        export_eta_seconds: card.export_eta_seconds,
        export_size_bytes: card.export_size_bytes,
        first_seen_ms: card.first_seen_ms,
        last_frame_ms: card.last_frame_ms,
        variants: card.variants.iter().map(variant_to_proto).collect(),
        note: card.note.clone(),
    }
}

fn source_to_proto(source: &str) -> pb::RenderedSource {
    match source {
        "TRD" => pb::RenderedSource::Trd,
        "ADOPTED" => pb::RenderedSource::Adopted,
        "IMPORTED" => pb::RenderedSource::Imported,
        _ => pb::RenderedSource::Unspecified,
    }
}

/// A rendered video row as the API shows it. `file_exists` is the caller's answer.
fn video_to_proto(row: &RenderedVideo, file_exists: bool) -> pb::RenderedVideo {
    pb::RenderedVideo {
        id: row.id.clone(),
        rendered_at_ms: row.rendered_at,
        sequence_name: row.sequence_name.clone(),
        export_path: row.export_path.clone(),
        output_filename: row.output_filename.clone(),
        output_path: row.output_path.clone(),
        variant_key: row.variant_key.clone(),
        codec: row.codec.clone(),
        resolution_label: row.resolution_label.clone(),
        frame_rate: u32_of(row.frame_rate),
        frame_count: u32_of(row.frame_count),
        size_bytes: row.size_bytes,
        width: u32_of(row.width),
        height: u32_of(row.height),
        duration_seconds: row.duration_seconds,
        render_seconds: row.render_seconds as i64,
        has_thumbnail: row.thumbnail_path.is_some(),
        file_exists,
        xmp_embedded: row.xmp_embedded,
        source: source_to_proto(&row.source) as i32,
    }
}

/// A snapshot as the API shows it. Failure cards take their extra fields from `failures`.
pub fn snapshot_to_proto(snap: &Snapshot, failures: &HashMap<String, Failure>) -> pb::Snapshot {
    pb::Snapshot {
        revision: snap.revision,
        generated_at_ms: snap.generated_at_ms,
        phase: phase_to_proto(snap.phase) as i32,
        global_pause: snap.global_pause,
        problems: snap.problems.iter().map(problem_to_proto).collect(),
        failures: snap
            .failures
            .iter()
            .map(|card| failure_to_proto(card, failures.get(&card.id)))
            .collect(),
        in_progress: snap.in_progress.iter().map(card_to_proto).collect(),
        recently_rendered: snap
            .recently_rendered
            .iter()
            .map(|video| pb::RenderedVideo {
                id: video.id.clone(),
                rendered_at_ms: video.rendered_at_ms,
                sequence_name: video.sequence_name.clone(),
                export_path: video.export_path.clone(),
                output_filename: video.output_filename.clone(),
                output_path: video.output_path.clone(),
                variant_key: video.variant_key.clone(),
                codec: video.codec.clone(),
                resolution_label: video.resolution_label.clone(),
                frame_rate: u32_of(video.frame_rate),
                frame_count: u32_of(video.frame_count),
                size_bytes: video.size_bytes,
                width: u32_of(video.width),
                height: u32_of(video.height),
                duration_seconds: video.duration_seconds,
                render_seconds: video.render_seconds,
                has_thumbnail: video.has_thumbnail,
                file_exists: video.file_exists,
                xmp_embedded: video.xmp_embedded,
                source: source_to_proto(&video.source) as i32,
            })
            .collect(),
        recently_rendered_total: snap.recently_rendered_total,
        counts: Some(pb::Counts {
            rendering: snap.counts.rendering,
            suspended: snap.counts.suspended,
            queued: snap.counts.queued,
            exporting: snap.counts.exporting,
            paused_sequences: snap.counts.paused_sequences,
            failures: snap.counts.failures,
            rendered_last_24h: snap.counts.rendered_last_24h,
        }),
        last_scan_completed_ms: snap.last_scan_completed_ms,
        last_scan_duration_ms: snap.last_scan_duration_ms,
        config_path: snap.config_path.clone(),
    }
}

// ── mapping: store rows ──────────────────────────────────────────────────────

fn platform_to_proto(platform: &str) -> pb::DevicePlatform {
    match platform {
        "ios" => pb::DevicePlatform::Ios,
        "android" => pb::DevicePlatform::Android,
        "desktop" => pb::DevicePlatform::Desktop,
        _ => pb::DevicePlatform::Unspecified,
    }
}

fn device_to_proto(device: &Device) -> pb::PairedDevice {
    pb::PairedDevice {
        device_id: device.id.clone(),
        name: device.name.clone(),
        platform: platform_to_proto(&device.platform) as i32,
        paired_at_ms: device.paired_at,
        last_seen_ms: device.last_seen.unwrap_or(0),
    }
}

fn log_level_to_proto(level: LogLevel) -> pb::LogLevel {
    match level {
        LogLevel::Debug => pb::LogLevel::LogDebug,
        LogLevel::Info => pb::LogLevel::LogInfo,
        LogLevel::Warn => pb::LogLevel::LogWarn,
        LogLevel::Error => pb::LogLevel::LogError,
    }
}

/// The level a request names. UNSPECIFIED means INFO (§7).
fn min_level_of(value: i32) -> LogLevel {
    match pb::LogLevel::try_from(value) {
        Ok(pb::LogLevel::LogDebug) => LogLevel::Debug,
        Ok(pb::LogLevel::LogWarn) => LogLevel::Warn,
        Ok(pb::LogLevel::LogError) => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

fn event_to_proto(event: Option<&str>) -> pb::EventKind {
    match event {
        Some("EVENT_SEQUENCE_READY") => pb::EventKind::EventSequenceReady,
        Some("EVENT_RENDER_STARTED") => pb::EventKind::EventRenderStarted,
        Some("EVENT_RENDER_FINISHED") => pb::EventKind::EventRenderFinished,
        Some("EVENT_SEQUENCE_FINISHED") => pb::EventKind::EventSequenceFinished,
        Some("EVENT_RENDER_FAILED") => pb::EventKind::EventRenderFailed,
        Some("EVENT_SEQUENCE_DELETED") => pb::EventKind::EventSequenceDeleted,
        Some("EVENT_CONFIG_RELOADED") => pb::EventKind::EventConfigReloaded,
        _ => pb::EventKind::EventNone,
    }
}

fn log_to_proto(row: &LogRow) -> pb::LogEntry {
    pb::LogEntry {
        seq: u64::try_from(row.seq).unwrap_or(0),
        at_ms: row.at,
        level: log_level_to_proto(row.level) as i32,
        subject: row.subject.clone(),
        message: row.message.clone(),
        event: event_to_proto(row.fields.event.as_deref()) as i32,
        sequence_name: row.fields.sequence_name.clone().unwrap_or_default(),
        output_filename: row.fields.output_filename.clone().unwrap_or_default(),
        rendered_video_id: row.fields.rendered_video_id.clone().unwrap_or_default(),
        failure_id: row.fields.failure_id.clone().unwrap_or_default(),
        video_count: row.fields.video_count.map_or(0, u32_of),
        total_bytes: row.fields.total_bytes.unwrap_or(0),
    }
}

fn job_status_to_proto(status: JobStatus) -> pb::VariantStatus {
    match status {
        JobStatus::Queued => pb::VariantStatus::VariantQueued,
        JobStatus::Running => pb::VariantStatus::VariantRunning,
        JobStatus::Suspended => pb::VariantStatus::VariantSuspended,
        JobStatus::Done => pb::VariantStatus::VariantDone,
        JobStatus::Failed => pb::VariantStatus::VariantFailed,
        JobStatus::Cancelled => pb::VariantStatus::Unspecified,
    }
}

/// The render's plan, when its `plan_json` parses.
fn plan_of(job: &RenderJob) -> Option<trd_core::plan::Plan> {
    serde_json::from_str(&job.plan_json).ok()
}

/// The last `lines` lines of the file at `path`, or none when it cannot be read.
fn log_tail(path: Option<&str>, lines: usize) -> Vec<String> {
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let all: Vec<String> = text.lines().map(str::to_owned).collect();
    all[all.len().saturating_sub(lines)..].to_vec()
}

fn render_details(job: &RenderJob, command_line: Option<&str>) -> pb::RenderDetails {
    let plan = plan_of(job);
    let running = matches!(job.status, JobStatus::Running | JobStatus::Suspended);
    pb::RenderDetails {
        render_id: job.id.clone(),
        sequence_name: job.sequence_name.clone(),
        export_path: job.export_path.clone(),
        variant_key: job.variant_key.clone(),
        input_pattern: plan
            .as_ref()
            .map(trd_core::plan::Plan::input_pattern)
            .unwrap_or_default(),
        start_number: plan
            .as_ref()
            .and_then(|p| p.start_number)
            .map_or(-1, |n| n as i32),
        total_frames: plan.as_ref().map_or(0, |p| u32_of(p.total_frames)),
        output_path: job.output_path.clone(),
        partial_path: job.partial_path.clone(),
        command_line: command_line.map_or_else(
            || {
                plan.as_ref()
                    .map(|p| p.argv().join(" "))
                    .unwrap_or_default()
            },
            str::to_owned,
        ),
        log_tail: log_tail(job.log_path.as_deref(), 200),
        started_at_ms: job.started_at.unwrap_or(0),
        finished_at_ms: job.finished_at.unwrap_or(0),
        pid: if running {
            job.pid.map_or(0, |pid| pid as i32)
        } else {
            0
        },
        adopted: job.adopted,
        status: job_status_to_proto(job.status) as i32,
    }
}

// ── mapping: config ──────────────────────────────────────────────────────────

fn issue_to_proto(issue: &core_config::ConfigIssue) -> pb::ConfigIssue {
    pb::ConfigIssue {
        severity: match issue.severity {
            core_config::Severity::Error => pb::IssueSeverity::IssueError,
            core_config::Severity::Warning => pb::IssueSeverity::IssueWarning,
        } as i32,
        field: issue.field.clone(),
        message: issue.message.clone(),
    }
}

fn render_to_proto(render: &RenderType) -> pb::RenderVariant {
    pb::RenderVariant {
        codec: render.codec.clone(),
        frame_rate: u32_of(render.frame_rate),
        resolution: render
            .resolution
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(u32_of)
            .collect(),
        variant_key: render.variant_key(),
        example_filename: String::new(),
    }
}

fn config_to_proto(config: &CoreConfig) -> pb::Config {
    pb::Config {
        export_paths: config.export_paths.clone(),
        source_paths: config.source_paths.clone(),
        backup_paths: config.backup_paths.clone(),
        master_videos: config.master_videos.clone(),
        max_processes: Some(pb::MaxProcesses {
            ffmpeg: u32_of(config.max_processes.ffmpeg),
            exiftool: u32_of(config.max_processes.exiftool),
        }),
        max_concurrent_renders: u32_of(config.max_cuncurrent_renders),
        image_name_prefix: config.image_name_prefix.clone(),
        image_sequence_dirname_prefix: config.image_sequence_dirname_prefix.clone(),
        raw_types: config.raw_types.clone(),
        image_extensions: config.image_extensions.clone(),
        rendering: config.rendering.iter().map(render_to_proto).collect(),
        log_in_color: config.log_in_color,
        sleep_time_seconds: config.sleep_time_seconds,
        scan_interval_seconds: config.scan_interval_seconds,
        progress_bar_width: u32_of(config.progress_bar_width),
        ignore_sequence_regexes: config.ignore_sequence_regexes.clone(),
        delete_sequence_after_render_regexes: config.delete_sequence_after_render_regexes.clone(),
        paused_sequence_names: config.paused_sequence_names.clone(),
        paused_variants: config
            .paused_variants
            .iter()
            .map(|pv| pb::PausedVariant {
                sequence_name: pv.sequence_name.clone(),
                variant_key: pv.variant_key.clone(),
            })
            .collect(),
        recent_render_history_days: u32_of(config.recent_render_history_days),
        render_history_path: config.render_history_path.clone().unwrap_or_default(),
    }
}

/// The core config of a proto config. Fields the proto does not carry keep their defaults.
fn config_from_proto(config: &pb::Config) -> CoreConfig {
    let max = config.max_processes.unwrap_or_default();
    CoreConfig {
        export_paths: config.export_paths.clone(),
        source_paths: config.source_paths.clone(),
        backup_paths: config.backup_paths.clone(),
        master_videos: config.master_videos.clone(),
        max_processes: MaxProcesses {
            ffmpeg: i64::from(max.ffmpeg),
            exiftool: i64::from(max.exiftool),
        },
        max_cuncurrent_renders: i64::from(config.max_concurrent_renders),
        image_name_prefix: config.image_name_prefix.clone(),
        image_sequence_dirname_prefix: config.image_sequence_dirname_prefix.clone(),
        raw_types: config.raw_types.clone(),
        image_extensions: config.image_extensions.clone(),
        rendering: config
            .rendering
            .iter()
            .map(|render| RenderType {
                codec: render.codec.clone(),
                frame_rate: i64::from(render.frame_rate),
                resolution: if render.resolution.is_empty() {
                    None
                } else {
                    Some(render.resolution.iter().map(|v| i64::from(*v)).collect())
                },
            })
            .collect(),
        log_in_color: config.log_in_color,
        sleep_time_seconds: config.sleep_time_seconds,
        scan_interval_seconds: config.scan_interval_seconds,
        progress_bar_width: i64::from(config.progress_bar_width),
        ignore_sequence_regexes: config.ignore_sequence_regexes.clone(),
        delete_sequence_after_render_regexes: config.delete_sequence_after_render_regexes.clone(),
        paused_sequence_names: config.paused_sequence_names.clone(),
        paused_variants: config
            .paused_variants
            .iter()
            .map(|pv| PausedVariant {
                sequence_name: pv.sequence_name.clone(),
                variant_key: pv.variant_key.clone(),
            })
            .collect(),
        recent_render_history_days: i64::from(config.recent_render_history_days),
        render_history_path: Some(config.render_history_path.clone())
            .filter(|path| !path.is_empty()),
        extra: Default::default(),
    }
}

fn codec_to_proto(name: &str, codec: &codecs::VideoCodec) -> pb::VideoCodec {
    pb::VideoCodec {
        name: name.to_owned(),
        ffmpeg: Some(pb::CodecFfmpeg {
            codec: codec.ffmpeg.codec.clone(),
            pixel_format: codec.ffmpeg.pixel_format.clone(),
            color_range: codec.ffmpeg.color_range.clone(),
            has_profile: codec.ffmpeg.profile.is_some(),
            profile: codec.ffmpeg.profile.clone().unwrap_or_default(),
            extra_args: codec.ffmpeg.extra_args.clone().unwrap_or_default(),
        }),
        filename: Some(pb::CodecFilename {
            pixel_format: codec.filename.pixel_format.clone(),
            codec: codec.filename.codec.clone(),
            quality: codec.filename.quality.clone(),
        }),
    }
}

/// The codec table from its proto form. A name is needed and must be unique.
fn codecs_from_proto(list: &[pb::VideoCodec]) -> Result<VideoCodecs, Status> {
    let mut table = VideoCodecs::new();
    for item in list {
        if item.name.is_empty() {
            return Err(Status::invalid_argument("a codec has no name"));
        }
        let ffmpeg = item.ffmpeg.clone().unwrap_or_default();
        let filename = item.filename.clone().unwrap_or_default();
        let codec = codecs::VideoCodec {
            ffmpeg: codecs::FfmpegCodec {
                codec: ffmpeg.codec,
                pixel_format: ffmpeg.pixel_format,
                color_range: ffmpeg.color_range,
                profile: ffmpeg.has_profile.then_some(ffmpeg.profile),
                extra_args: (!ffmpeg.extra_args.is_empty()).then_some(ffmpeg.extra_args),
            },
            filename: codecs::FilenameCodec {
                pixel_format: filename.pixel_format,
                codec: filename.codec,
                quality: filename.quality,
            },
        };
        if table.insert(item.name.clone(), codec).is_some() {
            return Err(Status::invalid_argument(format!(
                "codec {} is listed twice",
                item.name
            )));
        }
    }
    Ok(table)
}

/// The API's view of the config: the config in use, its issues, the codec table and both revisions.
fn config_state(slot: &ConfigSlot, now: i64) -> pb::ConfigState {
    let exists = !matches!(slot.store.status(), ConfigLoadStatus::Missing);
    let path = slot.store.path().to_path_buf();
    let raw_json = if exists {
        fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };
    let codecs_revision = match slot.codecs.path() {
        Some(codecs_path) => fs::read(codecs_path)
            .map(|bytes| core_config::revision_of(&bytes))
            .unwrap_or_default(),
        None => String::new(),
    };
    let default_config = CoreConfig::default();
    let config = slot.store.config().unwrap_or(&default_config);
    pb::ConfigState {
        path: path.to_string_lossy().into_owned(),
        path_source: match slot.source {
            PathSource::Flag => pb::ConfigPathSource::ConfigPathFlag,
            PathSource::Setting => pb::ConfigPathSource::ConfigPathSetting,
            PathSource::Default => pb::ConfigPathSource::ConfigPathDefault,
        } as i32,
        exists,
        config: Some(config_to_proto(config)),
        revision: if exists {
            slot.store.revision().to_owned()
        } else {
            String::new()
        },
        issues: slot.store.issues().iter().map(issue_to_proto).collect(),
        codecs: slot
            .codecs
            .codecs()
            .iter()
            .map(|(name, codec)| codec_to_proto(name, codec))
            .collect(),
        codecs_path: slot
            .codecs
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        codecs_revision,
        raw_json,
        loaded_at_ms: now,
    }
}

/// The revision a file's bytes give, or `""` when there is no file.
fn file_revision(path: &Path) -> String {
    fs::read(path)
        .map(|bytes| core_config::revision_of(&bytes))
        .unwrap_or_default()
}

/// The config with every entry the file has, including those a load drops for an ERROR. The last
/// config that parsed when the file does not parse now.
fn config_as_written(store: &ConfigStore) -> CoreConfig {
    fs::read(store.path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| CoreConfig::from_value(&value).ok())
        .or_else(|| store.config().cloned())
        .unwrap_or_default()
}

/// The ERRORs `config` has with the codec table `next` and not with `current`: a codec a
/// `rendering` entry uses was removed or renamed (§6.2). An ERROR the config already has does not
/// block a codec edit.
fn codec_table_errors(
    config: &CoreConfig,
    current: &VideoCodecs,
    next: &VideoCodecs,
    year: i32,
) -> Vec<core_config::ConfigIssue> {
    let before = core_config::validate(config, current, year);
    core_config::validate(config, next, year)
        .into_iter()
        .filter(|issue| issue.is_error() && !before.contains(issue))
        .collect()
}

/// Maps a config edit's refusal to a status (§7).
fn config_error_status(err: ConfigError) -> Status {
    match err {
        ConfigError::StaleRevision { current } => Status::failed_precondition(format!(
            "config.json changed since it was read (current revision {current})"
        )),
        ConfigError::Unparseable(message) => Status::failed_precondition(format!(
            "config.json cannot be edited until it parses: {message}"
        )),
        ConfigError::Refused(issues) => {
            let messages: Vec<String> = issues
                .iter()
                .map(|issue| format!("{}: {}", issue.field, issue.message))
                .collect();
            Status::invalid_argument(format!("config has errors: {}", messages.join("; ")))
        }
        ConfigError::Io(err) => internal(err),
    }
}

// ── mapping: directories ─────────────────────────────────────────────────────

/// The roots the directory browser starts from, per OS (§7 `ListDirectory`).
fn roots(os: Os, home: &Path) -> Vec<String> {
    let mut out = vec!["/".to_owned()];
    match os {
        Os::MacOs => {
            if let Ok(volumes) = fs::read_dir("/Volumes") {
                let mut names: Vec<PathBuf> = volumes
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect();
                names.sort();
                out.extend(names.into_iter().map(|p| p.to_string_lossy().into_owned()));
            }
        }
        Os::Linux => {}
        Os::Windows => {
            out.clear();
            for letter in 'A'..='Z' {
                let drive = format!("{letter}:\\");
                if Path::new(&drive).exists() {
                    out.push(drive);
                }
            }
        }
    }
    if !home.as_os_str().is_empty() {
        out.push(home.to_string_lossy().into_owned());
    }
    out
}

/// The entries of `dir`: directories first, then files when `include_files`, each sorted by name.
/// Dot entries are left out unless `show_hidden`.
fn list_entries(
    dir: &Path,
    include_files: bool,
    show_hidden: bool,
) -> io::Result<Vec<pb::DirEntry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else {
            continue;
        };
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let path = item.path();
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let is_dir = meta.is_dir();
        if !is_dir && !include_files {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        entries.push(pb::DirEntry {
            name,
            path: path.to_string_lossy().into_owned(),
            is_dir,
            size_bytes: if is_dir { 0 } else { meta.len() as i64 },
            modified_ms,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

/// How long a directory listing may take before `ListDirectory` answers DEADLINE_EXCEEDED (§7).
const LIST_LIMIT: Duration = Duration::from_secs(5);

/// Runs `listing` for at most `limit`. A listing that runs out of time answers DEADLINE_EXCEEDED;
/// otherwise its own result stands. The limit is a parameter so tests can use a limit that
/// expires at once, without waiting.
async fn within<T>(
    limit: Duration,
    listing: impl Future<Output = Result<T, Status>>,
) -> Result<T, Status> {
    match tokio::time::timeout(limit, listing).await {
        Ok(result) => result,
        Err(_) => Err(Status::deadline_exceeded(format!(
            "the directory did not list within {} seconds",
            limit.as_secs()
        ))),
    }
}

fn io_status(err: &io::Error, path: &str) -> Status {
    match err.kind() {
        io::ErrorKind::NotFound => Status::not_found(format!("{path} does not exist")),
        io::ErrorKind::PermissionDenied => {
            Status::permission_denied(format!("{path} cannot be read: {err}"))
        }
        _ => internal(format!("{path}: {err}")),
    }
}

// ── the service ──────────────────────────────────────────────────────────────

/// A stream of messages, fed by a task that ends when the client goes or the daemon stops. On the
/// LAN it also ends with `UNAUTHENTICATED` when the device is revoked (§8.3).
type Messages<T> = UntilRevoked<ReceiverStream<Result<T, Status>>>;

/// The rate a `WatchSnapshots` stream samples at: 0 is 4, the rest clamp to 1..=10 (§7).
fn sample_rate(requested: u32) -> u32 {
    match requested {
        0 => 4,
        n => n.clamp(1, 10),
    }
}

/// Feeds one `WatchSnapshots` stream from `updates` (§7): the current value at once, then the latest
/// one whenever its revision changed, at most one message per `period`. The channel holds one
/// message and the latest value is read only once there is room for it, so a reader that stalls
/// finds at most one old message and then the newest. Ends with UNAVAILABLE on shutdown.
fn feed_snapshots<S, T>(
    mut updates: watch::Receiver<S>,
    mut shutdown: watch::Receiver<bool>,
    period: Duration,
    revision: impl Fn(&S) -> u64 + Send + Sync + 'static,
    message: impl Fn(&S) -> T + Send + Sync + 'static,
) -> mpsc::Receiver<Result<T, Status>>
where
    S: Clone + Send + Sync + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        async {
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut sent = None;
            loop {
                // The first message does not wait for a change.
                if sent.is_some() {
                    tokio::select! {
                        changed = updates.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                        () = wait_shutdown(&mut shutdown) => return,
                    }
                }
                let permit = tokio::select! {
                    reserved = tx.reserve() => match reserved {
                        Ok(permit) => permit,
                        Err(_) => return,
                    },
                    () = wait_shutdown(&mut shutdown) => return,
                };
                // The first tick of an interval is immediate; the next ones pace the messages.
                tokio::select! {
                    _ = ticker.tick() => {}
                    () = wait_shutdown(&mut shutdown) => return,
                }
                // Only the newest value is sent, once per revision.
                let latest = updates.borrow_and_update().clone();
                let current = revision(&latest);
                if sent == Some(current) {
                    continue;
                }
                sent = Some(current);
                permit.send(Ok(message(&latest)));
            }
        }
        .await;
        end_for_shutdown(&tx, &shutdown).await;
    });
    rx
}

/// The page size of `ListRenderedVideos`: 0 is 100, and 500 is the most.
fn videos_limit(requested: u32) -> usize {
    match requested {
        0 => 100,
        n => n.min(500) as usize,
    }
}

/// The page size of `ListLog`: 0 is 200, and 1000 is the most.
fn log_limit(requested: u32) -> usize {
    match requested {
        0 => 200,
        n => n.min(1000) as usize,
    }
}

/// The levels a `WatchLog` or `ListLog` answer includes.
fn at_least(level: LogLevel, min: LogLevel) -> bool {
    level >= min
}

#[tonic::async_trait]
impl TrdService for Server {
    async fn get_server_info(
        &self,
        request: Request<pb::GetServerInfoRequest>,
    ) -> Result<Response<pb::ServerInfo>, Status> {
        let shared = &self.shared;
        let tools = {
            let tools = shared.tools.lock().unwrap_or_else(PoisonError::into_inner);
            tools.infos()
        };
        let tools = tools
            .into_iter()
            .map(|info| pb::ToolInfo {
                name: info.name.to_owned(),
                path: info
                    .path
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                version: info.version,
                available: info.available,
                problem: match info.problem {
                    None => String::new(),
                    Some(ToolProblem::Missing { .. }) => "not found".to_owned(),
                    Some(ToolProblem::Broken { detail, .. }) => detail,
                },
            })
            .collect();
        Ok(Response::new(pb::ServerInfo {
            server_id: shared.server_id.clone(),
            name: shared.server_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: 1,
            platform: crate::cli::platform(),
            cert_fingerprint_sha256: fingerprint(shared),
            lan_port: u32::from(shared.lan_port.load(AtomicOrdering::SeqCst)),
            caller_is_local: listener_of(&request) == Listener::Loopback,
            caller_is_paired: request.extensions().get::<Caller>().is_some(),
            tools,
            sandbox: shared.sandbox,
        }))
    }

    async fn pair(
        &self,
        request: Request<pb::PairRequest>,
    ) -> Result<Response<pb::PairResponse>, Status> {
        let request = request.into_inner();
        let shared = &self.shared;
        let now = now_ms();
        // The code is the authentication, so it is checked before the arguments: a request with
        // no valid code is UNAUTHENTICATED whatever it contains. The correct code is consumed
        // only once the arguments are also valid, so a bad request does not use up the code.
        let mut pairing = shared
            .pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Err(refusal) = pairing.verify(now, &request.code) {
            tracing::info!(?refusal, "a pairing was refused");
            return Err(refusal_status(refusal));
        }
        let platform = match request.platform() {
            pb::DevicePlatform::Ios => "ios",
            pb::DevicePlatform::Android => "android",
            pb::DevicePlatform::Desktop => "desktop",
            pb::DevicePlatform::Unspecified => {
                return Err(Status::invalid_argument("the device platform is required"));
            }
        };
        let name = request.device_name.trim();
        if name.is_empty() {
            return Err(Status::invalid_argument("the device name is required"));
        }
        pairing.consume();
        drop(pairing);
        let (device, token) = new_device(name, platform, now);
        shared.store.put_device(&device).map_err(internal)?;
        shared.log(
            LogLevel::Info,
            "pairing",
            "device paired",
            &LogFields::default(),
        );
        Ok(Response::new(pb::PairResponse {
            token,
            device_id: device.id,
            server_id: shared.server_id.clone(),
            server_name: shared.server_name.clone(),
        }))
    }

    async fn get_snapshot(
        &self,
        request: Request<pb::GetSnapshotRequest>,
    ) -> Result<Response<pb::Snapshot>, Status> {
        authorize(&request)?;
        let snap = self.shared.handle.snapshot();
        Ok(Response::new(self.shared.snapshot_message(&snap)))
    }

    type WatchSnapshotsStream = Messages<pb::Snapshot>;

    async fn watch_snapshots(
        &self,
        request: Request<pb::WatchSnapshotsRequest>,
    ) -> Result<Response<Self::WatchSnapshotsStream>, Status> {
        authorize(&request)?;
        let rate = sample_rate(request.get_ref().max_per_second);
        let period = Duration::from_millis(1000 / u64::from(rate));
        let revoked = revocation_of(&request);
        let shared = self.shared.clone();
        let rx = feed_snapshots(
            shared.handle.subscribe(),
            shared.shutdown.clone(),
            period,
            |snap| snap.revision,
            move |snap| shared.snapshot_message(snap),
        );
        Ok(Response::new(until_revoked(
            ReceiverStream::new(rx),
            revoked,
        )))
    }

    async fn list_rendered_videos(
        &self,
        request: Request<pb::ListRenderedVideosRequest>,
    ) -> Result<Response<pb::ListRenderedVideosResponse>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        let limit = videos_limit(request.limit);
        let search = request.search.to_lowercase();
        let rows = self.shared.store.rendered_videos().map_err(internal)?;
        let matching: Vec<&RenderedVideo> = rows
            .iter()
            .filter(|row| {
                search.is_empty()
                    || row.sequence_name.to_lowercase().contains(&search)
                    || row.output_filename.to_lowercase().contains(&search)
            })
            .collect();
        let start = if request.before_id.is_empty() {
            0
        } else {
            matching
                .iter()
                .position(|row| row.id == request.before_id)
                .map(|at| at + 1)
                .ok_or_else(|| {
                    Status::not_found(format!("no rendered video {}", request.before_id))
                })?
        };
        let page: Vec<pb::RenderedVideo> = matching
            .iter()
            .skip(start)
            .take(limit)
            .map(|row| video_to_proto(row, Path::new(&row.output_path).exists()))
            .collect();
        let more = matching.len() > start + page.len();
        Ok(Response::new(pb::ListRenderedVideosResponse {
            videos: page,
            more,
        }))
    }

    async fn get_thumbnail(
        &self,
        request: Request<pb::GetThumbnailRequest>,
    ) -> Result<Response<pb::Thumbnail>, Status> {
        authorize(&request)?;
        let id = request.into_inner().rendered_video_id;
        let shared = self.shared.clone();
        let made = tokio::task::spawn_blocking(
            move || -> Result<Option<(Vec<u8>, RenderedVideo)>, Status> {
                let Some(row) = shared.store.rendered_video(&id).map_err(internal)? else {
                    return Ok(None);
                };
                let path = ensure_thumbnail(&shared.store, &shared.thumbs, &id, now_ms())
                    .map_err(internal)?;
                let Some(path) = path else {
                    return Ok(None);
                };
                let bytes =
                    fs::read(&path).map_err(|err| io_status(&err, &path.to_string_lossy()))?;
                Ok(Some((bytes, row)))
            },
        )
        .await
        .map_err(internal)??;
        let Some((jpeg, row)) = made else {
            return Err(Status::not_found("no thumbnail exists or can be made"));
        };
        // The thumbnail is one width; its height follows the video's aspect ratio.
        let width = trd_core::thumbs::THUMBNAIL_WIDTH;
        let height = if row.width > 0 {
            (f64::from(width) * row.height as f64 / row.width as f64).round() as u32
        } else {
            0
        };
        Ok(Response::new(pb::Thumbnail {
            jpeg,
            width,
            height,
        }))
    }

    async fn get_render_details(
        &self,
        request: Request<pb::GetRenderDetailsRequest>,
    ) -> Result<Response<pb::RenderDetails>, Status> {
        authorize(&request)?;
        use pb::get_render_details_request::Target;
        let store = &self.shared.store;
        let target = request.into_inner().target.ok_or_else(|| {
            Status::invalid_argument("one of render_id, rendered_video_id, failure_id is needed")
        })?;
        let details = match target {
            Target::RenderId(id) => {
                let job = store
                    .render_job(&id)
                    .map_err(internal)?
                    .ok_or_else(|| Status::not_found(format!("no render {id}")))?;
                render_details(&job, None)
            }
            Target::RenderedVideoId(id) => {
                let video = store
                    .rendered_video(&id)
                    .map_err(internal)?
                    .ok_or_else(|| Status::not_found(format!("no rendered video {id}")))?;
                let job = store
                    .render_jobs(Some(JobStatus::Done))
                    .map_err(internal)?
                    .into_iter()
                    .find(|job| job.output_path == video.output_path)
                    .ok_or_else(|| Status::not_found(format!("no render produced {id}")))?;
                render_details(&job, None)
            }
            Target::FailureId(id) => {
                let failure = store
                    .failure(&id)
                    .map_err(internal)?
                    .ok_or_else(|| Status::not_found(format!("no failure {id}")))?;
                let render_id = failure
                    .render_id
                    .clone()
                    .ok_or_else(|| Status::not_found(format!("failure {id} has no render")))?;
                let job = store
                    .render_job(&render_id)
                    .map_err(internal)?
                    .ok_or_else(|| Status::not_found(format!("no render {render_id}")))?;
                render_details(&job, Some(failure.command_line.as_str()))
            }
        };
        Ok(Response::new(details))
    }

    async fn list_log(
        &self,
        request: Request<pb::ListLogRequest>,
    ) -> Result<Response<pb::ListLogResponse>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        let limit = log_limit(request.limit);
        let min = min_level_of(request.min_level);
        let before = i64::try_from(request.before_seq).unwrap_or(i64::MAX);
        let mut rows = self
            .shared
            .store
            .log_rows_before(before, min, limit as i64 + 1)
            .map_err(internal)?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        Ok(Response::new(pb::ListLogResponse {
            entries: rows.iter().map(log_to_proto).collect(),
            more,
        }))
    }

    type WatchLogStream = Messages<pb::LogEntry>;

    async fn watch_log(
        &self,
        request: Request<pb::WatchLogRequest>,
    ) -> Result<Response<Self::WatchLogStream>, Status> {
        authorize(&request)?;
        let revoked = revocation_of(&request);
        let request = request.into_inner();
        let min = min_level_of(request.min_level);
        let shared = self.shared.clone();
        let mut live = shared.store.subscribe_logs();
        let mut shutdown = shared.shutdown.clone();
        let mut last = i64::try_from(request.after_seq).unwrap_or(i64::MAX);
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            // Stored rows after `last`, oldest first. Used for the backlog, after a lag, and to
            // fill a gap in the live sequence, so no row is sent twice or skipped.
            async fn catch_up(
                shared: &Shared,
                tx: &mpsc::Sender<Result<pb::LogEntry, Status>>,
                last: &mut i64,
                min: LogLevel,
                shutdown: &mut watch::Receiver<bool>,
            ) -> bool {
                let rows = match shared.store.log_rows_after(*last) {
                    Ok(rows) => rows,
                    Err(err) => {
                        tracing::warn!(error = %err, "the log could not be read for WatchLog");
                        return true;
                    }
                };
                for row in rows {
                    if row.seq <= *last {
                        continue;
                    }
                    *last = row.seq;
                    if at_least(row.level, min)
                        && !send_unless_stopped(tx, Ok(log_to_proto(&row)), shutdown).await
                    {
                        return false;
                    }
                }
                true
            }

            async {
                // Subscribed before the backlog is read: a row written in between is in both, and
                // `last` keeps it from being sent twice.
                if !catch_up(&shared, &tx, &mut last, min, &mut shutdown).await {
                    return;
                }
                loop {
                    tokio::select! {
                        received = live.recv() => match received {
                            Ok(row) => {
                                if row.seq <= last {
                                    continue;
                                }
                                if row.seq > last + 1
                                    && !catch_up(&shared, &tx, &mut last, min, &mut shutdown).await
                                {
                                    return;
                                }
                                if row.seq <= last {
                                    continue;
                                }
                                last = row.seq;
                                if at_least(row.level, min)
                                    && !send_unless_stopped(
                                        &tx,
                                        Ok(log_to_proto(&row)),
                                        &mut shutdown,
                                    )
                                    .await
                                {
                                    return;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                if !catch_up(&shared, &tx, &mut last, min, &mut shutdown).await {
                                    return;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                        },
                        () = wait_shutdown(&mut shutdown) => return,
                    }
                }
            }
            .await;
            end_for_shutdown(&tx, &shutdown).await;
        });
        Ok(Response::new(until_revoked(
            ReceiverStream::new(rx),
            revoked,
        )))
    }

    async fn set_global_pause(
        &self,
        request: Request<pb::SetGlobalPauseRequest>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let control = if request.into_inner().paused {
            Control::PauseAllRendering
        } else {
            Control::ResumeAllRendering
        };
        self.control(control).await
    }

    async fn pause_sequence(
        &self,
        request: Request<pb::SequenceRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let name = request.into_inner().sequence_name;
        self.control(Control::PauseSequence(name)).await
    }

    async fn resume_sequence(
        &self,
        request: Request<pb::SequenceRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let name = request.into_inner().sequence_name;
        self.control(Control::ResumeSequence(name)).await
    }

    async fn pause_variant(
        &self,
        request: Request<pb::VariantRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        self.control(Control::PauseVariant {
            sequence: request.sequence_name,
            variant_key: request.variant_key,
        })
        .await
    }

    async fn resume_variant(
        &self,
        request: Request<pb::VariantRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        self.control(Control::ResumeVariant {
            sequence: request.sequence_name,
            variant_key: request.variant_key,
        })
        .await
    }

    async fn suspend_render(
        &self,
        request: Request<pb::RenderRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let id = request.into_inner().render_id;
        self.control(Control::SuspendRender(id)).await
    }

    async fn resume_render(
        &self,
        request: Request<pb::RenderRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let id = request.into_inner().render_id;
        self.control(Control::ResumeRender(id)).await
    }

    async fn stop_render(
        &self,
        request: Request<pb::RenderRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let id = request.into_inner().render_id;
        self.control(Control::StopRender(id)).await
    }

    async fn retry_variant(
        &self,
        request: Request<pb::VariantRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        self.control(Control::Retry {
            sequence: request.sequence_name,
            variant_key: request.variant_key,
        })
        .await
    }

    async fn dismiss_failure(
        &self,
        request: Request<pb::FailureRef>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let id = request.into_inner().failure_id;
        self.control(Control::DismissFailure(id)).await
    }

    async fn dismiss_all_failures(
        &self,
        request: Request<pb::DismissAllFailuresRequest>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        let ids: Vec<String> = self
            .shared
            .store
            .failures()
            .map_err(internal)?
            .into_iter()
            .map(|failure| failure.id)
            .collect();
        for id in ids {
            self.control(Control::DismissFailure(id)).await?;
        }
        Ok(Response::new(pb::Ack {}))
    }

    async fn rescan_now(
        &self,
        request: Request<pb::RescanNowRequest>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        self.control(Control::RescanNow).await
    }

    async fn get_config(
        &self,
        request: Request<pb::GetConfigRequest>,
    ) -> Result<Response<pb::ConfigState>, Status> {
        authorize(&request)?;
        let mut slot = self.shared.lock_config();
        slot.reload();
        Ok(Response::new(config_state(&slot, now_ms())))
    }

    async fn validate_config(
        &self,
        request: Request<pb::ValidateConfigRequest>,
    ) -> Result<Response<pb::ValidateConfigResponse>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        let config = config_from_proto(&request.config.unwrap_or_default());
        let table = if request.codecs.is_empty() {
            self.shared.lock_config().codecs.codecs().clone()
        } else {
            codecs_from_proto(&request.codecs)?
        };
        let issues = core_config::validate(&config, &table, SystemClock.year());
        let rendering = config
            .rendering
            .iter()
            .map(|render| {
                let mut out = render_to_proto(render);
                out.example_filename = config.output_filename(&table, render, "LRT_example");
                out
            })
            .collect();
        Ok(Response::new(pb::ValidateConfigResponse {
            issues: issues.iter().map(issue_to_proto).collect(),
            rendering,
        }))
    }

    async fn update_config(
        &self,
        request: Request<pb::UpdateConfigRequest>,
    ) -> Result<Response<pb::ConfigState>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        let new_config = config_from_proto(&request.config.unwrap_or_default());
        let state = {
            let mut slot = self.shared.lock_config();
            let path = slot.store.path().to_path_buf();
            if !path.exists() {
                // A first run has no folder yet (Application Support, for one).
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(internal)?;
                }
            }
            // An empty revision means "the file does not exist" (§7).
            let expected = if request.expected_revision.is_empty() {
                if path.exists() {
                    return Err(Status::failed_precondition(format!(
                        "config.json exists (revision {}); quote it to change it",
                        file_revision(&path)
                    )));
                }
                core_config::revision_of(&[])
            } else {
                request.expected_revision.clone()
            };
            let ConfigSlot { store, codecs, .. } = &mut *slot;
            store
                .apply(
                    |cfg| {
                        // Keys this version does not know stay as they are in the file.
                        let extra = std::mem::take(&mut cfg.extra);
                        *cfg = new_config;
                        cfg.extra = extra;
                    },
                    &expected,
                    codecs.codecs(),
                )
                .map_err(config_error_status)?;
            self.shared.log(
                LogLevel::Info,
                "config",
                &format!("config.json updated over the API ({})", path.display()),
                &LogFields::default(),
            );
            config_state(&slot, now_ms())
        };
        // The engine picks the edit up on its next scan.
        let _ = self.shared.handle.control(Control::RescanNow).await;
        Ok(Response::new(state))
    }

    async fn update_codecs(
        &self,
        request: Request<pb::UpdateCodecsRequest>,
    ) -> Result<Response<pb::ConfigState>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        let table = codecs_from_proto(&request.codecs)?;
        // Held across the write and the push, so it cannot interleave with SetConfigPath.
        let _switch = self.shared.config_switch.lock().await;
        let (state, engine_table) = {
            let mut slot = self.shared.lock_config();
            slot.reload();
            let target = match slot.codecs.path() {
                Some(path) => path.to_path_buf(),
                None => codecs::sibling_codecs_path(slot.store.path()),
            };
            let current = file_revision(&target);
            if current != request.expected_revision {
                return Err(Status::failed_precondition(format!(
                    "{} changed since it was read (current revision {current:?})",
                    target.display()
                )));
            }
            let errors = codec_table_errors(
                &config_as_written(&slot.store),
                slot.codecs.codecs(),
                &table,
                SystemClock.year(),
            );
            if !errors.is_empty() {
                return Err(config_error_status(ConfigError::Refused(errors)));
            }
            let value = serde_json::to_value(&table).map_err(internal)?;
            let mut text = core_config::pretty_sorted(&value);
            text.push('\n');
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(internal)?;
            }
            core_config::write_atomic(&target, text.as_bytes()).map_err(internal)?;
            let explicit = slot.explicit_codecs.clone();
            let path = slot.store.path().to_path_buf();
            slot.codecs = CodecStore::open(explicit, path);
            self.shared.log(
                LogLevel::Info,
                "config",
                &format!(
                    "video_codecs.json updated over the API ({})",
                    target.display()
                ),
                &LogFields::default(),
            );
            (config_state(&slot, now_ms()), slot.codec_table())
        };
        self.shared
            .handle
            .set_codecs(engine_table)
            .await
            .map_err(|_| shutting_down())?;
        Ok(Response::new(state))
    }

    async fn set_config_path(
        &self,
        request: Request<pb::SetConfigPathRequest>,
    ) -> Result<Response<pb::ConfigState>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        if !core_config::is_absolute_path(&request.path) {
            return Err(Status::invalid_argument("the config path must be absolute"));
        }
        let path = PathBuf::from(&request.path);
        if request.create_if_missing && !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(internal)?;
            }
            let text = CoreConfig::default().to_json_pretty();
            core_config::write_atomic(&path, text.as_bytes()).map_err(internal)?;
        }
        let _switch = self.shared.config_switch.lock().await;
        self.shared
            .store
            .set_config_path(&request.path)
            .map_err(internal)?;
        let (state, codecs) = {
            let mut slot = self.shared.lock_config();
            let explicit = slot.explicit_codecs.clone();
            *slot = ConfigSlot::open(path.clone(), PathSource::Setting, explicit);
            slot.reload();
            self.shared.log(
                LogLevel::Info,
                "config",
                &format!("config path set to {}", request.path),
                &LogFields::default(),
            );
            (config_state(&slot, now_ms()), slot.codec_table())
        };
        let history = crate::daemon::history_path_from_config(&path);
        self.shared
            .handle
            .set_config_path(path, codecs, history)
            .await
            .map_err(|_| shutting_down())?;
        Ok(Response::new(state))
    }

    async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        authorize(&request)?;
        let request = request.into_inner();
        let roots = roots(self.shared.os, &self.shared.home);
        if request.path.is_empty() {
            return Ok(Response::new(pb::ListDirectoryResponse {
                path: String::new(),
                parent: String::new(),
                entries: Vec::new(),
                roots,
            }));
        }
        if !core_config::is_absolute_path(&request.path) {
            return Err(Status::invalid_argument("the path must be absolute"));
        }
        let dir = PathBuf::from(&request.path);
        let include_files = request.include_files;
        let show_hidden = request.show_hidden;
        let path = request.path.clone();
        let listing = async move {
            let joined =
                tokio::task::spawn_blocking(move || list_entries(&dir, include_files, show_hidden))
                    .await;
            joined
                .map_err(internal)?
                .map_err(|err| io_status(&err, &path))
        };
        let entries = within(LIST_LIMIT, listing).await?;
        let parent = Path::new(&request.path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Response::new(pb::ListDirectoryResponse {
            path: request.path,
            parent,
            entries,
            roots,
        }))
    }

    async fn start_pairing(
        &self,
        request: Request<pb::StartPairingRequest>,
    ) -> Result<Response<pb::StartPairingResponse>, Status> {
        authorize(&request)?;
        require_loopback(&request)?;
        let shared = &self.shared;
        let issued = shared
            .pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .issue(now_ms(), new_code());
        // The code itself is never logged.
        tracing::info!("pairing code issued");
        Ok(Response::new(pb::StartPairingResponse {
            code: issued.code,
            expires_at_ms: issued.expires_at_ms,
            server_id: shared.server_id.clone(),
            server_name: shared.server_name.clone(),
            cert_fingerprint_sha256: fingerprint(shared),
            lan_port: u32::from(shared.lan_port.load(AtomicOrdering::SeqCst)),
            lan_addresses: shared.lan_addresses.clone(),
        }))
    }

    async fn list_paired_devices(
        &self,
        request: Request<pb::ListPairedDevicesRequest>,
    ) -> Result<Response<pb::ListPairedDevicesResponse>, Status> {
        authorize(&request)?;
        require_loopback(&request)?;
        let devices = self
            .shared
            .store
            .devices()
            .map_err(internal)?
            .iter()
            .filter(|device| !device.revoked)
            .map(device_to_proto)
            .collect();
        Ok(Response::new(pb::ListPairedDevicesResponse { devices }))
    }

    async fn revoke_device(
        &self,
        request: Request<pb::RevokeDeviceRequest>,
    ) -> Result<Response<pb::Ack>, Status> {
        authorize(&request)?;
        require_loopback(&request)?;
        let id = request.into_inner().device_id;
        let mut device = self
            .shared
            .store
            .device(&id)
            .map_err(internal)?
            .filter(|device| !device.revoked)
            .ok_or_else(|| Status::not_found(format!("no paired device {id}")))?;
        device.revoked = true;
        self.shared.store.put_device(&device).map_err(internal)?;
        self.shared.revocations.revoke(&id);
        Ok(Response::new(pb::Ack {}))
    }
}

impl Server {
    /// Applies one control and maps its error to a status.
    async fn control(&self, control: Control) -> Result<Response<pb::Ack>, Status> {
        self.shared
            .handle
            .control(control)
            .await
            .map_err(control_status)?;
        Ok(Response::new(pb::Ack {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{Settings, assemble};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_stream::StreamExt;
    use trd_core::adopt::AdoptPolicy;
    use trd_core::dirs::Dirs;
    use trd_core::process::FakeProcessTable;

    /// Stops a test that waits for a state that never comes. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);

    /// A server over fresh temporary folders, with an empty fake process table. Its engine is
    /// built but never run, so no scan or render happens.
    /// The returned sender keeps the shutdown watch open; dropping it ends the streams.
    fn server_in(dir: &Path) -> (Server, watch::Sender<bool>) {
        server_with(dir, |_| {})
    }

    /// [`server_in`] with `adjust` applied to the settings first.
    fn server_with(
        dir: &Path,
        adjust: impl FnOnce(&mut Settings),
    ) -> (Server, watch::Sender<bool>) {
        let (stop, stop_rx) = watch::channel(false);
        let mut settings = Settings {
            dirs: Dirs {
                config_default: dir.join("config").join("config.json"),
                data: dir.join("data"),
                logs: dir.join("logs"),
            },
            config: None,
            codecs: None,
            advertise_name: Some("unit".to_owned()),
            policy: AdoptPolicy::None,
            fda_check: false,
            sandbox: false,
            home: dir.to_path_buf(),
            os: Os::current(),
            lan: None,
            mdns: false,
        };
        adjust(&mut settings);
        let table = Box::new(FakeProcessTable::new(Vec::new()));
        let (shared, _engine) = assemble(settings, table, stop_rx).expect("the daemon assembles");
        (Server::new(shared), stop)
    }

    fn temp_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("trd-server-unit-")
            .tempdir()
            .expect("a temporary directory")
    }

    fn marked<T>(message: T, listener: Listener) -> Request<T> {
        let mut request = Request::new(message);
        request.extensions_mut().insert(listener);
        request
    }

    /// A loopback request: the loopback interceptor marks every call this way.
    fn loopback<T>(message: T) -> Request<T> {
        marked(message, Listener::Loopback)
    }

    /// A LAN request from a paired device: the interceptor has attached its caller.
    fn paired<T>(message: T) -> Request<T> {
        let mut request = marked(message, Listener::Lan);
        request.extensions_mut().insert(Caller {
            revoked: watch::channel(false).1,
        });
        request
    }

    /// Device RPCs answer UNAUTHENTICATED on the LAN without a paired device's caller (A2), and
    /// PERMISSION_DENIED with one. Loopback works.
    #[tokio::test(flavor = "multi_thread")]
    async fn lan_requests_are_refused_for_devices() {
        let dir = tempfile::Builder::new()
            .prefix("trd-server-unit-")
            .tempdir()
            .expect("a temporary directory");
        let (server, _stop) = server_in(dir.path());

        let lan = Listener::Lan;
        assert_eq!(
            server
                .list_paired_devices(marked(pb::ListPairedDevicesRequest {}, lan))
                .await
                .expect_err("LAN without a token is refused")
                .code(),
            tonic::Code::Unauthenticated
        );
        assert_eq!(
            server
                .start_pairing(marked(pb::StartPairingRequest {}, lan))
                .await
                .expect_err("LAN without a token is refused")
                .code(),
            tonic::Code::Unauthenticated
        );
        assert_eq!(
            server
                .list_paired_devices(Request::new(pb::ListPairedDevicesRequest {}))
                .await
                .expect_err("an unmarked request is refused")
                .code(),
            tonic::Code::Unauthenticated
        );
        assert_eq!(
            server
                .list_paired_devices(paired(pb::ListPairedDevicesRequest {}))
                .await
                .expect_err("a paired device is refused device RPCs")
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            server
                .revoke_device(paired(pb::RevokeDeviceRequest {
                    device_id: "any".to_owned()
                }))
                .await
                .expect_err("a paired device is refused device RPCs")
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            server
                .start_pairing(paired(pb::StartPairingRequest {}))
                .await
                .expect_err("a paired device cannot start pairing")
                .code(),
            tonic::Code::PermissionDenied
        );

        let loopback = Listener::Loopback;
        server
            .list_paired_devices(marked(pb::ListPairedDevicesRequest {}, loopback))
            .await
            .expect("loopback lists the devices");
        let code = server
            .start_pairing(marked(pb::StartPairingRequest {}, loopback))
            .await
            .expect("loopback starts pairing")
            .into_inner()
            .code;
        assert_eq!(code.len(), 6);
    }

    #[test]
    fn sample_rate_defaults_to_four_and_clamps_to_ten() {
        assert_eq!(sample_rate(0), 4, "zero asks for the default");
        assert_eq!(sample_rate(1), 1);
        assert_eq!(sample_rate(7), 7);
        assert_eq!(sample_rate(10), 10);
        assert_eq!(sample_rate(11), 10);
        assert_eq!(sample_rate(u32::MAX), 10);
    }

    #[test]
    fn io_errors_map_to_statuses() {
        let code = |kind| io_status(&io::Error::from(kind), "/somewhere").code();
        assert_eq!(code(io::ErrorKind::NotFound), tonic::Code::NotFound);
        assert_eq!(
            code(io::ErrorKind::PermissionDenied),
            tonic::Code::PermissionDenied
        );
        assert_eq!(code(io::ErrorKind::TimedOut), tonic::Code::Internal);
        assert_eq!(code(io::ErrorKind::Other), tonic::Code::Internal);
    }

    /// A listing that never finishes answers DEADLINE_EXCEEDED once its limit is past. A zero
    /// limit is past at once, so the test does not wait for the clock.
    #[tokio::test]
    async fn a_listing_past_its_limit_is_deadline_exceeded() {
        let never = std::future::pending::<Result<(), Status>>();
        let status = within(Duration::ZERO, never)
            .await
            .expect_err("the listing times out");
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
        assert!(status.message().contains("seconds"));

        // A listing that finishes in time keeps its own result, success or failure.
        assert_eq!(
            within(LIST_LIMIT, async { Ok(7) })
                .await
                .expect("the listing finishes"),
            7
        );
        let failed = within(LIST_LIMIT, async {
            Err::<(), _>(Status::not_found("gone"))
        })
        .await
        .expect_err("the listing fails");
        assert_eq!(failed.code(), tonic::Code::NotFound);
        assert_eq!(LIST_LIMIT, Duration::from_secs(5), "§7 sets 5 s");
    }

    #[test]
    fn control_errors_map_to_statuses() {
        let code = |err| control_status(err).code();
        assert_eq!(
            code(ControlError::NotFound("x".to_owned())),
            tonic::Code::NotFound
        );
        assert_eq!(
            code(ControlError::FailedPrecondition("x".to_owned())),
            tonic::Code::FailedPrecondition
        );
        assert_eq!(
            code(ControlError::Internal("x".to_owned())),
            tonic::Code::Internal
        );
    }

    #[test]
    fn config_errors_map_to_statuses() {
        let code = |err| config_error_status(err).code();
        assert_eq!(
            code(ConfigError::StaleRevision {
                current: "abc".to_owned()
            }),
            tonic::Code::FailedPrecondition
        );
        assert_eq!(
            code(ConfigError::Unparseable("expected value".to_owned())),
            tonic::Code::FailedPrecondition
        );
        assert_eq!(
            code(ConfigError::Io(io::Error::from(io::ErrorKind::Other))),
            tonic::Code::Internal
        );

        let refused =
            config_error_status(ConfigError::Refused(vec![core_config::ConfigIssue::error(
                "/max_processes/ffmpeg",
                "must be at least 1",
            )]));
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);
        assert!(
            refused.message().contains("/max_processes/ffmpeg"),
            "the message names the field: {}",
            refused.message()
        );
    }

    #[test]
    fn roots_start_at_the_system_root_and_end_at_home() {
        let home = Path::new("/home/someone");
        assert_eq!(roots(Os::Linux, home), ["/", "/home/someone"]);
        assert_eq!(roots(Os::Linux, Path::new("")), ["/"], "no home, no root");
        let mac = roots(Os::MacOs, home);
        assert_eq!(mac.first().map(String::as_str), Some("/"));
        assert_eq!(mac.last().map(String::as_str), Some("/home/someone"));
    }

    fn video(id: &str, sequence: &str) -> RenderedVideo {
        RenderedVideo {
            id: id.to_owned(),
            rendered_at: 1,
            sequence_name: sequence.to_owned(),
            export_path: "/export".to_owned(),
            output_filename: format!("{id}.mov"),
            output_path: format!("/export/{id}.mov"),
            variant_key: "ProResUHQ_25_1280x720".to_owned(),
            codec: "ProResUHQ".to_owned(),
            resolution_label: "1280x720".to_owned(),
            frame_rate: 25,
            frame_count: 3,
            size_bytes: 1,
            width: 1280,
            height: 720,
            duration_seconds: 0.12,
            render_seconds: 1.0,
            thumbnail_path: None,
            xmp_embedded: false,
            source: "trd".to_owned(),
        }
    }

    /// `ListRenderedVideos` pages with `before_id` and no repeats, and searches names without case.
    #[tokio::test(flavor = "multi_thread")]
    async fn rendered_videos_page_and_search() {
        let dir = tempfile::Builder::new()
            .prefix("trd-server-unit-")
            .tempdir()
            .expect("a temporary directory");
        let (server, _stop) = server_in(dir.path());
        for (id, sequence) in [
            ("v1", "alpha"),
            ("v2", "beta"),
            ("v3", "Alpha_two"),
            ("v4", "gamma"),
        ] {
            server
                .shared
                .store
                .put_rendered_video(&video(id, sequence))
                .expect("the row is stored");
        }

        let first = server
            .list_rendered_videos(loopback(pb::ListRenderedVideosRequest {
                limit: 3,
                ..Default::default()
            }))
            .await
            .expect("the first page")
            .into_inner();
        assert_eq!(first.videos.len(), 3);
        assert!(first.more, "one row is left");
        let last = first.videos.last().expect("a row").id.clone();
        let second = server
            .list_rendered_videos(loopback(pb::ListRenderedVideosRequest {
                before_id: last,
                limit: 3,
                ..Default::default()
            }))
            .await
            .expect("the second page")
            .into_inner();
        assert_eq!(second.videos.len(), 1);
        assert!(!second.more);

        let mut ids: Vec<String> = first
            .videos
            .iter()
            .chain(second.videos.iter())
            .map(|video| video.id.clone())
            .collect();
        ids.sort();
        assert_eq!(ids, ["v1", "v2", "v3", "v4"], "every row once");

        let found = server
            .list_rendered_videos(loopback(pb::ListRenderedVideosRequest {
                search: "ALPHA".to_owned(),
                ..Default::default()
            }))
            .await
            .expect("the search")
            .into_inner();
        let mut found_ids: Vec<String> = found.videos.iter().map(|v| v.id.clone()).collect();
        found_ids.sort();
        assert_eq!(found_ids, ["v1", "v3"], "names match without case");
        assert!(!found.more);
    }

    /// The next message of a stream, under the hang guard.
    async fn next_of<T>(
        stream: &mut (impl tokio_stream::Stream<Item = Result<T, Status>> + Unpin),
    ) -> T {
        tokio::time::timeout(HANG_GUARD, stream.next())
            .await
            .expect("the stream answers")
            .expect("the stream stays open")
            .expect("a message")
    }

    /// A reader that stalls while many revisions are published finds the one message that was
    /// waiting and then the newest, never a backlog; an unchanged revision is not sent again.
    #[tokio::test]
    async fn watch_snapshots_gives_a_slow_reader_the_newest() {
        let (publish, updates) = watch::channel(1_u64);
        let (stop, shutdown) = watch::channel(false);
        // Counts the values the feed has read, so the test knows when it has seen one.
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = reads.clone();
        let rx = feed_snapshots(
            updates,
            shutdown,
            Duration::from_nanos(1),
            move |revision| {
                counted.fetch_add(1, Ordering::SeqCst);
                *revision
            },
            |revision| *revision,
        );
        // The first message arrives with nothing published and nothing read.
        tokio::time::timeout(HANG_GUARD, async {
            while rx.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the current snapshot is sent at once");
        for revision in 2..=50 {
            publish.send(revision).expect("the feed listens");
            tokio::task::yield_now().await;
            assert_eq!(rx.len(), 1, "one message at most waits for the reader");
        }

        let mut stream = ReceiverStream::new(rx);
        assert_eq!(next_of(&mut stream).await, 1, "the one that was waiting");
        assert_eq!(next_of(&mut stream).await, 50, "then the newest");

        let before = reads.load(Ordering::SeqCst);
        publish.send(50).expect("the feed listens");
        tokio::time::timeout(HANG_GUARD, async {
            while reads.load(Ordering::SeqCst) == before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the feed reads the repeated revision");
        assert!(stream.as_mut().try_recv().is_err(), "50 is not sent twice");
        publish.send(51).expect("the feed listens");
        assert_eq!(next_of(&mut stream).await, 51);

        stop.send(true).expect("the feed listens");
        let end = tokio::time::timeout(HANG_GUARD, stream.next())
            .await
            .expect("the stream ends")
            .expect("a last item")
            .expect_err("a status");
        assert_eq!(end.code(), tonic::Code::Unavailable);
    }

    fn write_log(server: &Server, level: LogLevel, message: &str) -> u64 {
        let row = log_event(
            &server.shared.store,
            1,
            level,
            "unit",
            message,
            &LogFields::default(),
        )
        .expect("the row is stored");
        u64::try_from(row.seq).expect("a positive seq")
    }

    /// `WatchLog` sends only rows after a non-zero `after_seq`, and `min_level` filters the stored
    /// rows and the live ones.
    #[tokio::test(flavor = "multi_thread")]
    async fn watch_log_filters_by_after_seq_and_level() {
        let dir = temp_dir();
        let (server, _stop) = server_in(dir.path());
        write_log(&server, LogLevel::Error, "old error");
        let after = write_log(&server, LogLevel::Warn, "the last one seen");
        let info = write_log(&server, LogLevel::Info, "stored info");
        let warn = write_log(&server, LogLevel::Warn, "stored warn");

        let watch = |min_level: i32| {
            server.watch_log(loopback(pb::WatchLogRequest {
                after_seq: after,
                min_level,
            }))
        };
        let mut all = watch(0).await.expect("WatchLog").into_inner();
        assert_eq!(next_of(&mut all).await.seq, info);
        assert_eq!(next_of(&mut all).await.seq, warn);

        let mut warnings = watch(pb::LogLevel::LogWarn as i32)
            .await
            .expect("WatchLog at WARN")
            .into_inner();
        assert_eq!(
            next_of(&mut warnings).await.seq,
            warn,
            "stored INFO is left out"
        );
        write_log(&server, LogLevel::Info, "live info");
        let live = write_log(&server, LogLevel::Error, "live error");
        assert_eq!(
            next_of(&mut warnings).await.seq,
            live,
            "live INFO is left out"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rendered_videos_limit_defaults_and_caps() {
        let dir = temp_dir();
        let (server, _stop) = server_in(dir.path());
        for index in 0..501 {
            server
                .shared
                .store
                .put_rendered_video(&video(&format!("v{index}"), "many"))
                .expect("the row is stored");
        }
        for (limit, expected) in [(0, 100), (7, 7), (500, 500), (10_000, 500)] {
            let page = server
                .list_rendered_videos(loopback(pb::ListRenderedVideosRequest {
                    limit,
                    ..Default::default()
                }))
                .await
                .expect("a page")
                .into_inner();
            assert_eq!(page.videos.len(), expected, "limit {limit}");
            assert!(page.more);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_log_limit_defaults_and_caps() {
        let dir = temp_dir();
        let (server, _stop) = server_in(dir.path());
        for _ in 0..1001 {
            server
                .shared
                .store
                .insert_log(1, LogLevel::Info, "unit", "row", &LogFields::default())
                .expect("the row is stored");
        }
        for (limit, expected) in [(0, 200), (7, 7), (1000, 1000), (10_000, 1000)] {
            let page = server
                .list_log(loopback(pb::ListLogRequest {
                    before_seq: 0,
                    limit,
                    min_level: 0,
                }))
                .await
                .expect("a page")
                .into_inner();
            assert_eq!(page.entries.len(), expected, "limit {limit}");
            assert!(page.more);
        }
    }

    fn finished_job(log_path: &Path) -> RenderJob {
        RenderJob {
            id: "r1".to_owned(),
            sequence_name: "alpha".to_owned(),
            export_path: "/export".to_owned(),
            variant_key: "ProResUHQ_25_1280x720".to_owned(),
            output_filename: "v1.mov".to_owned(),
            output_path: "/export/v1.mov".to_owned(),
            partial_path: "/export/v1.partial.mov".to_owned(),
            plan_json: String::new(),
            status: JobStatus::Done,
            suspended_by: None,
            pid: Some(1),
            pid_start_time: None,
            queued_at: 1,
            started_at: Some(2),
            finished_at: Some(3),
            progress_path: None,
            log_path: Some(log_path.to_string_lossy().into_owned()),
            adopted: false,
            foreign: false,
        }
    }

    /// `GetRenderDetails` finds a render by its video and by its failure, and `log_tail` holds the
    /// last 200 lines only.
    #[tokio::test(flavor = "multi_thread")]
    async fn render_details_by_video_and_failure_with_the_log_tail() {
        let dir = temp_dir();
        let (server, _stop) = server_in(dir.path());
        let log = dir.path().join("render.log");
        let text: String = (1..=250).map(|n| format!("line {n}\n")).collect();
        fs::write(&log, text).expect("the render log");
        let store = &server.shared.store;
        store
            .put_render_job(&finished_job(&log))
            .expect("the job is stored");
        store
            .put_rendered_video(&video("v1", "alpha"))
            .expect("the video is stored");
        let failure = store
            .record_failure(&failure_row(Vec::new()))
            .expect("the failure is stored");

        use pb::get_render_details_request::Target;
        for (target, command_line) in [
            (Target::RenderedVideoId("v1".to_owned()), ""),
            (Target::FailureId(failure.id), "ffmpeg -i in out"),
        ] {
            let details = server
                .get_render_details(loopback(pb::GetRenderDetailsRequest {
                    target: Some(target),
                }))
                .await
                .expect("the details")
                .into_inner();
            assert_eq!(details.render_id, "r1");
            assert_eq!(details.command_line, command_line);
            assert_eq!(details.log_tail.len(), 200);
            assert_eq!(details.log_tail[0], "line 51");
            assert_eq!(details.log_tail[199], "line 250");
            assert_eq!(details.pid, 0, "a finished render has no pid");
        }
    }

    fn failure_row(tool_output: Vec<String>) -> Failure {
        Failure {
            id: "f1".to_owned(),
            output_path: "/export/v1.mov".to_owned(),
            kind: "FAILURE_DISK_FULL".to_owned(),
            first_seen: 1,
            last_seen: 2,
            occurrences: 1,
            sequence_name: "alpha".to_owned(),
            export_path: "/export".to_owned(),
            output_filename: "v1.mov".to_owned(),
            variant_key: "ProResUHQ_25_1280x720".to_owned(),
            summary: "summary".to_owned(),
            detail: "detail".to_owned(),
            exit_code: Some(1),
            command_line: "ffmpeg -i in out".to_owned(),
            tool_output_json: serde_json::to_string(&tool_output).expect("JSON"),
            variant_auto_paused: true,
            render_id: Some("r1".to_owned()),
        }
    }

    /// A failure card takes its extra fields from the stored row, with the last 20 output lines.
    #[test]
    fn failure_cards_carry_the_stored_row() {
        let row = failure_row((1..=30).map(|n| n.to_string()).collect());
        let card = trd_core::snapshot::FailureCard {
            id: row.id.clone(),
            first_seen_ms: 1,
            last_seen_ms: 2,
            sequence_name: row.sequence_name.clone(),
            output_filename: row.output_filename.clone(),
            output_path: row.output_path.clone(),
            variant_key: row.variant_key.clone(),
            kind: row.kind.clone(),
            summary: row.summary.clone(),
            detail: row.detail.clone(),
            occurrences: 1,
        };
        let message = failure_to_proto(&card, Some(&row));
        assert_eq!(message.tool_output.len(), 20);
        assert_eq!(message.tool_output[0], "11");
        assert!(message.has_exit_code && message.variant_auto_paused);
        assert_eq!(message.exit_code, 1);
        assert_eq!(message.command_line, row.command_line);
        assert_eq!(message.render_id, "r1");
        let bare = failure_to_proto(&card, None);
        assert!(bare.tool_output.is_empty() && !bare.has_exit_code);
    }

    /// A stored thumbnail is returned with its size; a video whose file is gone has none to make.
    #[tokio::test(flavor = "multi_thread")]
    async fn thumbnails_are_returned_or_not_found() {
        let dir = temp_dir();
        let (server, _stop) = server_in(dir.path());
        let jpeg = dir.path().join("v1.jpg");
        fs::write(&jpeg, b"jpeg").expect("a thumbnail");
        let mut with = video("v1", "alpha");
        with.thumbnail_path = Some(jpeg.to_string_lossy().into_owned());
        let mut without = video("v2", "beta");
        without.output_path = dir.path().join("gone.mov").to_string_lossy().into_owned();
        for row in [&with, &without] {
            server
                .shared
                .store
                .put_rendered_video(row)
                .expect("the row is stored");
        }
        let get = |id: &str| {
            server.get_thumbnail(loopback(pb::GetThumbnailRequest {
                rendered_video_id: id.to_owned(),
            }))
        };
        let found = get("v1").await.expect("a thumbnail").into_inner();
        assert_eq!(found.jpeg, b"jpeg");
        assert_eq!((found.width, found.height), (480, 270));
        assert_eq!(
            get("v2").await.expect_err("none can be made").code(),
            tonic::Code::NotFound
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn revoked_devices_leave_the_list() {
        let dir = temp_dir();
        let (server, _stop) = server_in(dir.path());
        server
            .shared
            .store
            .put_device(&Device {
                id: "d1".to_owned(),
                name: "Phone".to_owned(),
                platform: "ios".to_owned(),
                token_sha256: "0".repeat(64),
                paired_at: 1,
                last_seen: None,
                revoked: false,
            })
            .expect("the device is stored");
        let list = || {
            server.list_paired_devices(marked(pb::ListPairedDevicesRequest {}, Listener::Loopback))
        };
        let before = list().await.expect("the list").into_inner().devices;
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].platform, pb::DevicePlatform::Ios as i32);
        let revoke = || {
            server.revoke_device(marked(
                pb::RevokeDeviceRequest {
                    device_id: "d1".to_owned(),
                },
                Listener::Loopback,
            ))
        };
        revoke().await.expect("RevokeDevice");
        assert!(
            list()
                .await
                .expect("the list")
                .into_inner()
                .devices
                .is_empty()
        );
        assert_eq!(
            revoke().await.expect_err("already revoked").code(),
            tonic::Code::NotFound
        );
    }

    /// The config path comes from `--config`, else the stored setting, else the default, and
    /// `--codecs` names the codec table.
    #[tokio::test(flavor = "multi_thread")]
    async fn config_path_and_codecs_follow_the_flags() {
        let dir = temp_dir();
        let state = |server: Server| async move {
            server
                .get_config(loopback(pb::GetConfigRequest {}))
                .await
                .expect("GetConfig")
                .into_inner()
        };

        let (server, _stop) = server_in(dir.path());
        let stored = dir.path().join("stored.json");
        server
            .shared
            .store
            .set_config_path(&stored.to_string_lossy())
            .expect("the setting");
        let first = state(server).await;
        assert_eq!(
            first.path_source,
            pb::ConfigPathSource::ConfigPathDefault as i32
        );

        let (server, _stop) = server_in(dir.path());
        let second = state(server).await;
        assert_eq!(
            second.path_source,
            pb::ConfigPathSource::ConfigPathSetting as i32
        );
        assert_eq!(second.path, stored.to_string_lossy());

        let flag = dir.path().join("flag.json");
        let table = dir.path().join("table.json");
        let builtin = serde_json::to_string(&codecs::builtin()).expect("JSON");
        fs::write(&table, builtin).expect("a codec table");
        let (server, _stop) = server_with(dir.path(), |settings| {
            settings.config = Some(flag.clone());
            settings.codecs = Some(table.clone());
        });
        let third = state(server).await;
        assert_eq!(
            third.path_source,
            pb::ConfigPathSource::ConfigPathFlag as i32
        );
        assert_eq!(third.path, flag.to_string_lossy());
        assert_eq!(third.codecs_path, table.to_string_lossy());
    }

    #[test]
    fn codec_table_errors_are_only_the_new_ones() {
        let render = |codec: &str| core_config::RenderType {
            codec: codec.to_owned(),
            frame_rate: 25,
            resolution: None,
        };
        let config = CoreConfig {
            rendering: vec![render("ProResUHQ"), render("Missing")],
            ..CoreConfig::default()
        };
        let current = codecs::builtin();
        let mut next = current.clone();
        next.remove("h264");
        assert!(
            codec_table_errors(&config, &current, &next, 2026).is_empty(),
            "an unused codec goes, and the codec already missing blocks nothing"
        );
        next.remove("ProResUHQ");
        let errors = codec_table_errors(&config, &current, &next, 2026);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].field, "/rendering/0/codec");
    }

    #[test]
    fn config_as_written_keeps_entries_a_load_drops() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("config.json");
        let config = CoreConfig {
            rendering: vec![core_config::RenderType {
                codec: "ProResUHQ".to_owned(),
                frame_rate: 0,
                resolution: None,
            }],
            ..CoreConfig::default()
        };
        fs::write(&path, config.to_json_pretty()).expect("config.json");
        let mut store = ConfigStore::new(path, Arc::new(SystemClock));
        store.reload_if_changed(&codecs::builtin());
        assert!(store.config().expect("loaded").rendering.is_empty());
        assert_eq!(config_as_written(&store).rendering, config.rendering);
    }
}
