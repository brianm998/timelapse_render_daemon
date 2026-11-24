// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The SQLite store and log events through the public API only. Every database lives in a
//! temporary directory; no test reads the owner's data or uses a daemon port.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use trd_core::store::{
    DAY_MS, Device, ExifCacheEntry, Failure, JobStatus, LogFields, LogLevel, RenderJob,
    RenderedVideo, Store, SuspendedBy, log_event,
};

/// Serialises the tests that emit `tracing` events. `tracing` caches each callsite's interest
/// process-wide: a callsite hit while no subscriber is installed (on another test thread) is
/// cached as "never", and then a captured subscriber on a different thread would miss it.
static TRACE_LOCK: Mutex<()> = Mutex::new(());

fn trace_guard() -> MutexGuard<'static, ()> {
    TRACE_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Opens a store at `dir/<name>/trd.sqlite3`, so the parent directory is created too.
fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("nested").join("trd.sqlite3")).expect("open store")
}

fn sample_job(id: &str, status: JobStatus) -> RenderJob {
    RenderJob {
        id: id.to_owned(),
        sequence_name: "seq-a".to_owned(),
        export_path: "/exports".to_owned(),
        variant_key: "h264_1920x1080_25".to_owned(),
        output_filename: "seq-a_h264.mov".to_owned(),
        output_path: "/exports/seq-a_h264.mov".to_owned(),
        partial_path: "/exports/.seq-a_h264.partial.mov".to_owned(),
        plan_json: "{\"codec\":\"h264\"}".to_owned(),
        status,
        suspended_by: Some(SuspendedBy::Global),
        pid: Some(4242),
        pid_start_time: Some(1_700_000_000_000),
        queued_at: 1_000,
        started_at: Some(2_000),
        finished_at: None,
        progress_path: Some("/data/renders/job.progress".to_owned()),
        log_path: Some("/data/renders/job.log".to_owned()),
        adopted: true,
        foreign: false,
    }
}

fn sample_video(id: &str, rendered_at: i64, thumbnail: Option<&str>) -> RenderedVideo {
    RenderedVideo {
        id: id.to_owned(),
        rendered_at,
        sequence_name: "seq-a".to_owned(),
        export_path: "/exports".to_owned(),
        output_filename: format!("{id}.mov"),
        output_path: format!("/exports/{id}.mov"),
        variant_key: "ProResUHQ_ori_24".to_owned(),
        codec: "ProResUHQ".to_owned(),
        resolution_label: "OriRes".to_owned(),
        frame_rate: 24,
        frame_count: 1_200,
        size_bytes: 5_000_000,
        width: 3840,
        height: 2160,
        duration_seconds: 50.0,
        render_seconds: 12.5,
        thumbnail_path: thumbnail.map(str::to_owned),
        xmp_embedded: true,
        source: "TRD".to_owned(),
    }
}

fn sample_failure(output_path: &str, kind: &str, seen: i64) -> Failure {
    Failure {
        id: format!("failure-{kind}"),
        output_path: output_path.to_owned(),
        kind: kind.to_owned(),
        first_seen: seen,
        last_seen: seen,
        occurrences: 1,
        sequence_name: "seq-a".to_owned(),
        export_path: "/exports".to_owned(),
        output_filename: "seq-a_h264.mov".to_owned(),
        variant_key: "h264_1920x1080_25".to_owned(),
        summary: "ffmpeg failed".to_owned(),
        detail: "last lines".to_owned(),
        exit_code: Some(1),
        command_line: "ffmpeg -i x".to_owned(),
        tool_output_json: "[]".to_owned(),
        variant_auto_paused: false,
        render_id: Some("job-1".to_owned()),
    }
}

