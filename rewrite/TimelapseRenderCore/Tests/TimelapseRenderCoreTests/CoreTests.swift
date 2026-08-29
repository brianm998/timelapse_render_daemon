import XCTest
@testable import TimelapseRenderCore

final class CoreTests: XCTestCase {

    func testConfigDecodesPerlFormat() throws {
        let json = """
        {
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
        }
        """
        let cfg = try JSONDecoder().decode(DaemonConfig.self, from: Data(json.utf8))
        XCTAssertEqual(cfg.exportPaths, ["/a", "/b"])
        XCTAssertEqual(cfg.maxConcurrentRenders, 2)
        XCTAssertEqual(cfg.maxProcesses.ffmpeg, 4)
        XCTAssertEqual(cfg.rendering.count, 2)
        XCTAssertEqual(cfg.rendering[0].height, 1080)
        XCTAssertNil(cfg.rendering[1].resolution)
        XCTAssertEqual(cfg.imageNamePrefix, "LRT_")
    }

    func testFFmpegProgressParsing() {
        let line = "frame= 1569 fps=0.9 q=-0.0 Lsize=  286475kB time=00:00:52.26 bitrate=44900.6kbits/s speed=0.0292x"
        let p = FFmpegRunner.parseProgress(line)
        XCTAssertEqual(p?.frame, 1569)
        XCTAssertEqual(p?.fps, 0.9)
    }

    func testFFmpegProgressParsingFixedWidthSpacing() {
        // Real ffmpeg 8.x format: values are right-aligned to a fixed width
        // so there can be multiple spaces between `=` and the digits.  The
        // previous parser collapsed only one space and left "frame=" with no
        // value — yielding nil and silently freezing the GUI's progress bar.
        let line = "frame=   30 fps=0.0 q=-0.0 Lsize=       6KiB time=00:00:06.00 bitrate=   8.1kbits/s speed= 183x elapsed=0:00:00.03"
        let p = FFmpegRunner.parseProgress(line)
        XCTAssertEqual(p?.frame, 30)
        XCTAssertEqual(p?.fps, 0.0)
    }

    func testFFmpegProgressParsingNonProgressLineReturnsNil() {
        XCTAssertNil(FFmpegRunner.parseProgress("Stream mapping:"))
        XCTAssertNil(FFmpegRunner.parseProgress("  Press [q] to stop"))
        XCTAssertNil(FFmpegRunner.parseProgress(""))
    }

    func testAspectRatio() {
        XCTAssertEqual(AspectRatio.string(width: 1920, height: 1080), "16/9")
        XCTAssertEqual(AspectRatio.string(width: 3840, height: 2160), "16/9")
        XCTAssertEqual(AspectRatio.string(width: 1080, height: 720), "3/2")
    }

    func testVariantKeyStable() {
        let r1 = RenderType(resolution: [3840, 2160], codec: "ProResMQ", frameRate: 30)
        let r2 = RenderType(codec: "ProResUHQ", frameRate: 30)
        XCTAssertEqual(r1.variantKey, "ProResMQ_3840x2160_30")
        XCTAssertEqual(r2.variantKey, "ProResUHQ_ori_30")
    }

    func testWidthOnlyRenderType() {
        let r = RenderType(resolution: [1080], codec: "ProResMQ", frameRate: 30)
        XCTAssertEqual(r.width, 1080)
        XCTAssertNil(r.height)
        XCTAssertEqual(r.variantKey, "ProResMQ_1080w_30")
    }

    func testDefaultCodecTableIncludesH264() {
        let codecs = ConfigLoader.defaultCodecs
        let h264 = codecs["h264"]
        XCTAssertNotNil(h264)
        XCTAssertEqual(h264?.ffmpeg.codec, "libx264")
        XCTAssertEqual(h264?.ffmpeg.profile, "high")
        XCTAssertEqual(h264?.ffmpeg.extraArgs, ["-crf", "18"])
        // ProRes keeps the -profile:v 1 the Perl daemon hardcoded.
        XCTAssertEqual(codecs["ProResUHQ"]?.ffmpeg.profile, "1")
        XCTAssertEqual(codecs["ProResMQ"]?.ffmpeg.profile, "1")
    }

    func testCodecTableDecodesProfileAndExtraArgs() throws {
        let json = """
        {
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
        }
        """
        let codecs = try JSONDecoder().decode(VideoCodecs.self, from: Data(json.utf8))
        XCTAssertEqual(codecs["h264"]?.ffmpeg.profile, "high")
        XCTAssertEqual(codecs["h264"]?.ffmpeg.extraArgs, ["-crf", "18"])
        // Entries without the new keys still decode (older video_codecs.json).
        XCTAssertNil(codecs["legacy"]?.ffmpeg.profile)
        XCTAssertNil(codecs["legacy"]?.ffmpeg.extraArgs)
    }

    private func makePlan(codec: VideoCodec) -> FFmpegRunner.Plan {
        FFmpegRunner.Plan(
            inputDir: "/seq",
            imageNamePrefix: "LRT_",
            imageExtension: ".tif",
            outputPath: "/out/video.mov",
            frameRate: 30,
            aspect: "16/9",
            width: 1280,
            height: 720,
            codec: codec,
            metadata: [:],
            totalFrames: 100
        )
    }

