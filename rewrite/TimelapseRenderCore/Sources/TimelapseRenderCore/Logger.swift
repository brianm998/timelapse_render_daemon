import Foundation

/// How bad a message is.  The plain `log` / `timeLog` channel has no notion of
/// severity, which is why problems used to be indistinguishable from progress
/// chatter and got dropped by loggers that only cared about the latter.
public enum LogSeverity: Int, Sendable, Comparable {
    case info = 0
    case warning = 1
    case error = 2

    public static func < (a: LogSeverity, b: LogSeverity) -> Bool {
        a.rawValue < b.rawValue
    }
}

public protocol DaemonLogger: Sendable {
    /// `value` controls sort order in the updateable log; lower comes first.
    func log(name: String, message: String, value: Double)
    func timeLog(name: String, message: String, value: Double)
    /// Reports something that went wrong.  `message` may be several lines.
    /// The default implementation folds it back into `log` so existing
    /// loggers keep working; loggers that can render a multi-line block
    /// (console, file) override it.
    func problem(_ severity: LogSeverity, name: String, message: String)
}

public extension DaemonLogger {
    func problem(_ severity: LogSeverity, name: String, message: String) {
        log(name: name, message: message, value: severity == .error ? -2 : -1)
    }
    func info(_ message: String) { log(name: "info", message: message, value: 0) }
    func warn(_ message: String) { problem(.warning, name: "warn", message: message) }
    func error(_ message: String) { problem(.error, name: "error", message: message) }
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

/// A logger that drops every message.  Used as a fallback when no other sink
/// can be opened, and by tests.
public struct SilentLogger: DaemonLogger {
    public init() {}
    public func log(name: String, message: String, value: Double) {}
    public func timeLog(name: String, message: String, value: Double) {}
    public func problem(_ severity: LogSeverity, name: String, message: String) {}
}

/// Writes problems to stderr, preserving line breaks, so a GUI launched from a
/// terminal still reports failures where the user can see them.  Routine
/// progress chatter is dropped unless `verbose` is set — the point of this
/// logger is that anything it prints is worth reading.
public struct ConsoleLogger: DaemonLogger {

    public let verbose: Bool
    public let minimumSeverity: LogSeverity
    public let useColor: Bool

    public init(
        verbose: Bool = false,
        minimumSeverity: LogSeverity = .warning,
        useColor: Bool = ConsoleLogger.stderrIsTerminal
    ) {
        self.verbose = verbose
        self.minimumSeverity = minimumSeverity
        self.useColor = useColor
    }

    public func log(name: String, message: String, value: Double) {
        guard verbose else { return }
        write(stamped(message))
    }

    public func timeLog(name: String, message: String, value: Double) {
        guard verbose else { return }
        write(stamped(message))
    }

    public func problem(_ severity: LogSeverity, name: String, message: String) {
        guard severity >= minimumSeverity else { return }
        let tint = useColor ? (severity == .error ? AnsiColor.red : AnsiColor.yellow) : ""
        let reset = useColor ? AnsiColor.reset : ""
        let tag = severity == .error ? "ERROR" : "WARN"
        // Indent continuation lines so a multi-line report reads as one block.
        let lines = message.split(separator: "\n", omittingEmptySubsequences: false)
        var out = "\(tint)[\(Self.timestamp())] \(tag): \(lines.first ?? "")\(reset)\n"
        for line in lines.dropFirst() {
            out += "\(tint)  \(line)\(reset)\n"
        }
        write(out)
    }

    private func stamped(_ message: String) -> String {
        "[\(Self.timestamp())] \(message)\n"
    }

    private func write(_ s: String) {
        FileHandle.standardError.write(Data(s.utf8))
    }

    private static func timestamp() -> String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f.string(from: Date())
    }

    public static var stderrIsTerminal: Bool { isatty(2) == 1 }
}

/// Fans every message out to several loggers.  Lets the GUI keep its full
/// on-disk log while also putting failures on the console.
public struct TeeLogger: DaemonLogger {

    private let loggers: [any DaemonLogger]

    public init(_ loggers: [any DaemonLogger]) {
        self.loggers = loggers
    }

    public func log(name: String, message: String, value: Double) {
        for l in loggers { l.log(name: name, message: message, value: value) }
    }

    public func timeLog(name: String, message: String, value: Double) {
        for l in loggers { l.timeLog(name: name, message: message, value: value) }
    }

    public func problem(_ severity: LogSeverity, name: String, message: String) {
        for l in loggers { l.problem(severity, name: name, message: message) }
    }
}