#[test]
fn store_migrates_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    assert_eq!(store.schema_version().unwrap(), 1);

    // Every §6.13 table and index exists after the migration.
    let names: Vec<String> = {
        let conn =
            rusqlite::Connection::open(dir.path().join("nested").join("trd.sqlite3")).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'index') AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    for name in [
        "settings",
        "render_jobs",
        "rendered_videos",
        "failures",
        "variant_strikes",
        "exif_cache",
        "devices",
        "log",
        "rendered_videos_rendered_at",
    ] {
        assert!(
            names.iter().any(|n| n == name),
            "missing {name} in {names:?}"
        );
    }

    // settings, typed helpers included.
    assert_eq!(store.setting("missing").unwrap(), None);
    store.set_setting("custom", "value").unwrap();
    assert_eq!(store.setting("custom").unwrap().as_deref(), Some("value"));
    store.clear_setting("custom").unwrap();
    assert_eq!(store.setting("custom").unwrap(), None);
    assert_eq!(store.config_path().unwrap(), None);
    store.set_config_path("/configs/config.json").unwrap();
    assert_eq!(
        store.config_path().unwrap(),
        Some(PathBuf::from("/configs/config.json"))
    );
    assert!(!store.global_pause().unwrap());
    store.set_global_pause(true).unwrap();
    assert!(store.global_pause().unwrap());
    assert!(!store.swift_history_imported().unwrap());
    store.set_swift_history_imported(true).unwrap();
    assert!(store.swift_history_imported().unwrap());
    let server_id = store.server_id().unwrap();
    assert_eq!(store.server_id().unwrap(), server_id);
    assert_eq!(
        uuid::Uuid::parse_str(&server_id).unwrap().get_version_num(),
        4
    );

    // render_jobs: a full row, its status filter, and an update that replaces it.
    let queued = sample_job("job-1", JobStatus::Queued);
    let mut running = sample_job("job-2", JobStatus::Running);
    running.queued_at = 500;
    store.put_render_job(&queued).unwrap();
    store.put_render_job(&running).unwrap();
    assert_eq!(store.render_job("job-1").unwrap(), Some(queued.clone()));
    assert_eq!(store.render_job("nope").unwrap(), None);
    let all: Vec<String> = store
        .render_jobs(None)
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect();
    assert_eq!(all, ["job-2", "job-1"]);
    let only_running = store.render_jobs(Some(JobStatus::Running)).unwrap();
    assert_eq!(only_running, vec![running.clone()]);
    let mut done = queued.clone();
    done.status = JobStatus::Done;
    done.suspended_by = None;
    done.finished_at = Some(3_000);
    store.put_render_job(&done).unwrap();
    assert_eq!(store.render_job("job-1").unwrap(), Some(done));

    // rendered_videos, with and without a thumbnail.
    let with_thumb = sample_video("video-1", 10_000, Some("/data/thumbnails/video-1.jpg"));
    let without_thumb = sample_video("video-2", 20_000, None);
    store.put_rendered_video(&with_thumb).unwrap();
    store.put_rendered_video(&without_thumb).unwrap();
    assert_eq!(
        store.rendered_video("video-1").unwrap(),
        Some(with_thumb.clone())
    );
    assert_eq!(
        store.rendered_video("video-2").unwrap(),
        Some(without_thumb.clone())
    );
    let newest_first: Vec<String> = store
        .rendered_videos()
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(newest_first, ["video-2", "video-1"]);

    // failures: the first record is stored as given.
    let failure = sample_failure("/exports/seq-a_h264.mov", "ffmpeg", 5_000);
    let stored = store.record_failure(&failure).unwrap();
    assert_eq!(stored, failure);
    assert_eq!(store.failure(&failure.id).unwrap(), Some(failure.clone()));
    assert_eq!(store.failures().unwrap(), vec![failure.clone()]);
    assert!(store.delete_failure(&failure.id).unwrap());
    assert!(!store.delete_failure(&failure.id).unwrap());
    assert_eq!(store.failure(&failure.id).unwrap(), None);
    store.record_failure(&failure).unwrap();
    assert_eq!(
        store
            .delete_failures_for_output("/exports/seq-a_h264.mov")
            .unwrap(),
        1
    );
    assert_eq!(store.failures().unwrap(), vec![]);

    // variant_strikes: 0 until stored.
    assert_eq!(
        store.corrupt_strikes("seq-a", "h264_1920x1080_25").unwrap(),
        0
    );
    store
        .set_corrupt_strikes("seq-a", "h264_1920x1080_25", 2)
        .unwrap();
    assert_eq!(
        store.corrupt_strikes("seq-a", "h264_1920x1080_25").unwrap(),
        2
    );

    // exif_cache.
    assert_eq!(store.exif_cache("/raw/seq-a").unwrap(), None);
    let entry = ExifCacheEntry {
        raw_dir: "/raw/seq-a".to_owned(),
        histogram_json: "{\"ISO\":{\"100\":3}}".to_owned(),
        collected_at: 7_000,
    };
    store.put_exif_cache(&entry).unwrap();
    assert_eq!(store.exif_cache("/raw/seq-a").unwrap(), Some(entry));

    // devices, found by id and by token hash.
    let device = Device {
        id: "device-1".to_owned(),
        name: "Brian's phone".to_owned(),
        platform: "ios".to_owned(),
        token_sha256: "abc123".to_owned(),
        paired_at: 8_000,
        last_seen: None,
        revoked: false,
    };
    store.put_device(&device).unwrap();
    assert_eq!(store.device("device-1").unwrap(), Some(device.clone()));
    assert_eq!(
        store.device_by_token_hash("abc123").unwrap(),
        Some(device.clone())
    );
    assert_eq!(store.device_by_token_hash("other").unwrap(), None);
    assert_eq!(store.devices().unwrap(), vec![device]);
}

