import Foundation

/// A snapshot of what the daemon is currently doing. Re-published every time
/// the state changes; suitable for SwiftUI or any other observer.
public struct DaemonStateSnapshot: Sendable, Equatable {

    public enum Phase: String, Sendable, Equatable {
        case startingUp
        case scanning
        case rendering
        case idle
        case stopped
    }

    public var phase: Phase
    public var configPath: String?
    public var rawSequenceCount: Int
    public var sequences: [SequenceState]
    public var renders: [RenderJobState]
    /// Recently rendered videos, newest first.  Bounded length; the daemon
    /// trims old entries to avoid unbounded growth.
    public var completedRenders: [CompletedRenderResult]
    /// Renders that failed, newest first, each with a diagnosis of why.
    /// Retained across scan ticks — `cleanupFinishedRenders()` drops the
    /// `.failed` job itself within seconds, so without this a render that dies
    /// on launch leaves no trace in the UI.
    public var recentFailures: [RenderFailure]
    public var pausedSequenceNames: Set<String>
    public var pausedVariants: Set<PausedVariant>
    /// Codec names referenced by the config's rendering variants that have no
    /// entry in the daemon's codec table.  Variants using them are silently
    /// skipped by the render queue, so observers should surface these.
    public var unknownCodecs: [String]
    public var lastError: String?

    public init(
        phase: Phase = .startingUp,
        configPath: String? = nil,
        rawSequenceCount: Int = 0,
        sequences: [SequenceState] = [],
        renders: [RenderJobState] = [],
        completedRenders: [CompletedRenderResult] = [],
        recentFailures: [RenderFailure] = [],
        pausedSequenceNames: Set<String> = [],
        pausedVariants: Set<PausedVariant> = [],
        unknownCodecs: [String] = [],
        lastError: String? = nil
    ) {
        self.phase = phase
        self.configPath = configPath
        self.rawSequenceCount = rawSequenceCount
        self.sequences = sequences
        self.renders = renders
        self.completedRenders = completedRenders
        self.recentFailures = recentFailures
        self.pausedSequenceNames = pausedSequenceNames
        self.pausedVariants = pausedVariants
        self.unknownCodecs = unknownCodecs
        self.lastError = lastError
    }
}

public struct SequenceState: Sendable, Equatable, Identifiable, Hashable {
    public let exportPath: String
    public let dirname: String
    public let rawSequenceLength: Int
    public let generatedImageCount: Int
    public let isComplete: Bool
    /// True when all configured render variants have already produced output
    /// files for this sequence (no work remaining).  Used by the GUI to
    /// distinguish "fully done" from "in progress".
    public var allVariantsRendered: Bool = false
    public var id: String { "\(exportPath)/\(dirname)" }

    public var progress: Double {
        guard rawSequenceLength > 0 else { return 0 }
        return Double(generatedImageCount) / Double(rawSequenceLength)
    }
}

/// A single video that finished rendering successfully — recorded for the
/// "rendered" timeline in the GUI.
public struct CompletedRenderResult: Sendable, Equatable, Identifiable, Hashable, Codable {
    public let timestamp: Date
    public let sequenceName: String
    public let exportPath: String
    public let outputFilename: String
    public let outputPath: String
    public let frameCount: Int
    public let fileSize: Int64
    public var id: String { outputPath }

    public init(
        timestamp: Date,
        sequenceName: String,
        exportPath: String,
        outputFilename: String,
        outputPath: String,
        frameCount: Int,
        fileSize: Int64
    ) {
        self.timestamp = timestamp
        self.sequenceName = sequenceName
        self.exportPath = exportPath
        self.outputFilename = outputFilename
        self.outputPath = outputPath
        self.frameCount = frameCount
        self.fileSize = fileSize
    }
}

public struct RenderJobState: Sendable, Equatable, Identifiable, Hashable {
    public enum Status: String, Sendable, Equatable {
        case queued
        case running
        case finished
        case failed
        case skipped       // output already existed
        case paused
        case cancelled
    }

    public let sequenceName: String
    public let exportPath: String
    public let outputFilename: String
    public let variantKey: String
    public let totalFrames: Int
    public var frame: Int
    public var fps: Double
    public var status: Status
    /// Human-readable note about this job — for a failed render this is the
    /// one-line diagnosis, previously left nil for everything except
    /// corruption.
    public var message: String?
    /// Set when `status == .failed`, so the GUI can label the row without
    /// re-parsing `message`.
    public var failureKind: RenderFailure.Kind?
    /// True when this render is an ffmpeg process the daemon adopted — it
    /// was running at startup, left over from a previous session.  We have
    /// no pipe to it, so progress can't be tracked; callers should display
    /// an indeterminate progress indicator.
    public var isAdopted: Bool = false

    public var id: String { "\(exportPath)/\(outputFilename)" }
    public var progress: Double {
        guard totalFrames > 0 else { return 0 }
        return Double(frame) / Double(totalFrames)
    }
}
