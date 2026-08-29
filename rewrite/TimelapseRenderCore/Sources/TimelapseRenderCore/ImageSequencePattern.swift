import Foundation

/// The `prefix%0Nd suffix` shape of an exported image sequence, worked out by
/// looking at the files rather than assuming.
///
/// The daemon used to build ffmpeg's `-i` pattern from `image_name_prefix` in
/// the config plus a hardcoded five-digit frame number.  That only ever
/// matched sequences LRTimelapse happened to name `LRT_00001.tiff`; anything
/// else (`image_0001.tiff`, say) matched nothing and ffmpeg exited 254 in
/// under a second.  Only the suffix was detected from the files.  Now all
/// three parts are.
public struct ImageSequencePattern: Sendable, Equatable {

    /// Everything before the first run of digits, e.g. `image_`.
    public let prefix: String
    /// Width of the zero-padded frame number, e.g. 4 for `image_0001.tiff`.
    public let digits: Int
    /// Everything after the frame number, e.g. `.tiff` or `-severe-noise.tiff`.
    public let suffix: String
    /// Lowest frame number present.
    public let firstIndex: Int
    /// How many files matched this pattern.
    public let frameCount: Int
    /// A real filename from the matched set — the lowest-numbered one.  Used
    /// to probe the source dimensions for width-only renders.
    public let exampleFilename: String
    /// True when the detected prefix is the config's `image_name_prefix`.
    /// False means the exporter named the frames something else and the
    /// daemon adapted to it.
    public let matchesConfiguredPrefix: Bool

    public init(
        prefix: String,
        digits: Int,
        suffix: String,
        firstIndex: Int,
        frameCount: Int,
        exampleFilename: String,
        matchesConfiguredPrefix: Bool
    ) {
        self.prefix = prefix
        self.digits = digits
        self.suffix = suffix
        self.firstIndex = firstIndex
        self.frameCount = frameCount
        self.exampleFilename = exampleFilename
        self.matchesConfiguredPrefix = matchesConfiguredPrefix
    }

    /// The digit width the daemon assumed before detection existed.  Kept as
    /// the default for `FFmpegRunner.Plan` so hand-built plans (tests, tools)
    /// behave the way they always did.
    public static let legacyFrameNumberDigits = 5

    /// The filename part of ffmpeg's `-i` argument, e.g. `image_%04d.tiff`.
    public var patternDescription: String { "\(prefix)%0\(digits)d\(suffix)" }

    /// ffmpeg's image2 demuxer probes only indices 0..<`imageDemuxerProbeRange`
    /// when hunting for the first frame, so a sequence starting past that
    /// needs an explicit `-start_number`.  Sequences that start at 0 or 1 —
    /// which is nearly all of them — keep the exact argv they had before, so
    /// this cannot disturb a render that already works.
    public var requiresExplicitStartNumber: Bool {
        firstIndex >= RenderFailureDiagnosis.imageDemuxerProbeRange
    }

    /// Works out the pattern from the files in `dir`.  Returns nil when the
    /// directory can't be read or holds no numbered image files — there is
    /// nothing to hand ffmpeg in that case.
    ///
    /// When several differently-named sequences share a directory, the one
    /// matching `preferredPrefix` wins; otherwise the largest group does.
    /// That ordering matters: a directory holding both `LRT_00001.tiff` and
    /// some stray `image_0001.tiff` files must keep rendering the sequence
    /// the config asked for.
    public static func detect(
        inDirectory dir: String,
        imageExtensions: [String],
        preferredPrefix: String,
        fileManager: FileManager = .default
    ) -> ImageSequencePattern? {
        guard let entries = try? fileManager.contentsOfDirectory(atPath: dir) else { return nil }
        let wanted = Set(imageExtensions.map { $0.lowercased() })

        struct Key: Hashable {
            let prefix: String
            let digits: Int
            let suffix: String
        }
        struct Group {
            var count = 0
            var firstIndex = Int.max
            var example = ""
        }
        var groups: [Key: Group] = [:]

        for name in entries {
            if name.hasPrefix(".") { continue }
            guard let dot = name.lastIndex(of: ".") else { continue }
            let ext = String(name[name.index(after: dot)...]).lowercased()
            if !wanted.isEmpty && !wanted.contains(ext) { continue }
            guard let parts = RenderFailureDiagnosis.decompose(name),
                  let index = Int(parts.digits) else { continue }
            let key = Key(prefix: parts.prefix, digits: parts.digits.count, suffix: parts.suffix)
            var group = groups[key] ?? Group()
            group.count += 1
            if index < group.firstIndex {
                group.firstIndex = index
                group.example = name
            }
            groups[key] = group
        }
        guard !groups.isEmpty else { return nil }

        let preferred = groups.filter { $0.key.prefix == preferredPrefix }
        let pool = preferred.isEmpty ? groups : preferred
        // Largest group wins; ties break on the lexicographically smallest
        // example so the choice is stable across runs.
        guard let winner = pool.max(by: { a, b in
            if a.value.count != b.value.count { return a.value.count < b.value.count }
            return b.value.example < a.value.example
        }) else { return nil }

        return ImageSequencePattern(
            prefix: winner.key.prefix,
            digits: winner.key.digits,
            suffix: winner.key.suffix,
            firstIndex: winner.value.firstIndex,
            frameCount: winner.value.count,
            exampleFilename: winner.value.example,
            matchesConfiguredPrefix: winner.key.prefix == preferredPrefix
        )
    }
}