#[test]
fn log_seq_survives_reopen() {
    let _trace = trace_guard();
    let dir = tempfile::tempdir().unwrap();
    let fields = LogFields::default();

    let store = open_store(dir.path());
    let first = log_event(&store, 1, LogLevel::Info, "render", "first", &fields).unwrap();
    let second = log_event(&store, 2, LogLevel::Info, "render", "second", &fields).unwrap();
    assert!(second.seq > first.seq);
    drop(store);

    // Reopen: the next sequence number continues past the stored rows.
    let store = open_store(dir.path());
    let third = log_event(&store, 3, LogLevel::Warn, "render", "third", &fields).unwrap();
    assert!(third.seq > second.seq);

    // Prune every row, including the newest, then reopen again: still no reuse.
    store.prune_with(10, 30, 0, DAY_MS).unwrap();
    assert!(store.log_rows_after(0).unwrap().is_empty());
    drop(store);

    let store = open_store(dir.path());
    let fourth = log_event(&store, 4, LogLevel::Error, "render", "fourth", &fields).unwrap();
    assert!(
        fourth.seq > third.seq,
        "seq {} reused after {}",
        fourth.seq,
        third.seq
    );
}

#[test]
fn retention() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let now = 100 * DAY_MS;

    // rendered_videos: one past the 30-day window with a thumbnail, one past it without, one inside.
    store
        .put_rendered_video(&sample_video(
            "old-thumb",
            now - 31 * DAY_MS,
            Some("/t/old.jpg"),
        ))
        .unwrap();
    store
        .put_rendered_video(&sample_video("old-plain", now - 40 * DAY_MS, None))
        .unwrap();
    store
        .put_rendered_video(&sample_video("recent", now - DAY_MS, Some("/t/recent.jpg")))
        .unwrap();

    // log: seq 1 is older than 30 days; seqs 2 to 5 are recent, so the limit of 3 rows drops seq 2.
    let fields = LogFields::default();
    store
        .insert_log(now - 40 * DAY_MS, LogLevel::Info, "old", "old row", &fields)
        .unwrap();
    for (offset_days, message) in [(5, "two"), (4, "three"), (3, "four"), (2, "five")] {
        store
            .insert_log(
                now - offset_days * DAY_MS,
                LogLevel::Info,
                "recent",
                message,
                &fields,
            )
            .unwrap();
    }

    let thumbnails = store.prune_with(now, 30, 3, 30 * DAY_MS).unwrap();
    assert_eq!(thumbnails, vec!["/t/old.jpg".to_owned()]);

    let kept_videos: Vec<String> = store
        .rendered_videos()
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(kept_videos, ["recent"]);

    let kept_logs: Vec<(i64, String)> = store
        .log_rows_after(0)
        .unwrap()
        .into_iter()
        .map(|row| (row.seq, row.message))
        .collect();
    assert_eq!(
        kept_logs,
        [
            (3, "three".to_owned()),
            (4, "four".to_owned()),
            (5, "five".to_owned())
        ]
    );

    // Nothing left to delete: a second pass returns no thumbnails and keeps the rows.
    assert!(
        store
            .prune_with(now, 30, 3, 30 * DAY_MS)
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.log_rows_after(0).unwrap().len(), 3);
}

