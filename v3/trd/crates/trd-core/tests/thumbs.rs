// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Thumbnails (§6.16), the Swift history import (§6.17) and the history retention that deletes
//! thumbnails (§6.13). Every database and fixture lives in a temporary directory. The fake ffmpeg
//! is a small `sh` script written into that directory, so the tests need no binaries built by
//! another package and touch no owner data or port. The `thumbnail_written` and
//! `thumbnail_failure` tests need `sh` and so run on Unix only. The fakes are written at run time,
//! so no binary from another crate has to be built first: `cargo test -p trd-core --test thumbs`
//! works on a cold target, with or without `cargo test --workspace`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;
use trd_core::history_import::{
    ImportReport, ImportStatus, SOURCE_IMPORTED, default_history_path, import_swift_history,
};
use trd_core::store::{DAY_MS, LogLevel, RenderedVideo, Store};
use trd_core::thumbs::{
    Runner, THUMBNAILS_DIR, Thumbnailer, ensure_thumbnail, prune_history, thumbnail_args,
};
#[cfg(unix)]
use trd_core::tools::ToolCommand;

/// 2023-11-14T22:13:20Z in milliseconds.
const NOW: i64 = 1_700_000_000_000;
/// The longest a run waits for its partner in [`Rendezvous`]. It only guards against a hang.
const DEADLINE: Duration = Duration::from_secs(30);

fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("trd.sqlite3")).expect("open the store")
}

/// A rendered video whose file is `<dir>/<id>.mov`. The file is not written.
fn video(dir: &Path, id: &str, rendered_at: i64) -> RenderedVideo {
    let output = dir.join(format!("{id}.mov"));
    RenderedVideo {
        id: id.to_owned(),
        rendered_at,
        sequence_name: "seq-a".to_owned(),
        export_path: dir.to_string_lossy().into_owned(),
        output_filename: format!("{id}.mov"),
        output_path: output.to_string_lossy().into_owned(),
        variant_key: "h264_1920x1080_25".to_owned(),
        codec: "h264".to_owned(),
        resolution_label: "1080p".to_owned(),
        frame_rate: 25,
        frame_count: 48,
        size_bytes: 6,
        width: 1920,
        height: 1080,
        duration_seconds: 10.0,
        render_seconds: 3.5,
        thumbnail_path: None,
        xmp_embedded: false,
        source: "TRD".to_owned(),
    }
}

/// The argument that names the output: the last one.
fn output_of(args: &[String]) -> &str {
    args.last().expect("ffmpeg is given an output")
}

/// Writes an executable `ffmpeg` script into `dir` whose body runs after the output argument is
/// found: the last argument of the command (§6.16 puts the output last).
#[cfg(unix)]
fn write_fake_ffmpeg(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(dir).expect("the fake tools folder");
    let path = dir.join("ffmpeg");
    fs::write(&path, format!("#!/bin/sh\nfor out; do :; done\n{body}\n")).expect("the fake ffmpeg");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("make it executable");
    path
}

/// Serializes the tests that write and run a fake ffmpeg. A child forked by one test while another
/// test still has its script open for writing can hit ETXTBSY when it runs that script, so these
/// tests take this lock for their whole body. Poisoning is ignored: a panic in one test must not
/// block the rest.
#[cfg(unix)]
static FAKE_FFMPEG_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
fn fake_ffmpeg_guard() -> std::sync::MutexGuard<'static, ()> {
    FAKE_FFMPEG_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A fake ffmpeg that writes a JPEG to its output.
#[cfg(unix)]
const FFMPEG_WRITES_JPEG: &str = r#"printf '\377\330\377\340' > "$out""#;

/// A fake ffmpeg that writes a partial file to its output, reports on stderr and exits 1.
#[cfg(unix)]
const FFMPEG_FAILS: &str = r#"printf 'partial' > "$out"; echo 'Invalid data found' >&2; exit 1"#;

/// Two runs that wait for each other, so that a test can see them in flight together without any
/// sleep. A run whose partner never arrives panics after [`DEADLINE`] instead of hanging.
struct Rendezvous {
    arrived: Mutex<usize>,
    all: Condvar,
}

impl Rendezvous {
    fn new() -> Rendezvous {
        Rendezvous {
            arrived: Mutex::new(0),
            all: Condvar::new(),
        }
    }

