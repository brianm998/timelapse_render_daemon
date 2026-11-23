// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! EXIF collection, sidecars and metadata (§6.4, TRD-09). Every fixture lives in a temporary
//! directory; the store is a temporary database; no test reads the owner's data or uses a daemon
//! port. The real-process test runs the fake exiftool from trd-testtools, never a real one.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use trd_core::exif::{
    self, Histogram, SidecarOutcome, collect, collect_with, metadata, read_histogram, sidecar_path,
    write_sidecar,
};
use trd_core::store::Store;
use trd_core::tools::{EXIFTOOL, ResolvedTool, ToolInfo};

const NOW_MS: i64 = 1_790_000_000_000;

/// A store in `dir/db/trd.sqlite3`.
fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("db").join("trd.sqlite3")).expect("the store opens")
}

/// The names of `count` raw files: `IMG_0000.ARW`, `IMG_0001.ARW`, …
fn raw_names(count: usize) -> Vec<PathBuf> {
    (0..count)
        .map(|i| PathBuf::from(format!("IMG_{i:04}.ARW")))
        .collect()
}

/// The CSV that exiftool prints for `files`: a `SourceFile,Make` header and one row per file.
fn canned_csv(files: &[PathBuf]) -> String {
    let mut text = String::from("SourceFile,Make\n");
    for file in files {
        text.push_str(&format!("{},Canon\n", file.display()));
    }
    text
}

/// A small histogram with one single-valued tag, one multi-valued tag and one sorted column set.
fn sample_histogram() -> Histogram {
    let mut histogram = Histogram::new();
    histogram.insert("Make".to_owned(), BTreeMap::from([("Canon".to_owned(), 3)]));
    histogram.insert(
        "Model".to_owned(),
        BTreeMap::from([("EOS R5".to_owned(), 2), ("EOS R6".to_owned(), 1)]),
    );
    histogram.insert(
        "Lens".to_owned(),
        BTreeMap::from([("EF 50mm".to_owned(), 3)]),
    );
    histogram
}

/// The Swift daemon's sidecar layout: `{tag: {value: count}}`, pretty, sorted keys.
const SWIFT_SIDECAR: &str = r#"{
  "Lens": {
    "EF 50mm f/1.8": 2
  },
  "Make": {
    "Canon": 2
  },
  "Model": {
    "EOS 5D": 1,
    "EOS 6D": 1
  },
  "SourceFile": {
    "/raw/IMG_0001.CR2": 1,
    "/raw/IMG_0002.CR2": 1
  }
}
"#;

#[test]
fn batches_and_concurrency() {
    let files = raw_names(130);
    let split = exif::batches(&files);
    let sizes: Vec<usize> = split.iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![64, 64, 2]);

    for max in [1usize, 3] {
        let in_flight = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let seen = Mutex::new(Vec::new());
        let histogram = collect_with(&split, max, |batch| {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            seen.lock().unwrap().push(batch.len());
            let text = canned_csv(batch);
            in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(text)
        })
        .expect("the batches collect");

        assert!(
            peak.load(Ordering::SeqCst) <= max,
            "at most {max} batches at once"
        );
        let mut run_sizes = seen.into_inner().unwrap();
        run_sizes.sort_unstable();
        assert_eq!(run_sizes, vec![2, 64, 64], "each batch ran once");
        assert_eq!(histogram["SourceFile"].len(), 130);
        assert_eq!(histogram["Make"]["Canon"], 130);
    }
}

#[test]
fn skip_rules() {
    let csv = "\
SourceFile,ThumbnailImage,Thumbnail Image,PreviewImage,Preview Image,TiffMeteringImage,Tiff Metering Image,Make,Lens,Comment,Notes
/raw/a.ARW,(Binary data 10 kB),(Binary data 20 kB),(Binary data 1 B),(Binary data 2 B),(Binary data 3 B),(Binary data 4 B),Canon,EF 50mm,(Binary data 5 bytes),
/raw/b.ARW,(Binary data 11 kB),,(Binary data 1 B),,,,Canon,EF 50mm,hello,
";
    let mut histogram = Histogram::new();
    exif::parse_csv(csv, &mut histogram).expect("the table parses");

    assert_eq!(
        histogram["SourceFile"],
        BTreeMap::from([("/raw/a.ARW".to_owned(), 1), ("/raw/b.ARW".to_owned(), 1)]),
        "SourceFile is kept"
    );
    for skipped in [
        "ThumbnailImage",
        "Thumbnail Image",
        "PreviewImage",
        "Preview Image",
        "TiffMeteringImage",
        "Tiff Metering Image",
    ] {
        assert!(!histogram.contains_key(skipped), "{skipped} is skipped");
    }
    assert_eq!(histogram["Make"], BTreeMap::from([("Canon".to_owned(), 2)]));
    assert_eq!(
        histogram["Lens"],
        BTreeMap::from([("EF 50mm".to_owned(), 2)])
    );
    assert_eq!(
        histogram["Comment"],
        BTreeMap::from([("hello".to_owned(), 1)]),
        "a binary value in any column is skipped"
    );
    assert!(
        !histogram.contains_key("Notes"),
        "empty cells are not counted"
    );
}