#[test]
fn failures_pruned_after_30_days() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let now = 100 * DAY_MS;

    // A card last seen 31 days ago goes; one seen a day ago stays.
    store
        .record_failure(&sample_failure("/e/old.mov", "old_kind", now - 31 * DAY_MS))
        .unwrap();
    store
        .record_failure(&sample_failure(
            "/e/recent.mov",
            "recent_kind",
            now - DAY_MS,
        ))
        .unwrap();

    store.prune_with(now, 30, 1_000, 30 * DAY_MS).unwrap();

    let kept: Vec<String> = store
        .failures()
        .unwrap()
        .into_iter()
        .map(|failure| failure.output_path)
        .collect();
    assert_eq!(kept, ["/e/recent.mov".to_owned()]);
}

/// Collects every `tracing` event as its target, level and fields (strings), in order.
#[derive(Default, Clone)]
struct Captured(Arc<Mutex<Vec<CapturedEvent>>>);

#[derive(Debug, PartialEq)]
struct CapturedEvent {
    target: String,
    level: String,
    fields: BTreeMap<String, String>,
}

struct FieldMap<'a>(&'a mut BTreeMap<String, String>);

impl Visit for FieldMap<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S: Subscriber> Layer<S> for Captured {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldMap(&mut fields));
        self.0.lock().unwrap().push(CapturedEvent {
            target: event.metadata().target().to_owned(),
            level: event.metadata().level().to_string(),
            fields,
        });
    }
}

#[test]
fn log_event_writes_both() {
    let _trace = trace_guard();
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let mut live = store.subscribe_logs();

    let fields = LogFields {
        event: Some("render_finished".to_owned()),
        sequence_name: Some("seq-a".to_owned()),
        output_filename: Some("seq-a_h264.mov".to_owned()),
        rendered_video_id: Some("video-1".to_owned()),
        failure_id: None,
        video_count: Some(2),
        total_bytes: Some(9_000_000),
    };

    let captured = Captured::default();
    let subscriber = Registry::default().with(captured.clone());
    let row = tracing::subscriber::with_default(subscriber, || {
        log_event(
            &store,
            42,
            LogLevel::Info,
            "seq-a",
            "render finished",
            &fields,
        )
        .unwrap()
    });

    // One tracing record, carrying the same fields as the stored row.
    let events = captured.0.lock().unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert_eq!(event.target, "trd");
    assert_eq!(event.level, "INFO");
    let expected: BTreeMap<String, String> = [
        ("seq", row.seq.to_string()),
        ("subject", "seq-a".to_owned()),
        ("message", "render finished".to_owned()),
        ("event", "render_finished".to_owned()),
        ("sequence_name", "seq-a".to_owned()),
        ("output_filename", "seq-a_h264.mov".to_owned()),
        ("rendered_video_id", "video-1".to_owned()),
        ("video_count", "2".to_owned()),
        ("total_bytes", "9000000".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(event.fields, expected);

    // One log row, equal to the returned row, with the same fields.
    assert_eq!(row.at, 42);
    assert_eq!(row.level, LogLevel::Info);
    assert_eq!(row.fields, fields);
    assert_eq!(store.log_rows_after(0).unwrap(), vec![row.clone()]);
    assert_eq!(store.log_rows_after(row.seq).unwrap(), vec![]);

    // The live subscriber got the same row.
    assert_eq!(live.try_recv().unwrap(), row);
    assert!(live.try_recv().is_err());
}