    func testFFmpegArgsUseCodecProfileAndExtraArgs() throws {
        let h264 = try XCTUnwrap(ConfigLoader.defaultCodecs["h264"])
        let args = makePlan(codec: h264).arguments
        let profileIdx = try XCTUnwrap(args.firstIndex(of: "-profile:v"))
        XCTAssertEqual(args[profileIdx + 1], "high")
        let crfIdx = try XCTUnwrap(args.firstIndex(of: "-crf"))
        XCTAssertEqual(args[crfIdx + 1], "18")
        XCTAssertEqual(args[try XCTUnwrap(args.firstIndex(of: "-c:v")) + 1], "libx264")
    }

    func testFFmpegArgsProResUnchanged() throws {
        let prores = try XCTUnwrap(ConfigLoader.defaultCodecs["ProResUHQ"])
        let args = makePlan(codec: prores).arguments
        let profileIdx = try XCTUnwrap(args.firstIndex(of: "-profile:v"))
        XCTAssertEqual(args[profileIdx + 1], "1")
        XCTAssertFalse(args.contains("-crf"))
    }

    func testFFmpegArgsOmitProfileWhenNil() {
        var codec = ConfigLoader.defaultCodecs["ProResUHQ"]!
        codec.ffmpeg.profile = nil
        let args = makePlan(codec: codec).arguments
        XCTAssertFalse(args.contains("-profile:v"))
    }

    func testDetectStructuralCorruptionMatchesExifToolError() {
        // Real exiftool output observed against the corrupt
        // 01_18_2026-a7riii-1-aurora-topaz-star-v-0_10_6_*_UHQ.mov.
        let real = "Error: Truncated mdat atom - /path/to/file.mov"
        XCTAssertEqual(XMPEmbedder.detectStructuralCorruption(in: real), "Truncated mdat atom")
    }

    func testDetectStructuralCorruptionIgnoresUnrelatedFailures() {
        // "Error writing output file" surfaces from disk-full / permission /
        // leftover-tmp issues — NOT a structural problem with the video.
        let stale = "Error: Temporary file already exists: /path/foo.mov_exiftool_tmp"
        XCTAssertNil(XMPEmbedder.detectStructuralCorruption(in: stale))
        let perms = "Error: Error writing output file - /path/foo.mov"
        XCTAssertNil(XMPEmbedder.detectStructuralCorruption(in: perms))
        let missing = "Error: File not found - /path/sidecar.json"
        XCTAssertNil(XMPEmbedder.detectStructuralCorruption(in: missing))
    }

    func testDetectStructuralCorruptionMatchesOtherAtomErrors() {
        XCTAssertNotNil(XMPEmbedder.detectStructuralCorruption(in: "Error: Bad atom size in moov"))
        XCTAssertNotNil(XMPEmbedder.detectStructuralCorruption(in: "Error: Could not parse moov atom"))
        XCTAssertNotNil(XMPEmbedder.detectStructuralCorruption(in: "Error: Invalid mdat header"))
    }

    func testYearExpansionPreservesNonPlaceholderPaths() {
        // Without $YEAR, paths are returned unchanged (no existence check).
        let input = ["/foo/bar", "/baz"]
        XCTAssertEqual(PathExpander.expandYears(input), input)
    }

    func testProcessRunnerReportsFastExit() async throws {
        // /usr/bin/true exits immediately with status 0.  This used to hang
        // forever because the termination handler was set after process.run().
        let runner = ProcessRunner(executable: "/usr/bin/true", arguments: [])
        let output = try await runner.run { _ in }
        XCTAssertEqual(output.exitCode, 0)
        XCTAssertTrue(output.terminatedNormally)
    }

    func testProcessRunnerReportsNonZeroExit() async throws {
        // /usr/bin/false exits immediately with status 1.
        let runner = ProcessRunner(executable: "/usr/bin/false", arguments: [])
        let output = try await runner.run { _ in }
        XCTAssertEqual(output.exitCode, 1)
    }

    // The suffix cases the old `imageSuffixAfterDigits` covered, now expressed
    // against `decompose`, which splits all three parts of the pattern.

    func testDecomposeHandlesPlainExtension() {
        let d = RenderFailureDiagnosis.decompose("LRT_00001.tiff")
        XCTAssertEqual(d?.prefix, "LRT_")
        XCTAssertEqual(d?.digits, "00001")
        XCTAssertEqual(d?.suffix, ".tiff")
    }

    func testDecomposeHandlesDecorationBeforeExtension() {
        // The case that broke renders once before: LRT_00001-severe-noise.tiff
        // needs a pattern of LRT_%05d-severe-noise.tiff, so the suffix is
        // "-severe-noise.tiff", not just ".tiff".
        let d = RenderFailureDiagnosis.decompose("LRT_00001-severe-noise.tiff")
        XCTAssertEqual(d?.prefix, "LRT_")
        XCTAssertEqual(d?.suffix, "-severe-noise.tiff")
    }

    func testDecomposeWithoutPrefix() {
        let d = RenderFailureDiagnosis.decompose("00042.tif")
        XCTAssertEqual(d?.prefix, "")
        XCTAssertEqual(d?.digits, "00042")
        XCTAssertEqual(d?.suffix, ".tif")
    }

