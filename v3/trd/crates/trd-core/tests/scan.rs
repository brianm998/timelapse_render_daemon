// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Sequence scanner tests (§6.3) over a fake filesystem with an injected clock. No test sleeps:
//! the only real time is the path timeout in the two tests that hang a listing on purpose.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use trd_core::config::Config;
use trd_core::paths::ManualClock;
use trd_core::scan::{DirItem, PROBLEM_PATH_UNREACHABLE, RealFs, ScanFs, Scanner};

/// Blocks a listing until the test releases it.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

#[derive(Clone)]
struct FakeDir {
    mtime: SystemTime,
    items: Vec<DirItem>,
    size: u64,
}

/// An in-memory tree that counts listings and size calls.
#[derive(Default)]
struct Fake {
    dirs: Mutex<BTreeMap<String, FakeDir>>,
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    lists: Mutex<HashMap<String, usize>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    size_calls: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
}

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

impl Fake {
    fn dir(&self, path: &str) {
        self.dirs
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_insert(FakeDir {
                mtime: at(1),
                items: Vec::new(),
                size: 0,
            });
    }

    /// Adds an entry to `dir` and bumps its mtime, as a real change would.
    fn child(&self, dir: &str, name: &str, is_dir: bool, is_regular_file: bool) {
        let mut dirs = self.dirs.lock().unwrap();
        let d = dirs.get_mut(dir).expect("parent directory exists");
        d.items.push(DirItem {
            name: name.to_owned(),
            is_dir,
            is_regular_file,
        });
        d.mtime += Duration::from_secs(1);
    }

    fn subdir(&self, dir: &str, name: &str) {
        self.child(dir, name, true, false);
        self.dir(&format!("{dir}/{name}"));
    }

    fn file(&self, dir: &str, name: &str) {
        self.child(dir, name, false, true);
    }

    /// An entry that is neither a directory nor a regular file (a FIFO, say).
    fn odd(&self, dir: &str, name: &str) {
        self.child(dir, name, false, false);
    }

    /// A JSON backup sidecar.
    fn json(&self, dir: &str, name: &str, body: &str) {
        self.child(dir, name, false, true);
        self.files
            .lock()
            .unwrap()
            .insert(format!("{dir}/{name}"), body.as_bytes().to_vec());
    }

    fn touch(&self, dir: &str) {
        let mut dirs = self.dirs.lock().unwrap();
        dirs.get_mut(dir).expect("directory exists").mtime += Duration::from_secs(1);
    }

    fn set_size(&self, dir: &str, bytes: u64) {
        let mut dirs = self.dirs.lock().unwrap();
        dirs.get_mut(dir).expect("directory exists").size = bytes;
    }

    /// Makes listings of `dir` block until the returned gate is released.
    fn block(&self, dir: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        self.gates
            .lock()
            .unwrap()
            .insert(dir.to_owned(), gate.clone());
        gate
    }

    fn lists_of(&self, dir: &str) -> usize {
        self.lists.lock().unwrap().get(dir).copied().unwrap_or(0)
    }
}

