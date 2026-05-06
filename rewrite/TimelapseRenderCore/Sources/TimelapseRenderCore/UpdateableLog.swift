import Foundation

#if canImport(Glibc)
import Glibc
#elseif canImport(Musl)
import Musl
#elseif canImport(Darwin)
import Darwin
#endif

/// A log that maintains a stable on-screen line per `name`, redrawing in place.
/// Cross-platform: uses ANSI escape codes which work in modern terminals on
/// macOS, Linux, and Windows 10+.
public actor UpdateableLog: DaemonLogger {

    public struct Line: Sendable {
        public var name: String
        public var message: String
        public var primary: Double
        public var secondary: Double
    }

    private var lines: [Line] = []
    private var enableColor: Bool
    private var lastDrawnCount: Int = 0

    public init(enableColor: Bool = true) {
        self.enableColor = enableColor
    }

    public func setColor(_ enabled: Bool) {
        self.enableColor = enabled
    }

    public nonisolated func log(name: String, message: String, value: Double) {
        Task { await self._log(name: name, message: message, primary: value, secondary: 0) }
    }

    public nonisolated func timeLog(name: String, message: String, value: Double) {
        let stamp = Self.timeString()
        let yellow = AnsiColor.yellow
        let reset = AnsiColor.reset
        let prefixed = "\(yellow)\(stamp)\(reset) - \(message)"
        Task {
            await self._log(name: name, message: prefixed, primary: value, secondary: Date().timeIntervalSince1970)
        }
    }

    /// Snapshot of currently displayed lines, suitable for forwarding to
    /// other observers (a GUI status panel, for example).
    public func snapshot() -> [Line] { lines }

    private func _log(name: String, message: String, primary: Double, secondary: Double) {
        let cleaned = message.replacingOccurrences(of: "\n", with: " ")
        if let idx = lines.firstIndex(where: { $0.name == name }) {
            lines[idx] = Line(name: name, message: cleaned, primary: primary, secondary: secondary)
        } else {
            lines.append(Line(name: name, message: cleaned, primary: primary, secondary: secondary))
        }
        lines.sort { a, b in
            if a.primary == b.primary { return a.secondary < b.secondary }
            return a.primary < b.primary
        }
        redraw()
    }

    private func redraw() {
        // Move cursor up over previous output, clear those lines, then re-emit.
        var out = ""
        if lastDrawnCount > 0 {
            out += "\u{1B}[\(lastDrawnCount)A"
        }
        let cols = Self.terminalColumns() ?? 200
        for line in lines {
            // Clear to end of line, write the message, advance.
            let trimmed = Self.truncate(line.message, toVisibleColumns: cols)
            out += "\u{1B}[2K" + trimmed + "\n"
        }
        lastDrawnCount = lines.count
        FileHandle.standardOutput.write(Data(out.utf8))
    }

    /// Truncates a string with ANSI escape codes to fit `cols` visible columns.
    /// Imperfect (treats Unicode width as 1) but good enough for log lines.
    private static func truncate(_ s: String, toVisibleColumns cols: Int) -> String {
        var out = ""
        var visible = 0
        var iter = s.unicodeScalars.makeIterator()
        while let c = iter.next() {
            if c == "\u{1B}" {
                out.unicodeScalars.append(c)
                if let next = iter.next() {
                    out.unicodeScalars.append(next)
                    if next == "[" {
                        // CSI sequence: read until a final byte (0x40-0x7E).
                        while let p = iter.next() {
                            out.unicodeScalars.append(p)
                            if p.value >= 0x40 && p.value <= 0x7E { break }
                        }
                    }
                }
            } else {
                if visible >= cols { break }
                out.unicodeScalars.append(c)
                visible += 1
            }
        }
        return out
    }

    private static func timeString() -> String {
        let f = DateFormatter()
        f.dateFormat = "h:mm:ss a"
        return f.string(from: Date())
    }

    private static func terminalColumns() -> Int? {
        #if os(macOS) || os(Linux)
        var ws = winsize()
        #if os(macOS)
        let req = UInt(TIOCGWINSZ)
        #else
        let req = UInt(TIOCGWINSZ)
        #endif
        if ioctl(1, req, &ws) == 0, ws.ws_col > 0 {
            return Int(ws.ws_col)
        }
        #endif
        if let env = ProcessInfo.processInfo.environment["COLUMNS"], let n = Int(env) { return n }
        return nil
    }
}

public enum AnsiColor {
    public static let reset    = "\u{1B}[0m"
    public static let bold     = "\u{1B}[1m"
    public static let red      = "\u{1B}[31m"
    public static let green    = "\u{1B}[32m"
    public static let yellow   = "\u{1B}[33m"
    public static let blue     = "\u{1B}[34m"
    public static let magenta  = "\u{1B}[35m"
    public static let cyan     = "\u{1B}[36m"
}

public enum ProgressBar {
    public static func render(width: Int, percentage: Double, color: Bool = true) -> String {
        let filled = max(0, min(width, Int(Double(width) * max(0, min(1, percentage)))))
        if !color {
            return "[" + String(repeating: "*", count: filled) + String(repeating: "-", count: width - filled) + "]"
        }
        return AnsiColor.blue + "[" +
            AnsiColor.green + String(repeating: "*", count: filled) +
            AnsiColor.yellow + String(repeating: "-", count: width - filled) +
            AnsiColor.blue + "]" + AnsiColor.reset
    }
}