    /// Counts this run and waits until two runs have arrived. Later runs pass straight through.
    fn meet(&self) {
        let mut arrived = self.arrived.lock().expect("the rendezvous lock");
        *arrived += 1;
        self.all.notify_all();
        let (_arrived, timeout) = self
            .all
            .wait_timeout_while(arrived, DEADLINE, |count| *count < 2)
            .expect("the rendezvous wait");
        assert!(
            !timeout.timed_out(),
            "no second run reached the rendezvous: the gate let fewer than two run at once"
        );
    }
}

/// A runner that writes the output file and counts its calls.
fn writing_runner(calls: Arc<AtomicUsize>) -> Runner {
    Box::new(move |args| {
        calls.fetch_add(1, Ordering::SeqCst);
        fs::write(output_of(args), b"jpeg").map_err(|err| err.to_string())
    })
}

#[cfg(unix)]
#[test]
fn thumbnail_written() {
    let _lock = fake_ffmpeg_guard();
    let tmp = TempDir::new().expect("a temporary directory");
    let ffmpeg = write_fake_ffmpeg(&tmp.path().join("bin"), FFMPEG_WRITES_JPEG);
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let render = video(tmp.path(), "render-1", NOW);
    fs::write(&render.output_path, b"render").expect("the rendered file");
    store.put_rendered_video(&render).expect("store the row");

    // The real command path: the fake ffmpeg writes the poster frame it is asked for.
    let thumbnailer = Thumbnailer::with_tool(&data, ToolCommand::direct(&ffmpeg));
    assert!(thumbnailer.make(&store, &render, NOW).expect("make"));

    let thumb = thumbnailer.path_for("render-1");
    let bytes = fs::read(&thumb).expect("the thumbnail exists");
    assert!(bytes.starts_with(&[0xFF, 0xD8]), "a JPEG was written");
    let stored = store
        .rendered_video("render-1")
        .expect("read the row")
        .expect("the row exists");
    assert_eq!(
        stored.thumbnail_path.as_deref(),
        Some(thumb.to_str().expect("UTF-8 path"))
    );
}

#[cfg(unix)]
#[test]
fn thumbnail_failure() {
    let _lock = fake_ffmpeg_guard();
    let tmp = TempDir::new().expect("a temporary directory");
    let ffmpeg = write_fake_ffmpeg(&tmp.path().join("bin"), FFMPEG_FAILS);
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let render = video(tmp.path(), "render-2", NOW);
    fs::write(&render.output_path, b"render").expect("the rendered file");
    store.put_rendered_video(&render).expect("store the row");

    // The fake writes its output and then exits 1: the partial file must not survive.
    let thumbnailer = Thumbnailer::with_tool(&data, ToolCommand::direct(&ffmpeg));
    assert!(!thumbnailer.make(&store, &render, NOW).expect("make"));

    let stored = store
        .rendered_video("render-2")
        .expect("read the row")
        .expect("the row exists");
    assert_eq!(stored.thumbnail_path, None, "has_thumbnail stays false");
    assert!(
        !thumbnailer.path_for("render-2").exists(),
        "no partial file"
    );

    let rows = store.log_rows_after(0).expect("the log");
    let warning = rows
        .iter()
        .find(|row| row.fields.event.as_deref() == Some("thumbnail_failed"))
        .expect("a thumbnail_failed row");
    assert_eq!(warning.level, LogLevel::Warn);
    assert_eq!(
        warning.fields.rendered_video_id.as_deref(),
        Some("render-2")
    );
    assert!(
        warning.message.contains("Invalid data found"),
        "{}",
        warning.message
    );
}

#[derive(Default)]
struct Counters {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
}

