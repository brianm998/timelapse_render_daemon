import Foundation

/// Async wrapper around `Foundation.Process`.  Runs a command, streams its
/// merged stdout+stderr as lines via an `AsyncStream`, and reports the final
/// exit status when the process terminates.
///
/// Cross-platform: Foundation.Process is available on macOS, Linux, and Windows
/// (Swift 5.9+).
public struct ProcessRunner: Sendable {

    public struct Output: Sendable {
        public let exitCode: Int32
        public let terminatedNormally: Bool
    }

    public enum RunError: Error, CustomStringConvertible {
        case launchFailed(String)
        case cancelled
        public var description: String {
            switch self {
            case .launchFailed(let s): return "launch failed: \(s)"
            case .cancelled: return "cancelled"
            }
        }
    }

    public let executable: String
    public let arguments: [String]
    public let environment: [String: String]?
    public let workingDirectory: String?

    public init(
        executable: String,
        arguments: [String],
        environment: [String: String]? = nil,
        workingDirectory: String? = nil
    ) {
        self.executable = executable
        self.arguments = arguments
        self.environment = environment
        self.workingDirectory = workingDirectory
    }

    /// Run the process. `onLaunched` is called once with the child's PID
    /// after the OS hands it back; callers can save the pid to send signals
    /// (SIGSTOP / SIGCONT for pause / resume).  `onLine` is called for each
    /// output line as it arrives.  Returns the final exit status.  Throws if
    /// launch fails or the parent task is cancelled.
    public func run(
        onLaunched: @escaping @Sendable (pid_t) -> Void = { _ in },
        onLine: @escaping @Sendable (String) -> Void
    ) async throws -> Output {
        let process = Process()
        if let exe = Self.resolveExecutable(executable) {
            process.executableURL = URL(fileURLWithPath: exe)
        } else {
            // fall back to letting Foundation try; on Windows it may search PATH
            process.executableURL = URL(fileURLWithPath: executable)
        }
        process.arguments = arguments
        if let env = environment { process.environment = env }
        if let wd = workingDirectory {
            process.currentDirectoryURL = URL(fileURLWithPath: wd)
        }

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        // Detach the child's stdin from any inherited terminal.  Without this,
        // ffmpeg (and other tools that read from stdin) may try to read from
        // the parent's tty, get SIGTTIN because they're in a different process
        // group, and end up stopped (state "T" in ps) — looking like they're
        // running but using 0% CPU and producing no output.
        if let nullDevice = FileHandle(forReadingAtPath: "/dev/null") {
            process.standardInput = nullDevice
        }

        // Reader runs in REAL TIME — drives live progress updates in the GUI.
        // Uses FileHandle.readabilityHandler so the read happens on
        // Foundation's internal dispatch source (notified by the kernel when
        // data arrives), not on a Task that blocks a Swift concurrency thread.
        //
        // The previous implementation used Task.detached + read(upToCount:),
        // which appeared to coalesce all incoming data and only deliver lines
        // when the pipe hit EOF — manifesting as "ffmpeg is rendering but the
        // GUI shows 0/N forever".
        let readerHandle = pipe.fileHandleForReading
        let lineBuffer = StreamingLineBuffer(onLine: onLine)
        let readerDone = ReaderDoneSignal()
        readerHandle.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                // EOF — the child closed its write-end and there's no more
                // data coming.  Flush any trailing partial line and stop.
                handle.readabilityHandler = nil
                lineBuffer.flush()
                readerDone.signal()
                return
            }
            lineBuffer.append(data: data)
        }

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Output, Error>) in
                // CRITICAL: install the termination handler BEFORE process.run().
                // If we set it after, a process that exits in microseconds
                // (missing input file, unmounted volume, etc.) will have its
                // exit reported by the kernel before our handler is in place,
                // the handler is silently dropped, and the continuation hangs
                // forever — leaving the render stuck "running" with no real
                // ffmpeg behind it.
                let didResume = ResumeOnceFlag()
                process.terminationHandler = { proc in
                    let exitCode = proc.terminationStatus
                    let normal = proc.terminationReason == .exit
                    Task {
                        // Wait for the reader to see EOF (after the child
                        // closed its write-end on exit) before reporting.
                        await readerDone.wait()
                        if didResume.takeOwnership() {
                            cont.resume(returning: Output(
                                exitCode: exitCode,
                                terminatedNormally: normal
                            ))
                        }
                    }
                }

                do {
                    try process.run()
                    onLaunched(process.processIdentifier)
                } catch {
                    process.terminationHandler = nil
                    // No child means nobody will ever write to this pipe.
                    // Close the write-end so the reader sees EOF and stops.
                    try? pipe.fileHandleForWriting.close()
                    Task {
                        await readerDone.wait()
                        if didResume.takeOwnership() {
                            cont.resume(throwing: RunError.launchFailed("\(error)"))
                        }
                    }
                }
            }
        } onCancel: {
            if process.isRunning { process.terminate() }
        }
    }

    /// Resolve a bare executable name against PATH if necessary.
    private static func resolveExecutable(_ name: String) -> String? {
        if name.contains("/") || name.contains("\\") {
            return name
        }
        guard let pathEnv = ProcessInfo.processInfo.environment["PATH"] else { return nil }
        #if os(Windows)
        let separator: Character = ";"
        let exts = [".exe", ".cmd", ".bat", ""]
        #else
        let separator: Character = ":"
        let exts = [""]
        #endif
        for dir in pathEnv.split(separator: separator) {
            for ext in exts {
                let candidate = String(dir) + "/" + name + ext
                if FileManager.default.isExecutableFile(atPath: candidate) {
                    return candidate
                }
            }
        }
        return nil
    }
}

