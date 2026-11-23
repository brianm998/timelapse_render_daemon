// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Failure diagnosis (§6.9), ported from the Swift `RenderFailureDiagnosis` and `RenderFailure`
//! tests with the same inputs and the same assertions.
//!
//! Every test is inside `mod diagnosis`, so `cargo test -p trd-core diagnosis` selects them all.
//! Fixtures live in the system temporary directory; nothing here starts a process.

mod diagnosis {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;
    use trd_core::codecs;
    use trd_core::diagnosis::{
        FailureKind, Problem, RenderFailure, analyze_input_sequence, decompose, describe, diagnose,
        significant_lines,
    };
    use trd_core::plan::Plan;

    fn extensions() -> Vec<String> {
        ["tif", "tiff"].iter().map(|e| (*e).to_owned()).collect()
    }

    /// The Swift test plan: `LRT_%05d.tiff` into `/tmp/out.mov`, with the first built-in codec.
    fn make_plan(input_dir: &str, prefix: &str, suffix: &str, digits: usize) -> Plan {
        let codec = codecs::builtin()
            .into_values()
            .next()
            .expect("the built-in codec table is not empty");
        Plan {
            input_dir: input_dir.to_owned(),
            image_name_prefix: prefix.to_owned(),
            image_suffix: suffix.to_owned(),
            frame_number_digits: digits,
            start_number: None,
            output_path: "/tmp/out.mov".to_owned(),
            progress_path: "/tmp/progress.txt".to_owned(),
            partial_path: "/tmp/.out.partial.mov".to_owned(),
            frame_rate: 30,
            aspect: "16/9".to_owned(),
            width: None,
            height: None,
            codec,
            metadata: BTreeMap::new(),
            total_frames: 10,
        }
    }

    fn plan_for(input_dir: &str) -> Plan {
        make_plan(input_dir, "LRT_", ".tiff", 5)
    }

    /// Creates `count` frames named `<prefix><index, zero-padded to digits><ext>` in `dir`.
    fn make_frames(dir: &Path, prefix: &str, digits: usize, ext: &str, first: i64, count: i64) {
        for index in first..first + count {
            let name = format!("{prefix}{index:0digits$}{ext}");
            fs::write(dir.join(name), b"x").expect("write fixture frame");
        }
    }

    fn new_dir() -> TempDir {
        TempDir::new().expect("temporary directory")
    }

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn significant_lines_strips_ffmpeg_banner() {
        let raw = lines(&[
            "ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers",
            "  built with Apple clang version 17.0.0 (clang-1700.4.4.1)",
            "  configuration: --prefix=/usr/local/Cellar/ffmpeg/8.1 --enable-shared",
            "  libavutil      60. 26.100 / 60. 26.100",
            "  libavcodec     62. 28.100 / 62. 28.100",
            "frame=   30 fps=0.0 q=-0.0 Lsize=       6KiB time=00:00:06.00",
            "Error opening input file /rp/tmp/x/LRT_%05d.tiff.",
        ]);
        let kept = significant_lines(&raw);
        assert_eq!(
            kept,
            lines(&["Error opening input file /rp/tmp/x/LRT_%05d.tiff."])
        );
    }

    #[test]
    fn significant_lines_drops_press_q_hint_and_blank_lines() {
        let raw = lines(&["   ", "Press [q] to stop, [?] for help", "\tkept line  "]);
        // Kept lines are returned as they were given, untrimmed, as the Swift filter does.
        assert_eq!(significant_lines(&raw), lines(&["\tkept line  "]));
    }

    #[test]
    fn decompose_image_filename() {
        let a = decompose("image_0001.tiff").expect("numbered name");
        assert_eq!(a.prefix, "image_");
        assert_eq!(a.digits, "0001");
        assert_eq!(a.suffix, ".tiff");

        let b = decompose("LRT_00001-severe-noise.tiff").expect("numbered name");
        assert_eq!(b.prefix, "LRT_");
        assert_eq!(b.digits, "00001");
        assert_eq!(b.suffix, "-severe-noise.tiff");

        assert!(decompose("no-digits-here.tiff").is_none());
    }

