import Foundation

/// One discovered raw image sequence (or a backup-json hint that one exists).
public struct RawSequence: Sendable, Hashable {
    public let parentPath: String   // The source_path it lives under.
    public let name: String         // The directory name.
    public let imageCount: Int      // Total raw images expected for this sequence.
}

/// One discovered exported image sequence ready (or partially ready) for render.
public struct ExportSequence: Sendable, Hashable {
    public let exportPath: String
    public let dirname: String
    public let rawSequenceParent: String
    public let rawSequenceName: String
    public let rawSequenceLength: Int
    public let generatedImageCount: Int

    public var isComplete: Bool {
        rawSequenceLength > 0 && generatedImageCount == rawSequenceLength
    }
    public var fullPath: String { "\(exportPath)/\(dirname)" }
    public var progress: Double {
        guard rawSequenceLength > 0 else { return 0 }
        return Double(generatedImageCount) / Double(rawSequenceLength)
    }
}

/// Scans the filesystem for raw and exported sequences.  Designed to:
///   * never throw out of fatal errors when a path is unreachable,
///   * cache directory mtimes so unchanged dirs aren't re-listed,
///   * scan all paths concurrently with per-path timeouts.
public actor SequenceScanner {

    private struct DirCacheEntry {
        var modTime: TimeInterval
        var entries: [String]
    }

    /// Seeded once at startup from backup_paths' .json files.  Continues to
    /// be refreshed in the background and is merged into raw sequence counts.
    private var backupSequenceCounts: [String: [String: Int]] = [:]

    /// (parentPath, dirname) -> raw image count, observed from source_paths.
    private var rawSequenceCounts: [String: [String: Int]] = [:]

    private var dirCache: [String: DirCacheEntry] = [:]

    public init() {}

    public func setBackupSequenceCounts(_ counts: [String: [String: Int]]) {
        self.backupSequenceCounts = counts
    }

    public func mergeBackupEntry(parentPath: String, name: String, count: Int) {
        backupSequenceCounts[parentPath, default: [:]][name] = count
    }

    /// Resets all caches; useful for "re-scan everything" actions from the GUI.
    public func clearCaches() {
        rawSequenceCounts.removeAll()
        dirCache.removeAll()
    }

    /// Scans every source_path concurrently and updates rawSequenceCounts.
    /// Backup-derived entries are preserved as a fallback.
    public func refreshRawSequenceCounts(
        config: DaemonConfig,
        timeoutSeconds: Double = 10,
        logger: DaemonLogger
    ) async {
        let rawTypes = Set(config.rawTypes.map { $0.lowercased() })
        await withTaskGroup(of: (String, [String: Int]).self) { group in
            for path in config.expandedSourcePaths {
                group.addTask {
                    return await Self.countRawSequences(
                        in: path,
                        rawTypes: rawTypes,
                        timeoutSeconds: timeoutSeconds,
                        logger: logger
                    )
                }
            }
            var newCounts: [String: [String: Int]] = [:]
            for await (path, counts) in group {
                newCounts[path] = counts
            }
            // Merge: source_paths take precedence, backup data fills gaps.
            for (parent, entries) in backupSequenceCounts {
                for (name, count) in entries {
                    if newCounts[parent]?[name] == nil {
                        newCounts[parent, default: [:]][name] = count
                    }
                }
            }
            self.rawSequenceCounts = newCounts
        }
    }

    public func currentRawSequences() -> [RawSequence] {
        var out: [RawSequence] = []
        for (parent, entries) in rawSequenceCounts {
            for (name, count) in entries {
                out.append(RawSequence(parentPath: parent, name: name, imageCount: count))
            }
        }
        return out
    }

    /// Scans every export_path concurrently for exported sequences that
    /// reference a known raw sequence by name.
    public func discoverExportSequences(
        config: DaemonConfig,
        timeoutSeconds: Double = 10,
        logger: DaemonLogger
    ) async -> [ExportSequence] {
        let imageExts = Set(config.imageExtensions.map { "." + $0.lowercased() })
        let prefix = config.imageSequenceDirnamePrefix
        let raw = rawSequenceCounts
        let ignore = config.ignoreSequenceRegexes.compactMap {
            try? NSRegularExpression(pattern: $0)
        }

        return await withTaskGroup(of: [ExportSequence].self) { group in
            for path in config.expandedExportPaths {
                group.addTask {
                    return await Self.discoverFromExportDir(
                        exportPath: path,
                        rawSequenceCounts: raw,
                        ignoreRegexes: ignore,
                        sequenceDirnamePrefix: prefix,
                        imageExtensions: imageExts,
                        timeoutSeconds: timeoutSeconds,
                        logger: logger
                    )
                }
            }
            var all: [ExportSequence] = []
            for await batch in group { all.append(contentsOf: batch) }
            return all
        }
    }

    /// Reads backup JSON files; populates `backupSequenceCounts` opportunistically
    /// and yields each entry as it's discovered.  Designed to not block startup -
    /// the daemon kicks this off in a Task and proceeds immediately.
    public func loadBackupSequenceCountsLazily(
        backupPaths: [String],
        timeoutSeconds: Double = 30,
        logger: DaemonLogger,
        onEntry: (@Sendable (_ parent: String, _ name: String, _ count: Int) -> Void)? = nil
    ) async {
        await withTaskGroup(of: Void.self) { group in
            for path in backupPaths {
                group.addTask {
                    await Self.loadBackupSequencesIn(
                        path: path,
                        timeoutSeconds: timeoutSeconds,
                        logger: logger,
                        onEntry: onEntry
                    )
                }
            }
        }
    }

    // MARK: - Private static helpers

    private static func countRawSequences(
        in path: String,
        rawTypes: Set<String>,
        timeoutSeconds: Double,
        logger: DaemonLogger
    ) async -> (String, [String: Int]) {
        let result = await withTimeout(seconds: timeoutSeconds) { () -> [String: Int] in
            var counts: [String: Int] = [:]
            guard let entries = try? FileManager.default.contentsOfDirectory(atPath: path) else {
                return counts
            }
            for entry in entries {
                if entry.hasPrefix(".") { continue }
                let full = "\(path)/\(entry)"
                var isDir: ObjCBool = false
                guard FileManager.default.fileExists(atPath: full, isDirectory: &isDir), isDir.boolValue else {
                    continue
                }
                guard let sub = try? FileManager.default.contentsOfDirectory(atPath: full) else { continue }
                var rawCount = 0
                for f in sub {
                    if let dot = f.lastIndex(of: ".") {
                        let ext = String(f[f.index(after: dot)...]).lowercased()
                        if rawTypes.contains(ext) { rawCount += 1 }
                    }
                }
                counts[entry] = rawCount
            }
            return counts
        }
        switch result {
        case .success(let counts):
            return (path, counts)
        case .timedOut:
            logger.warn("source path scan timed out: \(path)")
            return (path, [:])
        case .failure(let err):
            logger.warn("source path scan failed for \(path): \(err)")
            return (path, [:])
        }
    }

    private static func discoverFromExportDir(
        exportPath: String,
        rawSequenceCounts: [String: [String: Int]],
        ignoreRegexes: [NSRegularExpression],
        sequenceDirnamePrefix: String,
        imageExtensions: Set<String>,
        timeoutSeconds: Double,
        logger: DaemonLogger
    ) async -> [ExportSequence] {
        let result = await withTimeout(seconds: timeoutSeconds) { () -> [ExportSequence] in
            var out: [ExportSequence] = []
            guard let entries = try? FileManager.default.contentsOfDirectory(atPath: exportPath) else {
                return out
            }
            for entry in entries {
                if entry.hasPrefix(".") { continue }
                let full = "\(exportPath)/\(entry)"
                var isDir: ObjCBool = false
                guard FileManager.default.fileExists(atPath: full, isDirectory: &isDir), isDir.boolValue else {
                    continue
                }
                if ignoreRegexes.contains(where: { Self.regexMatches($0, in: entry) }) {
                    continue
                }
                // Find a raw sequence whose name appears in this dirname after the prefix.
                var rawParent: String? = nil
                var rawName: String? = nil
                var rawLength = 0
                outer: for (parent, names) in rawSequenceCounts {
                    for (name, count) in names {
                        let needle = sequenceDirnamePrefix + name
                        if entry.contains(needle) {
                            rawParent = parent
                            rawName = name
                            rawLength = count
                            break outer
                        }
                    }
                }
                guard let parent = rawParent, let name = rawName else { continue }

                // Count generated images in the export dir.
                var generated = 0
                if let sub = try? FileManager.default.contentsOfDirectory(atPath: full) {
                    for f in sub {
                        if f.hasPrefix(".") { continue }
                        guard let dot = f.lastIndex(of: ".") else { continue }
                        let ext = String(f[dot...]).lowercased()
                        if imageExtensions.contains(ext) { generated += 1 }
                    }
                }
                out.append(ExportSequence(
                    exportPath: exportPath,
                    dirname: entry,
                    rawSequenceParent: parent,
                    rawSequenceName: name,
                    rawSequenceLength: rawLength,
                    generatedImageCount: generated
                ))
            }
            return out
        }
        switch result {
        case .success(let list): return list
        case .timedOut:
            logger.warn("export path scan timed out: \(exportPath)")
            return []
        case .failure(let err):
            logger.warn("export path scan failed for \(exportPath): \(err)")
            return []
        }
    }

    private static func loadBackupSequencesIn(
        path: String,
        timeoutSeconds: Double,
        logger: DaemonLogger,
        onEntry: (@Sendable (_ parent: String, _ name: String, _ count: Int) -> Void)?
    ) async {
        let result = await withTimeout(seconds: timeoutSeconds) { () -> Void in
            guard let entries = try? FileManager.default.contentsOfDirectory(atPath: path) else {
                return
            }
            for entry in entries {
                // Match Perl daemon's `^([^.]+)[.]json$` — only single-component
                // names (no dots in the stem) count as backup metadata files.
                guard let match = entry.firstMatch(of: #/^([^.]+)\.json$/#) else { continue }
                let name = String(match.output.1)
                let full = "\(path)/\(entry)"
                guard let data = try? Data(contentsOf: URL(fileURLWithPath: full)) else { continue }
                guard let any = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                    continue
                }
                var count = 0
                if let imageSize = any["ImageSize"] as? [String: Any] {
                    if let firstKey = imageSize.keys.sorted().first,
                       let val = imageSize[firstKey] as? Int {
                        count = val
                    }
                }
                if count == 0, let n = any["number_source_images"] as? Int { count = n }
                onEntry?(path, name, count)
            }
        }
        if case .timedOut = result {
            logger.warn("backup path scan timed out: \(path)")
        }
    }

    private static func regexMatches(_ regex: NSRegularExpression, in s: String) -> Bool {
        let range = NSRange(s.startIndex..., in: s)
        return regex.firstMatch(in: s, options: [], range: range) != nil
    }
}

// MARK: - Generic timeout helper

enum TimeoutResult<T: Sendable>: Sendable {
    case success(T)
    case timedOut
    case failure(Error)
}

func withTimeout<T: Sendable>(
    seconds: Double,
    operation: @escaping @Sendable () async -> T
) async -> TimeoutResult<T> {
    return await withTaskGroup(of: TimeoutResult<T>.self) { group in
        group.addTask {
            return .success(await operation())
        }
        group.addTask {
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            return .timedOut
        }
        let first = await group.next() ?? .timedOut
        group.cancelAll()
        return first
    }
}
