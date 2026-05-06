import Foundation

/// A logger that appends all messages to a file on disk.  Use this instead of
/// `SilentLogger` when you want to debug the daemon without console output.
///
/// Thread-safe via an internal `NSLock` (we can't be an actor because the
/// `DaemonLogger` protocol's `log` method is non-isolated to keep the API
/// usable from any context).
public final class FileLogger: DaemonLogger, @unchecked Sendable {

    public let url: URL
    private let handle: FileHandle
    private let lock = NSLock()
    private let formatter: DateFormatter

    public init(url: URL) throws {
        self.url = url

        // Make sure the parent directory exists.
        let dir = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(
            at: dir, withIntermediateDirectories: true
        )

        // Create the file if needed, then open in append mode.
        if !FileManager.default.fileExists(atPath: url.path) {
            FileManager.default.createFile(atPath: url.path, contents: nil)
        }
        self.handle = try FileHandle(forWritingTo: url)
        try self.handle.seekToEnd()

        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm:ss.SSS"
        self.formatter = f

        // Mark the start of this run with a separator so multiple launches
        // are easy to tell apart in the log file.
        write("\n--- session started \(f.string(from: Date())) ---\n")
    }

    deinit {
        try? handle.close()
    }

    public func log(name: String, message: String, value: Double) {
        write(format(name: name, message: message))
    }

    public func timeLog(name: String, message: String, value: Double) {
        write(format(name: name, message: message))
    }

    private func format(name: String, message: String) -> String {
        let stamp = formatter.string(from: Date())
        let cleaned = message.replacingOccurrences(of: "\n", with: " ")
        return "\(stamp) [\(name)] \(cleaned)\n"
    }

    private func write(_ s: String) {
        guard let data = s.data(using: .utf8) else { return }
        lock.lock()
        defer { lock.unlock() }
        try? handle.write(contentsOf: data)
    }

    /// Default log path under the user's `~/Library/Logs/` directory.
    public static func defaultURL() -> URL {
        let fm = FileManager.default
        if let logs = fm.urls(for: .libraryDirectory, in: .userDomainMask).first {
            return logs
                .appendingPathComponent("Logs")
                .appendingPathComponent("TimelapseRenderDaemon")
                .appendingPathComponent("daemon.log")
        }
        return URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("timelapse-render-daemon.log")
    }
}