#[test]
fn thumbnail_concurrency() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let counters = Arc::new(Counters::default());
    // The first two runs wait for each other, so they are known to overlap; no timing is involved.
    let rendezvous = Arc::new(Rendezvous::new());

    let shared = Arc::clone(&counters);
    let met = Arc::clone(&rendezvous);
    let runner: Runner = Box::new(move |args| {
        shared.calls.fetch_add(1, Ordering::SeqCst);
        let now = shared.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        shared.peak.fetch_max(now, Ordering::SeqCst);
        met.meet();
        let result = fs::write(output_of(args), b"jpeg").map_err(|err| err.to_string());
        shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        result
    });
    let thumbnailer = Thumbnailer::new(&data, runner);

    let videos: Vec<RenderedVideo> = (0..6)
        .map(|index| video(tmp.path(), &format!("render-{index}"), NOW))
        .collect();
    for render in &videos {
        fs::write(&render.output_path, b"render").expect("the rendered file");
        store.put_rendered_video(render).expect("store the row");
    }

    // Shared references, so each spawned closure can own a copy of them.
    let (thumbnailer, store) = (&thumbnailer, &store);
    thread::scope(|scope| {
        let handles: Vec<_> = videos
            .iter()
            .map(|render| scope.spawn(move || thumbnailer.make(store, render, NOW)))
            .collect();
        for handle in handles {
            assert!(handle.join().expect("no panic").expect("make"));
        }
    });

    assert_eq!(counters.calls.load(Ordering::SeqCst), 6);
    assert!(
        counters.peak.load(Ordering::SeqCst) <= 2,
        "two thumbnails run at once, never more"
    );
}

#[test]
fn thumbnail_on_demand() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let calls = Arc::new(AtomicUsize::new(0));
    let thumbnailer = Thumbnailer::new(&data, writing_runner(Arc::clone(&calls)));

    // An imported row with its file and no thumbnail: one is made.
    let with_file = video(tmp.path(), "imported-a", NOW);
    fs::write(&with_file.output_path, b"render").expect("the rendered file");
    store.put_rendered_video(&with_file).expect("store the row");
    let made = ensure_thumbnail(&store, &thumbnailer, "imported-a", NOW).expect("ensure");
    assert_eq!(made, Some(thumbnailer.path_for("imported-a")));
    assert!(thumbnailer.path_for("imported-a").is_file());
    let stored = store
        .rendered_video("imported-a")
        .expect("read the row")
        .expect("the row exists");
    assert!(
        stored.thumbnail_path.is_some(),
        "the row now has a thumbnail"
    );

    // A row whose file is gone: none is made, and ffmpeg is not run.
    let without_file = video(tmp.path(), "imported-b", NOW);
    store
        .put_rendered_video(&without_file)
        .expect("store the row");
    assert_eq!(
        ensure_thumbnail(&store, &thumbnailer, "imported-b", NOW).expect("ensure"),
        None
    );
    assert!(!thumbnailer.path_for("imported-b").exists());

    // A row that does not exist: none either.
    assert_eq!(
        ensure_thumbnail(&store, &thumbnailer, "no-such-row", NOW).expect("ensure"),
        None
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "ffmpeg ran for the one row only"
    );
}

#[test]
fn thumbnail_retention() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let thumbs = data.join(THUMBNAILS_DIR);
    fs::create_dir_all(&thumbs).expect("the thumbnails folder");

    // One row older than the 30-day window and one inside it, each with a thumbnail file.
    let old = video(tmp.path(), "old", NOW - 31 * DAY_MS);
    let recent = video(tmp.path(), "recent", NOW - DAY_MS);
    for (mut row, name) in [(old, "old.jpg"), (recent, "recent.jpg")] {
        let thumb = thumbs.join(name);
        fs::write(&thumb, b"jpeg").expect("the thumbnail");
        row.thumbnail_path = Some(thumb.to_string_lossy().into_owned());
        store.put_rendered_video(&row).expect("store the row");
    }

    let deleted = prune_history(&store, NOW, 30).expect("retention");
    assert_eq!(deleted, 1, "one thumbnail file was deleted");
    assert!(
        !thumbs.join("old.jpg").exists(),
        "the removed row's thumbnail is gone"
    );
    assert!(
        thumbs.join("recent.jpg").exists(),
        "a kept row's thumbnail stays"
    );
    let ids: Vec<String> = store
        .rendered_videos()
        .expect("list the rows")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(ids, ["recent"]);
}

/// A Swift history: two entries inside the 30-day window, one older, and one with a date that is
/// not ISO 8601.
fn history_fixture() -> String {
    json!([
        history_entry("2023-11-14T22:00:00Z", "seq-a", "one.mov"),
        history_entry("2023-11-13T00:00:00.500Z", "seq-a", "two.mov"),
        history_entry("2023-09-01T00:00:00Z", "seq-a", "old.mov"),
        history_entry("yesterday", "seq-a", "bad.mov"),
    ])
    .to_string()
}

