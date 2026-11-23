// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of config.json and video_codecs.json (§6.1, §6.2) and $YEAR expansion (§6.1).
//!
//! Every test is inside `mod config`, so `cargo test -p trd-core config` selects them all.
//! The repository's own JSON files are only ever read. Writes go to temporary directories.

mod config {
    use std::fs::{self, FileTimes};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use trd_core::codecs::{
        self, CodecError, CodecSource, CodecStore, FfmpegCodec, FilenameCodec, VideoCodec,
        VideoCodecs,
    };
    use trd_core::config::{
        self, Config, ConfigError, ConfigIssue, ConfigStore, PROBLEM_CONFIG_INVALID,
        PROBLEM_CONFIG_ISSUES, PROBLEM_CONFIG_MISSING, PROBLEM_UNKNOWN_CODEC, Severity, Status,
    };
    use trd_core::paths::{self, Clock, ExpansionCache, ManualClock};

    const YEAR: i32 = 2026;

    /// A file of the repository root, read in place. Never written.
    fn repo_file(name: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .join(name);
        assert!(
            path.is_file(),
            "repository file {} is missing",
            path.display()
        );
        path
    }

    fn repo_codecs() -> VideoCodecs {
        let text =
            fs::read_to_string(repo_file("video_codecs.json")).expect("read video_codecs.json");
        codecs::parse_codecs(&text).expect("video_codecs.json parses")
    }

    fn cfg(json: &str) -> Config {
        let value: Value = serde_json::from_str(json).expect("test JSON parses");
        Config::from_value(&value).expect("test config decodes")
    }

    fn validate_json(json: &str) -> Vec<ConfigIssue> {
        config::validate(&cfg(json), &codecs::builtin(), YEAR)
    }

    /// Asserts that the only ERROR is at `field`.
    fn assert_single_error(json: &str, field: &str) {
        let errors: Vec<ConfigIssue> = validate_json(json)
            .into_iter()
            .filter(ConfigIssue::is_error)
            .collect();
        assert_eq!(
            errors.len(),
            1,
            "expected exactly one ERROR, got {errors:?}"
        );
        assert_eq!(errors[0].field, field);
    }

    fn assert_warning(issues: &[ConfigIssue], field: &str) {
        assert!(
            issues
                .iter()
                .any(|i| i.severity == Severity::Warning && i.field == field),
            "expected a WARNING at {field}, got {issues:?}"
        );
    }

    fn temp() -> TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn store_for(path: &Path) -> ConfigStore {
        ConfigStore::new(path, Arc::new(ManualClock::new(YEAR)))
    }

    /// Copies a repository file into `dir` under `name`.
    fn copy_repo_file(dir: &Path, repo_name: &str, name: &str) -> PathBuf {
        let target = dir.join(name);
        fs::copy(repo_file(repo_name), &target).expect("copy repository file");
        target
    }

    /// Moves a file's mtime an hour ahead, so a reload sees the change.
    fn touch_later(path: &Path) {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for set_times");
        let when = SystemTime::now() + Duration::from_secs(3600);
        file.set_times(FileTimes::new().set_modified(when))
            .expect("set mtime");
    }

