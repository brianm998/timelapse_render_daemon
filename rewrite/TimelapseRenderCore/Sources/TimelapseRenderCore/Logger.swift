import Foundation

public protocol DaemonLogger: Sendable {
    /// `value` controls sort order in the updateable log; lower comes first.
    func log(name: String, message: String, value: Double)
    func timeLog(name: String, message: String, value: Double)
}

public extension DaemonLogger {
    func info(_ message: String) { log(name: "info", message: message, value: 0) }
    func warn(_ message: String) { log(name: "warn", message: message, value: -1) }
    func error(_ message: String) { log(name: "error", message: message, value: -2) }
}

/// A simple logger that writes a single line per call to stdout.
public struct PlainLogger: DaemonLogger {
    public init() {}
    public func log(name: String, message: String, value: Double) {
        FileHandle.standardOutput.write(Data((message + "\n").utf8))
    }
    public func timeLog(name: String, message: String, value: Double) {
        let stamp = ISO8601DateFormatter().string(from: Date())
        FileHandle.standardOutput.write(Data(("\(stamp) - \(message)\n").utf8))
    }
}

/// A logger that drops every message.  Used by the GUI, which surfaces state
/// through the daemon's snapshot stream rather than stdout.
public struct SilentLogger: DaemonLogger {
    public init() {}
    public func log(name: String, message: String, value: Double) {}
    public func timeLog(name: String, message: String, value: Double) {}
}