#[test]
fn cache_fallback() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    let store = open_store(tmp.path());
    let histogram = sample_histogram();

    // A directory where the sidecar belongs makes the rename fail, on every OS and as root.
    let sidecar = sidecar_path(&raw);
    fs::create_dir(&sidecar).unwrap();

    let outcome =
        write_sidecar(&histogram, &sidecar, &raw, &store, NOW_MS).expect("the fallback succeeds");
    assert_eq!(outcome, SidecarOutcome::Cached);

    let entry = store
        .exif_cache(&raw.to_string_lossy())
        .unwrap()
        .expect("the histogram is cached under the raw directory");
    assert_eq!(entry.collected_at, NOW_MS);
    let cached: Histogram = serde_json::from_str(&entry.histogram_json).unwrap();
    assert_eq!(cached, histogram);

    let tags = metadata(&cached);
    assert_eq!(tags.get("Make").map(String::as_str), Some("Canon"));
    assert_eq!(tags.get("Lens").map(String::as_str), Some("EF 50mm"));
    assert!(!tags.contains_key("Model"), "a multi-valued tag is dropped");

    let read = read_histogram(&sidecar, &raw, &store).unwrap();
    assert_eq!(read, Some(histogram));
}

#[test]
fn reads_swift_sidecar() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("IMG_SEQ");
    fs::create_dir(&raw).unwrap();
    let sidecar = sidecar_path(&raw);
    assert_eq!(sidecar, tmp.path().join("IMG_SEQ.json"));
    fs::write(&sidecar, SWIFT_SIDECAR).unwrap();
    let store = open_store(tmp.path());

    let histogram = read_histogram(&sidecar, &raw, &store)
        .unwrap()
        .expect("the Swift sidecar reads");
    assert_eq!(histogram["Model"].len(), 2);
    assert_eq!(histogram["SourceFile"]["/raw/IMG_0001.CR2"], 1);

    let expected = BTreeMap::from([
        ("Lens".to_owned(), "EF 50mm f/1.8".to_owned()),
        ("Make".to_owned(), "Canon".to_owned()),
    ]);
    assert_eq!(metadata(&histogram), expected);
}

#[test]
fn atomic_write_leaves_no_temp_file() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    let store = open_store(tmp.path());
    let histogram = sample_histogram();
    let sidecar = sidecar_path(&raw);

    let outcome = write_sidecar(&histogram, &sidecar, &raw, &store, NOW_MS).unwrap();
    assert_eq!(outcome, SidecarOutcome::Sidecar(sidecar.clone()));

    let mut names: Vec<String> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["db", "raw", "raw.json"], "no temp file is left");

    let text = fs::read_to_string(&sidecar).unwrap();
    let parsed: Histogram = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, histogram);
    assert!(text.contains("\n  \""), "the sidecar is pretty-printed");
    assert!(store.exif_cache(&raw.to_string_lossy()).unwrap().is_none());
}

#[test]
fn sidecar_preferred_over_cache() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    let store = open_store(tmp.path());
    let sidecar = sidecar_path(&raw);

    let on_disk = sample_histogram();
    fs::write(&sidecar, serde_json::to_string(&on_disk).unwrap()).unwrap();
    let cached = Histogram::from([("Make".to_owned(), BTreeMap::from([("Nikon".to_owned(), 1)]))]);
    store
        .put_exif_cache(&trd_core::store::ExifCacheEntry {
            raw_dir: raw.to_string_lossy().into_owned(),
            histogram_json: serde_json::to_string(&cached).unwrap(),
            collected_at: NOW_MS,
        })
        .unwrap();

    assert_eq!(
        read_histogram(&sidecar, &raw, &store).unwrap(),
        Some(on_disk)
    );
}

#[test]
fn corrupt_sidecar_falls_back() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    let store = open_store(tmp.path());
    let sidecar = sidecar_path(&raw);
    fs::write(&sidecar, "{ this is not json").unwrap();

    assert_eq!(read_histogram(&sidecar, &raw, &store).unwrap(), None);

    let cached = sample_histogram();
    store
        .put_exif_cache(&trd_core::store::ExifCacheEntry {
            raw_dir: raw.to_string_lossy().into_owned(),
            histogram_json: serde_json::to_string(&cached).unwrap(),
            collected_at: NOW_MS,
        })
        .unwrap();
    assert_eq!(
        read_histogram(&sidecar, &raw, &store).unwrap(),
        Some(cached)
    );
}

