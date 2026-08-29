import Foundation

/// Turns an ffmpeg failure into an explanation a human can act on.
///
/// ffmpeg's own output is mostly banner noise — version, build flags and a
/// dozen library versions — with the actual cause buried at the end.  These
/// helpers strip the noise, recognise the failure signatures we keep hitting,
/// and, for the most common one (the image2 demuxer matching no frames), go
/// back to the filesystem to say exactly how the files on disk differ from the
/// pattern the daemon asked for.
public enum RenderFailureDiagnosis {

    /// The number of leading indices ffmpeg's image2 demuxer probes when
    /// looking for the first frame of a sequence.  If the sequence starts
    /// above this, ffmpeg gives up without ever reading a file.
    public static let imageDemuxerProbeRange = 5

    // MARK: - Output filtering

    /// Drops ffmpeg's startup banner, library-version block and progress
    /// spam, leaving the lines that actually say something.
    public static func significantLines(_ lines: [String]) -> [String] {
        lines.filter { raw in
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty { return false }
            if line.hasPrefix("ffmpeg version ") { return false }
            if line.hasPrefix("built with ") { return false }
            if line.hasPrefix("configuration:") { return false }
            if line.hasPrefix("Press [q]") { return false }
            // "libavcodec     62. 28.100 / 62. 28.100"
            if line.firstMatch(of: #/^lib[a-z]+\s+\d/#) != nil { return false }
            // Progress lines — how far it got is already tracked separately.
            if FFmpegRunner.parseProgress(line) != nil { return false }
            return true
        }
    }

    // MARK: - Filename decomposition

    /// Splits an image filename into the three parts ffmpeg's `prefix%0Nd
    /// suffix` pattern is built from.  `image_0001.tiff` decomposes to
    /// `("image_", "0001", ".tiff")`; `LRT_00001-severe-noise.tiff` to
    /// `("LRT_", "00001", "-severe-noise.tiff")`.  Splits on the first run of
    /// digits, the same rule the Perl daemon used for the suffix.
    ///
    /// Hand-rolled rather than a regex because `ImageSequencePattern.detect`
    /// calls this once per file, and these sequence directories run to
    /// thousands of frames.
    public static func decompose(
        _ filename: String
    ) -> (prefix: String, digits: String, suffix: String)? {
        let chars = Array(filename)
        guard let start = chars.firstIndex(where: { $0.isASCII && $0.isNumber }) else {
            return nil
        }
        var end = start
        while end < chars.count, chars[end].isASCII, chars[end].isNumber { end += 1 }
        return (String(chars[..<start]), String(chars[start..<end]), String(chars[end...]))
    }

    // MARK: - Input sequence analysis

    public struct InputSequenceReport: Sendable, Equatable {
        public enum Problem: Sendable, Equatable {
            case directoryMissing
            case directoryUnreadable
            case directoryEmpty
            case noImageFiles(sample: [String])
            case framesNotNumbered(sample: [String])
            case prefixMismatch(expected: String, found: String)
            case digitWidthMismatch(expected: Int, found: Int)
            case suffixMismatch(expected: String, found: String)
            case firstIndexOutOfProbeRange(firstIndex: Int)
            case unexplained
        }

        public var problems: [Problem]
        public var exampleFilename: String?
        public var imageFileCount: Int
    }

    /// Compares what ffmpeg was told to look for against what is actually in
    /// the input directory.
    public static func analyzeInputSequence(
        inputDir: String,
        expectedPrefix: String,
        expectedDigitWidth: Int,
        expectedSuffix: String,
        imageExtensions: [String],
        fileManager: FileManager = .default
    ) -> InputSequenceReport {
        var isDir: ObjCBool = false
        guard fileManager.fileExists(atPath: inputDir, isDirectory: &isDir), isDir.boolValue else {
            return InputSequenceReport(
                problems: [.directoryMissing], exampleFilename: nil, imageFileCount: 0
            )
        }
        guard let entries = try? fileManager.contentsOfDirectory(atPath: inputDir) else {
            return InputSequenceReport(
                problems: [.directoryUnreadable], exampleFilename: nil, imageFileCount: 0
            )
        }
        let visible = entries.filter { !$0.hasPrefix(".") }.sorted()
        if visible.isEmpty {
            return InputSequenceReport(
                problems: [.directoryEmpty], exampleFilename: nil, imageFileCount: 0
            )
        }

        let wanted = Set(imageExtensions.map { $0.lowercased() })
        let images = visible.filter { name in
            guard let dot = name.lastIndex(of: ".") else { return false }
            let ext = String(name[name.index(after: dot)...]).lowercased()
            return wanted.isEmpty || wanted.contains(ext)
        }
        if images.isEmpty {
            return InputSequenceReport(
                problems: [.noImageFiles(sample: Array(visible.prefix(5)))],
                exampleFilename: nil,
                imageFileCount: 0
            )
        }

        // Ask the same detector the render queue uses, so a report can never
        // describe the directory differently from the pattern that was built
        // from it.
        guard let actual = ImageSequencePattern.detect(
            inDirectory: inputDir,
            imageExtensions: imageExtensions,
            preferredPrefix: expectedPrefix,
            fileManager: fileManager
        ) else {
            return InputSequenceReport(
                problems: [.framesNotNumbered(sample: Array(images.prefix(5)))],
                exampleFilename: images.first,
                imageFileCount: images.count
            )
        }

        var problems: [InputSequenceReport.Problem] = []
        if actual.prefix != expectedPrefix {
            problems.append(.prefixMismatch(expected: expectedPrefix, found: actual.prefix))
        }
        if actual.digits != expectedDigitWidth {
            problems.append(.digitWidthMismatch(expected: expectedDigitWidth, found: actual.digits))
        }
        if actual.suffix.caseInsensitiveCompare(expectedSuffix) != .orderedSame {
            problems.append(.suffixMismatch(expected: expectedSuffix, found: actual.suffix))
        }

        if problems.isEmpty {
            // Naming is right, so the only remaining explanation is that the
            // sequence starts past the handful of indices ffmpeg probes.
            if actual.requiresExplicitStartNumber {
                problems.append(.firstIndexOutOfProbeRange(firstIndex: actual.firstIndex))
            } else {
                problems.append(.unexplained)
            }
        }

        return InputSequenceReport(
            problems: problems,
            exampleFilename: actual.exampleFilename,
            imageFileCount: images.count
        )
    }

    /// Renders an `InputSequenceReport` as the multi-line `detail` text of a
    /// `RenderFailure`.
    public static func describe(
        _ report: InputSequenceReport,
        inputDir: String,
        pattern: String
    ) -> String {
        var lines: [String] = []
        lines.append("ffmpeg was told to read \(pattern)")
        lines.append("in \(inputDir)")

        if let example = report.exampleFilename {
            lines.append("The directory holds \(report.imageFileCount) image file(s); "
                       + "the first is \(example)")
        }

        for problem in report.problems {
            switch problem {
            case .directoryMissing:
                lines.append("- that directory does not exist (deleted, renamed, or on an "
                           + "unmounted volume?)")
            case .directoryUnreadable:
                lines.append("- that directory exists but could not be listed — check "
                           + "permissions and that the volume is still mounted")
            case .directoryEmpty:
                lines.append("- that directory is empty")
            case .noImageFiles(let sample):
                lines.append("- it holds no files with a configured image extension"
                           + (sample.isEmpty ? "" : "; it does hold: \(sample.joined(separator: ", "))"))
            case .framesNotNumbered(let sample):
                lines.append("- its image files carry no frame number, so there is no sequence to "
                           + "render: \(sample.joined(separator: ", "))")
            case .prefixMismatch(let expected, let found):
                lines.append("- FILENAME PREFIX: files are named \"\(found)…\" but this render was "
                           + "queued for \"\(expected)…\". The daemon detects the prefix from the "
                           + "directory when it queues a render, so the contents changed in "
                           + "between, or the directory holds more than one sequence")
            case .digitWidthMismatch(let expected, let found):
                lines.append("- FRAME NUMBER WIDTH: files use \(found)-digit frame numbers but this "
                           + "render was queued for \(expected) (%0\(expected)d). ffmpeg needs an "
                           + "exact width, and the directory appears to hold mixed numbering")
            case .suffixMismatch(let expected, let found):
                lines.append("- SUFFIX: files end in \"\(found)\" but the pattern expects "
                           + "\"\(expected)\"")
            case .firstIndexOutOfProbeRange(let first):
                lines.append("- FIRST FRAME NUMBER: the sequence starts at \(first), but ffmpeg's "
                           + "image2 demuxer only probes indices 0-\(imageDemuxerProbeRange - 1) "
                           + "when looking for the first frame, and no -start_number was passed")
            case .unexplained:
                lines.append("- the daemon could not work out how the files on disk differ from "
                           + "the pattern; run the command below by hand to see ffmpeg's own view")
            }
        }
        return lines.joined(separator: "\n")
    }

    // MARK: - Top-level diagnosis

    public struct Diagnosis: Sendable, Equatable {
        public let kind: RenderFailure.Kind
        public let summary: String
        public let detail: String?
    }

    /// Works out why an ffmpeg run failed, consulting the filesystem when the
    /// output points at an input-pattern problem.
    public static func diagnose(
        plan: FFmpegRunner.Plan,
        exitCode: Int32?,
        terminatedNormally: Bool,
        outputLines: [String],
        imageExtensions: [String],
        outputFileExists: Bool,
        fileManager: FileManager = .default
    ) -> Diagnosis {
        let interesting = significantLines(outputLines)
        let haystack = interesting.joined(separator: "\n").lowercased()

        func firstLine(containing needle: String) -> String? {
            interesting.first { $0.lowercased().contains(needle) }
        }

        // 1. The image2 demuxer found nothing.  By far the most common
        //    failure, and the one that used to disappear without a trace.
        if haystack.contains("could find no file or sequence with path")
            || (haystack.contains("error opening input file") && haystack.contains("no such file or directory"))
        {
            let report = analyzeInputSequence(
                inputDir: plan.inputDir,
                expectedPrefix: plan.imageNamePrefix,
                expectedDigitWidth: plan.frameNumberDigits,
                expectedSuffix: plan.imageExtension,
                imageExtensions: imageExtensions,
                fileManager: fileManager
            )
            let summary: String
            switch report.problems.first {
            case .directoryMissing:
                summary = "the input directory is gone: \(plan.inputDir)"
            case .directoryUnreadable:
                summary = "the input directory could not be read: \(plan.inputDir)"
            case .directoryEmpty, .noImageFiles, .framesNotNumbered:
                summary = "there are no images to render in \(plan.inputDir)"
            default:
                summary = "no frames matched \(plan.inputPatternDescription) — "
                        + "the files on disk are named differently"
            }
            let kind: RenderFailure.Kind = {
                switch report.problems.first {
                case .directoryUnreadable: return .inputUnreadable
                default: return .inputFramesNotFound
                }
            }()
            return Diagnosis(
                kind: kind,
                summary: summary,
                detail: describe(report, inputDir: plan.inputDir, pattern: plan.inputPatternDescription)
            )
        }

        // 2. Everything else, most specific signature first.
        if haystack.contains("no space left on device") {
            return Diagnosis(
                kind: .diskFull,
                summary: "the volume holding \(plan.outputPath) is full",
                detail: "Free space on that volume and the variant will be re-queued on the next scan."
            )
        }
        if haystack.contains("permission denied") {
            let mentionsOutput = interesting.contains {
                $0.lowercased().contains("permission denied") && $0.contains(plan.outputPath)
            }
            return Diagnosis(
                kind: mentionsOutput ? .outputNotWritable : .inputUnreadable,
                summary: mentionsOutput
                    ? "cannot write \(plan.outputPath) — permission denied"
                    : "permission denied reading the source frames",
                detail: firstLine(containing: "permission denied")
            )
        }
        if haystack.contains("unknown encoder")
            || haystack.contains("unrecognized option")
            || haystack.contains("error splitting the argument list")
            || haystack.contains("option not found")
            || haystack.contains("invalid argument")
        {
            let line = firstLine(containing: "unknown encoder")
                ?? firstLine(containing: "unrecognized option")
                ?? firstLine(containing: "option not found")
                ?? firstLine(containing: "invalid argument")
                ?? firstLine(containing: "error splitting the argument list")
            return Diagnosis(
                kind: .ffmpegRejectedArguments,
                summary: line ?? "ffmpeg rejected the command line",
                detail: "The codec entry for this variant produced arguments this build of "
                      + "ffmpeg does not accept. Check the codec's ffmpeg settings in "
                      + "video_codecs.json against `ffmpeg -encoders`."
            )
        }
        if haystack.contains("invalid data found when processing input")
            || haystack.contains("error while decoding")
            || haystack.contains("could not open file")
        {
            return Diagnosis(
                kind: .sourceFrameUnreadable,
                summary: firstLine(containing: "invalid data found when processing input")
                      ?? firstLine(containing: "error while decoding")
                      ?? "a source frame could not be decoded",
                detail: "One of the exported frames in \(plan.inputDir) is truncated or corrupt. "
                      + "Re-export the sequence."
            )
        }

        // 3. No signature matched — fall back to exit status.
        if !terminatedNormally {
            return Diagnosis(
                kind: .terminatedBySignal,
                summary: "ffmpeg was killed before it finished"
                       + (exitCode.map { " (signal \($0))" } ?? ""),
                detail: "Something outside the daemon stopped the process — the OOM killer, a "
                      + "manual kill, or the machine going to sleep."
            )
        }
        if exitCode == 0 && !outputFileExists {
            return Diagnosis(
                kind: .producedNoOutput,
                summary: "ffmpeg reported success but \(plan.outputPath) does not exist",
                detail: "The output volume may have gone away mid-render, or something deleted "
                      + "the file immediately after it was written."
            )
        }
        let lastLine = interesting.last
        return Diagnosis(
            kind: .unknown,
            summary: lastLine ?? "ffmpeg exited \(exitCode.map(String.init) ?? "abnormally") "
                              + "without a recognisable error message",
            detail: interesting.isEmpty
                ? "ffmpeg produced no output beyond its startup banner."
                : nil
        )
    }
}