    func testDecomposeFindsFirstDigitRunAnywhereInTheName() {
        let d = RenderFailureDiagnosis.decompose("image-001.tiff")
        XCTAssertEqual(d?.prefix, "image-")
        XCTAssertEqual(d?.digits, "001")
        XCTAssertEqual(d?.suffix, ".tiff")
    }

    func testDecomposeReturnsNilWhenThereIsNoFrameNumber() {
        XCTAssertNil(RenderFailureDiagnosis.decompose("weird-no-numbers.tiff"))
    }

    func testProcessRunnerStreamsLinesWhileChildIsAlive() async throws {
        // Verifies that lines from a long-running child are delivered as
        // they arrive, not all at once on termination — which is what was
        // hiding ffmpeg progress in the GUI (the reader had been collapsed
        // into the termination handler in a previous fix).
        let runner = ProcessRunner(
            executable: "/bin/sh",
            arguments: ["-c", "echo a; sleep 0.3; echo b; sleep 0.3; echo c"]
        )
        let firstLineTime = FirstLineTime()
        let start = Date()
        _ = try await runner.run { line in
            if line == "a" { firstLineTime.recordIfFirst() }
        }
        // Process runs for ~600ms.  If streaming works, line "a" arrives
        // very early (within ~50ms).  If lines are buffered until the
        // process terminates, "a" arrives at ~600ms.
        guard let firstAt = firstLineTime.value else {
            XCTFail("never received line 'a'")
            return
        }
        let firstLineElapsed = firstAt.timeIntervalSince(start)
        XCTAssertLessThan(
            firstLineElapsed, 0.250,
            "line 'a' arrived after \(firstLineElapsed)s — reader is not streaming"
        )
    }

    func testCompletedRenderResultRoundTripsThroughJSON() throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let r = CompletedRenderResult(
            timestamp: now,
            sequenceName: "seq-foo",
            exportPath: "/rp/tmp",
            outputFilename: "foo.mov",
            outputPath: "/rp/tmp/foo.mov",
            frameCount: 1234,
            fileSize: 56_789
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let data = try encoder.encode([r])
        let back = try decoder.decode([CompletedRenderResult].self, from: data)
        XCTAssertEqual(back.count, 1)
        XCTAssertEqual(back[0].sequenceName, "seq-foo")
        XCTAssertEqual(back[0].timestamp, now)
        XCTAssertEqual(back[0].fileSize, 56_789)
    }

    func testConfigDefaultsForRenderHistory() throws {
        // Old-format config (no history fields) should still decode and
        // pick up sensible defaults.
        let json = """
        { "rendering": [ { "codec": "ProResMQ", "frame_rate": 30 } ] }
        """
        let cfg = try JSONDecoder().decode(DaemonConfig.self, from: Data(json.utf8))
        XCTAssertEqual(cfg.recentRenderHistoryDays, 30)
        XCTAssertNil(cfg.renderHistoryPath)
    }

    func testOrphanScannerParsesFFmpegLine() {
        let line = "12345 /usr/local/bin/ffmpeg -nostdin -y -r 30 -i /rp/tmp/LRT_07_06_2024-foo/LRT_%05d-severe-noise.tiff -aspect 16/9 -c:v prores_ks /rp/tmp/foo_ProRes-422_2160p_30_MQ.mov"
        let orphan = OrphanScanner.parse(line: line)
        XCTAssertEqual(orphan?.pid, 12345)
        XCTAssertEqual(orphan?.inputPattern, "/rp/tmp/LRT_07_06_2024-foo/LRT_%05d-severe-noise.tiff")
        XCTAssertEqual(orphan?.outputPath, "/rp/tmp/foo_ProRes-422_2160p_30_MQ.mov")
        XCTAssertEqual(orphan?.exportPath, "/rp/tmp")
        XCTAssertEqual(orphan?.sequenceName, "LRT_07_06_2024-foo")
        XCTAssertEqual(orphan?.outputFilename, "foo_ProRes-422_2160p_30_MQ.mov")
    }

    func testOrphanScannerRejectsNonFFmpeg() {
        XCTAssertNil(OrphanScanner.parse(line: "100  /bin/sh -c something"))
        XCTAssertNil(OrphanScanner.parse(line: "100  /usr/local/bin/ffmpeg -i /tmp/single.mov out.mov"))
        XCTAssertNil(OrphanScanner.parse(line: ""))
    }

    func testProcessRunnerThrowsLaunchFailedForMissingBinary() async {
        let runner = ProcessRunner(
            executable: "/var/empty/definitely-not-a-real-binary",
            arguments: []
        )
        do {
            _ = try await runner.run { _ in }
            XCTFail("expected RunError.launchFailed")
        } catch ProcessRunner.RunError.launchFailed {
            // expected
        } catch {
            XCTFail("expected RunError.launchFailed, got \(error)")
        }
    }

