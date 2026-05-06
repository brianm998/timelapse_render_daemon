import Foundation

// Mirrors the JSON config format used by the original Perl daemon.
// Keys match the Perl version exactly, including the historical
// "max_cuncurrent_renders" misspelling, so the same config.json works.

public struct DaemonConfig: Codable, Sendable, Equatable {

    public var exportPaths: [String]
    public var sourcePaths: [String]
    public var backupPaths: [String]
    public var masterVideos: [String]

    public var maxProcesses: MaxProcesses
    public var maxConcurrentRenders: Int

    public var imageNamePrefix: String
    public var imageSequenceDirnamePrefix: String

    public var rawTypes: [String]
    public var imageExtensions: [String]

    public var rendering: [RenderType]

    public var logInColor: Bool
    public var sleepTimeSeconds: Double
    public var scanIntervalSeconds: Double
    public var progressBarWidth: Int

    public var ignoreSequenceRegexes: [String]
    public var deleteSequenceAfterRenderRegexes: [String]

    // Pause flags - set/cleared via the daemon API, not normally written into
    // config.json by the user, but allowed there for persistence across runs.
    public var pausedSequenceNames: [String]
    public var pausedVariants: [PausedVariant]

    public init(
        exportPaths: [String] = [],
        sourcePaths: [String] = [],
        backupPaths: [String] = [],
        masterVideos: [String] = [],
        maxProcesses: MaxProcesses = .defaults,
        maxConcurrentRenders: Int = 1,
        imageNamePrefix: String = "",
        imageSequenceDirnamePrefix: String = "",
        rawTypes: [String] = ["ARW", "ORF", "DNG"],
        imageExtensions: [String] = ["tif", "tiff"],
        rendering: [RenderType] = [],
        logInColor: Bool = true,
        sleepTimeSeconds: Double = 12,
        scanIntervalSeconds: Double = 2,
        progressBarWidth: Int = 40,
        ignoreSequenceRegexes: [String] = [],
        deleteSequenceAfterRenderRegexes: [String] = [],
        pausedSequenceNames: [String] = [],
        pausedVariants: [PausedVariant] = []
    ) {
        self.exportPaths = exportPaths
        self.sourcePaths = sourcePaths
        self.backupPaths = backupPaths
        self.masterVideos = masterVideos
        self.maxProcesses = maxProcesses
        self.maxConcurrentRenders = maxConcurrentRenders
        self.imageNamePrefix = imageNamePrefix
        self.imageSequenceDirnamePrefix = imageSequenceDirnamePrefix
        self.rawTypes = rawTypes
        self.imageExtensions = imageExtensions
        self.rendering = rendering
        self.logInColor = logInColor
        self.sleepTimeSeconds = sleepTimeSeconds
        self.scanIntervalSeconds = scanIntervalSeconds
        self.progressBarWidth = progressBarWidth
        self.ignoreSequenceRegexes = ignoreSequenceRegexes
        self.deleteSequenceAfterRenderRegexes = deleteSequenceAfterRenderRegexes
        self.pausedSequenceNames = pausedSequenceNames
        self.pausedVariants = pausedVariants
    }

    enum CodingKeys: String, CodingKey {
        case exportPaths = "export_paths"
        case sourcePaths = "source_paths"
        case backupPaths = "backup_paths"
        case masterVideos = "master_videos"
        case maxProcesses = "max_processes"
        case maxConcurrentRenders = "max_cuncurrent_renders"
        case imageNamePrefix = "image_name_prefix"
        case imageSequenceDirnamePrefix = "image_sequence_dirname_prefix"
        case rawTypes = "raw_types"
        case imageExtensions = "image_extensions"
        case rendering
        case logInColor = "log_in_color"
        case sleepTimeSeconds = "sleep_time_seconds"
        case scanIntervalSeconds = "scan_interval_seconds"
        case progressBarWidth = "progress_bar_width"
        case ignoreSequenceRegexes = "ignore_sequence_regexes"
        case deleteSequenceAfterRenderRegexes = "delete_sequence_after_render_regexes"
        case pausedSequenceNames = "paused_sequence_names"
        case pausedVariants = "paused_variants"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        exportPaths = try c.decodeIfPresent([String].self, forKey: .exportPaths) ?? []
        sourcePaths = try c.decodeIfPresent([String].self, forKey: .sourcePaths) ?? []
        backupPaths = try c.decodeIfPresent([String].self, forKey: .backupPaths) ?? []
        masterVideos = try c.decodeIfPresent([String].self, forKey: .masterVideos) ?? []
        maxProcesses = try c.decodeIfPresent(MaxProcesses.self, forKey: .maxProcesses) ?? .defaults
        maxConcurrentRenders = try c.decodeIfPresent(Int.self, forKey: .maxConcurrentRenders) ?? 1
        imageNamePrefix = try c.decodeIfPresent(String.self, forKey: .imageNamePrefix) ?? ""
        imageSequenceDirnamePrefix = try c.decodeIfPresent(String.self, forKey: .imageSequenceDirnamePrefix) ?? ""
        rawTypes = try c.decodeIfPresent([String].self, forKey: .rawTypes) ?? ["ARW", "ORF", "DNG"]
        imageExtensions = try c.decodeIfPresent([String].self, forKey: .imageExtensions) ?? ["tif", "tiff"]
        rendering = try c.decodeIfPresent([RenderType].self, forKey: .rendering) ?? []
        logInColor = try c.decodeIfPresent(Bool.self, forKey: .logInColor) ?? true
        sleepTimeSeconds = try c.decodeIfPresent(Double.self, forKey: .sleepTimeSeconds) ?? 12
        scanIntervalSeconds = try c.decodeIfPresent(Double.self, forKey: .scanIntervalSeconds) ?? 2
        progressBarWidth = try c.decodeIfPresent(Int.self, forKey: .progressBarWidth) ?? 40
        ignoreSequenceRegexes = try c.decodeIfPresent([String].self, forKey: .ignoreSequenceRegexes) ?? []
        deleteSequenceAfterRenderRegexes = try c.decodeIfPresent([String].self, forKey: .deleteSequenceAfterRenderRegexes) ?? []
        pausedSequenceNames = try c.decodeIfPresent([String].self, forKey: .pausedSequenceNames) ?? []
        pausedVariants = try c.decodeIfPresent([PausedVariant].self, forKey: .pausedVariants) ?? []
    }
}