impl ScanFs for Fake {
    fn list(&self, dir: &Path) -> io::Result<Vec<DirItem>> {
        let k = key(dir);
        *self.lists.lock().unwrap().entry(k.clone()).or_insert(0) += 1;
        let gate = self.gates.lock().unwrap().get(&k).cloned();
        if let Some(gate) = gate {
            gate.wait();
        }
        let listing = {
            let dirs = self.dirs.lock().unwrap();
            dirs.get(&k).map(|d| d.items.clone())
        };
        listing.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, k))
    }

    fn mtime(&self, dir: &Path) -> io::Result<SystemTime> {
        self.dirs
            .lock()
            .unwrap()
            .get(&key(dir))
            .map(|d| d.mtime)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn size(&self, dir: &Path) -> io::Result<u64> {
        self.size_calls.fetch_add(1, Ordering::SeqCst);
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let size = self
            .dirs
            .lock()
            .unwrap()
            .get(&key(dir))
            .map(|d| d.size)
            .unwrap_or(0);
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(size)
    }

    fn read(&self, file: &Path) -> io::Result<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(&key(file))
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn config(sources: &[String], exports: &[String]) -> Config {
    let mut cfg = Config::default();
    cfg.source_paths = sources.to_vec();
    cfg.export_paths = exports.to_vec();
    cfg
}

fn setup() -> (Arc<Fake>, Arc<ManualClock>) {
    (Arc::new(Fake::default()), Arc::new(ManualClock::new(2026)))
}

fn scanner(fake: &Arc<Fake>, clock: &Arc<ManualClock>) -> Scanner {
    Scanner::new(fake.clone(), clock.clone())
}

#[tokio::test]
async fn longest_match_wins() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "A");
    fake.file("/fake/src/A", "a1.ARW");
    fake.subdir("/fake/src", "A_B");
    fake.file("/fake/src/A_B", "b1.ARW");
    fake.file("/fake/src/A_B", "b2.ARW");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "A_B_render");
    fake.file("/fake/exp/A_B_render", "f.tif");
    fake.subdir("/fake/exp", "A_render");
    fake.file("/fake/exp/A_render", "f.tif");

    let result = scanner(&fake, &clock)
        .scan(&config(&strings(&["/fake/src"]), &strings(&["/fake/exp"])))
        .await;

    let find = |name: &str| {
        result
            .exports
            .iter()
            .find(|e| e.name == name)
            .expect("export found")
            .clone()
    };
    let long = find("A_B_render");
    assert_eq!(long.raw_name, "A_B");
    assert_eq!(long.raw_frames, 2);
    assert_eq!(find("A_render").raw_name, "A");
    assert!(result.unreachable.is_empty());
}

#[tokio::test]
async fn slow_path_does_not_block() {
    let (fake, clock) = setup();
    fake.dir("/fake/slow");
    let gate = fake.block("/fake/slow");
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "R1");
    fake.file("/fake/src/R1", "r.ARW");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "R1_x");
    fake.file("/fake/exp/R1_x", "x.tif");

    let scanner = scanner(&fake, &clock).with_path_timeout(Duration::from_secs(1));
    let result = scanner
        .scan(&config(
            &strings(&["/fake/src", "/fake/slow"]),
            &strings(&["/fake/exp"]),
        ))
        .await;

    assert_eq!(result.unreachable.len(), 1);
    assert_eq!(result.unreachable[0].path, "/fake/slow");
    assert_eq!(result.unreachable[0].code, PROBLEM_PATH_UNREACHABLE);
    assert!(result.raw.iter().any(|r| r.name == "R1" && r.frames == 1));
    assert!(result.exports.iter().any(|e| e.name == "R1_x"));
    // The scan returned while the hung listing is still blocked.
    assert_eq!(scanner.in_flight_count(), 1);
    gate.release();
}

#[tokio::test]
async fn in_flight_path_not_relisted() {
    let (fake, clock) = setup();
    fake.dir("/fake/slow");
    fake.subdir("/fake/slow", "R");
    fake.file("/fake/slow/R", "r.ARW");
    let gate = fake.block("/fake/slow");

    let scanner = scanner(&fake, &clock).with_path_timeout(Duration::from_secs(1));
    let cfg = config(&strings(&["/fake/slow"]), &[]);

    let first = scanner.scan(&cfg).await;
    assert_eq!(first.unreachable.len(), 1);
    assert_eq!(fake.lists_of("/fake/slow"), 1);

    let second = scanner.scan(&cfg).await;
    assert_eq!(second.unreachable.len(), 1);
    assert!(second.unreachable[0].reason.contains("previous listing"));
    assert_eq!(
        fake.lists_of("/fake/slow"),
        1,
        "in-flight path was listed again"
    );

    gate.release();
    let mut spins = 0usize;
    while scanner.in_flight_count() != 0 {
        spins += 1;
        assert!(spins < 1_000_000, "blocked listing never returned");
        tokio::task::yield_now().await;
    }

    let third = scanner.scan(&cfg).await;
    assert!(third.unreachable.is_empty());
    assert!(third.raw.iter().any(|r| r.name == "R"));
    assert_eq!(fake.lists_of("/fake/slow"), 2);
}