fn history_entry(timestamp: &str, sequence: &str, file: &str) -> Value {
    json!({
        "timestamp": timestamp,
        "sequenceName": sequence,
        "exportPath": "/exports/a",
        "outputFilename": file,
        "outputPath": format!("/exports/a/{file}"),
        "frameCount": 48,
        "fileSize": 1234,
    })
}

#[test]
fn import_once() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let path = tmp.path().join("render_history.json");

    // No file: nothing is imported, and the setting stays unset so a later start can import it.
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::Missing
    );
    assert!(!store.swift_history_imported().expect("setting"));

    fs::write(&path, history_fixture()).expect("the history file");
    let status = import_swift_history(&path, &store, 30, NOW).expect("import");
    assert_eq!(
        status,
        ImportStatus::Imported(ImportReport {
            imported: 2,
            too_old: 1,
            invalid: 1,
        })
    );
    assert_eq!(store.rendered_videos().expect("rows").len(), 2);
    assert!(store.swift_history_imported().expect("setting"));

    // A later start does not read the file again, even when it has changed.
    let mut changed: Vec<Value> = serde_json::from_str(&history_fixture()).expect("json");
    changed.push(history_entry("2023-11-14T23:00:00Z", "seq-b", "three.mov"));
    fs::write(&path, Value::Array(changed).to_string()).expect("the history file");
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::AlreadyImported
    );
    assert_eq!(store.rendered_videos().expect("rows").len(), 2);
}

#[test]
fn import_parses_names() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let path = tmp.path().join("render_history.json");
    let names = [
        "seq-a_ProRes-yuv422p10le_Rec.709F_1080p_25_HQ.mov",
        "seq-b_ProRes-yuv422p10le_Rec.709F_OriRes_24_UHQ.mov",
        "seq-c_MyCodec_720w_30.mov",
        "final.mov",
    ];
    let entries: Vec<Value> = names
        .iter()
        .map(|name| history_entry("2023-11-14T22:00:00Z", "seq-a", name))
        .collect();
    fs::write(&path, Value::Array(entries).to_string()).expect("the history file");

    let status = import_swift_history(&path, &store, 30, NOW).expect("import");
    assert_eq!(
        status,
        ImportStatus::Imported(ImportReport {
            imported: 4,
            too_old: 0,
            invalid: 0,
        })
    );

    let rows = store.rendered_videos().expect("rows");
    let row = |name: &str| {
        rows.iter()
            .find(|row| row.output_filename == name)
            .unwrap_or_else(|| panic!("a row for {name}"))
    };

    let known = row(names[0]);
    assert_eq!(known.codec, "ProRes");
    assert_eq!(known.resolution_label, "1080p");
    assert_eq!(known.frame_rate, 25);

    let original = row(names[1]);
    assert_eq!(original.codec, "ProRes");
    assert_eq!(original.resolution_label, "OriRes");
    assert_eq!(original.frame_rate, 24);

    let unknown = row(names[2]);
    assert_eq!(unknown.codec, "MyCodec");
    assert_eq!(unknown.resolution_label, "720w");
    assert_eq!(unknown.frame_rate, 30);

    // A name that matches no §6.5 pattern keeps its variant fields empty.
    let other = row(names[3]);
    assert_eq!(other.codec, "");
    assert_eq!(other.resolution_label, "");
    assert_eq!(other.frame_rate, 0);

    // The rest of an imported row: the Swift values, the source and the unknown render time. The
    // entries are dated 2023-11-14T22:00:00Z.
    assert_eq!(known.rendered_at, 1_699_999_200_000);
    assert_eq!(known.variant_key, "");
    assert_eq!(known.frame_count, 48);
    assert_eq!(known.size_bytes, 1234);
    assert_eq!(known.render_seconds, -1.0);
    assert_eq!(known.source, SOURCE_IMPORTED);
    assert_eq!(known.thumbnail_path, None);
}

#[test]
fn thumbnail_args_match_spec() {
    let args = thumbnail_args(
        Path::new("/d/render.mov"),
        10.0,
        Path::new("/d/thumbnails/render-1.jpg"),
    );
    let expected = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-ss",
        "5.000",
        "-i",
        "/d/render.mov",
        "-frames:v",
        "1",
        "-vf",
        "scale=480:-2",
        "-q:v",
        "3",
        "/d/thumbnails/render-1.jpg",
    ];
    assert_eq!(args, expected);
}