public struct MaxProcesses: Codable, Sendable, Equatable {
    public var ffmpeg: Int
    public var exiftool: Int

    public init(ffmpeg: Int = 1, exiftool: Int = 10) {
        self.ffmpeg = ffmpeg
        self.exiftool = exiftool
    }

    public static let defaults = MaxProcesses(ffmpeg: 1, exiftool: 10)
}

public struct RenderType: Codable, Sendable, Equatable, Identifiable {
    public var resolution: [Int]?
    public var codec: String
    public var frameRate: Int

    public init(resolution: [Int]? = nil, codec: String, frameRate: Int) {
        self.resolution = resolution
        self.codec = codec
        self.frameRate = frameRate
    }

    enum CodingKeys: String, CodingKey {
        case resolution
        case codec
        case frameRate = "frame_rate"
    }

    /// Stable string used to identify a variant of a render set.
    public var variantKey: String {
        let res: String
        if let r = resolution, r.count == 2 {
            res = "\(r[0])x\(r[1])"
        } else {
            res = "ori"
        }
        return "\(codec)_\(res)_\(frameRate)"
    }

    public var id: String { variantKey }

    public var width: Int? {
        guard let r = resolution, r.count == 2 else { return nil }
        return r[0]
    }
    public var height: Int? {
        guard let r = resolution, r.count == 2 else { return nil }
        return r[1]
    }
}

public struct PausedVariant: Codable, Sendable, Equatable, Hashable {
    public var sequenceName: String
    public var variantKey: String

    public init(sequenceName: String, variantKey: String) {
        self.sequenceName = sequenceName
        self.variantKey = variantKey
    }

    enum CodingKeys: String, CodingKey {
        case sequenceName = "sequence_name"
        case variantKey = "variant_key"
    }
}

public struct VideoCodec: Codable, Sendable, Equatable {
    public var ffmpeg: FFmpeg
    public var filename: Filename

    public struct FFmpeg: Codable, Sendable, Equatable {
        public var codec: String
        public var pixelFormat: String
        public var colorRange: String

        enum CodingKeys: String, CodingKey {
            case codec
            case pixelFormat = "pixel_format"
            case colorRange = "color_range"
        }
    }
    public struct Filename: Codable, Sendable, Equatable {
        public var pixelFormat: String
        public var codec: String
        public var quality: String

        enum CodingKeys: String, CodingKey {
            case pixelFormat = "pixel_format"
            case codec
            case quality
        }
    }
}

public typealias VideoCodecs = [String: VideoCodec]

public enum ConfigLoader {

    public static func load(at path: String) throws -> DaemonConfig {
        let url = URL(fileURLWithPath: path)
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(DaemonConfig.self, from: data)
    }

    public static func write(_ config: DaemonConfig, to path: String) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(config)
        let url = URL(fileURLWithPath: path)
        try data.write(to: url, options: .atomic)
    }

    public static func loadCodecs(at path: String) throws -> VideoCodecs {
        let url = URL(fileURLWithPath: path)
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(VideoCodecs.self, from: data)
    }

    /// Built-in codec table that matches the original `video_codecs.json`,
    /// used as a fallback when the file is not present.
    public static var defaultCodecs: VideoCodecs {
        [
            "ProResUHQ": VideoCodec(
                ffmpeg: .init(codec: "prores_ks", pixelFormat: "yuv422p10le", colorRange: "pc"),
                filename: .init(pixelFormat: "444", codec: "ProRes", quality: "UHQ")
            ),
            "ProResMQ": VideoCodec(
                ffmpeg: .init(codec: "prores_ks", pixelFormat: "yuv444p10le", colorRange: "pc"),
                filename: .init(pixelFormat: "422", codec: "ProRes", quality: "MQ")
            ),
        ]
    }
}