    /// The real-world failure: LRTimelapse wrote `image_0001.tiff` into a dir named `LRT_<sequence>`,
    /// so the daemon's `LRT_%05d.tiff` pattern matched nothing: wrong prefix AND wrong digit width.
    #[test]
    fn analyze_input_sequence_reports_prefix_and_digit_width_mismatch() {
        let dir = new_dir();
        make_frames(dir.path(), "image_", 4, ".tiff", 1, 3);

        let report = analyze_input_sequence(
            dir.path().to_str().unwrap(),
            "LRT_",
            5,
            ".tiff",
            &extensions(),
        );
        assert_eq!(report.image_file_count, 3);
        assert_eq!(report.example_filename.as_deref(), Some("image_0001.tiff"));
        assert!(report.problems.contains(&Problem::PrefixMismatch {
            expected: "LRT_".to_owned(),
            found: "image_".to_owned(),
        }));
        assert!(report.problems.contains(&Problem::DigitWidthMismatch {
            expected: 5,
            found: 4,
        }));
        // The suffix matches, so the list is exactly the two mismatches above.
        assert_eq!(
            report.problems,
            vec![
                Problem::PrefixMismatch {
                    expected: "LRT_".to_owned(),
                    found: "image_".to_owned(),
                },
                Problem::DigitWidthMismatch {
                    expected: 5,
                    found: 4,
                },
            ]
        );
    }

    /// Only the suffix differs: the files are `.png` where the pattern expects `.tiff`.
    #[test]
    fn analyze_input_sequence_reports_suffix_mismatch() {
        let dir = new_dir();
        make_frames(dir.path(), "LRT_", 5, ".png", 1, 3);
        let png = vec!["png".to_owned()];

        let report = analyze_input_sequence(dir.path().to_str().unwrap(), "LRT_", 5, ".tiff", &png);
        assert_eq!(
            report.problems,
            vec![Problem::SuffixMismatch {
                expected: ".tiff".to_owned(),
                found: ".png".to_owned(),
            }]
        );
        let text = describe(&report, dir.path().to_str().unwrap(), "LRT_%05d.tiff");
        assert!(
            text.contains("- SUFFIX: files end in \".png\" but the pattern expects \".tiff\""),
            "{text}"
        );
    }

    /// Correctly named and numbered from 0, so the probe range is enough and ffmpeg should find
    /// the frames: nothing in the directory explains the failure.
    #[test]
    fn analyze_input_sequence_reports_unexplained() {
        let dir = new_dir();
        make_frames(dir.path(), "LRT_", 5, ".tiff", 0, 3);
        let dir_text = dir.path().to_str().unwrap();

        let report = analyze_input_sequence(dir_text, "LRT_", 5, ".tiff", &extensions());
        assert_eq!(report.problems, vec![Problem::Unexplained]);
        let text = describe(&report, dir_text, "LRT_%05d.tiff");
        assert!(
            text.contains("could not work out how the files on disk differ"),
            "{text}"
        );
    }