#[test]
fn thumbnail_args_first_frame_without_duration() {
    // An imported row has no duration, so its poster frame is the first frame (a deliberate
    // deviation from duration/2, recorded in docs/decisions/1009 and on thumbnail_args).
    let args = thumbnail_args(
        Path::new("/d/render.mov"),
        0.0,
        Path::new("/d/thumbnails/render-1.jpg"),
    );
    assert_eq!(args[6], "0.000");
}

#[test]
fn default_history_path_is_macos_only() {
    let home = PathBuf::from("/Users/tester");
    assert_eq!(
        default_history_path(&home, "macos"),
        Some(home.join("Library/Application Support/TimelapseRenderDaemon/render_history.json"))
    );
    assert_eq!(default_history_path(&home, "linux"), None);
}

#[test]
fn thumbnail_no_file_is_failure() {
    // ffmpeg exits 0 but writes nothing: the row gets no thumbnail and a WARN row is logged.
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let render = video(tmp.path(), "render-empty", NOW);
    fs::write(&render.output_path, b"render").expect("the rendered file");
    store.put_rendered_video(&render).expect("store the row");
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let runner: Runner = Box::new(move |_args| {
        counted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let thumbnailer = Thumbnailer::new(&data, runner);

    assert!(!thumbnailer.make(&store, &render, NOW).expect("make"));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "ffmpeg ran once");
    assert!(!thumbnailer.path_for("render-empty").exists());
    let stored = store
        .rendered_video("render-empty")
        .expect("read the row")
        .expect("the row exists");
    assert_eq!(stored.thumbnail_path, None, "has_thumbnail stays false");

    let rows = store.log_rows_after(0).expect("the log");
    let warning = rows
        .iter()
        .find(|row| row.fields.event.as_deref() == Some("thumbnail_failed"))
        .expect("a thumbnail_failed row");
    assert_eq!(warning.level, LogLevel::Warn);
    assert!(
        warning.message.contains("wrote no file"),
        "{}",
        warning.message
    );
}

#[test]
fn thumbnail_on_demand_regenerates_missing_file() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let calls = Arc::new(AtomicUsize::new(0));
    let thumbnailer = Thumbnailer::new(&data, writing_runner(Arc::clone(&calls)));

    // The row records a thumbnail whose file has since been deleted; its render file exists.
    let mut render = video(tmp.path(), "imported-c", NOW);
    fs::write(&render.output_path, b"render").expect("the rendered file");
    let gone = thumbnailer.path_for("imported-c");
    render.thumbnail_path = Some(gone.to_string_lossy().into_owned());
    store.put_rendered_video(&render).expect("store the row");
    assert!(!gone.exists());

    let made = ensure_thumbnail(&store, &thumbnailer, "imported-c", NOW).expect("ensure");
    assert_eq!(made, Some(gone.clone()));
    assert!(gone.is_file(), "the thumbnail is written again");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "ffmpeg ran once");

    // Now the file exists: a later call returns it and runs nothing.
    let again = ensure_thumbnail(&store, &thumbnailer, "imported-c", NOW).expect("ensure");
    assert_eq!(again, Some(gone));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no second run");
}