#[tokio::test]
async fn backup_zero_skipped() {
    let (fake, clock) = setup();
    fake.dir("/fake/bak");
    fake.json("/fake/bak", "a.json", r#"{"ImageSize":{"5472x3648":0}}"#);
    fake.json("/fake/bak", "b.json", r#"{"number_source_images":7}"#);
    fake.json(
        "/fake/bak",
        "c.json",
        r#"{"ImageSize":{"1x1":4},"number_source_images":9}"#,
    );
    fake.json("/fake/bak", "x.y.json", r#"{"number_source_images":3}"#);
    fake.file("/fake/bak", "e.txt");

    let scanner = scanner(&fake, &clock);
    scanner.refresh_backups(&strings(&["/fake/bak"])).await;
    let result = scanner.scan(&config(&[], &[])).await;

    let found: Vec<(&str, u64)> = result
        .raw
        .iter()
        .map(|r| (r.name.as_str(), r.frames))
        .collect();
    assert_eq!(found, vec![("b", 7), ("c", 4)]);
    assert!(result.raw.iter().all(|r| r.from_backup));
}

#[tokio::test]
async fn source_overrides_backup() {
    let (fake, clock) = setup();
    fake.dir("/fake/bak");
    fake.json("/fake/bak", "S1.json", r#"{"number_source_images":5}"#);
    fake.json("/fake/bak", "S2.json", r#"{"number_source_images":4}"#);
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "S1");
    fake.file("/fake/src/S1", "1.ARW");
    fake.file("/fake/src/S1", "2.ARW");

    let scanner = scanner(&fake, &clock);
    scanner.refresh_backups(&strings(&["/fake/bak"])).await;
    let result = scanner.scan(&config(&strings(&["/fake/src"]), &[])).await;

    let s1: Vec<_> = result.raw.iter().filter(|r| r.name == "S1").collect();
    assert_eq!(s1.len(), 1);
    assert_eq!(s1[0].frames, 2);
    assert!(!s1[0].from_backup);
    let s2: Vec<_> = result.raw.iter().filter(|r| r.name == "S2").collect();
    assert_eq!(s2.len(), 1);
    assert!(s2[0].from_backup);
}

#[tokio::test]
async fn export_rate() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "E");
    for i in 0..120 {
        fake.file("/fake/src/E", &format!("f{i}.ARW"));
    }
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "E_1");
    for i in 0..10 {
        fake.file("/fake/exp/E_1", &format!("o{i}.tif"));
    }
    let scanner = scanner(&fake, &clock);
    let cfg = config(&strings(&["/fake/src"]), &strings(&["/fake/exp"]));

    let first = scanner.scan(&cfg).await;
    let e = &first.exports[0];
    assert_eq!(e.frames, 10);
    assert_eq!(e.export_frames_per_minute, 0.0, "one sample has no rate");
    assert_eq!(e.export_eta_seconds, -1);

    clock.advance(Duration::from_secs(60));
    for i in 10..60 {
        fake.file("/fake/exp/E_1", &format!("o{i}.tif"));
    }
    let second = scanner.scan(&cfg).await;
    let e = &second.exports[0];
    assert_eq!(e.frames, 60);
    assert!((e.export_frames_per_minute - 50.0).abs() < 1e-9);
    // 60 frames remain at 50 per minute: 72 seconds.
    assert_eq!(e.export_eta_seconds, 72);
    assert!(e.in_progress);
}

#[tokio::test]
async fn ignore_regexes() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "R");
    fake.file("/fake/src/R", "r.ARW");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "R_skip_1");
    fake.file("/fake/exp/R_skip_1", "o.tif");
    fake.subdir("/fake/exp", "R_1");
    fake.file("/fake/exp/R_1", "o.tif");

    let mut cfg = config(&strings(&["/fake/src"]), &strings(&["/fake/exp"]));
    cfg.ignore_sequence_regexes = strings(&["skip"]);
    let result = scanner(&fake, &clock).scan(&cfg).await;

    let names: Vec<&str> = result.exports.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["R_1"]);
}