    #[test]
    fn analyze_input_sequence_reports_empty_directory() {
        let dir = new_dir();
        let dir_text = dir.path().to_str().unwrap();

        let report = analyze_input_sequence(dir_text, "LRT_", 5, ".tiff", &extensions());
        assert_eq!(report.problems, vec![Problem::DirectoryEmpty]);
        let text = describe(&report, dir_text, "LRT_%05d.tiff");
        assert!(text.contains("- that directory is empty"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn analyze_input_sequence_reports_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;

        /// Restores the directory's permissions on drop so the temporary directory can be removed
        /// even when an assertion fails.
        struct Restore(std::path::PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
            }
        }

        let dir = new_dir();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        make_frames(&locked, "LRT_", 5, ".tiff", 1, 3);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let _restore = Restore(locked.clone());
        let locked_text = locked.to_str().unwrap();

        let report = analyze_input_sequence(locked_text, "LRT_", 5, ".tiff", &extensions());
        assert_eq!(report.problems, vec![Problem::DirectoryUnreadable]);
        let text = describe(&report, locked_text, "LRT_%05d.tiff");
        assert!(text.contains("could not be listed"), "{text}");

        // diagnose() turns an unreadable input directory into InputUnreadable.
        let output = lines(&[
            "Could find no file or sequence with path 'x/LRT_%05d.tiff' and index in the range 0-4",
        ]);
        let d = diagnose(
            &plan_for(locked_text),
            Some(254),
            true,
            &output,
            &extensions(),
            false,
        );
        assert_eq!(d.kind, FailureKind::InputUnreadable);
        assert_eq!(
            d.summary,
            format!("the input directory could not be read: {locked_text}")
        );
    }

    #[test]
    fn describe_explains_a_missing_directory() {
        let report = analyze_input_sequence(
            "/definitely/not/here-missing",
            "LRT_",
            5,
            ".tiff",
            &extensions(),
        );
        let text = describe(&report, "/definitely/not/here-missing", "LRT_%05d.tiff");
        assert!(text.contains("that directory does not exist"), "{text}");
    }

    #[test]
    fn diagnose_reports_empty_input_directory() {
        let dir = new_dir();
        let dir_text = dir.path().to_str().unwrap();
        let output = lines(&[
            "Could find no file or sequence with path 'x/LRT_%05d.tiff' and index in the range 0-4",
        ]);
        let d = diagnose(
            &plan_for(dir_text),
            Some(254),
            true,
            &output,
            &extensions(),
            false,
        );
        assert_eq!(d.kind, FailureKind::InputFramesNotFound);
        assert_eq!(
            d.summary,
            format!("there are no images to render in {dir_text}")
        );
        assert!(
            d.detail
                .expect("a detail")
                .contains("that directory is empty")
        );
    }

    #[test]
    fn analyze_input_sequence_reports_start_index_beyond_probe_range() {
        let dir = new_dir();
        // Correctly named, but the sequence starts at 100: past the handful of indices ffmpeg's
        // image2 demuxer probes.
        make_frames(dir.path(), "LRT_", 5, ".tiff", 100, 3);

        let report = analyze_input_sequence(
            dir.path().to_str().unwrap(),
            "LRT_",
            5,
            ".tiff",
            &extensions(),
        );
        assert_eq!(
            report.problems,
            vec![Problem::FirstIndexOutOfProbeRange { first_index: 100 }]
        );
    }

    #[test]
    fn analyze_input_sequence_reports_missing_directory() {
        let missing = format!("/definitely/not/here-{}", unique_suffix());
        let report = analyze_input_sequence(&missing, "LRT_", 5, ".tiff", &extensions());
        assert_eq!(report.problems, vec![Problem::DirectoryMissing]);
    }

    #[test]
    fn analyze_input_sequence_reports_no_image_files() {
        let dir = new_dir();
        fs::write(dir.path().join("notes.txt"), b"x").unwrap();

        let report = analyze_input_sequence(
            dir.path().to_str().unwrap(),
            "LRT_",
            5,
            ".tiff",
            &extensions(),
        );
        assert_eq!(
            report.problems,
            vec![Problem::NoImageFiles {
                sample: vec!["notes.txt".to_owned()],
            }]
        );
    }

    #[test]
    fn analyze_input_sequence_reports_unnumbered_frames() {
        let dir = new_dir();
        fs::write(dir.path().join("panorama.tiff"), b"x").unwrap();
        let dir_text = dir.path().to_str().unwrap();

        let report = analyze_input_sequence(dir_text, "LRT_", 5, ".tiff", &extensions());
        assert_eq!(
            report.problems,
            vec![Problem::FramesNotNumbered {
                sample: vec!["panorama.tiff".to_owned()],
            }]
        );
        let text = describe(&report, dir_text, "LRT_%05d.tiff");
        assert!(text.contains("no frame number"), "{text}");
    }

    /// End to end, using ffmpeg 8.1's actual output for this failure.
    #[test]
    fn explains_no_frames() {
        let dir = new_dir();
        make_frames(dir.path(), "image_", 4, ".tiff", 1, 3);
        let dir_text = dir.path().to_str().unwrap();

        let plan = plan_for(dir_text);
        let output = vec![
            "ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers".to_owned(),
            "  libavcodec     62. 28.100 / 62. 28.100".to_owned(),
            format!(
                "[in#0 @ 0x7fba07706480] Could find no file or sequence with path \
                 '{dir_text}/LRT_%05d.tiff' and index in the range 0-4"
            ),
            "[in#0 @ 0x7fba07706480] Error opening input: No such file or directory".to_owned(),
            "Error opening input files: No such file or directory".to_owned(),
        ];
        let d = diagnose(&plan, Some(254), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::InputFramesNotFound);
        assert!(d.summary.contains("LRT_%05d.tiff"), "{}", d.summary);
        let detail = d.detail.expect("a detail");
        assert!(detail.contains("image_0001.tiff"), "{detail}");
        assert!(detail.contains("FILENAME PREFIX"), "{detail}");
        assert!(detail.contains("FRAME NUMBER WIDTH"), "{detail}");
    }

    #[test]
    fn diagnose_reports_missing_input_directory() {
        let missing = format!("/definitely/not/here-{}", unique_suffix());
        let plan = plan_for(&missing);
        let output = vec![
            format!("Error opening input file {missing}/LRT_%05d.tiff."),
            "Error opening input files: No such file or directory".to_owned(),
        ];
        let d = diagnose(&plan, Some(254), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::InputFramesNotFound);
        assert!(
            d.summary.contains("input directory is gone"),
            "{}",
            d.summary
        );
    }

    #[test]
    fn diagnose_recognises_disk_full() {
        let plan = plan_for("/tmp");
        let d = diagnose(
            &plan,
            Some(1),
            true,
            &lines(&["av_interleaved_write_frame(): No space left on device"]),
            &extensions(),
            false,
        );
        assert_eq!(d.kind, FailureKind::DiskFull);
    }

    #[test]
    fn diagnose_recognises_unknown_encoder() {
        let plan = plan_for("/tmp");
        let d = diagnose(
            &plan,
            Some(1),
            true,
            &lines(&["Unknown encoder 'prores_ks_typo'"]),
            &extensions(),
            false,
        );
        assert_eq!(d.kind, FailureKind::FfmpegRejectedArguments);
        assert!(d.summary.contains("prores_ks_typo"), "{}", d.summary);
    }

    #[test]
    fn diagnose_recognises_killed_process() {
        let plan = plan_for("/tmp");
        let d = diagnose(&plan, Some(9), false, &[], &extensions(), false);
        assert_eq!(d.kind, FailureKind::TerminatedBySignal);
        assert_eq!(d.summary, "ffmpeg was killed before it finished (signal 9)");
    }

    #[test]
    fn diagnose_recognises_success_with_no_output_file() {
        let plan = plan_for("/tmp");
        let d = diagnose(&plan, Some(0), true, &[], &extensions(), false);
        assert_eq!(d.kind, FailureKind::ProducedNoOutput);
    }

    #[test]
    fn significant_lines_keeps_progress_line_whose_fps_does_not_parse() {
        let raw = lines(&["frame=    5 fps=1.2.3 q=-0.0", "frame=    6 fps=0.0 q=-0.0"]);
        // The second line parses both captures, so it is progress and dropped; the first has an
        // fps that is not a number, so it is not a progress line and is kept.
        assert_eq!(
            significant_lines(&raw),
            lines(&["frame=    5 fps=1.2.3 q=-0.0"])
        );
    }

    #[test]
    fn diagnose_reports_permission_denied_on_the_output() {
        let plan = plan_for("/tmp");
        let output = lines(&["/tmp/out.mov: Permission denied"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::OutputNotWritable);
        assert_eq!(d.summary, "cannot write /tmp/out.mov — permission denied");
        assert_eq!(d.detail.as_deref(), Some("/tmp/out.mov: Permission denied"));
    }

    #[test]
    fn diagnose_reports_permission_denied_on_the_source() {
        let plan = plan_for("/tmp");
        let output = lines(&["/tmp/frames/LRT_00001.tiff: Permission denied"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::InputUnreadable);
        assert_eq!(d.summary, "permission denied reading the source frames");
        assert_eq!(
            d.detail.as_deref(),
            Some("/tmp/frames/LRT_00001.tiff: Permission denied")
        );
    }

    #[test]
    fn diagnose_reports_unrecognized_option() {
        let plan = plan_for("/tmp");
        let output = lines(&[
            "Unrecognized option 'crf'.",
            "Error splitting the argument list",
        ]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::FfmpegRejectedArguments);
        assert_eq!(d.summary, "Unrecognized option 'crf'.");
        assert!(d.detail.expect("a detail").contains("video_codecs.json"));
    }

    #[test]
    fn diagnose_reports_option_not_found() {
        let plan = plan_for("/tmp");
        let output = lines(&["Option not found: -profile:v"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::FfmpegRejectedArguments);
        assert_eq!(d.summary, "Option not found: -profile:v");
    }

    #[test]
    fn diagnose_reports_invalid_argument() {
        let plan = plan_for("/tmp");
        let output = lines(&["Invalid argument"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::FfmpegRejectedArguments);
        assert_eq!(d.summary, "Invalid argument");
    }

    #[test]
    fn diagnose_reports_error_splitting_the_argument_list() {
        let plan = plan_for("/tmp");
        let output = lines(&["Error splitting the argument list: Unknown error"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::FfmpegRejectedArguments);
        assert_eq!(
            d.summary,
            "Error splitting the argument list: Unknown error"
        );
    }

    #[test]
    fn diagnose_reports_invalid_data_in_a_source_frame() {
        let plan = plan_for("/rp/tmp/frames");
        let output = lines(&["Invalid data found when processing input"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::SourceFrameUnreadable);
        assert_eq!(d.summary, "Invalid data found when processing input");
        assert_eq!(
            d.detail.as_deref(),
            Some(
                "One of the exported frames in /rp/tmp/frames is truncated or corrupt. \
                 Re-export the sequence."
            )
        );
    }

    #[test]
    fn diagnose_reports_decoding_error_in_a_source_frame() {
        let plan = plan_for("/tmp");
        let output = lines(&["Error while decoding stream #0:0"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::SourceFrameUnreadable);
        assert_eq!(d.summary, "Error while decoding stream #0:0");
    }

    #[test]
    fn diagnose_reports_unopenable_source_frame_with_fallback_summary() {
        let plan = plan_for("/tmp");
        let output = lines(&["Could not open file LRT_00002.tiff"]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), false);
        assert_eq!(d.kind, FailureKind::SourceFrameUnreadable);
        assert_eq!(d.summary, "a source frame could not be decoded");
    }

    #[test]
    fn diagnose_reports_signal_without_exit_code() {
        let plan = plan_for("/tmp");
        let d = diagnose(&plan, None, false, &[], &extensions(), false);
        assert_eq!(d.kind, FailureKind::TerminatedBySignal);
        assert_eq!(d.summary, "ffmpeg was killed before it finished");
    }

    #[test]
    fn diagnose_unknown_uses_last_significant_line() {
        let plan = plan_for("/tmp");
        let output = lines(&[
            "ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers",
            "frame=   30 fps=0.0 q=-0.0",
            "Something odd happened",
            "Then something else",
        ]);
        let d = diagnose(&plan, Some(1), true, &output, &extensions(), true);
        assert_eq!(d.kind, FailureKind::Unknown);
        assert_eq!(d.summary, "Then something else");
        assert_eq!(d.detail, None);
    }

    #[test]
    fn diagnose_unknown_with_banner_only_output_names_the_exit_code() {
        let plan = plan_for("/tmp");
        let output = lines(&["ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers"]);
        let d = diagnose(&plan, Some(3), true, &output, &extensions(), true);
        assert_eq!(d.kind, FailureKind::Unknown);
        assert_eq!(
            d.summary,
            "ffmpeg exited 3 without a recognisable error message"
        );
        assert_eq!(
            d.detail.as_deref(),
            Some("ffmpeg produced no output beyond its startup banner.")
        );
    }

    #[test]
    fn diagnose_unknown_without_exit_code_says_abnormally() {
        let plan = plan_for("/tmp");
        let d = diagnose(&plan, None, true, &[], &extensions(), true);
        assert_eq!(d.kind, FailureKind::Unknown);
        assert_eq!(
            d.summary,
            "ffmpeg exited abnormally without a recognisable error message"
        );
    }

    #[test]
    fn render_failure_merging_counts_occurrences() {
        let make = |summary: &str| RenderFailure {
            timestamp: 0,
            sequence_name: "seq".to_owned(),
            export_path: "/e".to_owned(),
            output_filename: "o.mov".to_owned(),
            output_path: "/e/o.mov".to_owned(),
            variant_key: "v".to_owned(),
            kind: FailureKind::InputFramesNotFound,
            summary: summary.to_owned(),
            detail: None,
            exit_code: None,
            command_line: None,
            tool_output: Vec::new(),
            occurrences: 1,
        };
        let first = make("first");
        assert!(first.same_failure(&make("second")));
        let merged = first.merging(make("second"));
        assert_eq!(merged.occurrences, 2);
        assert_eq!(merged.summary, "second");
        assert!(
            merged.short_message().contains("x2"),
            "{}",
            merged.short_message()
        );
    }

    #[test]
    fn render_failure_console_report_includes_the_essentials() {
        let f = RenderFailure {
            timestamp: 0,
            sequence_name: "seq".to_owned(),
            export_path: "/e".to_owned(),
            output_filename: "o.mov".to_owned(),
            output_path: "/e/o.mov".to_owned(),
            variant_key: "ProRes_30".to_owned(),
            kind: FailureKind::InputFramesNotFound,
            summary: "no frames matched LRT_%05d.tiff".to_owned(),
            detail: Some("line one\nline two".to_owned()),
            exit_code: Some(254),
            command_line: Some("ffmpeg -i /seq/LRT_%05d.tiff /e/o.mov".to_owned()),
            tool_output: lines(&["Error opening input files: No such file or directory"]),
            occurrences: 1,
        };
        let report = f.console_report();
        assert!(report.contains("RENDER FAILED"), "{report}");
        assert!(report.contains("/e/o.mov"), "{report}");
        assert!(report.contains("ffmpeg exit code: 254"), "{report}");
        assert!(report.contains("no frames matched"), "{report}");
        assert!(report.contains("line two"), "{report}");
        assert!(report.contains("Error opening input files"), "{report}");
        assert!(report.contains("ffmpeg -i /seq/LRT_%05d.tiff"), "{report}");
    }

    /// The exact layout of the Swift `consoleReport`: the occurrence suffix, the optional exit code
    /// and tool-output and command lines, and the seven-space indent of the detail.
    #[test]
    fn console_report_layout_is_exact() {
        let f = RenderFailure {
            timestamp: 0,
            sequence_name: "seq".to_owned(),
            export_path: "/e".to_owned(),
            output_filename: "o.mov".to_owned(),
            output_path: "/e/o.mov".to_owned(),
            variant_key: "ProRes_30".to_owned(),
            kind: FailureKind::InputFramesNotFound,
            summary: "no frames matched LRT_%05d.tiff".to_owned(),
            detail: Some("line one\nline two".to_owned()),
            exit_code: Some(254),
            command_line: Some("ffmpeg -i /seq/LRT_%05d.tiff /e/o.mov".to_owned()),
            tool_output: lines(&["Error opening input files: No such file or directory"]),
            occurrences: 2,
        };
        let expected = [
            "RENDER FAILED  o.mov  —  no input frames  (occurrence 2)",
            "  sequence: seq",
            "  output:   /e/o.mov",
            "  variant:  ProRes_30",
            "  ffmpeg exit code: 254",
            "  why: no frames matched LRT_%05d.tiff",
            "       line one",
            "       line two",
            "  tool output:",
            "    | Error opening input files: No such file or directory",
            "  command: ffmpeg -i /seq/LRT_%05d.tiff /e/o.mov",
        ]
        .join("\n");
        assert_eq!(f.console_report(), expected);
    }

    #[test]
    fn console_report_omits_the_optional_lines_when_absent() {
        let f = RenderFailure {
            timestamp: 0,
            sequence_name: "seq".to_owned(),
            export_path: "/e".to_owned(),
            output_filename: "o.mov".to_owned(),
            output_path: "/e/o.mov".to_owned(),
            variant_key: "v".to_owned(),
            kind: FailureKind::Unknown,
            summary: "boom".to_owned(),
            detail: Some(String::new()),
            exit_code: None,
            command_line: None,
            tool_output: Vec::new(),
            occurrences: 1,
        };
        let expected = [
            "RENDER FAILED  o.mov  —  render failed",
            "  sequence: seq",
            "  output:   /e/o.mov",
            "  variant:  v",
            "  why: boom",
        ]
        .join("\n");
        assert_eq!(f.console_report(), expected);
    }

    #[test]
    fn failure_kind_labels_and_codes() {
        let cases = [
            (
                FailureKind::InputFramesNotFound,
                "no input frames",
                "FAILURE_INPUT_FRAMES_NOT_FOUND",
            ),
            (
                FailureKind::InputUnreadable,
                "input unreadable",
                "FAILURE_INPUT_UNREADABLE",
            ),
            (
                FailureKind::SourceFrameUnreadable,
                "bad source frame",
                "FAILURE_SOURCE_FRAME_UNREADABLE",
            ),
            (
                FailureKind::FfmpegRejectedArguments,
                "ffmpeg rejected arguments",
                "FAILURE_FFMPEG_REJECTED_ARGUMENTS",
            ),
            (
                FailureKind::OutputNotWritable,
                "output not writable",
                "FAILURE_OUTPUT_NOT_WRITABLE",
            ),
            (FailureKind::DiskFull, "disk full", "FAILURE_DISK_FULL"),
            (
                FailureKind::TerminatedBySignal,
                "killed",
                "FAILURE_TERMINATED_BY_SIGNAL",
            ),
            (
                FailureKind::ProducedNoOutput,
                "no output produced",
                "FAILURE_PRODUCED_NO_OUTPUT",
            ),
            (
                FailureKind::StructuralCorruption,
                "corrupt output",
                "FAILURE_STRUCTURAL_CORRUPTION",
            ),
            (
                FailureKind::LaunchFailed,
                "ffmpeg would not start",
                "FAILURE_LAUNCH_FAILED",
            ),
            (
                FailureKind::NotQueueable,
                "cannot start render",
                "FAILURE_NOT_QUEUEABLE",
            ),
            (FailureKind::Unknown, "render failed", "FAILURE_UNKNOWN"),
        ];
        for (kind, label, code) in cases {
            assert_eq!(kind.label(), label);
            assert_eq!(kind.code(), code);
        }
    }

    #[test]
    fn same_failure_compares_output_path_and_kind() {
        let base = RenderFailure {
            timestamp: 0,
            sequence_name: "seq".to_owned(),
            export_path: "/e".to_owned(),
            output_filename: "o.mov".to_owned(),
            output_path: "/e/o.mov".to_owned(),
            variant_key: "v".to_owned(),
            kind: FailureKind::DiskFull,
            summary: "a".to_owned(),
            detail: None,
            exit_code: None,
            command_line: None,
            tool_output: Vec::new(),
            occurrences: 1,
        };
        let mut other_kind = base.clone();
        other_kind.kind = FailureKind::Unknown;
        let mut other_path = base.clone();
        other_path.output_path = "/e/p.mov".to_owned();
        assert!(base.same_failure(&base.clone()));
        assert!(!base.same_failure(&other_kind));
        assert!(!base.same_failure(&other_path));
    }

    /// A suffix unlikely to name an existing path, without a random-number dependency.
    fn unique_suffix() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    }
}