    fn read_value(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).expect("read file")).expect("file parses")
    }

    fn codec(codec: &str, pixel_format: &str) -> VideoCodec {
        VideoCodec {
            ffmpeg: FfmpegCodec {
                codec: codec.to_owned(),
                pixel_format: pixel_format.to_owned(),
                color_range: "pc".to_owned(),
                profile: None,
                extra_args: None,
            },
            filename: FilenameCodec {
                pixel_format: "420".to_owned(),
                codec: "Test".to_owned(),
                quality: "HQ".to_owned(),
            },
        }
    }

    // ---- Ported from the Swift CoreTests ----

    #[test]
    fn config_decodes_perl_format() {
        let json = r#"{
            "export_paths": ["/a", "/b"],
            "source_paths": ["/x"],
            "backup_paths": [],
            "master_videos": ["/m"],
            "max_processes": { "ffmpeg": 4, "exiftool": 20 },
            "max_cuncurrent_renders": 2,
            "image_name_prefix": "LRT_",
            "image_sequence_dirname_prefix": "LRT_",
            "raw_types": ["ARW", "ORF"],
            "rendering": [
              { "resolution": [1920, 1080], "codec": "ProResMQ", "frame_rate": 30 },
              { "codec": "ProResUHQ", "frame_rate": 30 }
            ],
            "log_in_color": true,
            "sleep_time_seconds": 12,
            "progress_bar_width": 40,
            "ignore_sequence_regexes": ["-aligned$"],
            "delete_sequence_after_render_regexes": ["-merge$"]
        }"#;
        let c = cfg(json);
        assert_eq!(c.export_paths, ["/a", "/b"]);
        assert_eq!(c.max_cuncurrent_renders, 2);
        assert_eq!(c.max_processes.ffmpeg, 4);
        assert_eq!(c.rendering.len(), 2);
        assert_eq!(c.rendering[0].height(), Some(1080));
        assert_eq!(c.rendering[1].resolution, None);
        assert_eq!(c.image_name_prefix, "LRT_");
        assert_eq!(c.ignore_sequence_regexes, ["-aligned$"]);
    }

    #[test]
    fn config_defaults_for_render_history() {
        // An old-format config with no history fields still decodes, with the defaults.
        let c = cfg(r#"{ "rendering": [ { "codec": "ProResMQ", "frame_rate": 30 } ] }"#);
        assert_eq!(c.recent_render_history_days, 30);
        assert_eq!(c.render_history_path, None);
        // And every other default of §6.1 holds for an empty object.
        let d = cfg("{}");
        assert_eq!(d, Config::default());
        assert_eq!(d.max_processes.ffmpeg, 1);
        assert_eq!(d.max_processes.exiftool, 10);
        assert_eq!(d.max_cuncurrent_renders, 1);
        assert_eq!(d.raw_types, ["ARW", "ORF", "DNG"]);
        assert_eq!(d.image_extensions, ["tif", "tiff"]);
        assert_eq!(d.scan_interval_seconds, 2.0);
        assert_eq!(d.sleep_time_seconds, 12.0);
        assert_eq!(d.progress_bar_width, 40);
        assert!(d.log_in_color);
    }

    #[test]
    fn year_expansion_preserves_non_placeholder_paths() {
        // Without $YEAR, paths are returned unchanged (no existence check).
        let input = vec!["/foo/bar".to_owned(), "/baz".to_owned()];
        let mut cache = ExpansionCache::new();
        assert_eq!(
            paths::expand_years(&input, &ManualClock::new(YEAR), &mut cache),
            input
        );
    }

    #[test]
    fn year_expansion_expands_existing_directories_only() {
        // Three "year" directories in a temp tree; only those come back, newest first.
        let root = temp();
        for year in [2020, 2023, 2025] {
            fs::create_dir(root.path().join(year.to_string())).expect("make year dir");
        }
        let pattern = format!("{}/$YEAR", root.path().display());
        let expanded = paths::expand_year_path(&pattern, 2026);
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0], format!("{}/2025", root.path().display()));
        assert_eq!(expanded[1], format!("{}/2023", root.path().display()));
        assert_eq!(expanded[2], format!("{}/2020", root.path().display()));
        assert!(!expanded.iter().any(|p| p.ends_with("/2024")));
    }

    #[test]
    fn codecs_path_alongside_config() {
        let root = temp();
        let config_path = root.path().join("config.json");
        fs::write(&config_path, "{}").expect("write config");
        // No video_codecs.json next to the config: None.
        assert_eq!(codecs::codecs_path_alongside_config(&config_path), None);
        let codecs_path = root.path().join("video_codecs.json");
        fs::write(&codecs_path, "{}").expect("write codecs");
        assert_eq!(
            codecs::codecs_path_alongside_config(&config_path),
            Some(codecs_path)
        );
    }

    #[test]
    fn default_codec_table_includes_h264() {
        let table = codecs::builtin();
        let h264 = table.get("h264").expect("h264 is built in");
        assert_eq!(h264.ffmpeg.codec, "libx264");
        assert_eq!(h264.ffmpeg.profile.as_deref(), Some("high"));
        assert_eq!(
            h264.ffmpeg.extra_args,
            Some(vec!["-crf".to_owned(), "18".to_owned()])
        );
        // ProRes keeps the -profile:v 1 the Perl daemon hardcoded.
        assert_eq!(table["ProResUHQ"].ffmpeg.profile.as_deref(), Some("1"));
        assert_eq!(table["ProResMQ"].ffmpeg.profile.as_deref(), Some("1"));
    }

    #[test]
    fn codec_table_decodes_profile_and_extra_args() {
        let json = r#"{
          "h264": {
            "ffmpeg": { "codec": "libx264", "pixel_format": "yuv420p",
                        "color_range": "pc", "profile": "high",
                        "extra_args": ["-crf", "18"] },
            "filename": { "pixel_format": "420", "codec": "H264", "quality": "HQ" }
          },
          "legacy": {
            "ffmpeg": { "codec": "prores_ks", "pixel_format": "yuv422p10le",
                        "color_range": "pc" },
            "filename": { "pixel_format": "444", "codec": "ProRes", "quality": "UHQ" }
          }
        }"#;
        let table = codecs::parse_codecs(json).expect("table decodes");
        assert_eq!(table["h264"].ffmpeg.profile.as_deref(), Some("high"));
        assert_eq!(
            table["h264"].ffmpeg.extra_args,
            Some(vec!["-crf".to_owned(), "18".to_owned()])
        );
        // Entries without the new keys still decode (older video_codecs.json).
        assert_eq!(table["legacy"].ffmpeg.profile, None);
        assert_eq!(table["legacy"].ffmpeg.extra_args, None);
    }

    #[test]
    fn variant_key_stable() {
        let r1 = config::RenderType {
            codec: "ProResMQ".into(),
            frame_rate: 30,
            resolution: Some(vec![3840, 2160]),
        };
        let r2 = config::RenderType {
            codec: "ProResUHQ".into(),
            frame_rate: 30,
            resolution: None,
        };
        assert_eq!(r1.variant_key(), "ProResMQ_3840x2160_30");
        assert_eq!(r2.variant_key(), "ProResUHQ_ori_30");
    }

    #[test]
    fn width_only_render_type() {
        let r = config::RenderType {
            codec: "ProResMQ".into(),
            frame_rate: 30,
            resolution: Some(vec![1080]),
        };
        assert_eq!(r.width(), Some(1080));
        assert_eq!(r.height(), None);
        assert_eq!(r.variant_key(), "ProResMQ_1080w_30");
        assert_eq!(r.resolution_label(), "1080w");
    }

    // ---- Repository files and the Perl/Swift file format ----

    #[test]
    fn config_repo_files_load_clean() {
        let path = repo_file("config.json");
        let mut store = store_for(&path);
        assert!(store.reload_if_changed(&repo_codecs()));
        assert_eq!(store.status(), Status::Loaded);
        let errors: Vec<&ConfigIssue> = store.issues().iter().filter(|i| i.is_error()).collect();
        assert!(
            errors.is_empty(),
            "repository config.json has ERROR issues: {errors:?}"
        );
        assert!(store.problem_ids().is_empty());
        // The repository codec table loads with no issues either.
        let table = CodecStore::open(None, path);
        assert_eq!(table.source(), CodecSource::Sibling);
        assert!(table.issues().is_empty());
    }

    #[test]
    fn config_repo_round_trip_keeps_keys() {
        let text = fs::read_to_string(repo_file("config.json")).expect("read config.json");
        let raw: Value = serde_json::from_str(&text).expect("config.json parses");
        let loaded = Config::from_value(&raw).expect("config.json decodes");
        let written = loaded.to_json_pretty();
        let again: Value = serde_json::from_str(&written).expect("written config parses");
        // Every key of the file is still there with the same value.
        for (key, value) in raw.as_object().expect("object") {
            assert_eq!(
                again.get(key),
                Some(value),
                "key {key} changed on round trip"
            );
        }
        // Every known key is written, including the defaulted ones absent from the file.
        for key in config::KNOWN_KEYS {
            if key == "render_history_path" {
                continue;
            }
            assert!(
                again.get(key).is_some(),
                "known key {key} missing from the output"
            );
        }
        // And reading the written text gives back the same config.
        assert_eq!(Config::from_value(&again).expect("decodes"), loaded);
    }

    #[test]
    fn config_gui_config_loads() {
        // gui_config.json is valid JSON: it loads, and reports no parse problem.
        let mut store = store_for(&repo_file("gui_config.json"));
        store.reload_if_changed(&repo_codecs());
        assert_eq!(store.status(), Status::Loaded);
        assert!(!store.problem_ids().contains(&PROBLEM_CONFIG_INVALID));
        assert_eq!(
            store.config().map(|c| c.recent_render_history_days),
            Some(30)
        );
    }

    #[test]
    fn config_null_known_key_takes_default() {
        let c = cfg(
            r#"{ "scan_interval_seconds": null, "max_processes": { "ffmpeg": null, "exiftool": 3 } }"#,
        );
        assert_eq!(c.scan_interval_seconds, 2.0);
        assert_eq!(c.max_processes.ffmpeg, 1);
        assert_eq!(c.max_processes.exiftool, 3);
    }

    #[test]
    fn config_wrong_type_fails_whole_file() {
        // A known key of the wrong JSON type fails the file, as the Swift decoder did.
        for json in [
            r#"{ "export_paths": "/a" }"#,
            r#"{ "max_cuncurrent_renders": 1.5 }"#,
        ] {
            let value: Value = serde_json::from_str(json).expect("parses");
            assert!(
                Config::from_value(&value).is_err(),
                "{json} should not decode"
            );
        }
    }

    #[test]
    fn invalid_json_keeps_last_good() {
        // A2: gui_config.json with a trailing comma injected is invalid. The last good config stays.
        let dir = temp();
        let path = copy_repo_file(dir.path(), "gui_config.json", "config.json");
        let mut store = store_for(&path);
        store.reload_if_changed(&repo_codecs());
        let good = store.config().cloned().expect("the good config loaded");

        let text = fs::read_to_string(&path).expect("read");
        let end = text.rfind('}').expect("object end");
        let broken = format!("{},{}", &text[..end], &text[end..]);
        assert!(
            serde_json::from_str::<Value>(&broken).is_err(),
            "the injected comma must break JSON"
        );
        fs::write(&path, broken).expect("write broken");
        touch_later(&path);

        assert!(
            store.reload_if_changed(&repo_codecs()),
            "a changed file is a change"
        );
        assert_eq!(store.status(), Status::Invalid);
        assert_eq!(store.problem_ids(), [PROBLEM_CONFIG_INVALID]);
        assert_eq!(store.config(), Some(&good));
        assert_eq!(store.issues().len(), 1);
        assert!(
            !store.reload_if_changed(&repo_codecs()),
            "the same bad file is logged once, not again"
        );
    }

    #[test]
    fn unknown_keys_survive() {
        // A3: a top-level key this version does not know survives apply.
        let dir = temp();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{ "scan_interval_seconds": 2, "owner_note": { "a": [1, 2] }, "zeta": true }"#,
        )
        .expect("write");
        let codecs = codecs::builtin();
        let mut store = store_for(&path);
        store.reload_if_changed(&codecs);
        let revision = store.revision().to_owned();

        store
            .apply(|c| c.progress_bar_width = 50, &revision, &codecs)
            .expect("the edit applies");

        let written = read_value(&path);
        assert_eq!(written["owner_note"], json!({ "a": [1, 2] }));
        assert_eq!(written["zeta"], json!(true));
        assert_eq!(written["progress_bar_width"], json!(50));
        assert_eq!(written["scan_interval_seconds"], json!(2));
    }

    #[test]
    fn stale_revision_refused() {
        // A4: an edit with a stale revision is refused and the file is untouched.
        let dir = temp();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{ "progress_bar_width": 40 }"#).expect("write");
        let codecs = codecs::builtin();
        let mut store = store_for(&path);
        store.reload_if_changed(&codecs);
        let before = fs::read(&path).expect("read");

        let refused = store.apply(|c| c.progress_bar_width = 1, "stale-revision", &codecs);
        assert!(
            matches!(refused, Err(ConfigError::StaleRevision { .. })),
            "got {refused:?}"
        );
        assert_eq!(
            fs::read(&path).expect("read"),
            before,
            "a refused edit writes nothing"
        );

        // A revision that was current but has been superseded is stale too.
        let first = store.revision().to_owned();
        store
            .apply(|c| c.progress_bar_width = 41, &first, &codecs)
            .expect("applies");
        let again = store.apply(|c| c.progress_bar_width = 42, &first, &codecs);
        assert!(
            matches!(again, Err(ConfigError::StaleRevision { .. })),
            "got {again:?}"
        );
        assert_eq!(read_value(&path)["progress_bar_width"], json!(41));
    }

    #[test]
    fn apply_refused_when_result_has_an_error() {
        let dir = temp();
        let path = dir.path().join("config.json");
        fs::write(&path, "{}").expect("write");
        let codecs = codecs::builtin();
        let mut store = store_for(&path);
        store.reload_if_changed(&codecs);
        let revision = store.revision().to_owned();
        let before = fs::read(&path).expect("read");

        let refused = store.apply(|c| c.scan_interval_seconds = 0.1, &revision, &codecs);
        match refused {
            Err(ConfigError::Refused(issues)) => {
                assert!(issues.iter().all(ConfigIssue::is_error));
                assert_eq!(issues[0].field, "/scan_interval_seconds");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        assert_eq!(fs::read(&path).expect("read"), before);
    }

    #[test]
    fn apply_creates_a_missing_file_with_defaults() {
        let dir = temp();
        let path = dir.path().join("config.json");
        let codecs = codecs::builtin();
        let mut store = store_for(&path);
        store.reload_if_changed(&codecs);
        assert_eq!(store.status(), Status::Missing);
        let revision = store.revision().to_owned();

        store
            .apply(|c| c.log_in_color = false, &revision, &codecs)
            .expect("creates the file");

        let written = read_value(&path);
        assert_eq!(written["log_in_color"], json!(false));
        assert_eq!(written["max_cuncurrent_renders"], json!(1));
        assert_eq!(store.status(), Status::Loaded);
    }

    #[test]
    fn apply_write_is_not_reloaded() {
        // After a write, the store has recorded the new mtime and size, so it does not reload.
        let dir = temp();
        let path = dir.path().join("config.json");
        fs::write(&path, "{}").expect("write");
        let codecs = codecs::builtin();
        let mut store = store_for(&path);
        store.reload_if_changed(&codecs);
        let revision = store.revision().to_owned();
        store
            .apply(|c| c.progress_bar_width = 44, &revision, &codecs)
            .expect("applies");
        assert!(!store.reload_if_changed(&codecs));
    }

    #[test]
    fn missing_file_is_problem_config_missing() {
        let dir = temp();
        let mut store = store_for(&dir.path().join("absent.json"));
        assert!(
            !store.reload_if_changed(&codecs::builtin()),
            "no file, no change"
        );
        assert_eq!(store.status(), Status::Missing);
        assert_eq!(store.problem_ids(), [PROBLEM_CONFIG_MISSING]);
        assert!(store.config().is_none());
    }

    #[test]
    fn reload_picks_up_an_external_edit() {
        let dir = temp();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{ "progress_bar_width": 40 }"#).expect("write");
        let codecs = codecs::builtin();
        let mut store = store_for(&path);
        store.reload_if_changed(&codecs);
        assert_eq!(store.config().map(|c| c.progress_bar_width), Some(40));

        fs::write(&path, r#"{ "progress_bar_width": 60 }"#).expect("write");
        touch_later(&path);
        assert!(store.reload_if_changed(&codecs));
        assert_eq!(store.config().map(|c| c.progress_bar_width), Some(60));
        assert!(
            !store.reload_if_changed(&codecs),
            "an unchanged file is not read again"
        );
    }

    #[test]
    fn load_drops_bad_entries_keeps_rest() {
        // An entry with an ERROR is ignored, the rest is used, and the problems are raised.
        let text = r#"{
            "export_paths": ["/abs", "relative"],
            "ignore_sequence_regexes": ["ok$", "(unclosed"],
            "max_processes": { "ffmpeg": 0, "exiftool": 7 },
            "rendering": [
              { "codec": "h264", "frame_rate": 30 },
              { "codec": "nope", "frame_rate": 30 }
            ],
            "progress_bar_width": 33
        }"#;
        let loaded = config::load_text(text, &codecs::builtin(), YEAR).expect("parses");
        assert_eq!(loaded.config.export_paths, ["/abs"]);
        assert_eq!(loaded.config.ignore_sequence_regexes, ["ok$"]);
        assert_eq!(
            loaded.config.max_processes.ffmpeg, 1,
            "a scalar with an ERROR takes its default"
        );
        assert_eq!(loaded.config.max_processes.exiftool, 7);
        assert_eq!(loaded.config.rendering.len(), 1);
        assert_eq!(loaded.config.rendering[0].codec, "h264");
        assert_eq!(loaded.config.progress_bar_width, 33);
        assert!(loaded.problems.contains(&PROBLEM_CONFIG_ISSUES));
        assert!(loaded.problems.contains(&PROBLEM_UNKNOWN_CODEC));
    }

    #[test]
    fn unknown_codec_raises_only_unknown_codec_problem() {
        // The unknown codec is reported as PROBLEM_UNKNOWN_CODEC in place of the ERROR problem.
        let text = r#"{ "rendering": [ { "codec": "nope", "frame_rate": 30 } ] }"#;
        let loaded = config::load_text(text, &codecs::builtin(), YEAR).expect("parses");
        assert_eq!(loaded.problems, [PROBLEM_UNKNOWN_CODEC]);
        assert!(loaded.config.rendering.is_empty());
    }

    #[test]
    fn output_filename_strips_prefixes_repeatedly() {
        let table = codecs::builtin();
        let c = cfg(r#"{ "image_name_prefix": "LRT_", "image_sequence_dirname_prefix": "LRT_" }"#);
        let mq = config::RenderType {
            codec: "ProResMQ".into(),
            frame_rate: 30,
            resolution: Some(vec![1920, 1080]),
        };
        assert_eq!(
            c.output_filename(&table, &mq, "LRT_example"),
            "example_ProRes-422_Rec.709F_1080p_30_MQ.mov"
        );
        // Repeated prefixes are all stripped.
        assert_eq!(
            c.output_filename(&table, &mq, "LRT_LRT_x"),
            "x_ProRes-422_Rec.709F_1080p_30_MQ.mov"
        );
        let h264 = config::RenderType {
            codec: "h264".into(),
            frame_rate: 30,
            resolution: Some(vec![720]),
        };
        assert_eq!(
            c.output_filename(&table, &h264, "LRT_example"),
            "example_H264-420_Rec.709F_720w_30_HQ.mov"
        );
        let unknown = config::RenderType {
            codec: "nope".into(),
            frame_rate: 25,
            resolution: None,
        };
        assert_eq!(
            c.output_filename(&table, &unknown, "LRT_example"),
            "example_nope_OriRes_25.mov"
        );
    }

    #[test]
    fn resolution_label_forms() {
        let two = config::RenderType {
            codec: "h264".into(),
            frame_rate: 30,
            resolution: Some(vec![1920, 1080]),
        };
        let none = config::RenderType {
            codec: "h264".into(),
            frame_rate: 30,
            resolution: None,
        };
        assert_eq!(two.resolution_label(), "1080p");
        assert_eq!(none.resolution_label(), "OriRes");
    }

    // ---- Validation: one test per §6.1 rule ----

    #[test]
    fn validation_absolute_path_error() {
        assert_single_error(
            r#"{ "export_paths": ["relative/path"] }"#,
            "/export_paths/0",
        );
        assert_single_error(r#"{ "backup_paths": [""] }"#, "/backup_paths/0");
    }

    #[test]
    fn absolute_path_check_is_cross_platform() {
        for p in ["/a", "\\\\server\\share", "C:\\x", "D:/y"] {
            assert!(config::is_absolute_path(p), "{p} is absolute");
        }
        for p in ["relative", "C:x", ""] {
            assert!(!config::is_absolute_path(p), "{p} is not absolute");
        }
    }

    #[test]
    fn validation_bad_regex_error() {
        assert_single_error(
            r#"{ "ignore_sequence_regexes": ["(unclosed"] }"#,
            "/ignore_sequence_regexes/0",
        );
        assert_single_error(
            r#"{ "delete_sequence_after_render_regexes": ["ok", "[bad"] }"#,
            "/delete_sequence_after_render_regexes/1",
        );
    }

    #[test]
    fn validation_empty_codec_error() {
        assert_single_error(
            r#"{ "rendering": [ { "codec": "", "frame_rate": 30 } ] }"#,
            "/rendering/0/codec",
        );
    }

    #[test]
    fn validation_frame_rate_error() {
        assert_single_error(
            r#"{ "rendering": [ { "codec": "h264", "frame_rate": 0 } ] }"#,
            "/rendering/0/frame_rate",
        );
        assert_single_error(
            r#"{ "rendering": [ { "codec": "h264", "frame_rate": 241 } ] }"#,
            "/rendering/0/frame_rate",
        );
        // The bounds themselves are fine.
        assert!(
            validate_json(r#"{ "rendering": [ { "codec": "h264", "frame_rate": 1 } ] }"#)
                .iter()
                .all(|i| !i.is_error())
        );
        assert!(
            validate_json(r#"{ "rendering": [ { "codec": "h264", "frame_rate": 240 } ] }"#)
                .iter()
                .all(|i| !i.is_error())
        );
    }

    #[test]
    fn validation_resolution_too_many_entries_error() {
        assert_single_error(
            r#"{ "rendering": [ { "codec": "h264", "frame_rate": 30, "resolution": [1, 2, 3] } ] }"#,
            "/rendering/0/resolution",
        );
    }

    #[test]
    fn validation_resolution_zero_entry_error() {
        assert_single_error(
            r#"{ "rendering": [ { "codec": "h264", "frame_rate": 30, "resolution": [1920, 0] } ] }"#,
            "/rendering/0/resolution",
        );
    }

    #[test]
    fn validation_max_processes_ffmpeg_error() {
        assert_single_error(
            r#"{ "max_processes": { "ffmpeg": 0 } }"#,
            "/max_processes/ffmpeg",
        );
    }

    #[test]
    fn validation_max_processes_exiftool_error() {
        assert_single_error(
            r#"{ "max_processes": { "exiftool": 0 } }"#,
            "/max_processes/exiftool",
        );
    }

    #[test]
    fn validation_max_concurrent_renders_error() {
        assert_single_error(
            r#"{ "max_cuncurrent_renders": 0 }"#,
            "/max_cuncurrent_renders",
        );
    }

    #[test]
    fn validation_scan_interval_error() {
        assert_single_error(
            r#"{ "scan_interval_seconds": 0.49 }"#,
            "/scan_interval_seconds",
        );
        assert!(
            validate_json(r#"{ "scan_interval_seconds": 0.5 }"#)
                .iter()
                .all(|i| !i.is_error())
        );
    }

    #[test]
    fn validation_history_days_error() {
        assert_single_error(
            r#"{ "recent_render_history_days": 0 }"#,
            "/recent_render_history_days",
        );
    }

    #[test]
    fn validation_unknown_codec_error() {
        assert_single_error(
            r#"{ "rendering": [ { "codec": "nope", "frame_rate": 30 } ] }"#,
            "/rendering/0/codec",
        );
    }

    #[test]
    fn validation_duplicate_variant_error() {
        let json = r#"{ "rendering": [
            { "codec": "h264", "frame_rate": 30, "resolution": [720] },
            { "codec": "h264", "frame_rate": 30, "resolution": [720] }
        ] }"#;
        assert_single_error(json, "/rendering/1");
    }

    #[test]
    fn validation_missing_path_warning() {
        let dir = temp();
        let missing = dir.path().join("not-there");
        let json = json!({ "export_paths": [missing.display().to_string()] }).to_string();
        let issues = validate_json(&json);
        assert!(
            issues.iter().all(|i| !i.is_error()),
            "a missing path is a warning"
        );
        assert_warning(&issues, "/export_paths/0");
    }

    #[test]
    fn validation_year_expands_nothing_warning() {
        let dir = temp();
        let pattern = format!("{}/$YEAR", dir.path().display());
        let empty = json!({ "source_paths": [pattern] }).to_string();
        assert_warning(&validate_json(&empty), "/source_paths/0");
        // With a year directory present, the entry expands and there is no warning.
        fs::create_dir(dir.path().join("2025")).expect("make dir");
        let expanded = validate_json(&empty);
        assert!(
            !expanded.iter().any(|i| i.field == "/source_paths/0"),
            "{expanded:?}"
        );
    }

    #[test]
    fn validation_empty_export_warning() {
        let json =
            r#"{ "source_paths": ["/x"], "rendering": [ { "codec": "h264", "frame_rate": 30 } ] }"#;
        assert_warning(&validate_json(json), "/export_paths");
    }

    #[test]
    fn validation_empty_source_warning() {
        let json =
            r#"{ "export_paths": ["/x"], "rendering": [ { "codec": "h264", "frame_rate": 30 } ] }"#;
        assert_warning(&validate_json(json), "/source_paths");
    }

    #[test]
    fn validation_empty_rendering_warning() {
        let json = r#"{ "export_paths": ["/x"], "source_paths": ["/y"] }"#;
        assert_warning(&validate_json(json), "/rendering");
    }

    // ---- $YEAR expansion with a fixed clock and the 60 s cache ----

    #[test]
    fn year_expansion_starts_at_clock_year() {
        let root = temp();
        fs::create_dir(root.path().join("2025")).expect("make dir");
        fs::create_dir(root.path().join("2026")).expect("make dir");
        let pattern = format!("{}/$YEAR", root.path().display());
        let mut cache = ExpansionCache::new();
        let clock = ManualClock::new(2025);
        let expanded = paths::expand_years(std::slice::from_ref(&pattern), &clock, &mut cache);
        assert_eq!(expanded, [format!("{}/2025", root.path().display())]);
    }

    #[test]
    fn year_expansion_cache_holds_at_59_and_refreshes_at_61() {
        let root = temp();
        fs::create_dir(root.path().join("2025")).expect("make dir");
        let pattern = format!("{}/$YEAR", root.path().display());
        let list = std::slice::from_ref(&pattern);
        let clock = ManualClock::new(YEAR);
        let mut cache = ExpansionCache::new();

        let first = paths::expand_years(list, &clock, &mut cache);
        assert_eq!(first, [format!("{}/2025", root.path().display())]);

        // A new year directory appears. Within the cache window the old answer is kept.
        fs::create_dir(root.path().join("2024")).expect("make dir");
        clock.advance(Duration::from_secs(59));
        assert_eq!(
            paths::expand_years(list, &clock, &mut cache),
            first,
            "still cached at 59 s"
        );

        // At 61 s the expansion is recomputed, newest first.
        clock.advance(Duration::from_secs(2));
        assert_eq!(
            paths::expand_years(list, &clock, &mut cache),
            [
                format!("{}/2025", root.path().display()),
                format!("{}/2024", root.path().display())
            ],
            "refreshed at 61 s"
        );
        // The clock is a trait object too, as the store uses it.
        let as_dyn: &dyn Clock = &clock;
        assert_eq!(as_dyn.year(), YEAR);
    }

    // ---- The codec table: resolution, reload, in-use rule ----

    fn write_table(path: &Path, table: &VideoCodecs) {
        let mut text = serde_json::to_string_pretty(table).expect("serialize table");
        text.push('\n');
        fs::write(path, text).expect("write table");
    }

    #[test]
    fn codec_resolution_explicit_first() {
        let dir = temp();
        let config_path = dir.path().join("config.json");
        let sibling = dir.path().join("video_codecs.json");
        let explicit = dir.path().join("explicit.json");
        write_table(
            &sibling,
            &VideoCodecs::from([("Sibling".to_owned(), codec("a", "yuv420p"))]),
        );
        write_table(
            &explicit,
            &VideoCodecs::from([("Explicit".to_owned(), codec("b", "yuv420p"))]),
        );

        let store = CodecStore::open(Some(explicit.clone()), config_path);
        assert_eq!(store.source(), CodecSource::Explicit);
        assert_eq!(store.path(), Some(explicit.as_path()));
        assert!(store.codecs().contains_key("Explicit"));
        assert!(store.issues().is_empty());
    }

    #[test]
    fn codec_resolution_sibling_second() {
        let dir = temp();
        let config_path = dir.path().join("config.json");
        write_table(
            &dir.path().join("video_codecs.json"),
            &VideoCodecs::from([("Sibling".to_owned(), codec("a", "yuv420p"))]),
        );
        let store = CodecStore::open(None, config_path);
        assert_eq!(store.source(), CodecSource::Sibling);
        assert!(store.codecs().contains_key("Sibling"));
    }

    #[test]
    fn codec_resolution_builtin_last() {
        let dir = temp();
        let store = CodecStore::open(None, dir.path().join("config.json"));
        assert_eq!(store.source(), CodecSource::Builtin);
        assert_eq!(store.path(), None);
        assert_eq!(store.codecs(), &codecs::builtin());
    }

    #[test]
    fn codec_missing_explicit_falls_through_with_issue() {
        let dir = temp();
        write_table(
            &dir.path().join("video_codecs.json"),
            &VideoCodecs::from([("Sibling".to_owned(), codec("a", "yuv420p"))]),
        );
        let store = CodecStore::open(
            Some(dir.path().join("gone.json")),
            dir.path().join("config.json"),
        );
        assert_eq!(store.source(), CodecSource::Sibling);
        assert_eq!(store.issues().len(), 1);
    }

    #[test]
    fn codec_unparseable_falls_through_to_builtin() {
        let dir = temp();
        fs::write(dir.path().join("video_codecs.json"), "{ not json").expect("write");
        let store = CodecStore::open(None, dir.path().join("config.json"));
        assert_eq!(store.source(), CodecSource::Builtin);
        assert_eq!(store.codecs(), &codecs::builtin());
        assert_eq!(store.issues().len(), 1);
    }

    #[test]
    fn codec_reload_keeps_last_good() {
        let dir = temp();
        let sibling = dir.path().join("video_codecs.json");
        write_table(
            &sibling,
            &VideoCodecs::from([("Good".to_owned(), codec("a", "yuv420p"))]),
        );
        let mut store = CodecStore::open(None, dir.path().join("config.json"));
        assert!(store.codecs().contains_key("Good"));

        fs::write(&sibling, "{ broken").expect("write");
        touch_later(&sibling);
        assert!(store.reload_if_changed());
        assert!(
            store.codecs().contains_key("Good"),
            "the last good table is kept"
        );
        assert_eq!(store.issues().len(), 1);
    }

    #[test]
    fn codec_remove_refused_while_in_use() {
        let dir = temp();
        let config_path = dir.path().join("config.json");
        let mut store = CodecStore::open(None, config_path);
        let config = cfg(r#"{ "rendering": [ { "codec": "h264", "frame_rate": 30 } ] }"#);
        match store.remove("h264", &config) {
            Err(CodecError::InUse { codec, entries }) => {
                assert_eq!(codec, "h264");
                assert_eq!(entries, [0]);
            }
            other => panic!("expected InUse, got {other:?}"),
        }
        assert!(store.codecs().contains_key("h264"));
        assert!(
            !dir.path().join("video_codecs.json").exists(),
            "a refused remove writes nothing"
        );
    }

    #[test]
    fn codec_rename_refused_while_in_use() {
        let dir = temp();
        let mut store = CodecStore::open(None, dir.path().join("config.json"));
        let config = cfg(r#"{ "rendering": [ { "codec": "ProResMQ", "frame_rate": 30 } ] }"#);
        let refused = store.rename("ProResMQ", "Renamed", &config);
        assert!(
            matches!(refused, Err(CodecError::InUse { .. })),
            "got {refused:?}"
        );
        assert!(store.codecs().contains_key("ProResMQ"));
    }

    #[test]
    fn codec_remove_allowed_when_unused() {
        let dir = temp();
        let config_path = dir.path().join("config.json");
        let mut store = CodecStore::open(None, config_path);
        let config = cfg(r#"{ "rendering": [ { "codec": "ProResUHQ", "frame_rate": 30 } ] }"#);
        store
            .remove("h264", &config)
            .expect("an unused codec can be removed");
        assert!(!store.codecs().contains_key("h264"));
        // The table was built-in, so the write created video_codecs.json beside the config.
        let written: VideoCodecs = codecs::parse_codecs(
            &fs::read_to_string(dir.path().join("video_codecs.json")).expect("written"),
        )
        .expect("parses");
        assert!(!written.contains_key("h264"));
        assert!(written.contains_key("ProResMQ"));
    }

    #[test]
    fn codec_rename_allowed_when_unused() {
        let dir = temp();
        let mut store = CodecStore::open(None, dir.path().join("config.json"));
        let config = cfg(r#"{ "rendering": [ { "codec": "ProResUHQ", "frame_rate": 30 } ] }"#);
        store
            .rename("h264", "H264Renamed", &config)
            .expect("an unused codec can be renamed");
        assert!(store.codecs().contains_key("H264Renamed"));
        assert!(!store.codecs().contains_key("h264"));
        let text = fs::read_to_string(dir.path().join("video_codecs.json")).expect("written");
        assert!(text.contains("H264Renamed") && !text.contains("\"h264\""));
    }

    #[test]
    fn codec_rename_onto_existing_refused() {
        let dir = temp();
        let mut store = CodecStore::open(None, dir.path().join("config.json"));
        let refused = store.rename("h264", "ProResMQ", &cfg("{}"));
        assert!(
            matches!(refused, Err(CodecError::Exists(_))),
            "got {refused:?}"
        );
    }

    #[test]
    fn codec_set_writes_the_table_beside_config() {
        let dir = temp();
        let mut store = CodecStore::open(None, dir.path().join("config.json"));
        store
            .set("Extra", codec("libx265", "yuv420p"))
            .expect("sets");
        let reopened = CodecStore::open(None, dir.path().join("config.json"));
        assert_eq!(reopened.source(), CodecSource::Sibling);
        assert_eq!(
            reopened.codecs().get("Extra"),
            Some(&codec("libx265", "yuv420p"))
        );
        assert_eq!(reopened.codecs().get("h264"), codecs::builtin().get("h264"));
    }
}