#[tokio::test]
async fn hidden_and_irregular_files() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", ".hidden");
    fake.file("/fake/src/.hidden", "h0.ARW");
    fake.subdir("/fake/src", "H");
    fake.file("/fake/src/H", "h1.ARW");
    fake.odd("/fake/src/H", "bad.ARW");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", ".H_hidden");
    fake.file("/fake/exp/.H_hidden", "o.tif");
    fake.subdir("/fake/exp", "H_1");
    fake.file("/fake/exp/H_1", "a.tif");
    fake.file("/fake/exp/H_1", ".hidden.tif");
    fake.odd("/fake/exp/H_1", "fifo.tif");

    let result = scanner(&fake, &clock)
        .scan(&config(&strings(&["/fake/src"]), &strings(&["/fake/exp"])))
        .await;

    let raw: Vec<(&str, u64)> = result
        .raw
        .iter()
        .map(|r| (r.name.as_str(), r.frames))
        .collect();
    assert_eq!(raw, vec![("H", 1)]);
    let exports: Vec<(&str, u64)> = result
        .exports
        .iter()
        .map(|e| (e.name.as_str(), e.frames))
        .collect();
    assert_eq!(exports, vec![("H_1", 1)]);
}

#[tokio::test]
async fn extension_case() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "E");
    fake.file("/fake/src/E", "a.arw");
    fake.file("/fake/src/E", "b.Arw");
    fake.file("/fake/src/E", "c.DNG");
    fake.file("/fake/src/E", "d.txt");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "E_1");
    fake.file("/fake/exp/E_1", "1.TIF");
    fake.file("/fake/exp/E_1", "2.Tiff");
    fake.file("/fake/exp/E_1", "3.jpg");

    let result = scanner(&fake, &clock)
        .scan(&config(&strings(&["/fake/src"]), &strings(&["/fake/exp"])))
        .await;

    assert_eq!(result.raw[0].frames, 3);
    assert_eq!(result.exports[0].frames, 2);
}

#[tokio::test]
async fn zero_frames_ignored() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "Z");
    fake.file("/fake/src/Z", "a.ARW");
    fake.file("/fake/src/Z", "b.ARW");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "Z_0");
    fake.file("/fake/exp/Z_0", "a.ARW");
    fake.subdir("/fake/exp", "Z_1");
    fake.file("/fake/exp/Z_1", "a.tif");

    let result = scanner(&fake, &clock)
        .scan(&config(&strings(&["/fake/src"]), &strings(&["/fake/exp"])))
        .await;

    let names: Vec<&str> = result.exports.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Z_1"]);
}

#[tokio::test]
async fn mtime_cache() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    fake.subdir("/fake/src", "M");
    fake.file("/fake/src/M", "m.ARW");
    fake.dir("/fake/exp");
    fake.subdir("/fake/exp", "M_1");
    fake.file("/fake/exp/M_1", "m.tif");
    let scanner = scanner(&fake, &clock);
    let cfg = config(&strings(&["/fake/src"]), &strings(&["/fake/exp"]));
    let raw_dir = "/fake/src/M";
    let export_dir = "/fake/exp/M_1";

    scanner.scan(&cfg).await;
    assert_eq!(fake.lists_of(raw_dir), 1);
    assert_eq!(fake.lists_of(export_dir), 1);

    // Same instant, unchanged mtimes: both reused.
    scanner.scan(&cfg).await;
    assert_eq!(fake.lists_of(raw_dir), 1);
    assert_eq!(fake.lists_of(export_dir), 1);

    // 60 s: the export is recounted, the raw directory is not (10 min).
    clock.advance(Duration::from_secs(60));
    scanner.scan(&cfg).await;
    assert_eq!(fake.lists_of(raw_dir), 1);
    assert_eq!(fake.lists_of(export_dir), 2);

    // 600 s: both are due.
    clock.advance(Duration::from_secs(540));
    scanner.scan(&cfg).await;
    assert_eq!(fake.lists_of(raw_dir), 2);
    assert_eq!(fake.lists_of(export_dir), 3);

    // A changed raw mtime relists at once; the export counted at 600 s is still fresh.
    fake.touch(raw_dir);
    scanner.scan(&cfg).await;
    assert_eq!(fake.lists_of(raw_dir), 3);
    assert_eq!(fake.lists_of(export_dir), 3);
}