#[test]
fn thumbnail_delete_failed_warns() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let thumbs = data.join(THUMBNAILS_DIR);
    fs::create_dir_all(&thumbs).expect("the thumbnails folder");

    // A thumbnail path that is a folder: deleting it fails with something other than NotFound on
    // every OS. (A path under a regular file reports NotFound on Windows, which is not a failure.)
    let stuck_path = tmp.path().join("stuck.jpg");
    fs::create_dir_all(&stuck_path).expect("a folder where the thumbnail should be");
    let mut stuck = video(tmp.path(), "stuck", NOW - 31 * DAY_MS);
    stuck.thumbnail_path = Some(stuck_path.to_string_lossy().into_owned());
    store.put_rendered_video(&stuck).expect("store the row");

    // A second expired row whose thumbnail can be deleted: the failure above does not stop it.
    let mut removable = video(tmp.path(), "removable", NOW - 31 * DAY_MS);
    let thumb = thumbs.join("removable.jpg");
    fs::write(&thumb, b"jpeg").expect("the thumbnail");
    removable.thumbnail_path = Some(thumb.to_string_lossy().into_owned());
    store.put_rendered_video(&removable).expect("store the row");

    let deleted = prune_history(&store, NOW, 30).expect("retention");
    assert_eq!(deleted, 1, "only the deletable file counts");
    assert!(!thumb.exists());
    assert!(
        store.rendered_videos().expect("rows").is_empty(),
        "both expired rows are removed"
    );

    let rows = store.log_rows_after(0).expect("the log");
    let warning = rows
        .iter()
        .find(|row| row.fields.event.as_deref() == Some("thumbnail_delete_failed"))
        .expect("a thumbnail_delete_failed row");
    assert_eq!(warning.level, LogLevel::Warn);
    assert!(warning.message.contains("stuck.jpg"), "{}", warning.message);
}

#[test]
fn import_unreadable_retries() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let path = tmp.path().join("render_history.json");

    // Not JSON: a WARN row, and the setting stays unset.
    fs::write(&path, "not json").expect("the history file");
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::Unreadable
    );
    assert!(!store.swift_history_imported().expect("setting"));
    assert!(store.rendered_videos().expect("rows").is_empty());
    let rows = store.log_rows_after(0).expect("the log");
    let warning = rows
        .iter()
        .find(|row| row.fields.event.as_deref() == Some("history_import_failed"))
        .expect("a history_import_failed row");
    assert_eq!(warning.level, LogLevel::Warn);

    // JSON that is not an array: unreadable too.
    fs::write(&path, r#"{"timestamp": "2023-11-14T22:00:00Z"}"#).expect("the history file");
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::Unreadable
    );

    // A folder where the file should be: it cannot be read either.
    fs::remove_file(&path).expect("remove the file");
    fs::create_dir(&path).expect("a folder in its place");
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::Unreadable
    );
    assert!(!store.swift_history_imported().expect("setting"));

    // Once the file is readable, a later start imports it.
    fs::remove_dir(&path).expect("remove the folder");
    fs::write(&path, history_fixture()).expect("the history file");
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::Imported(ImportReport {
            imported: 2,
            too_old: 1,
            invalid: 1,
        })
    );
    assert!(store.swift_history_imported().expect("setting"));
}

#[test]
fn retention_keeps_row_at_cutoff() {
    // The cutoff is 30 days before NOW: 2023-10-15T22:13:20Z. Rows dated exactly at it are kept
    // by both the import and the prune; one millisecond older is dropped by both.
    let cutoff = NOW - 30 * DAY_MS;
    assert_eq!(cutoff, 1_697_408_000_000);

    let tmp = TempDir::new().expect("a temporary directory");
    let store = open_store(tmp.path());
    let path = tmp.path().join("render_history.json");
    let entries = json!([
        history_entry("2023-10-15T22:13:20Z", "seq-a", "at-cutoff.mov"),
        history_entry("2023-10-15T22:13:19.999Z", "seq-a", "before-cutoff.mov"),
    ]);
    fs::write(&path, entries.to_string()).expect("the history file");
    assert_eq!(
        import_swift_history(&path, &store, 30, NOW).expect("import"),
        ImportStatus::Imported(ImportReport {
            imported: 1,
            too_old: 1,
            invalid: 0,
        })
    );
    let imported = store.rendered_videos().expect("rows");
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].output_filename, "at-cutoff.mov");
    assert_eq!(imported[0].rendered_at, cutoff);

    let prune_store_dir = TempDir::new().expect("a temporary directory");
    let prune_store = open_store(prune_store_dir.path());
    let kept = video(prune_store_dir.path(), "at-cutoff", cutoff);
    let dropped = video(prune_store_dir.path(), "before-cutoff", cutoff - 1);
    prune_store
        .put_rendered_video(&kept)
        .expect("store the row");
    prune_store
        .put_rendered_video(&dropped)
        .expect("store the row");
    assert_eq!(
        prune_history(&prune_store, NOW, 30).expect("retention"),
        0,
        "no thumbnail file was there to delete"
    );
    let ids: Vec<String> = prune_store
        .rendered_videos()
        .expect("list the rows")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(ids, ["at-cutoff"]);
}