#[test]
fn multi_valued_tags_dropped() {
    let histogram = sample_histogram();
    let tags = metadata(&histogram);
    assert_eq!(
        tags.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["Lens", "Make"]
    );
}

#[test]
fn sorted_output() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    let store = open_store(tmp.path());
    let sidecar = sidecar_path(&raw);

    // Inserted out of order; the sidecar must still list tags and values sorted.
    let mut histogram = Histogram::new();
    histogram.insert(
        "Model".to_owned(),
        BTreeMap::from([("Z".to_owned(), 1), ("A".to_owned(), 1)]),
    );
    histogram.insert("Lens".to_owned(), BTreeMap::from([("EF".to_owned(), 1)]));
    write_sidecar(&histogram, &sidecar, &raw, &store, NOW_MS).unwrap();

    let text = fs::read_to_string(&sidecar).unwrap();
    let lens = text.find("\"Lens\"").unwrap();
    let model = text.find("\"Model\"").unwrap();
    let value_a = text.find("\"A\"").unwrap();
    let value_z = text.find("\"Z\"").unwrap();
    assert!(lens < model && value_a < value_z);
}

#[test]
fn collect_reads_raw_files_only() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    for name in ["b.arw", "a.ARW", "c.ORF", "notes.txt", "d.jpg"] {
        fs::write(raw.join(name), b"").unwrap();
    }
    let types = vec!["ARW".to_owned(), "ORF".to_owned()];
    let files = exif::raw_files(&raw, &types).unwrap();
    let names: Vec<String> = files
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["a.ARW", "b.arw", "c.ORF"]);
}

#[test]
fn missing_command_is_unavailable() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let tool = ResolvedTool {
        info: ToolInfo {
            name: EXIFTOOL,
            path: None,
            version: String::new(),
            available: false,
            problem: None,
        },
        command: None,
    };
    let err = collect(tmp.path(), &["ARW".to_owned()], 2, &tool).unwrap_err();
    assert!(matches!(err, exif::ExifError::ToolUnavailable));
}

/// The fake exiftool reads its script from `TRD_FAKE_EXIFTOOL`; a `sh` wrapper sets that
/// variable for the one child process, so the test process's environment is never touched.
#[cfg(unix)]
#[test]
fn real_process_matches_canned_output() {
    use trd_core::tools::ToolCommand;
    use trd_testtools::FakeTools;

    let tmp = tempfile::tempdir().expect("a temporary directory");
    let raw = tmp.path().join("raw");
    fs::create_dir(&raw).unwrap();
    for name in ["a.ARW", "b.arw", "c.ORF", "d.jpg"] {
        fs::write(raw.join(name), b"").unwrap();
    }
    let rows = serde_json::json!({
        "rows": {
            "a.ARW": {"Make": "Canon", "Lens": "EF 50mm"},
            "b.arw": {"Make": "Canon", "Lens": "EF 24mm"},
            "c.ORF": {"Make": "Olympus", "Lens": "EF 50mm"},
            "d.jpg": {"Make": "Apple", "Lens": "Wide"}
        }
    });
    let config = tmp.path().join("exiftool.json");
    fs::write(&config, rows.to_string()).unwrap();

    let tools = FakeTools::new().expect("the fake tools");
    let fake = tools.path().join("exiftool");
    let command = ToolCommand {
        program: "/bin/sh".into(),
        leading_args: vec![
            "-c".into(),
            "TRD_FAKE_EXIFTOOL=\"$1\"; export TRD_FAKE_EXIFTOOL; fake=\"$2\"; shift 2; exec \"$fake\" \"$@\""
                .into(),
            "sh".into(),
            config.clone().into_os_string(),
            fake.clone().into_os_string(),
        ],
    };
    let tool = ResolvedTool {
        info: ToolInfo {
            name: EXIFTOOL,
            path: Some(fake),
            version: "13.00".to_owned(),
            available: true,
            problem: None,
        },
        command: Some(command),
    };
    let types = vec!["ARW".to_owned(), "ORF".to_owned()];
    let real = collect(&raw, &types, 2, &tool).expect("the real run collects");

    // The same input through the canned-output path.
    let files = exif::raw_files(&raw, &types).unwrap();
    let mut canned = Histogram::new();
    let text = format!(
        "SourceFile,Lens,Make\n{},EF 50mm,Canon\n{},EF 24mm,Canon\n{},EF 50mm,Olympus\n",
        raw.join("a.ARW").display(),
        raw.join("b.arw").display(),
        raw.join("c.ORF").display(),
    );
    assert_eq!(files.len(), 3);
    exif::parse_csv(&text, &mut canned).unwrap();
    assert_eq!(real, canned);

    assert_eq!(real["Make"]["Canon"], 2);
    assert!(!real.contains_key("Apple"), "the JPEG is not a raw file");
    let tags = metadata(&real);
    assert!(tags.is_empty(), "every tag here has more than one value");
}