#[tokio::test]
async fn year_paths_scanned() {
    let root = tempfile::tempdir().expect("temp dir");
    let base = root.path().to_string_lossy().into_owned();
    let src = root.path().join("2026").join("src").join("Y1");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.ARW"), b"raw").unwrap();
    let exp = root.path().join("2026").join("exp").join("Y1_1");
    std::fs::create_dir_all(&exp).unwrap();
    std::fs::write(exp.join("a.tif"), b"img").unwrap();

    let cfg = config(
        &[format!("{base}/$YEAR/src")],
        &[format!("{base}/$YEAR/exp")],
    );
    let scanner = Scanner::new(Arc::new(RealFs), Arc::new(ManualClock::new(2026)));
    let result = scanner.scan(&cfg).await;

    assert!(result.unreachable.is_empty(), "{:?}", result.unreachable);
    assert!(result.raw.iter().any(|r| r.name == "Y1" && r.frames == 1));
    assert!(
        result
            .exports
            .iter()
            .any(|e| e.name == "Y1_1" && e.frames == 1 && e.raw_name == "Y1")
    );
}

#[tokio::test]
async fn export_size_measurement() {
    let (fake, clock) = setup();
    fake.dir("/fake/src");
    for raw in ["S1", "S2", "S3", "C"] {
        fake.subdir("/fake/src", raw);
        let frames = if raw == "C" { 1 } else { 3 };
        for i in 0..frames {
            fake.file(&format!("/fake/src/{raw}"), &format!("f{i}.ARW"));
        }
    }
    fake.dir("/fake/exp");
    for name in ["S1_x", "S2_x", "S3_x", "C_x"] {
        fake.subdir("/fake/exp", name);
        fake.file(&format!("/fake/exp/{name}"), "o.tif");
        fake.set_size(&format!("/fake/exp/{name}"), 4096);
    }
    let scanner = scanner(&fake, &clock);
    let cfg = config(&strings(&["/fake/src"]), &strings(&["/fake/exp"]));

    let first = scanner.scan(&cfg).await;
    assert!(first.exports.iter().all(|e| e.export_size == -1));

    scanner.measure_sizes().await;
    assert_eq!(
        fake.size_calls.load(Ordering::SeqCst),
        3,
        "only in-progress sequences"
    );
    assert!(fake.peak.load(Ordering::SeqCst) <= 2);

    let measured = scanner.scan(&cfg).await;
    let size_of = |name: &str| {
        measured
            .exports
            .iter()
            .find(|e| e.name == name)
            .expect("export found")
            .export_size
    };
    assert_eq!(size_of("S1_x"), 4096);
    assert_eq!(size_of("S3_x"), 4096);
    assert_eq!(size_of("C_x"), -1, "complete sequences are never measured");

    scanner.measure_sizes().await;
    assert_eq!(
        fake.size_calls.load(Ordering::SeqCst),
        3,
        "measured again within 60 s"
    );

    clock.advance(Duration::from_secs(60));
    scanner.scan(&cfg).await;
    scanner.measure_sizes().await;
    assert_eq!(fake.size_calls.load(Ordering::SeqCst), 6);
    assert!(fake.peak.load(Ordering::SeqCst) <= 2);
}

#[tokio::test]
async fn backup_loaded_and_refreshed() {
    let (fake, clock) = setup();
    fake.dir("/fake/bak");
    fake.json("/fake/bak", "B.json", r#"{"number_source_images":3}"#);
    let scanner = scanner(&fake, &clock);

    assert!(scanner.backups_due(), "never loaded");
    scanner.refresh_backups(&strings(&["/fake/bak"])).await;
    assert!(!scanner.backups_due());
    let result = scanner.scan(&config(&[], &[])).await;
    assert_eq!(result.raw.len(), 1);
    assert_eq!(result.raw[0].name, "B");

    clock.advance(Duration::from_secs(599));
    assert!(!scanner.backups_due());
    clock.advance(Duration::from_secs(1));
    assert!(scanner.backups_due(), "due 10 min after the last load");
}
