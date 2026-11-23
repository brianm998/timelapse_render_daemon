// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Render planning (§6.5): output names, frame patterns, the source size probe, the aspect ratio
//! and the ffmpeg argument vector. The Swift tests of the same behaviour are ported here with the
//! same expectations.
//!
//! Every test is inside `mod plan`, so `cargo test -p trd-core plan` selects them all. Fixtures
//! live in the system temporary directory. The size probe runs the fake ffprobe from
//! `trd-testtools` through `/usr/bin/env`, which sets its variable for that child only.

mod plan {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use trd_core::aspect::aspect_string;
    use trd_core::codecs::{self, VideoCodec};
    use trd_core::config::{Config, RenderType};
    use trd_core::pattern::{self, ImageSequencePattern, decompose};
    use trd_core::plan::{
        FAILURE_NOT_QUEUEABLE, Job, JobStatus, Plan, PlanError, PlanRequest, already_rendered,
        final_size, output_filename, plan_render, probe_size,
    };
    use trd_core::tools::{ResolvedTool, ToolCommand, ToolInfo, Tools};
    use trd_testtools::FakeTools;

    const EXTENSIONS: [&str; 2] = ["tif", "tiff"];
    const SOURCE_6000X4000: &str = r#"{"width":6000,"height":4000}"#;

    fn extensions() -> Vec<String> {
        EXTENSIONS.iter().map(|e| (*e).to_owned()).collect()
    }

    /// The config of the Swift tests: LRT_ prefixes and the tif/tiff extensions.
    fn config() -> Config {
        Config {
            image_name_prefix: "LRT_".to_owned(),
            image_sequence_dirname_prefix: "LRT_".to_owned(),
            ..Config::default()
        }
    }

    fn render(codec: &str, resolution: Option<Vec<i64>>) -> RenderType {
        RenderType {
            codec: codec.to_owned(),
            frame_rate: 30,
            resolution,
        }
    }

    /// Writes `count` empty frames `<prefix><index, zero-padded to digits><ext>` into `dir`.
    fn make_frames(dir: &Path, prefix: &str, digits: usize, ext: &str, first: i64, count: i64) {
        for index in first..first + count {
            let name = format!("{prefix}{index:0digits$}{ext}");
            fs::write(dir.join(name), b"x").expect("write a frame");
        }
    }

    /// A temporary export directory holding one sequence `seq_dir` of `LRT_` frames.
    fn export_with_sequence(seq_dir: &str, digits: usize, first: i64, count: i64) -> TempDir {
        let tmp = tempfile::tempdir().expect("temporary directory");
        let seq = tmp.path().join("export").join(seq_dir);
        fs::create_dir_all(&seq).expect("sequence directory");
        make_frames(&seq, "LRT_", digits, ".tiff", first, count);
        tmp
    }

    fn export_path(tmp: &TempDir) -> String {
        tmp.path().join("export").to_string_lossy().into_owned()
    }

    fn data_dir(tmp: &TempDir) -> PathBuf {
        tmp.path().join("data")
    }

    /// Tools whose ffprobe is `ffprobe` (or missing when `None`); the other two are unused here.
    fn tools(ffprobe: Option<ToolCommand>) -> Tools {
        fn resolved(name: &'static str, command: Option<ToolCommand>) -> ResolvedTool {
            ResolvedTool {
                info: ToolInfo {
                    name,
                    path: None,
                    version: String::new(),
                    available: command.is_some(),
                    problem: None,
                },
                command,
            }
        }
        Tools {
            ffmpeg: resolved("ffmpeg", None),
            ffprobe: resolved("ffprobe", ffprobe),
            exiftool: resolved("exiftool", None),
        }
    }

    /// The fake ffprobe, run through `/usr/bin/env` with `TRD_FAKE_FFPROBE` set to `script`.
    fn fake_ffprobe(fakes: &FakeTools, script: &str) -> ToolCommand {
        ToolCommand {
            program: OsString::from("/usr/bin/env"),
            leading_args: vec![
                OsString::from(format!("TRD_FAKE_FFPROBE={script}")),
                fakes.path().join("ffprobe").into_os_string(),
            ],
        }
    }

