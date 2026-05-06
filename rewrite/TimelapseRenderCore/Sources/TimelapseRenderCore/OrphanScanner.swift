import Foundation

/// Information about a currently-running ffmpeg process that we *didn't*
/// spawn — typically left over from a previous TimelapseRenderGUI session
/// that was killed while ffmpeg was mid-render.
public struct OrphanedRender: Sendable, Equatable {
    public let pid: pid_t
    public let inputPattern: String     // e.g. /rp/tmp/LRT_xxx/LRT_%05d.tiff
    public let outputPath: String       // e.g. /rp/tmp/foo.mov

    public init(pid: pid_t, inputPattern: String, outputPath: String) {
        self.pid = pid
        self.inputPattern = inputPattern
        self.outputPath = outputPath
    }

    public var inputDir: String {
        URL(fileURLWithPath: inputPattern).deletingLastPathComponent().path
    }
    public var sequenceName: String {
        URL(fileURLWithPath: inputDir).lastPathComponent
    }
    public var exportPath: String {
        URL(fileURLWithPath: inputDir).deletingLastPathComponent().path
    }
    public var outputFilename: String {
        URL(fileURLWithPath: outputPath).lastPathComponent
    }
}

/// Finds running ffmpeg processes that match the shape of a render this
/// daemon would launch.  Keyed off two characteristics in the command line:
/// 1. an `-i <pattern>` arg whose pattern contains `%` (image2 demuxer glob), and
/// 2. a trailing argument ending in `.mov` / `.mp4` / `.mkv` / `.avi`.
public enum OrphanScanner {

    public static func scan(logger: DaemonLogger) async -> [OrphanedRender] {
        // -axww: all processes, all users, no width truncation.  Without -ww
        // macOS truncates command lines around 132 chars; with our metadata
        // arg explosion we'd see only the first few bytes.
        let runner = ProcessRunner(
            executable: "/bin/ps",
            arguments: ["-axww", "-o", "pid=,command="]
        )
        let collector = LineCollector()
        do {
            _ = try await runner.run { line in collector.append(line) }
        } catch {
            logger.warn("orphan scan: could not run /bin/ps: \(error)")
            return []
        }
        return collector.snapshot().compactMap { Self.parse(line: $0) }
    }

    /// Parse one `ps` output line. Public for testing.
    public static func parse(line: String) -> OrphanedRender? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard let firstSpace = trimmed.firstIndex(of: " ") else { return nil }
        let pidPart = String(trimmed[..<firstSpace])
        guard let pid = pid_t(pidPart) else { return nil }
        let command = String(trimmed[trimmed.index(after: firstSpace)...])
            .trimmingCharacters(in: .whitespaces)

        // Quick reject: must look like an ffmpeg invocation.
        guard command.contains("ffmpeg") else { return nil }

        // Pull the value after " -i ".
        guard let iRange = command.range(of: " -i ") else { return nil }
        let afterI = command[iRange.upperBound...]
        guard let endOfPattern = afterI.firstIndex(of: " ") else { return nil }
        let inputPattern = String(afterI[..<endOfPattern])
        // image2 demuxer pattern: must contain a % (e.g. %05d).
        guard inputPattern.contains("%") else { return nil }

        // Walk tokens from the end looking for a video output filename.
        let videoExts = [".mov", ".mp4", ".mkv", ".avi", ".m4v"]
        var outputPath: String?
        for token in command.split(separator: " ").reversed() {
            let s = String(token)
            if videoExts.contains(where: { s.hasSuffix($0) }) {
                outputPath = s
                break
            }
        }
        guard let output = outputPath else { return nil }

        return OrphanedRender(pid: pid, inputPattern: inputPattern, outputPath: output)
    }
}

private final class LineCollector: @unchecked Sendable {
    private let lock = NSLock()
    private var lines: [String] = []
    func append(_ line: String) { lock.lock(); lines.append(line); lock.unlock() }
    func snapshot() -> [String] {
        lock.lock(); defer { lock.unlock() }
        return lines
    }
}
