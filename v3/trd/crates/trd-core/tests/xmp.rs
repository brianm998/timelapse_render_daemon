// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! XMP embedding and corrupt-render detection (§6.6, TRD-10). Every video a test writes is a copy
//! in a temporary directory; the fake exiftool stands in for the real one except in the
//! `TRD_E2E` test, which runs the real exiftool on a copy of the fixture. No test reads the
//! owner's data or uses a daemon port.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use trd_core::exif::{ExifError, Histogram};
use trd_core::tools::{EXIFTOOL, ResolvedTool, ToolCommand, ToolInfo, Tools};
use trd_core::xmp::{XmpResult, assignments, detect_structural_corruption, embed};

/// The histogram of a rendered sequence: one value per tag, except `Model` (two values, equal
/// counts) and `DateTimeOriginal` (a later timestamp more frequent than the earliest one).
fn sample_histogram() -> Histogram {
    let counts = |pairs: &[(&str, u64)]| -> BTreeMap<String, u64> {
        pairs
            .iter()
            .map(|(value, count)| ((*value).to_owned(), *count))
            .collect()
    };
    let mut histogram = Histogram::new();
    histogram.insert("Make".to_owned(), counts(&[("Canon", 3)]));
    histogram.insert("Model".to_owned(), counts(&[("EOS R5", 2), ("EOS R6", 1)]));
    histogram.insert("LensModel".to_owned(), counts(&[("RF 24-70mm", 3)]));
    histogram.insert("ISO".to_owned(), counts(&[("400", 2), ("800", 1)]));
    histogram.insert("FNumber".to_owned(), counts(&[("2.8", 3)]));
    histogram.insert("ExposureTime".to_owned(), counts(&[("1/250", 3)]));
    histogram.insert("FocalLength".to_owned(), counts(&[("35.0 mm", 3)]));
    histogram.insert(
        "ExposureProgram".to_owned(),
        counts(&[("Manual", 2), ("Aperture-priority AE", 1)]),
    );
    histogram.insert("ExposureMode".to_owned(), counts(&[("Auto bracket", 3)]));
    histogram.insert("WhiteBalance".to_owned(), counts(&[("Manual", 3)]));
    histogram.insert(
        "DateTimeOriginal".to_owned(),
        counts(&[("2026:03:04 09:00:00", 5), ("2026:03:04 08:30:00", 1)]),
    );
    histogram
}

#[test]
fn assignments_match_swift() {
    assert_eq!(
        assignments(&sample_histogram()),
        vec![
            "-XMP-tiff:Make=Canon",
            "-XMP-tiff:Model=EOS R5",
            "-XMP-aux:Lens=RF 24-70mm",
            "-XMP-exif:ISO=400",
            "-XMP-exif:FNumber=2.8",
            "-XMP-exif:ExposureTime=1/250",
            "-XMP-exif:FocalLength=35.0",
            "-XMP-exif:ExposureProgram#=1",
            "-XMP-exif:ExposureMode#=2",
            "-XMP-exif:WhiteBalance#=1",
            "-XMP-exif:DateTimeOriginal=2026-03-04T08:30:00",
        ]
    );
}

#[test]
fn mode_ties_pick_smallest_key() {
    let mut histogram = Histogram::new();
    histogram.insert(
        "Model".to_owned(),
        BTreeMap::from([("EOS R6".to_owned(), 1), ("EOS R5".to_owned(), 1)]),
    );
    histogram.insert(
        "ISO".to_owned(),
        BTreeMap::from([
            ("800".to_owned(), 2),
            ("400".to_owned(), 2),
            ("200".to_owned(), 1),
        ]),
    );
    assert_eq!(
        assignments(&histogram),
        vec!["-XMP-tiff:Model=EOS R5", "-XMP-exif:ISO=400"]
    );
}