/// One-shot flag used to ensure a CheckedContinuation is resumed exactly once
/// even if both the launch-error path and the termination handler fire.
private final class ResumeOnceFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var taken = false
    func takeOwnership() -> Bool {
        lock.lock(); defer { lock.unlock() }
        if taken { return false }
        taken = true
        return true
    }
}

/// Accumulates bytes from a pipe and emits whole lines to an `onLine`
/// callback.  Splits on '\n' or '\r' so ffmpeg progress lines (which end in
/// '\r' to overwrite the previous line) are reported individually.
/// Thread-safe.
private final class StreamingLineBuffer: @unchecked Sendable {
    private let lock = NSLock()
    private var buffer = Data()
    private let onLine: @Sendable (String) -> Void

    init(onLine: @escaping @Sendable (String) -> Void) {
        self.onLine = onLine
    }

    func append(data: Data) {
        var emitted: [String] = []
        lock.lock()
        buffer.append(data)
        while let idx = buffer.firstIndex(where: { $0 == 0x0A || $0 == 0x0D }) {
            let lineData = buffer.subdata(in: 0..<idx)
            buffer.removeSubrange(0...idx)
            if !lineData.isEmpty,
               let line = String(data: lineData, encoding: .utf8),
               !line.isEmpty {
                emitted.append(line)
            }
        }
        lock.unlock()
        // Call onLine OUTSIDE the lock so the callback can re-enter buffer
        // operations (or any other lock) without deadlocking.
        for line in emitted { onLine(line) }
    }

    func flush() {
        var trailing: String?
        lock.lock()
        if !buffer.isEmpty,
           let line = String(data: buffer, encoding: .utf8),
           !line.isEmpty {
            trailing = line
        }
        buffer.removeAll()
        lock.unlock()
        if let trailing { onLine(trailing) }
    }
}

/// One-shot async signal: callers `await wait()`; when `signal()` is called,
/// the awaiter resumes.  Used to coordinate "reader has seen EOF" with the
/// continuation that reports the process's exit status.
private final class ReaderDoneSignal: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false
    private var continuations: [CheckedContinuation<Void, Never>] = []

    func signal() {
        lock.lock()
        done = true
        let waiters = continuations
        continuations.removeAll()
        lock.unlock()
        for c in waiters { c.resume() }
    }

    func wait() async {
        await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
            lock.lock()
            if done {
                lock.unlock()
                cont.resume()
            } else {
                continuations.append(cont)
                lock.unlock()
            }
        }
    }
}

public enum ToolValidator {
    public static func isAvailable(_ executable: String) -> Bool {
        if executable.contains("/") || executable.contains("\\") {
            return FileManager.default.isExecutableFile(atPath: executable)
        }
        guard let pathEnv = ProcessInfo.processInfo.environment["PATH"] else { return false }
        #if os(Windows)
        let separator: Character = ";"
        let exts = [".exe", ".cmd", ".bat", ""]
        #else
        let separator: Character = ":"
        let exts = [""]
        #endif
        for dir in pathEnv.split(separator: separator) {
            for ext in exts {
                let candidate = String(dir) + "/" + executable + ext
                if FileManager.default.isExecutableFile(atPath: candidate) {
                    return true
                }
            }
        }
        return false
    }
}
