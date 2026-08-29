import Foundation

/// A render that did not produce a usable video, together with a
/// human-readable explanation of why.
///
/// Failures used to exist only as a `.failed` status that
/// `cleanupFinishedRenders()` swept away on the next scan tick, plus one
/// collapsed line in the log file.  They are now retained here, published in
/// `DaemonStateSnapshot.recentFailures`, and printed to the console — so a
/// render that dies two seconds after it starts leaves something behind that
/// says why.
public struct RenderFailure: Sendable, Equatable, Hashable, Identifiable {

    public enum Kind: String, Sendable, Equatable, Hashable, Codable {
        /// ffmpeg's image2 demuxer matched none of the files in the input dir.
        case inputFramesNotFound
        /// The input directory could not be listed / opened.
        case inputUnreadable
        /// A source frame is present but not decodable.
        case sourceFrameUnreadable
        /// ffmpeg rejected the command line (unknown encoder, bad option…).
        case ffmpegRejectedArguments
        /// The output path could not be written.
        case outputNotWritable
        case diskFull
        /// ffmpeg was killed by a signal rather than exiting on its own.
        case terminatedBySignal
        /// ffmpeg exited 0 but there is no output file on disk.
        case producedNoOutput
        /// The render completed but the container is structurally broken.
        case structuralCorruption
        /// ffmpeg could not be launched at all.
        case launchFailed
        /// The variant could not even be queued (no images in the sequence…).
        case notQueueable
        /// No signature matched.
        case unknown

        /// Short label for the GUI badge and the console header.
        public var label: String {
            switch self {
            case .inputFramesNotFound:     return "no input frames"
            case .inputUnreadable:         return "input unreadable"
            case .sourceFrameUnreadable:   return "bad source frame"
            case .ffmpegRejectedArguments: return "ffmpeg rejected arguments"
            case .outputNotWritable:       return "output not writable"
            case .diskFull:                return "disk full"
            case .terminatedBySignal:      return "killed"
            case .producedNoOutput:        return "no output produced"
            case .structuralCorruption:    return "corrupt output"
            case .launchFailed:            return "ffmpeg would not start"
            case .notQueueable:            return "cannot start render"
            case .unknown:                 return "render failed"
            }
        }
    }

    /// When this failure was last seen.
    public var timestamp: Date
    public let sequenceName: String
    public let exportPath: String
    public let outputFilename: String
    public let outputPath: String
    public let variantKey: String
    public let kind: Kind
    /// One line: what went wrong.
    public let summary: String
    /// Optional multi-line elaboration: what the daemon expected, what it
    /// actually found, and what to change to fix it.
    public let detail: String?
    public let exitCode: Int32?
    /// The ffmpeg command line, so the render can be reproduced by hand.
    public let commandLine: String?
    /// Tool output worth reading.  ffmpeg's banner and library-version lines
    /// are stripped by `RenderFailureDiagnosis.significantLines`.
    public let toolOutput: [String]
    /// How many times this same failure has been seen.  The daemon re-queues a
    /// failed variant on every scan tick, so without this the list fills up
    /// with identical entries.
    public var occurrences: Int

    public var id: String { outputPath }

    public init(
        timestamp: Date = Date(),
        sequenceName: String,
        exportPath: String,
        outputFilename: String,
        outputPath: String,
        variantKey: String,
        kind: Kind,
        summary: String,
        detail: String? = nil,
        exitCode: Int32? = nil,
        commandLine: String? = nil,
        toolOutput: [String] = [],
        occurrences: Int = 1
    ) {
        self.timestamp = timestamp
        self.sequenceName = sequenceName
        self.exportPath = exportPath
        self.outputFilename = outputFilename
        self.outputPath = outputPath
        self.variantKey = variantKey
        self.kind = kind
        self.summary = summary
        self.detail = detail
        self.exitCode = exitCode
        self.commandLine = commandLine
        self.toolOutput = toolOutput
        self.occurrences = occurrences
    }

    /// Compact one-liner for `RenderJobState.message` and log lines.
    public var shortMessage: String {
        var s = "\(kind.label): \(summary)"
        if occurrences > 1 { s += " (x\(occurrences))" }
        return s
    }

    /// Multi-line block written to the console (and, indented, to the log
    /// file).  Deliberately verbose: this is the thing that has to answer
    /// "why did my render fail?" without the user going digging.
    public var consoleReport: String {
        var out: [String] = []
        var header = "RENDER FAILED  \(outputFilename)  —  \(kind.label)"
        if occurrences > 1 { header += "  (occurrence \(occurrences))" }
        out.append(header)
        out.append("  sequence: \(sequenceName)")
        out.append("  output:   \(outputPath)")
        out.append("  variant:  \(variantKey)")
        if let exitCode {
            out.append("  ffmpeg exit code: \(exitCode)")
        }
        out.append("  why: \(summary)")
        if let detail, !detail.isEmpty {
            for line in detail.split(separator: "\n", omittingEmptySubsequences: false) {
                out.append("       \(line)")
            }
        }
        if !toolOutput.isEmpty {
            out.append("  tool output:")
            for line in toolOutput {
                out.append("    | \(line)")
            }
        }
        if let commandLine {
            out.append("  command: \(commandLine)")
        }
        return out.joined(separator: "\n")
    }

    /// Folds a repeat of the same failure into this one, bumping the counter
    /// and taking the newer timestamp and output.
    public func merging(_ newer: RenderFailure) -> RenderFailure {
        var merged = newer
        merged.occurrences = occurrences + 1
        return merged
    }
}
