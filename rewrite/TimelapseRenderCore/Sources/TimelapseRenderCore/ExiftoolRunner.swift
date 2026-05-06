import Foundation

/// Wraps `exiftool -csv FILE` and produces a key/value map for one file.
public struct ExiftoolRunner: Sendable {

    public let filename: String
    public init(filename: String) { self.filename = filename }

    public func run() async throws -> [String: String] {
        let runner = ProcessRunner(executable: "exiftool", arguments: ["-csv", filename])
        let collected = LineCollector()
        _ = try await runner.run { line in
            collected.append(line)
        }
        return parseCSV(collected.joined())
    }

    private final class LineCollector: @unchecked Sendable {
        private let lock = NSLock()
        private var lines: [String] = []
        func append(_ line: String) { lock.lock(); lines.append(line); lock.unlock() }
        func joined() -> String { lock.lock(); defer { lock.unlock() }; return lines.joined(separator: "\n") + "\n" }
    }

    /// Tiny CSV parser: the exiftool -csv output for a single file has a header
    /// line and one data line.  Handles quoted values with embedded commas.
    private func parseCSV(_ text: String) -> [String: String] {
        let rows = parseRows(text).filter { !$0.isEmpty }
        guard rows.count >= 2 else { return [:] }
        let header = rows[0]
        let values = rows[1]
        var map: [String: String] = [:]
        for i in 0..<min(header.count, values.count) {
            map[header[i]] = values[i]
        }
        return map
    }

    private func parseRows(_ text: String) -> [[String]] {
        var rows: [[String]] = []
        var current: [String] = []
        var field = ""
        var inQuotes = false
        var i = text.startIndex
        while i < text.endIndex {
            let c = text[i]
            if inQuotes {
                if c == "\"" {
                    let next = text.index(after: i)
                    if next < text.endIndex && text[next] == "\"" {
                        field.append("\"")
                        i = text.index(after: next)
                        continue
                    } else {
                        inQuotes = false
                    }
                } else {
                    field.append(c)
                }
            } else {
                switch c {
                case "\"":
                    inQuotes = true
                case ",":
                    current.append(field)
                    field = ""
                case "\n", "\r":
                    if !field.isEmpty || !current.isEmpty {
                        current.append(field)
                        rows.append(current)
                        current = []
                        field = ""
                    }
                default:
                    field.append(c)
                }
            }
            i = text.index(after: i)
        }
        if !field.isEmpty || !current.isEmpty {
            current.append(field)
            rows.append(current)
        }
        return rows
    }
}

public enum ExifAccumulator {
    /// Folds a per-file exif map into a totals dictionary that counts how
    /// often each value occurs for each key.  Mirrors the Perl
    /// `process_exif` behaviour, including the binary-key skip list.
    public static func accumulate(
        into totals: inout [String: [String: Int]],
        from map: [String: String]
    ) {
        let skipKeys: Set<String> = [
            "Thumbnail Image", "Preview Image", "Tiff Metering Image",
        ]
        for (key, value) in map {
            if skipKeys.contains(key) { continue }
            totals[key, default: [:]][value, default: 0] += 1
        }
    }

    /// From the totals map, produce the single-value-per-key map used as
    /// ffmpeg metadata.  Only keys with exactly one observed value are kept.
    public static func collapseToSingleValues(_ totals: [String: [String: Int]]) -> [String: String] {
        var out: [String: String] = [:]
        for (key, counts) in totals where counts.count == 1 {
            out[key] = counts.first?.key
        }
        return out
    }
}