#[test]
fn assignments_normalise_units_enums_and_dates() {
    let mut histogram = Histogram::new();
    histogram.insert(
        "FocalLength".to_owned(),
        BTreeMap::from([(" 85MM ".to_owned(), 1)]),
    );
    histogram.insert(
        "ExposureProgram".to_owned(),
        BTreeMap::from([("Shutter priority".to_owned(), 1)]),
    );
    histogram.insert(
        "ExposureMode".to_owned(),
        BTreeMap::from([("Auto-bracket".to_owned(), 1)]),
    );
    histogram.insert(
        "WhiteBalance".to_owned(),
        BTreeMap::from([("Auto".to_owned(), 1)]),
    );
    histogram.insert(
        "DateTimeOriginal".to_owned(),
        BTreeMap::from([("2026:03:04".to_owned(), 1)]),
    );
    assert_eq!(
        assignments(&histogram),
        vec![
            "-XMP-exif:FocalLength=85",
            "-XMP-exif:ExposureProgram#=4",
            "-XMP-exif:ExposureMode#=2",
            "-XMP-exif:WhiteBalance#=0",
            "-XMP-exif:DateTimeOriginal=2026:03:04",
        ]
    );
}

#[test]
fn assignments_skip_unmapped_columns() {
    let mut histogram = Histogram::new();
    histogram.insert(
        "SourceFile".to_owned(),
        BTreeMap::from([("/raw/IMG_0001.ARW".to_owned(), 1)]),
    );
    assert!(assignments(&histogram).is_empty());
    assert!(assignments(&Histogram::new()).is_empty());
}

#[test]
fn detect_structural_corruption_matches_exiftool_error() {
    assert_eq!(
        detect_structural_corruption("Error: Truncated mdat atom - /tmp/video.mov"),
        Some("Truncated mdat atom")
    );
}

#[test]
fn detect_structural_corruption_matches_other_atom_errors() {
    for (output, reason) in [
        ("Error: Bad atom size - /tmp/video.mov", "Bad atom size"),
        ("Error: Could not parse moov atom", "Could not parse moov"),
        ("Error: Invalid mdat atom", "Invalid mdat"),
    ] {
        assert_eq!(
            detect_structural_corruption(output),
            Some(reason),
            "{output}"
        );
    }
}

#[test]
fn detect_structural_corruption_ignores_unrelated_failures() {
    for output in [
        "Error: Temporary file already exists - /tmp/video.mov_exiftool_tmp",
        "Error: Error writing output file - /tmp/video.mov",
        "Error: File not found - /tmp/video.mov",
        "",
    ] {
        assert_eq!(detect_structural_corruption(output), None, "{output:?}");
    }
}

/// A fake exiftool whose write prints `output` and exits with `code`, run through a `sh` wrapper
/// that sets `TRD_FAKE_EXIFTOOL` for the child only.
/// The fake tools directory is returned too: the caller keeps it alive while the tool runs.
#[cfg(unix)]
fn fake_exiftool(output: &str, code: i32) -> (trd_testtools::FakeTools, ResolvedTool) {
    use trd_testtools::FakeTools;

    let script = serde_json::json!({
        "write": {"output": output, "exit_code": code}
    })
    .to_string();
    let tools = FakeTools::new().expect("the fake tools");
    let fake = tools.path().join("exiftool");
    let tool = ResolvedTool {
        info: ToolInfo {
            name: EXIFTOOL,
            path: Some(fake.clone()),
            version: "13.00".to_owned(),
            available: true,
            problem: None,
        },
        command: Some(ToolCommand {
            program: "/bin/sh".into(),
            leading_args: vec![
                "-c".into(),
                "TRD_FAKE_EXIFTOOL=\"$1\"; export TRD_FAKE_EXIFTOOL; fake=\"$2\"; shift 2; exec \"$fake\" \"$@\""
                    .into(),
                "sh".into(),
                script.into(),
                fake.into_os_string(),
            ],
        }),
    };
    (tools, tool)
}

/// A video path in a temporary directory. The fake exiftool never opens it.
fn video_in(dir: &Path) -> PathBuf {
    dir.join("video.mov")
}

#[cfg(unix)]
#[test]
fn embed_succeeds_when_exiftool_exits_zero() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let (_fake, tool) = fake_exiftool("", 0);
    assert_eq!(
        embed(&video_in(tmp.path()), &sample_histogram(), &tool).unwrap(),
        XmpResult::Succeeded
    );
}

