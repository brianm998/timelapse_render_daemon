// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Thumbnails (§6.16), the Swift history import (§6.17) and the history retention that deletes
//! thumbnails (§6.13). Every database and fixture lives in a temporary directory; the fake ffmpeg
//! comes from trd-testtools, and no test touches the owner's data or a port.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
use trd_core::tools::ToolCommand;
use trd_testtools::FakeTools;

/// 2023-11-14T22:13:20Z in milliseconds.
const NOW: i64 = 1_700_000_000_000;
/// How long a fake thumbnail stays in flight, so that overlapping runs would show.
const HOLD: Duration = Duration::from_millis(20);

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

/// A runner that runs the fake ffmpeg with `script` in its environment for that one command.
fn fake_runner(tools: &FakeTools, script: Value) -> Runner {
    let ffmpeg = tools.path().join("ffmpeg");
    Box::new(move |args| {
        let output = Command::new(&ffmpeg)
            .env("TRD_FAKE_FFMPEG", script.to_string())
            .args(args)
            .output()
            .map_err(|err| err.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "exit {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    })
}

/// A runner that writes the output file and counts its calls.
fn writing_runner(calls: Arc<AtomicUsize>) -> Runner {
    Box::new(move |args| {
        calls.fetch_add(1, Ordering::SeqCst);
        fs::write(output_of(args), b"jpeg").map_err(|err| err.to_string())
    })
}

#[test]
fn thumbnail_written() {
    let tmp = TempDir::new().expect("a temporary directory");
    let tools = FakeTools::new().expect("the fake tools");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let render = video(tmp.path(), "render-1", NOW);
    fs::write(&render.output_path, b"render").expect("the rendered file");
    store.put_rendered_video(&render).expect("store the row");

    // The real command path: the fake ffmpeg writes the poster frame it is asked for.
    let thumbnailer =
        Thumbnailer::with_tool(&data, ToolCommand::direct(&tools.path().join("ffmpeg")));
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

#[test]
fn thumbnail_failure() {
    let tmp = TempDir::new().expect("a temporary directory");
    let tools = FakeTools::new().expect("the fake tools");
    let store = open_store(tmp.path());
    let data = tmp.path().join("data");
    let render = video(tmp.path(), "render-2", NOW);
    fs::write(&render.output_path, b"render").expect("the rendered file");
    store.put_rendered_video(&render).expect("store the row");

    // The fake writes its output and then exits 1: the partial file must not survive.
    let script = json!({"exit_code": 1, "stderr_lines": ["Invalid data found"]});
    let thumbnailer = Thumbnailer::new(&data, fake_runner(&tools, script));
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

    let shared = Arc::clone(&counters);
    let runner: Runner = Box::new(move |args| {
        shared.calls.fetch_add(1, Ordering::SeqCst);
        let now = shared.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        shared.peak.fetch_max(now, Ordering::SeqCst);
        // Stay in flight a moment so that overlapping runs would be seen. The gate alone keeps
        // the peak at two; this sleep is not an assertion on timing.
        thread::sleep(HOLD);
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

    thread::scope(|scope| {
        let handles: Vec<_> = videos
            .iter()
            .map(|render| scope.spawn(|| thumbnailer.make(&store, render, NOW)))
            .collect();
        for handle in handles {
            assert!(handle.join().expect("no panic").expect("make"));
        }
    });

    assert_eq!(counters.calls.load(Ordering::SeqCst), 6);
    assert_eq!(
        counters.peak.load(Ordering::SeqCst),
        2,
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
fn default_history_path_is_macos_only() {
    let home = PathBuf::from("/Users/tester");
    assert_eq!(
        default_history_path(&home, "macos"),
        Some(home.join("Library/Application Support/TimelapseRenderDaemon/render_history.json"))
    );
    assert_eq!(default_history_path(&home, "linux"), None);
}
