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

    func testImageSuffixHandlesPlainExtension() {
        let s = RenderDaemon.imageSuffixAfterDigits(
            in: "LRT_00001.tiff", fallbackExtension: ".tif"
        )
        XCTAssertEqual(s, ".tiff")
    }

    func testImageSuffixHandlesDecorationBeforeExtension() {
        // The case that broke renders: LRT_00001-severe-noise.tiff
        // ffmpeg's pattern needs to be LRT_%05d-severe-noise.tiff so the
        // suffix is "-severe-noise.tiff", not just ".tiff".
        let s = RenderDaemon.imageSuffixAfterDigits(
            in: "LRT_00001-severe-noise.tiff", fallbackExtension: ".tif"
        )
        XCTAssertEqual(s, "-severe-noise.tiff")
    }

    func testImageSuffixWithoutPrefix() {
        let s = RenderDaemon.imageSuffixAfterDigits(
            in: "00042.tif", fallbackExtension: ".tif"
        )
        XCTAssertEqual(s, ".tif")
    }

    func testImageSuffixFindsFirstDigitRunEvenWithoutPrefix() {
        // If files are named like "image-001.tiff" (no LRT_-style prefix
        // configured, prefix empty), the Perl regex \d+(.*)$ correctly
        // captures ".tiff" — finding the digits anywhere in the name.
        let s = RenderDaemon.imageSuffixAfterDigits(
            in: "image-001.tiff", fallbackExtension: ".tif"
        )
        XCTAssertEqual(s, ".tiff")
    }

    func testImageSuffixFallbackWhenNoDigits() {
        let s = RenderDaemon.imageSuffixAfterDigits(
            in: "weird-no-numbers.tiff", fallbackExtension: ".tif"
        )
        XCTAssertEqual(s, ".tif")
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