    /// The Swift `makePlan` of the argv tests, built field by field.
    fn plan_with(
        codec: VideoCodec,
        prefix: &str,
        suffix: &str,
        digits: usize,
        start_number: Option<i64>,
    ) -> Plan {
        Plan {
            input_dir: "/seq".to_owned(),
            image_name_prefix: prefix.to_owned(),
            image_suffix: suffix.to_owned(),
            frame_number_digits: digits,
            start_number,
            output_path: "/out/video.mov".to_owned(),
            progress_path: "/data/renders/r.progress".to_owned(),
            partial_path: "/out/.video.partial.mov".to_owned(),
            frame_rate: 30,
            aspect: "16/9".to_owned(),
            width: Some(1280),
            height: Some(720),
            codec,
            metadata: BTreeMap::new(),
            total_frames: 100,
        }
    }

    fn codec(name: &str) -> VideoCodec {
        codecs::builtin()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("codec {name} is in the built-in table"))
    }

    fn position(args: &[String], flag: &str) -> usize {
        args.iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} is in {args:?}"))
    }

    /// The argument after `flag`.
    fn value_after(args: &[String], flag: &str) -> String {
        args[position(args, flag) + 1].clone()
    }

    // MARK: - Ported from CoreTests.swift: aspect, argv and decomposition

    #[test]
    fn aspect_ratio() {
        assert_eq!(aspect_string(1920, 1080), "16/9");
        assert_eq!(aspect_string(3840, 2160), "16/9");
        assert_eq!(aspect_string(1080, 720), "3/2");
    }

    #[test]
    fn aspect_ratio_whole_multiple_and_zero_height() {
        assert_eq!(aspect_string(3000, 1000), "3/1");
        assert_eq!(aspect_string(1920, 0), "1920/0");
        assert_eq!(aspect_string(7, 5), "7/5");
    }

    #[test]
    fn ffmpeg_args_use_codec_profile_and_extra_args() {
        let args = plan_with(codec("h264"), "LRT_", ".tiff", 5, None).argv();
        assert_eq!(value_after(&args, "-profile:v"), "high");
        assert_eq!(value_after(&args, "-crf"), "18");
        assert_eq!(value_after(&args, "-c:v"), "libx264");
    }

    #[test]
    fn ffmpeg_args_prores_unchanged() {
        let args = plan_with(codec("ProResUHQ"), "LRT_", ".tiff", 5, None).argv();
        assert_eq!(value_after(&args, "-profile:v"), "1");
        assert!(!args.contains(&"-crf".to_owned()));
    }

    #[test]
    fn ffmpeg_args_omit_profile_when_nil() {
        let mut prores = codec("ProResUHQ");
        prores.ffmpeg.profile = None;
        let args = plan_with(prores, "LRT_", ".tiff", 5, None).argv();
        assert!(!args.contains(&"-profile:v".to_owned()));
    }

    #[test]
    fn decompose_handles_plain_extension() {
        let d = decompose("LRT_00001.tiff").expect("digits");
        assert_eq!(d.prefix, "LRT_");
        assert_eq!(d.digits, "00001");
        assert_eq!(d.suffix, ".tiff");
    }

    #[test]
    fn decompose_handles_decoration_before_extension() {
        // LRT_00001-severe-noise.tiff needs LRT_%05d-severe-noise.tiff, so the suffix keeps the
        // decoration.
        let d = decompose("LRT_00001-severe-noise.tiff").expect("digits");
        assert_eq!(d.prefix, "LRT_");
        assert_eq!(d.suffix, "-severe-noise.tiff");
    }

    #[test]
    fn decompose_without_prefix() {
        let d = decompose("00042.tif").expect("digits");
        assert_eq!(d.prefix, "");
        assert_eq!(d.digits, "00042");
        assert_eq!(d.suffix, ".tif");
    }

    #[test]
    fn decompose_finds_first_digit_run_anywhere_in_the_name() {
        let d = decompose("image-001.tiff").expect("digits");
        assert_eq!(d.prefix, "image-");
        assert_eq!(d.digits, "001");
        assert_eq!(d.suffix, ".tiff");
    }

    #[test]
    fn decompose_returns_none_when_there_is_no_frame_number() {
        assert_eq!(decompose("weird-no-numbers.tiff"), None);
    }

    #[test]
    fn decompose_image_filename() {
        let a = decompose("image_0001.tiff").expect("digits");
        assert_eq!(a.prefix, "image_");
        assert_eq!(a.digits, "0001");
        assert_eq!(a.suffix, ".tiff");

        let b = decompose("LRT_00001-severe-noise.tiff").expect("digits");
        assert_eq!(b.prefix, "LRT_");
        assert_eq!(b.digits, "00001");
        assert_eq!(b.suffix, "-severe-noise.tiff");

        assert_eq!(decompose("no-digits-here.tiff"), None);
    }

    // MARK: - Ported: ImageSequencePattern.detect

    fn detect(dir: &Path, preferred: &str) -> Option<ImageSequencePattern> {
        pattern::detect(dir, &extensions(), preferred)
    }

    #[test]
    fn detect_pattern_from_non_configured_naming() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        make_frames(tmp.path(), "image_", 4, ".tiff", 1, 6);

        let p = detect(tmp.path(), "LRT_").expect("a pattern");
        assert_eq!(p.prefix, "image_");
        assert_eq!(p.digits, 4);
        assert_eq!(p.suffix, ".tiff");
        assert_eq!(p.first_index, 1);
        assert_eq!(p.frame_count, 6);
        assert_eq!(p.example_filename, "image_0001.tiff");
        assert!(!p.matches_configured_prefix);
        assert!(!p.requires_explicit_start_number());
        assert_eq!(p.pattern_description(), "image_%04d.tiff");
    }

    #[test]
    fn detect_pattern_keeps_the_configured_prefix_when_both_are_present() {
        // A directory with two sequences keeps the configured one, even when it is the smaller.
        let tmp = tempfile::tempdir().expect("temporary directory");
        make_frames(tmp.path(), "LRT_", 5, ".tiff", 1, 2);
        make_frames(tmp.path(), "image_", 4, ".tiff", 1, 9);

        let p = detect(tmp.path(), "LRT_").expect("a pattern");
        assert_eq!(p.prefix, "LRT_");
        assert_eq!(p.digits, 5);
        assert_eq!(p.frame_count, 2);
        assert!(p.matches_configured_prefix);
    }

    #[test]
    fn detect_pattern_ignores_stray_files_and_picks_the_dominant_group() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        make_frames(tmp.path(), "image_", 4, ".tiff", 1, 8);
        // Noise that must not win: a lone oddly-named frame, a non-image and a dotfile.
        fs::write(tmp.path().join("preview_01.tiff"), b"x").expect("write");
        fs::write(tmp.path().join("notes.txt"), b"x").expect("write");
        fs::write(tmp.path().join(".DS_Store"), b"x").expect("write");

        let p = detect(tmp.path(), "LRT_").expect("a pattern");
        assert_eq!(p.prefix, "image_");
        assert_eq!(p.frame_count, 8);
    }

    #[test]
    fn detect_pattern_keeps_decorated_suffixes() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        make_frames(tmp.path(), "LRT_", 5, "-severe-noise.tiff", 1, 3);

        let p = detect(tmp.path(), "LRT_").expect("a pattern");
        assert_eq!(p.pattern_description(), "LRT_%05d-severe-noise.tiff");
    }

    #[test]
    fn detect_pattern_flags_sequences_starting_past_the_probe_range() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        make_frames(tmp.path(), "LRT_", 5, ".tiff", 100, 3);

        let p = detect(tmp.path(), "LRT_").expect("a pattern");
        assert_eq!(p.first_index, 100);
        assert!(p.requires_explicit_start_number());
    }

    #[test]
    fn detect_pattern_does_not_flag_sequences_starting_at_zero_or_one() {
        for start in [0, 1] {
            let tmp = tempfile::tempdir().expect("temporary directory");
            make_frames(tmp.path(), "LRT_", 5, ".tiff", start, 3);

            let p = detect(tmp.path(), "LRT_").expect("a pattern");
            assert_eq!(p.first_index, start);
            assert!(!p.requires_explicit_start_number(), "start={start}");
        }
    }

    #[test]
    fn detect_pattern_returns_none_when_nothing_is_numbered() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        fs::write(tmp.path().join("just-a-name.tiff"), b"x").expect("write");

        assert!(detect(tmp.path(), "LRT_").is_none());
    }

    #[test]
    fn detect_pattern_returns_none_for_an_unreadable_directory() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        let missing = tmp.path().join("not-here");
        assert!(detect(&missing, "LRT_").is_none());
    }

    // MARK: - Ported: plans built from a detected pattern

    #[test]
    fn plan_uses_the_detected_digit_width_and_prefix() {
        let plan = Plan {
            input_dir: "/seq".to_owned(),
            image_name_prefix: "image_".to_owned(),
            image_suffix: ".tiff".to_owned(),
            frame_number_digits: 4,
            ..plan_with(codec("ProResMQ"), "", "", 5, None)
        };
        assert_eq!(plan.input_pattern_description(), "image_%04d.tiff");
        let args = plan.argv();
        assert_eq!(value_after(&args, "-i"), "/seq/image_%04d.tiff");
    }

    #[test]
    fn plan_omits_start_number_for_ordinary_sequences() {
        let args = plan_with(codec("ProResMQ"), "LRT_", ".tiff", 5, None).argv();
        assert!(!args.contains(&"-start_number".to_owned()));
    }

    /// `-start_number` is an input option, so it has to land before `-i`.
    #[test]
    fn plan_passes_start_number_before_the_input_argument() {
        let args = plan_with(codec("ProResMQ"), "LRT_", ".tiff", 5, Some(100)).argv();
        assert_eq!(value_after(&args, "-start_number"), "100");
        assert!(position(&args, "-start_number") < position(&args, "-i"));
    }

    /// The pattern quoted in a failure report has to be the one ffmpeg was given.
    #[test]
    fn input_pattern_matches_the_argument_passed_to_ffmpeg() {
        let plan = plan_with(codec("ProResMQ"), "LRT_", ".tiff", 5, None);
        let args = plan.argv();
        assert_eq!(value_after(&args, "-i"), plan.input_pattern());
        assert_eq!(plan.input_pattern(), "/seq/LRT_%05d.tiff");
        assert_eq!(plan.input_pattern_description(), "LRT_%05d.tiff");
    }

    // MARK: - A1: golden argv

    /// The argv of §6.5 for a `[1280, 720]` render of the sequence `LRT_seq` (three frames from
    /// `LRT_00001`), with two metadata tags given out of order.
    fn golden_request_plan(codec_name: &str, export: &str, data: &Path) -> Plan {
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render(codec_name, Some(vec![1280, 720]));
        let tools = tools(None);
        let mut metadata = BTreeMap::new();
        metadata.insert("title".to_owned(), "Sunset".to_owned());
        metadata.insert("artist".to_owned(), "Brian".to_owned());
        let request = PlanRequest {
            config: &cfg,
            codecs: &codecs,
            tools: &tools,
            export_path: export,
            sequence_dir: "LRT_seq",
            render: &rt,
            data_dir: data,
            render_id: "render-1",
            metadata,
            total_frames: 3,
        };
        plan_render(&request).expect("a plan")
    }

    fn golden_fixture() -> TempDir {
        export_with_sequence("LRT_seq", 5, 1, 3)
    }

    #[test]
    fn golden_argv_prores_uhq() {
        let tmp = golden_fixture();
        let export = export_path(&tmp);
        let data = data_dir(&tmp);
        let plan = golden_request_plan("ProResUHQ", &export, &data);
        let progress = data
            .join("renders/render-1.progress")
            .to_string_lossy()
            .into_owned();
        let seq = format!("{export}/LRT_seq");
        let input = format!("{seq}/LRT_%05d.tiff");
        let partial = format!("{export}/.seq_ProRes-444_Rec.709F_720p_30_UHQ.partial.mov");

        assert_eq!(
            plan.output_path,
            format!("{export}/seq_ProRes-444_Rec.709F_720p_30_UHQ.mov")
        );
        assert_eq!(
            plan.argv(),
            [
                "-nostdin", "-hide_banner", "-nostats", "-y",
                "-progress", progress.as_str(), "-stats_period", "0.5",
                "-r", "30",
                "-i", input.as_str(),
                "-aspect", "16/9",
                "-filter_complex", "crop=floor(iw/2)*2:floor(ih/2)*2,scale=1280:720:in_range=full:out_range=full,setsar=1/1",
                "-c:v", "prores_ks", "-pix_fmt", "yuv422p10le", "-threads", "0",
                "-profile:v", "1",
                "-movflags", "+write_colr", "-an",
                "-color_range", "pc", "-color_primaries", "bt709", "-colorspace", "bt709", "-color_trc", "bt709",
                "-timecode", "00:00:00:00",
                "-metadata", "artist=Brian", "-metadata", "title=Sunset",
                partial.as_str(),
            ]
            .map(str::to_owned)
            .to_vec()
        );
    }

    #[test]
    fn golden_argv_prores_mq() {
        let tmp = golden_fixture();
        let export = export_path(&tmp);
        let data = data_dir(&tmp);
        let plan = golden_request_plan("ProResMQ", &export, &data);
        let progress = data
            .join("renders/render-1.progress")
            .to_string_lossy()
            .into_owned();
        let seq = format!("{export}/LRT_seq");
        let input = format!("{seq}/LRT_%05d.tiff");
        let partial = format!("{export}/.seq_ProRes-422_Rec.709F_720p_30_MQ.partial.mov");

        assert_eq!(
            plan.argv(),
            [
                "-nostdin", "-hide_banner", "-nostats", "-y",
                "-progress", progress.as_str(), "-stats_period", "0.5",
                "-r", "30",
                "-i", input.as_str(),
                "-aspect", "16/9",
                "-filter_complex", "crop=floor(iw/2)*2:floor(ih/2)*2,scale=1280:720:in_range=full:out_range=full,setsar=1/1",
                "-c:v", "prores_ks", "-pix_fmt", "yuv444p10le", "-threads", "0",
                "-profile:v", "1",
                "-movflags", "+write_colr", "-an",
                "-color_range", "pc", "-color_primaries", "bt709", "-colorspace", "bt709", "-color_trc", "bt709",
                "-timecode", "00:00:00:00",
                "-metadata", "artist=Brian", "-metadata", "title=Sunset",
                partial.as_str(),
            ]
            .map(str::to_owned)
            .to_vec()
        );
    }

    #[test]
    fn golden_argv_h264() {
        let tmp = golden_fixture();
        let export = export_path(&tmp);
        let data = data_dir(&tmp);
        let plan = golden_request_plan("h264", &export, &data);
        let progress = data
            .join("renders/render-1.progress")
            .to_string_lossy()
            .into_owned();
        let seq = format!("{export}/LRT_seq");
        let input = format!("{seq}/LRT_%05d.tiff");
        let partial = format!("{export}/.seq_H264-420_Rec.709F_720p_30_HQ.partial.mov");

        assert_eq!(
            plan.argv(),
            [
                "-nostdin", "-hide_banner", "-nostats", "-y",
                "-progress", progress.as_str(), "-stats_period", "0.5",
                "-r", "30",
                "-i", input.as_str(),
                "-aspect", "16/9",
                "-filter_complex", "crop=floor(iw/2)*2:floor(ih/2)*2,scale=1280:720:in_range=full:out_range=full,setsar=1/1",
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-threads", "0",
                "-profile:v", "high",
                "-crf", "18",
                "-movflags", "+write_colr", "-an",
                "-color_range", "pc", "-color_primaries", "bt709", "-colorspace", "bt709", "-color_trc", "bt709",
                "-timecode", "00:00:00:00",
                "-metadata", "artist=Brian", "-metadata", "title=Sunset",
                partial.as_str(),
            ]
            .map(str::to_owned)
            .to_vec()
        );
    }

    // MARK: - A2, A3: the source size

    #[test]
    fn original_size_aspect_from_source() {
        let fakes = FakeTools::new().expect("fake tools");
        let tmp = export_with_sequence("LRT_seq", 5, 1, 3);
        let export = export_path(&tmp);
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render("ProResUHQ", None);
        let tools = tools(Some(fake_ffprobe(&fakes, SOURCE_6000X4000)));
        let data = data_dir(&tmp);
        let request = PlanRequest {
            config: &cfg,
            codecs: &codecs,
            tools: &tools,
            export_path: &export,
            sequence_dir: "LRT_seq",
            render: &rt,
            data_dir: &data,
            render_id: "render-2",
            metadata: BTreeMap::new(),
            total_frames: 3,
        };
        let plan = plan_render(&request).expect("a plan");

        assert_eq!(plan.aspect, "3/2");
        assert_eq!(plan.width, None);
        assert_eq!(plan.height, None);
        let args = plan.argv();
        assert_eq!(value_after(&args, "-aspect"), "3/2");
        assert!(!args.contains(&"-filter_complex".to_owned()));
    }

    #[test]
    fn width_only() {
        let fakes = FakeTools::new().expect("fake tools");
        let tmp = export_with_sequence("LRT_seq", 5, 1, 3);
        let export = export_path(&tmp);
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render("ProResMQ", Some(vec![720]));
        let tools = tools(Some(fake_ffprobe(&fakes, SOURCE_6000X4000)));
        let data = data_dir(&tmp);
        let request = PlanRequest {
            config: &cfg,
            codecs: &codecs,
            tools: &tools,
            export_path: &export,
            sequence_dir: "LRT_seq",
            render: &rt,
            data_dir: &data,
            render_id: "render-3",
            metadata: BTreeMap::new(),
            total_frames: 3,
        };
        let plan = plan_render(&request).expect("a plan");

        assert_eq!((plan.width, plan.height), (Some(720), Some(480)));
        assert_eq!(plan.aspect, "3/2");
        assert!(plan.output_path.contains("_720w_"), "{}", plan.output_path);
        let args = plan.argv();
        assert_eq!(
            value_after(&args, "-filter_complex"),
            "crop=floor(iw/2)*2:floor(ih/2)*2,scale=720:480:in_range=full:out_range=full,setsar=1/1"
        );
    }

    #[test]
    fn final_size_from_width_and_source() {
        // Height is round(W * srcH / srcW), then one more when that is odd.
        assert_eq!(final_size(Some(720), None, (6000, 4000)), (720, 480));
        assert_eq!(final_size(Some(721), None, (1000, 1000)), (721, 722));
        assert_eq!(final_size(Some(1280), Some(720), (6000, 4000)), (1280, 720));
        assert_eq!(final_size(None, None, (6000, 4000)), (6000, 4000));
    }

    // MARK: - A5: names

    /// The `_720p_`, `_2160p_`, `_OriRes_` and `_720w_` names of config.json and gui_config.json,
    /// and every `rendering` entry of both files.
    #[test]
    fn names_match_older_daemons() {
        let codecs = codecs::builtin();
        let dir = "LRT_2026-05-01_sequence";

        let mq = render("ProResMQ", Some(vec![1080, 720]));
        assert_eq!(
            output_filename(&config(), &codecs, &mq, dir),
            "2026-05-01_sequence_ProRes-422_Rec.709F_720p_30_MQ.mov"
        );
        let mq_4k = render("ProResMQ", Some(vec![3840, 2160]));
        assert_eq!(
            output_filename(&config(), &codecs, &mq_4k, dir),
            "2026-05-01_sequence_ProRes-422_Rec.709F_2160p_30_MQ.mov"
        );
        let h264_720 = render("h264", Some(vec![720]));
        assert_eq!(
            output_filename(&config(), &codecs, &h264_720, dir),
            "2026-05-01_sequence_H264-420_Rec.709F_720w_30_HQ.mov"
        );
        let uhq = render("ProResUHQ", None);
        assert_eq!(
            output_filename(&config(), &codecs, &uhq, dir),
            "2026-05-01_sequence_ProRes-444_Rec.709F_OriRes_30_UHQ.mov"
        );
        // Both prefixes are stripped repeatedly.
        assert_eq!(
            output_filename(&config(), &codecs, &uhq, "LRT_LRT_2026_seq"),
            "2026_seq_ProRes-444_Rec.709F_OriRes_30_UHQ.mov"
        );
    }

    #[test]
    fn names_match_the_renderings_of_the_repository_configs() {
        let codecs = codecs::builtin();
        for file in ["config.json", "gui_config.json"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../..")
                .join(file);
            let text = fs::read_to_string(&path).expect("a repository config");
            let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
            let cfg = Config::from_value(&value).expect("a valid config");
            assert!(!cfg.rendering.is_empty(), "{file} has renderings");
            for rt in &cfg.rendering {
                let name = output_filename(&cfg, &codecs, rt, "LRT_2026_seq");
                let filename = &codecs[&rt.codec].filename;
                let expected = format!(
                    "2026_seq_{}-{}_Rec.709F_{}_{}_{}.mov",
                    filename.codec,
                    filename.pixel_format,
                    rt.resolution_label(),
                    rt.frame_rate,
                    filename.quality
                );
                assert_eq!(name, expected, "{file}: {}", rt.variant_key());
            }
        }
    }

    // MARK: - A6: already rendered

    #[test]
    fn already_rendered_locations() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        let root = tmp.path();
        let master_2025 = root.join("master").join("2025");
        let export = root.join("export");
        let elsewhere = root.join("elsewhere");
        for dir in [&master_2025, &export, &elsewhere] {
            fs::create_dir_all(dir).expect("directory");
        }
        let cfg = Config {
            master_videos: vec![
                root.join("master")
                    .join("$YEAR")
                    .to_string_lossy()
                    .into_owned(),
            ],
            export_paths: vec![export.to_string_lossy().into_owned()],
            ..Config::default()
        };
        let export_str = export.to_string_lossy().into_owned();
        let in_master = "in_master.mov";
        let in_export = "in_export.mov";
        let in_other = "in_other.mov";
        fs::write(master_2025.join(in_master), b"x").expect("write");
        fs::write(export.join(in_export), b"x").expect("write");
        fs::write(elsewhere.join(in_other), b"x").expect("write");

        // A `$YEAR` master directory expands to 2025 for the year 2026.
        assert!(already_rendered(&cfg, &export_str, in_master, &[], 2026));
        assert!(already_rendered(&cfg, &export_str, in_export, &[], 2026));
        // A file in a directory the config does not name is not rendered.
        assert!(!already_rendered(&cfg, &export_str, in_other, &[], 2026));
        // Nothing in the config's directories: not rendered.
        assert!(!already_rendered(&cfg, &export_str, "new.mov", &[], 2026));

        let target = format!("{export_str}/new.mov");
        let job = |status| Job {
            status,
            output_path: target.clone(),
        };
        for status in [JobStatus::Queued, JobStatus::Running, JobStatus::Suspended] {
            assert!(
                already_rendered(&cfg, &export_str, "new.mov", &[job(status)], 2026),
                "{status:?} owns its output"
            );
        }
        for status in [JobStatus::Done, JobStatus::Failed, JobStatus::Cancelled] {
            assert!(
                !already_rendered(&cfg, &export_str, "new.mov", &[job(status)], 2026),
                "{status:?} does not own its output"
            );
        }
        // An in-flight job for another output path does not count.
        let other = Job {
            status: JobStatus::Running,
            output_path: format!("{}/other.mov", export_str),
        };
        assert!(!already_rendered(
            &cfg,
            &export_str,
            "new.mov",
            &[other],
            2026
        ));
    }

    #[test]
    fn job_status_labels_match_render_jobs() {
        let labels: Vec<&str> = [
            JobStatus::Queued,
            JobStatus::Running,
            JobStatus::Suspended,
            JobStatus::Done,
            JobStatus::Failed,
            JobStatus::Cancelled,
        ]
        .map(JobStatus::label)
        .to_vec();
        assert_eq!(
            labels,
            [
                "queued",
                "running",
                "suspended",
                "done",
                "failed",
                "cancelled"
            ]
        );
    }

    // MARK: - A7: failures

    /// A `FAILURE_NOT_QUEUEABLE` error with the summary `summary`.
    fn not_queueable(error: &PlanError) -> &str {
        assert_eq!(error.failure_kind(), FAILURE_NOT_QUEUEABLE);
        match error {
            PlanError::NotQueueable { summary, .. } => summary,
            other => panic!("expected NotQueueable, got {other:?}"),
        }
    }

    #[test]
    fn probe_failure() {
        let fakes = FakeTools::new().expect("fake tools");
        let tmp = export_with_sequence("LRT_seq", 5, 1, 3);
        let export = export_path(&tmp);
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render("ProResMQ", Some(vec![720]));
        let data = data_dir(&tmp);
        let frame = Path::new(&export).join("LRT_seq").join("LRT_00001.tiff");
        let expected = format!("could not read the frame size of {}", frame.display());

        // The fake ffprobe exits non-zero on a script that is not JSON.
        let broken = tools(Some(fake_ffprobe(&fakes, "{\"width\":")));
        // And a tool that is not available at all.
        let missing = tools(None);
        for tools in [&broken, &missing] {
            let request = PlanRequest {
                config: &cfg,
                codecs: &codecs,
                tools,
                export_path: &export,
                sequence_dir: "LRT_seq",
                render: &rt,
                data_dir: &data,
                render_id: "render-4",
                metadata: BTreeMap::new(),
                total_frames: 3,
            };
            let error = plan_render(&request).expect_err("no frame size");
            assert_eq!(not_queueable(&error), expected);
        }
    }

    #[test]
    fn no_pattern() {
        let tmp = tempfile::tempdir().expect("temporary directory");
        let export = tmp.path().join("export");
        let seq = export.join("LRT_seq");
        fs::create_dir_all(&seq).expect("directory");
        fs::write(seq.join("notes.txt"), b"x").expect("write");
        let export_str = export.to_string_lossy().into_owned();
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render("ProResMQ", Some(vec![1080, 720]));
        let tools = tools(None);
        let data = data_dir(&tmp);
        let request = PlanRequest {
            config: &cfg,
            codecs: &codecs,
            tools: &tools,
            export_path: &export_str,
            sequence_dir: "LRT_seq",
            render: &rt,
            data_dir: &data,
            render_id: "render-5",
            metadata: BTreeMap::new(),
            total_frames: 0,
        };
        let error = plan_render(&request).expect_err("no numbered frames");
        assert_eq!(
            not_queueable(&error),
            format!("no numbered image sequence in {}", seq.display())
        );
    }

    #[test]
    fn unknown_codec_is_an_error_not_a_panic() {
        let tmp = export_with_sequence("LRT_seq", 5, 1, 3);
        let export = export_path(&tmp);
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render("NoSuchCodec", Some(vec![720]));
        let tools = tools(None);
        let data = data_dir(&tmp);
        let request = PlanRequest {
            config: &cfg,
            codecs: &codecs,
            tools: &tools,
            export_path: &export,
            sequence_dir: "LRT_seq",
            render: &rt,
            data_dir: &data,
            render_id: "render-6",
            metadata: BTreeMap::new(),
            total_frames: 3,
        };
        assert_eq!(
            plan_render(&request),
            Err(PlanError::UnknownCodec {
                codec: "NoSuchCodec".to_owned()
            })
        );
    }

    #[test]
    fn probe_size_reads_the_fake_ffprobe() {
        let fakes = FakeTools::new().expect("fake tools");
        let tmp = tempfile::tempdir().expect("temporary directory");
        let frame = tmp.path().join("LRT_00001.tiff");
        fs::write(&frame, b"x").expect("write");
        let tools = tools(Some(fake_ffprobe(&fakes, SOURCE_6000X4000)));
        assert_eq!(probe_size(&tools, &frame), Ok((6000, 4000)));
    }

    #[test]
    fn plan_starts_past_probe_range_with_start_number() {
        let tmp = export_with_sequence("LRT_seq", 5, 100, 3);
        let export = export_path(&tmp);
        let cfg = config();
        let codecs = codecs::builtin();
        let rt = render("ProResMQ", Some(vec![1280, 720]));
        let tools = tools(None);
        let data = data_dir(&tmp);
        let request = PlanRequest {
            config: &cfg,
            codecs: &codecs,
            tools: &tools,
            export_path: &export,
            sequence_dir: "LRT_seq",
            render: &rt,
            data_dir: &data,
            render_id: "render-7",
            metadata: BTreeMap::new(),
            total_frames: 3,
        };
        let plan = plan_render(&request).expect("a plan");
        assert_eq!(plan.start_number, Some(100));
        let args = plan.argv();
        assert_eq!(value_after(&args, "-start_number"), "100");
        assert!(position(&args, "-start_number") < position(&args, "-i"));
    }

    #[test]
    fn plan_round_trips_through_json() {
        let plan = plan_with(codec("h264"), "LRT_", ".tiff", 5, Some(7));
        let text = serde_json::to_string(&plan).expect("JSON");
        let back: Plan = serde_json::from_str(&text).expect("a plan");
        assert_eq!(back, plan);
        assert_eq!(back.argv(), plan.argv());
    }
}