#[cfg(unix)]
#[test]
fn embed_maps_truncated_mdat_to_structural_corruption() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let message = "Error: Truncated mdat atom - /tmp/video.mov";
    let (_fake, tool) = fake_exiftool(message, 1);
    assert_eq!(
        embed(&video_in(tmp.path()), &sample_histogram(), &tool).unwrap(),
        XmpResult::StructuralCorruption {
            reason: "Truncated mdat atom",
            output: message.to_owned(),
        }
    );
}

#[cfg(unix)]
#[test]
fn embed_maps_unrelated_failure_to_tool_error() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let message = "Error: Error writing output file - /tmp/video.mov";
    let (_fake, tool) = fake_exiftool(message, 2);
    assert_eq!(
        embed(&video_in(tmp.path()), &sample_histogram(), &tool).unwrap(),
        XmpResult::ToolError {
            output: message.to_owned(),
            code: 2,
        }
    );
}

#[cfg(unix)]
#[test]
fn embed_skips_an_empty_histogram() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    // The fake would fail if it ran, so a Skipped result shows exiftool was never started.
    let (_fake, tool) = fake_exiftool("exiftool must not run", 9);
    let result = embed(&video_in(tmp.path()), &Histogram::new(), &tool).unwrap();
    assert_eq!(
        result,
        XmpResult::Skipped {
            reason: "no tags resolved from sidecar".to_owned(),
        }
    );
}

#[test]
fn embed_without_a_tool_is_unavailable() {
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
    let err = embed(&video_in(tmp.path()), &sample_histogram(), &tool).unwrap_err();
    assert!(matches!(err, ExifError::ToolUnavailable));
}

#[test]
fn embed_reports_a_tool_that_cannot_start() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let missing = tmp.path().join("no-such-exiftool");
    let tool = ResolvedTool {
        info: ToolInfo {
            name: EXIFTOOL,
            path: Some(missing.clone()),
            version: String::new(),
            available: true,
            problem: None,
        },
        command: Some(ToolCommand::direct(&missing)),
    };
    let err = embed(&video_in(tmp.path()), &sample_histogram(), &tool).unwrap_err();
    assert!(matches!(err, ExifError::Launch { .. }), "{err}");
}

/// The eleven XMP tags `assignments` writes for `sample_histogram`, as exiftool names them.
const XMP_TAGS: [&str; 11] = [
    "-XMP-tiff:Make",
    "-XMP-tiff:Model",
    "-XMP-aux:Lens",
    "-XMP-exif:ISO",
    "-XMP-exif:FNumber",
    "-XMP-exif:ExposureTime",
    "-XMP-exif:FocalLength",
    "-XMP-exif:ExposureProgram",
    "-XMP-exif:ExposureMode",
    "-XMP-exif:WhiteBalance",
    "-XMP-exif:DateTimeOriginal",
];

/// The real exiftool writes the XMP block into a copy of the MOV fixture, and every tag reads back
/// non-empty. Runs only under `TRD_E2E=1`; it fails rather than skips when exiftool is missing.
#[test]
fn real_tool_writes_readable_xmp() {
    if std::env::var_os("TRD_E2E").is_none() {
        return;
    }
    let exe = std::env::current_exe().expect("the test executable's path");
    let tools = Tools::resolve(|key| std::env::var_os(key), &exe);
    let tool = tools.exiftool;
    let Some(command) = tool.command.clone() else {
        panic!(
            "TRD_E2E is set but exiftool is not available: {:?}",
            tool.info
        );
    };

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../exiftool/fixtures/tiny.mov");
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let video = tmp.path().join("tiny.mov");
    fs::copy(&fixture, &video).expect("the fixture copies");

    let result = embed(&video, &sample_histogram(), &tool).expect("exiftool runs");
    assert_eq!(
        result,
        XmpResult::Succeeded,
        "exiftool writes the XMP block"
    );

    let output = command
        .command()
        .args(["-s3"])
        .args(XMP_TAGS)
        .arg(&video)
        .output()
        .expect("exiftool reads the video back");
    let text = String::from_utf8_lossy(&output.stdout);
    let values: Vec<&str> = text.lines().collect();
    assert_eq!(
        values.len(),
        XMP_TAGS.len(),
        "every XMP tag reads back: {text}"
    );
    for (tag, value) in XMP_TAGS.iter().zip(&values) {
        assert!(!value.trim().is_empty(), "{tag} is empty");
    }
}
