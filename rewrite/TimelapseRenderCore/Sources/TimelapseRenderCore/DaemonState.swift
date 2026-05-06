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
    public var pausedSequenceNames: Set<String>
    public var pausedVariants: Set<PausedVariant>
    public var lastError: String?

    public init(
        phase: Phase = .startingUp,
        configPath: String? = nil,
        rawSequenceCount: Int = 0,
        sequences: [SequenceState] = [],
        renders: [RenderJobState] = [],
        pausedSequenceNames: Set<String> = [],
        pausedVariants: Set<PausedVariant> = [],
        lastError: String? = nil
    ) {
        self.phase = phase
        self.configPath = configPath
        self.rawSequenceCount = rawSequenceCount
        self.sequences = sequences
        self.renders = renders
        self.pausedSequenceNames = pausedSequenceNames
        self.pausedVariants = pausedVariants
        self.lastError = lastError
    }
}

public struct SequenceState: Sendable, Equatable, Identifiable {
    public let exportPath: String
    public let dirname: String
    public let rawSequenceLength: Int
    public let generatedImageCount: Int
    public let isComplete: Bool
    public var id: String { "\(exportPath)/\(dirname)" }

    public var progress: Double {
        guard rawSequenceLength > 0 else { return 0 }
        return Double(generatedImageCount) / Double(rawSequenceLength)
    }
}

public struct RenderJobState: Sendable, Equatable, Identifiable {
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
    public var message: String?
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