    func testYearExpansionExpandsExistingDirectoriesOnly() throws {
        // Build a temp tree with three "year" dirs, then verify only those
        // come back from expansion of "<tmp>/$YEAR".
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent(
            "expand-\(UUID().uuidString)"
        )
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: root) }

        for year in [2020, 2023, 2025] {
            try fm.createDirectory(
                at: root.appendingPathComponent(String(year)),
                withIntermediateDirectories: true
            )
        }

        let pattern = "\(root.path)/$YEAR"
        // Pin "now" so the test isn't year-sensitive.
        let now = Calendar(identifier: .gregorian)
            .date(from: DateComponents(year: 2026, month: 6, day: 1))!
        let expanded = PathExpander.expandYears([pattern], now: now)

        XCTAssertEqual(expanded.count, 3)
        XCTAssertTrue(expanded.contains("\(root.path)/2025"))
        XCTAssertTrue(expanded.contains("\(root.path)/2023"))
        XCTAssertTrue(expanded.contains("\(root.path)/2020"))
        // Non-existent year should not be present.
        XCTAssertFalse(expanded.contains("\(root.path)/2024"))
        // Most recent first.
        XCTAssertEqual(expanded.first, "\(root.path)/2025")
    }

    // MARK: - Codec table resolution

    func testCodecsPathAlongsideConfig() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent(
            "codecs-\(UUID().uuidString)"
        )
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: root) }

        let configPath = root.appendingPathComponent("config.json").path
        try Data("{}".utf8).write(to: URL(fileURLWithPath: configPath))

        // No video_codecs.json next to the config → nil.
        XCTAssertNil(ConfigLoader.codecsPath(alongsideConfig: configPath))

        let codecsPath = root.appendingPathComponent("video_codecs.json").path
        try Data("{}".utf8).write(to: URL(fileURLWithPath: codecsPath))
        XCTAssertEqual(ConfigLoader.codecsPath(alongsideConfig: configPath), codecsPath)
    }

    func testSnapshotReportsUnknownCodecs() async throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent(
            "unknown-codec-\(UUID().uuidString)"
        )
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: root) }

        let json = """
        {
          "rendering": [
            { "codec": "ProResMQ", "frame_rate": 30 },
            { "codec": "h264", "frame_rate": 30 },
            { "resolution": [1920, 1080], "codec": "h264", "frame_rate": 24 }
          ]
        }
        """
        let configPath = root.appendingPathComponent("config.json").path
        try Data(json.utf8).write(to: URL(fileURLWithPath: configPath))

        let daemon = try RenderDaemon(configPath: configPath, logger: SilentLogger())
        let snapshot = await daemon.currentSnapshot()
        // "h264" isn't in the built-in codec table; ProResMQ is.  Deduped.
        XCTAssertEqual(snapshot.unknownCodecs, ["h264"])
    }

    // MARK: - XMP embed

    /// End-to-end round trip: fixture sidecar + fixture .mov → embed → read
    /// back the XMP-EXIF tags and confirm they match what the mapper picked.
    /// Skips unless `XMP_INTEGRATION_FIXTURES` points at a directory holding
    /// `sidecar.json` and `video.mov`.  Keeps the test out of the default
    /// suite because it requires a real .mov file and the `exiftool` binary.
    func testXMPEmbedRoundTripWithFixture() async throws {
        guard let fixturesPath = ProcessInfo.processInfo
            .environment["XMP_INTEGRATION_FIXTURES"] else {
            throw XCTSkip("XMP_INTEGRATION_FIXTURES not set")
        }
        let sidecarSrc = "\(fixturesPath)/sidecar.json"
        let videoSrc = "\(fixturesPath)/video.mov"
        guard FileManager.default.fileExists(atPath: sidecarSrc),
              FileManager.default.fileExists(atPath: videoSrc) else {
            throw XCTSkip("fixture files missing under \(fixturesPath)")
        }
        // Work on a copy so we don't modify the fixture.
        let work = FileManager.default.temporaryDirectory
            .appendingPathComponent("xmp-roundtrip-\(UUID().uuidString).mov")
        try FileManager.default.copyItem(atPath: videoSrc, toPath: work.path)
        defer { try? FileManager.default.removeItem(at: work) }

        try await XMPEmbedder(videoPath: work.path, sidecarPath: sidecarSrc).embed()

        // Read back every mapped field and assert it's non-empty.  We
        // verify the *presence* of each field rather than its exact
        // value — different fixtures will have different modes, and the
        // mapper's correctness is covered by the round trip itself: if
        // an arg was malformed exiftool would have rejected it and we'd
        // see a blank readback.
        let readback = try await readbackXMPTags(videoPath: work.path)
        let expected = [
            "XMP-tiff:Make", "XMP-tiff:Model", "XMP-aux:Lens",
            "XMP-exif:ISO", "XMP-exif:FNumber", "XMP-exif:ExposureTime",
            "XMP-exif:FocalLength", "XMP-exif:ExposureProgram",
            "XMP-exif:ExposureMode", "XMP-exif:WhiteBalance",
            "XMP-exif:DateTimeOriginal",
        ]
        for tag in expected {
            XCTAssertNotNil(readback[tag], "missing \(tag) after embed; readback=\(readback)")
            XCTAssertFalse((readback[tag] ?? "").isEmpty, "empty \(tag) after embed")
        }
    }

    private func readbackXMPTags(videoPath: String) async throws -> [String: String] {
        let runner = ProcessRunner(executable: "exiftool", arguments: [
            "-G1", "-s",
            "-XMP-tiff:Make", "-XMP-tiff:Model", "-XMP-aux:Lens",
            "-XMP-exif:ISO", "-XMP-exif:FNumber", "-XMP-exif:ExposureTime",
            "-XMP-exif:FocalLength", "-XMP-exif:ExposureProgram",
            "-XMP-exif:ExposureMode", "-XMP-exif:WhiteBalance",
            "-XMP-exif:DateTimeOriginal",
            videoPath,
        ])
        let collector = LineCollector()
        _ = try await runner.run { collector.append($0) }
        var out: [String: String] = [:]
        // Lines look like: "[XMP-tiff]      Make                            : SONY"
        let pattern = #/^\[(?<group>[^\]]+)\]\s+(?<tag>\S+(?:\s\S+)*?)\s*:\s*(?<value>.*)$/#
        for raw in collector.snapshot() {
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard let m = line.firstMatch(of: pattern) else { continue }
            let group = String(m.output.group)
            var tag = String(m.output.tag).replacingOccurrences(of: " ", with: "")
            // ExifTool prints "Camera Model Name" for the Model tag — map back.
            if tag == "CameraModelName" { tag = "Model" }
            out["\(group):\(tag)"] = String(m.output.value).trimmingCharacters(in: .whitespaces)
        }
        return out
    }

    // MARK: - Image sequence pattern detection

    func testDetectPatternFromNonConfiguredNaming() throws {
        // The real failure: LRTimelapse wrote image_0001.tiff into a dir named
        // LRT_<sequence>, so the configured "LRT_" prefix matches nothing.
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "image_", digits: 4, ext: ".tiff", count: 6)

        let p = try XCTUnwrap(ImageSequencePattern.detect(
            inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
        XCTAssertEqual(p.prefix, "image_")
        XCTAssertEqual(p.digits, 4)
        XCTAssertEqual(p.suffix, ".tiff")
        XCTAssertEqual(p.firstIndex, 1)
        XCTAssertEqual(p.frameCount, 6)
        XCTAssertEqual(p.exampleFilename, "image_0001.tiff")
        XCTAssertFalse(p.matchesConfiguredPrefix)
        XCTAssertFalse(p.requiresExplicitStartNumber)
        XCTAssertEqual(p.patternDescription, "image_%04d.tiff")
    }

    func testDetectPatternKeepsTheConfiguredPrefixWhenBothArePresent() throws {
        // A directory holding two differently-named sequences must keep
        // rendering the one the config asked for, even if it is the smaller.
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "LRT_", digits: 5, ext: ".tiff", count: 2)
        try makeFrames(in: dir, prefix: "image_", digits: 4, ext: ".tiff", count: 9)

        let p = try XCTUnwrap(ImageSequencePattern.detect(
            inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
        XCTAssertEqual(p.prefix, "LRT_")
        XCTAssertEqual(p.digits, 5)
        XCTAssertEqual(p.frameCount, 2)
        XCTAssertTrue(p.matchesConfiguredPrefix)
    }

    func testDetectPatternIgnoresStrayFilesAndPicksTheDominantGroup() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "image_", digits: 4, ext: ".tiff", count: 8)
        // Noise that must not win: a lone oddly-named frame, a non-image, and
        // a dotfile.
        try Data("x".utf8).write(to: dir.appendingPathComponent("preview_01.tiff"))
        try Data("x".utf8).write(to: dir.appendingPathComponent("notes.txt"))
        try Data("x".utf8).write(to: dir.appendingPathComponent(".DS_Store"))

        let p = try XCTUnwrap(ImageSequencePattern.detect(
            inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
        XCTAssertEqual(p.prefix, "image_")
        XCTAssertEqual(p.frameCount, 8)
    }

    func testDetectPatternKeepsDecoratedSuffixes() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "LRT_", digits: 5, ext: "-severe-noise.tiff", count: 3)

        let p = try XCTUnwrap(ImageSequencePattern.detect(
            inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
        XCTAssertEqual(p.patternDescription, "LRT_%05d-severe-noise.tiff")
    }

    func testDetectPatternFlagsSequencesStartingPastTheProbeRange() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "LRT_", digits: 5, ext: ".tiff", startingAt: 100, count: 3)

        let p = try XCTUnwrap(ImageSequencePattern.detect(
            inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
        XCTAssertEqual(p.firstIndex, 100)
        XCTAssertTrue(p.requiresExplicitStartNumber)
    }

    func testDetectPatternDoesNotFlagSequencesStartingAtZeroOrOne() throws {
        let fm = FileManager.default
        for start in [0, 1] {
            let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? fm.removeItem(at: dir) }
            try makeFrames(in: dir, prefix: "LRT_", digits: 5, ext: ".tiff",
                           startingAt: start, count: 3)
            let p = try XCTUnwrap(ImageSequencePattern.detect(
                inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
            ))
            XCTAssertEqual(p.firstIndex, start)
            XCTAssertFalse(p.requiresExplicitStartNumber, "start=\(start)")
        }
    }

    func testDetectPatternReturnsNilWhenNothingIsNumbered() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("detect-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try Data("x".utf8).write(to: dir.appendingPathComponent("just-a-name.tiff"))

        XCTAssertNil(ImageSequencePattern.detect(
            inDirectory: dir.path, imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
    }

    func testDetectPatternReturnsNilForAnUnreadableDirectory() {
        XCTAssertNil(ImageSequencePattern.detect(
            inDirectory: "/definitely/not/here-\(UUID().uuidString)",
            imageExtensions: ["tif", "tiff"], preferredPrefix: "LRT_"
        ))
    }

    // MARK: - Plans built from a detected pattern

    func testPlanUsesTheDetectedDigitWidthAndPrefix() throws {
        let plan = try makePlan(
            inputDir: "/seq", prefix: "image_", ext: ".tiff", digits: 4
        )
        XCTAssertEqual(plan.inputPatternDescription, "image_%04d.tiff")
        let args = plan.arguments
        let i = try XCTUnwrap(args.firstIndex(of: "-i"))
        XCTAssertEqual(args[i + 1], "/seq/image_%04d.tiff")
    }

    func testPlanOmitsStartNumberForOrdinarySequences() throws {
        let plan = try makePlan(inputDir: "/seq")
        XCTAssertFalse(plan.arguments.contains("-start_number"))
    }

    /// `-start_number` is an input option, so it has to land before `-i` or
    /// ffmpeg applies it to the output.
    func testPlanPassesStartNumberBeforeTheInputArgument() throws {
        let plan = try makePlan(inputDir: "/seq", startNumber: 100)
        let args = plan.arguments
        let start = try XCTUnwrap(args.firstIndex(of: "-start_number"))
        let input = try XCTUnwrap(args.firstIndex(of: "-i"))
        XCTAssertEqual(args[start + 1], "100")
        XCTAssertLessThan(start, input)
    }

    // MARK: - Render failure diagnosis

    /// Builds a plan pointing at `dir`, with the codec table's ProRes entry.
    private func makePlan(
        inputDir: String,
        prefix: String = "LRT_",
        ext: String = ".tiff",
        digits: Int = 5,
        startNumber: Int? = nil,
        outputPath: String = "/tmp/out.mov"
    ) throws -> FFmpegRunner.Plan {
        let codec = try XCTUnwrap(ConfigLoader.defaultCodecs.values.first)
        return FFmpegRunner.Plan(
            inputDir: inputDir,
            imageNamePrefix: prefix,
            imageExtension: ext,
            frameNumberDigits: digits,
            startNumber: startNumber,
            outputPath: outputPath,
            frameRate: 30,
            aspect: "16/9",
            width: nil,
            height: nil,
            codec: codec,
            metadata: [:],
            totalFrames: 10
        )
    }

    /// Creates `count` frames named `<prefix><zero-padded index><ext>`.
    @discardableResult
    private func makeFrames(
        in dir: URL,
        prefix: String,
        digits: Int,
        ext: String,
        startingAt first: Int = 1,
        count: Int = 3
    ) throws -> [String] {
        var names: [String] = []
        for i in first..<(first + count) {
            let name = prefix + String(format: "%0\(digits)d", i) + ext
            try Data("x".utf8).write(to: dir.appendingPathComponent(name))
            names.append(name)
        }
        return names
    }

    func testSignificantLinesStripsFFmpegBanner() {
        let raw = [
            "ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers",
            "  built with Apple clang version 17.0.0 (clang-1700.4.4.1)",
            "  configuration: --prefix=/usr/local/Cellar/ffmpeg/8.1 --enable-shared",
            "  libavutil      60. 26.100 / 60. 26.100",
            "  libavcodec     62. 28.100 / 62. 28.100",
            "frame=   30 fps=0.0 q=-0.0 Lsize=       6KiB time=00:00:06.00",
            "Error opening input file /rp/tmp/x/LRT_%05d.tiff.",
        ]
        let kept = RenderFailureDiagnosis.significantLines(raw)
        XCTAssertEqual(kept, ["Error opening input file /rp/tmp/x/LRT_%05d.tiff."])
    }

    func testDecomposeImageFilename() {
        let a = RenderFailureDiagnosis.decompose("image_0001.tiff")
        XCTAssertEqual(a?.prefix, "image_")
        XCTAssertEqual(a?.digits, "0001")
        XCTAssertEqual(a?.suffix, ".tiff")

        let b = RenderFailureDiagnosis.decompose("LRT_00001-severe-noise.tiff")
        XCTAssertEqual(b?.prefix, "LRT_")
        XCTAssertEqual(b?.digits, "00001")
        XCTAssertEqual(b?.suffix, "-severe-noise.tiff")

        XCTAssertNil(RenderFailureDiagnosis.decompose("no-digits-here.tiff"))
    }

    /// The real-world failure: LRTimelapse wrote `image_0001.tiff` into a dir
    /// named `LRT_<sequence>`, so the daemon's `LRT_%05d.tiff` pattern matched
    /// nothing — wrong prefix AND wrong digit width.
    func testAnalyzeInputSequenceReportsPrefixAndDigitWidthMismatch() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("frames-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "image_", digits: 4, ext: ".tiff")

        let report = RenderFailureDiagnosis.analyzeInputSequence(
            inputDir: dir.path,
            expectedPrefix: "LRT_",
            expectedDigitWidth: 5,
            expectedSuffix: ".tiff",
            imageExtensions: ["tif", "tiff"]
        )
        XCTAssertEqual(report.imageFileCount, 3)
        XCTAssertEqual(report.exampleFilename, "image_0001.tiff")
        XCTAssertTrue(report.problems.contains(.prefixMismatch(expected: "LRT_", found: "image_")))
        XCTAssertTrue(report.problems.contains(.digitWidthMismatch(expected: 5, found: 4)))
        XCTAssertFalse(report.problems.contains(.suffixMismatch(expected: ".tiff", found: ".tiff")))
    }

    func testAnalyzeInputSequenceReportsStartIndexBeyondProbeRange() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("frames-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        // Correctly named, but the sequence starts at 100 — past the handful
        // of indices ffmpeg's image2 demuxer probes.
        try makeFrames(in: dir, prefix: "LRT_", digits: 5, ext: ".tiff", startingAt: 100)

        let report = RenderFailureDiagnosis.analyzeInputSequence(
            inputDir: dir.path,
            expectedPrefix: "LRT_",
            expectedDigitWidth: 5,
            expectedSuffix: ".tiff",
            imageExtensions: ["tif", "tiff"]
        )
        XCTAssertEqual(report.problems, [.firstIndexOutOfProbeRange(firstIndex: 100)])
    }

    func testAnalyzeInputSequenceReportsMissingDirectory() {
        let report = RenderFailureDiagnosis.analyzeInputSequence(
            inputDir: "/definitely/not/here-\(UUID().uuidString)",
            expectedPrefix: "LRT_",
            expectedDigitWidth: 5,
            expectedSuffix: ".tiff",
            imageExtensions: ["tif", "tiff"]
        )
        XCTAssertEqual(report.problems, [.directoryMissing])
    }

    func testAnalyzeInputSequenceReportsNoImageFiles() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("frames-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try Data("x".utf8).write(to: dir.appendingPathComponent("notes.txt"))

        let report = RenderFailureDiagnosis.analyzeInputSequence(
            inputDir: dir.path,
            expectedPrefix: "LRT_",
            expectedDigitWidth: 5,
            expectedSuffix: ".tiff",
            imageExtensions: ["tif", "tiff"]
        )
        XCTAssertEqual(report.problems, [.noImageFiles(sample: ["notes.txt"])])
    }

    func testAnalyzeInputSequenceReportsUnnumberedFrames() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("frames-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try Data("x".utf8).write(to: dir.appendingPathComponent("panorama.tiff"))

        let report = RenderFailureDiagnosis.analyzeInputSequence(
            inputDir: dir.path,
            expectedPrefix: "LRT_",
            expectedDigitWidth: 5,
            expectedSuffix: ".tiff",
            imageExtensions: ["tif", "tiff"]
        )
        XCTAssertEqual(report.problems, [.framesNotNumbered(sample: ["panorama.tiff"])])
        let text = RenderFailureDiagnosis.describe(report, inputDir: dir.path, pattern: "LRT_%05d.tiff")
        XCTAssertTrue(text.contains("no frame number"), text)
    }

    /// End to end, using ffmpeg 8.1's actual output for this failure.
    func testDiagnoseExplainsWhyNoFramesMatched() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("frames-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try makeFrames(in: dir, prefix: "image_", digits: 4, ext: ".tiff")

        let plan = try makePlan(inputDir: dir.path)
        let output = [
            "ffmpeg version 8.1 Copyright (c) 2000-2026 the FFmpeg developers",
            "  libavcodec     62. 28.100 / 62. 28.100",
            "[in#0 @ 0x7fba07706480] Could find no file or sequence with path "
                + "'\(dir.path)/LRT_%05d.tiff' and index in the range 0-4",
            "[in#0 @ 0x7fba07706480] Error opening input: No such file or directory",
            "Error opening input files: No such file or directory",
        ]
        let d = RenderFailureDiagnosis.diagnose(
            plan: plan,
            exitCode: 254,
            terminatedNormally: true,
            outputLines: output,
            imageExtensions: ["tif", "tiff"],
            outputFileExists: false
        )
        XCTAssertEqual(d.kind, .inputFramesNotFound)
        XCTAssertTrue(d.summary.contains("LRT_%05d.tiff"), d.summary)
        let detail = try XCTUnwrap(d.detail)
        XCTAssertTrue(detail.contains("image_0001.tiff"), detail)
        XCTAssertTrue(detail.contains("FILENAME PREFIX"), detail)
        XCTAssertTrue(detail.contains("FRAME NUMBER WIDTH"), detail)
    }

    func testDiagnoseReportsMissingInputDirectory() throws {
        let missing = "/definitely/not/here-\(UUID().uuidString)"
        let plan = try makePlan(inputDir: missing)
        let d = RenderFailureDiagnosis.diagnose(
            plan: plan,
            exitCode: 254,
            terminatedNormally: true,
            outputLines: ["Error opening input file \(missing)/LRT_%05d.tiff.",
                          "Error opening input files: No such file or directory"],
            imageExtensions: ["tif", "tiff"],
            outputFileExists: false
        )
        XCTAssertEqual(d.kind, .inputFramesNotFound)
        XCTAssertTrue(d.summary.contains("input directory is gone"), d.summary)
    }

    func testDiagnoseRecognisesDiskFull() throws {
        let plan = try makePlan(inputDir: "/tmp")
        let d = RenderFailureDiagnosis.diagnose(
            plan: plan,
            exitCode: 1,
            terminatedNormally: true,
            outputLines: ["av_interleaved_write_frame(): No space left on device"],
            imageExtensions: ["tif", "tiff"],
            outputFileExists: false
        )
        XCTAssertEqual(d.kind, .diskFull)
    }

    func testDiagnoseRecognisesUnknownEncoder() throws {
        let plan = try makePlan(inputDir: "/tmp")
        let d = RenderFailureDiagnosis.diagnose(
            plan: plan,
            exitCode: 1,
            terminatedNormally: true,
            outputLines: ["Unknown encoder 'prores_ks_typo'"],
            imageExtensions: ["tif", "tiff"],
            outputFileExists: false
        )
        XCTAssertEqual(d.kind, .ffmpegRejectedArguments)
        XCTAssertTrue(d.summary.contains("prores_ks_typo"), d.summary)
    }

    func testDiagnoseRecognisesKilledProcess() throws {
        let plan = try makePlan(inputDir: "/tmp")
        let d = RenderFailureDiagnosis.diagnose(
            plan: plan,
            exitCode: 9,
            terminatedNormally: false,
            outputLines: [],
            imageExtensions: ["tif", "tiff"],
            outputFileExists: false
        )
        XCTAssertEqual(d.kind, .terminatedBySignal)
    }

    func testDiagnoseRecognisesSuccessWithNoOutputFile() throws {
        let plan = try makePlan(inputDir: "/tmp")
        let d = RenderFailureDiagnosis.diagnose(
            plan: plan,
            exitCode: 0,
            terminatedNormally: true,
            outputLines: [],
            imageExtensions: ["tif", "tiff"],
            outputFileExists: false
        )
        XCTAssertEqual(d.kind, .producedNoOutput)
    }

    /// The pattern quoted in a failure report has to be the one ffmpeg was
    /// actually given, or the report sends people looking for the wrong thing.
    func testInputPatternMatchesTheArgumentPassedToFFmpeg() throws {
        let plan = try makePlan(inputDir: "/seq", prefix: "LRT_", ext: ".tiff")
        let args = plan.arguments
        let i = try XCTUnwrap(args.firstIndex(of: "-i"))
        XCTAssertEqual(args[i + 1], plan.inputPattern)
        XCTAssertEqual(plan.inputPattern, "/seq/LRT_%05d.tiff")
        XCTAssertEqual(plan.inputPatternDescription, "LRT_%05d.tiff")
    }

    func testRenderFailureMergingCountsOccurrences() {
        func make(_ summary: String) -> RenderFailure {
            RenderFailure(
                sequenceName: "seq", exportPath: "/e", outputFilename: "o.mov",
                outputPath: "/e/o.mov", variantKey: "v",
                kind: .inputFramesNotFound, summary: summary
            )
        }
        let first = make("first")
        let merged = first.merging(make("second"))
        XCTAssertEqual(merged.occurrences, 2)
        XCTAssertEqual(merged.summary, "second")
        XCTAssertTrue(merged.shortMessage.contains("x2"), merged.shortMessage)
    }

    func testRenderFailureConsoleReportIncludesTheEssentials() {
        let f = RenderFailure(
            sequenceName: "seq", exportPath: "/e", outputFilename: "o.mov",
            outputPath: "/e/o.mov", variantKey: "ProRes_30",
            kind: .inputFramesNotFound,
            summary: "no frames matched LRT_%05d.tiff",
            detail: "line one\nline two",
            exitCode: 254,
            commandLine: "ffmpeg -i /seq/LRT_%05d.tiff /e/o.mov",
            toolOutput: ["Error opening input files: No such file or directory"]
        )
        let report = f.consoleReport
        XCTAssertTrue(report.contains("RENDER FAILED"), report)
        XCTAssertTrue(report.contains("/e/o.mov"), report)
        XCTAssertTrue(report.contains("ffmpeg exit code: 254"), report)
        XCTAssertTrue(report.contains("no frames matched"), report)
        XCTAssertTrue(report.contains("line two"), report)
        XCTAssertTrue(report.contains("Error opening input files"), report)
        XCTAssertTrue(report.contains("ffmpeg -i /seq/LRT_%05d.tiff"), report)
    }
}

private final class LineCollector: @unchecked Sendable {
    private let lock = NSLock()
    private var lines: [String] = []
    func append(_ line: String) { lock.lock(); lines.append(line); lock.unlock() }
    func snapshot() -> [String] { lock.lock(); defer { lock.unlock() }; return lines }
}

private final class FirstLineTime: @unchecked Sendable {
    private let lock = NSLock()
    private(set) var value: Date?
    func recordIfFirst() {
        lock.lock(); defer { lock.unlock() }
        if value == nil { value = Date() }
    }

}
