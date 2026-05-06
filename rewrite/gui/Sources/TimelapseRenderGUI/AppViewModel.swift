import Foundation
import Observation
import SwiftUI
import TimelapseRenderCore

/// View-model that hosts the running daemon and republishes its state for SwiftUI.
@Observable
final class AppViewModel {

    var configPath: String?
    var config: DaemonConfig?
    var snapshot: DaemonStateSnapshot = DaemonStateSnapshot()
    var isRunning: Bool = false
    var lastError: String?

    private var daemon: RenderDaemon?
    private var runTask: Task<Void, Never>?
    private var observerTask: Task<Void, Never>?

    /// True until `bootstrap()` has run for the first time.
    private var didBootstrap: Bool = false

    init() {
        // Init does no I/O and no @Observable mutations: it runs during
        // @State setup, before the WindowGroup has a window, and any work
        // here can entangle with SwiftUI's launch flow.
    }

    /// Called from a SwiftUI view's `.task` modifier once the window is up.
    /// Idempotent: repeated calls are no-ops.
    @MainActor
    func bootstrap() {
        guard !didBootstrap else { return }
        didBootstrap = true

        // Resolution order for the startup config:
        //   1. -c <path> / --config <path> on the command line (autostart).
        //   2. The path last opened by the user (persisted in UserDefaults).
        //   3. ./config.json relative to the current working directory.
        if let cliPath = Self.configPathFromCommandLine() {
            loadConfig(at: cliPath)
            if config != nil { start() }
            return
        }
        if let lastPath = UserPreferences.defaults.string(forKey: UserPreferences.Key.lastConfigPath),
           FileManager.default.fileExists(atPath: lastPath) {
            loadConfig(at: lastPath)
            return
        }
        let cwd = FileManager.default.currentDirectoryPath
        let candidate = "\(cwd)/config.json"
        if FileManager.default.fileExists(atPath: candidate) {
            loadConfig(at: candidate)
        }
    }

    /// Look for `-c <path>`, `--config <path>`, `-c=<path>`, or
    /// `--config=<path>` in the process arguments and return the resolved
    /// path.  Bare positional arguments are NOT supported because AppKit
    /// intercepts them as document-open events and that suppresses the
    /// WindowGroup's primary window from being created.
    private static func configPathFromCommandLine() -> String? {
        let args = Array(CommandLine.arguments.dropFirst())
        var i = 0
        while i < args.count {
            let arg = args[i]
            // -c=PATH or --config=PATH
            for prefix in ["--config=", "-c="] {
                if arg.hasPrefix(prefix) {
                    return resolve(String(arg.dropFirst(prefix.count)))
                }
            }
            // -c PATH or --config PATH
            if arg == "-c" || arg == "--config" {
                if i + 1 < args.count {
                    return resolve(args[i + 1])
                }
                return nil
            }
            i += 1
        }
        return nil
    }

    private static func resolve(_ s: String) -> String {
        if s.hasPrefix("/") { return s }
        return FileManager.default.currentDirectoryPath + "/" + s
    }

    func pickConfig() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            loadConfig(at: url.path)
        }
    }

    func loadConfig(at path: String) {
        do {
            let cfg = try ConfigLoader.load(at: path)
            self.configPath = path
            self.config = cfg
            // Remember this path so a future restart can re-open it without
            // a CLI flag or Open dialog.
            UserPreferences.defaults.set(path, forKey: UserPreferences.Key.lastConfigPath)
        } catch {
            self.lastError = "could not load \(path): \(error)"
        }
    }

    func saveConfig() {
        guard let configPath, let config else { return }
        do {
            try ConfigLoader.write(config, to: configPath)
            // Tell the running daemon, if any, to pick up the new config.
            if let daemon {
                Task { await daemon.updateConfig(config, persist: false) }
            }
        } catch {
            self.lastError = "could not save \(configPath): \(error)"
        }
    }

    func toggleRunning() {
        if isRunning { stop() } else { start() }
    }

    func start() {
        guard let configPath else { return }
        do {
            // Log to ~/Library/Logs/TimelapseRenderDaemon/daemon.log so the
            // user can inspect ffmpeg failures, scan errors, etc. without us
            // spamming stdout.  Falls back to SilentLogger if the file can't
            // be opened (e.g. the directory isn't writable).
            let logger: DaemonLogger
            do {
                logger = try FileLogger(url: FileLogger.defaultURL())
                self.lastError = nil
            } catch {
                logger = SilentLogger()
                self.lastError = "could not open log file: \(error)"
            }
            let daemon = try RenderDaemon(configPath: configPath, logger: logger)
            self.daemon = daemon
            self.isRunning = true

            let stream = daemon.stateStream
            self.observerTask = Task { @MainActor [weak self] in
                for await snap in stream {
                    // Defer the @Observable mutation to the next runloop tick.
                    // Setting self.snapshot directly here can land while
                    // NSTableView is mid row-callback (the for-await body
                    // resumes whenever a snapshot arrives), triggering the
                    // "reentrant operation in NSTableView delegate" warning.
                    DispatchQueue.main.async { [weak self] in
                        self?.snapshot = snap
                    }
                }
            }
            let vmRef = self
            self.runTask = Task.detached(priority: .userInitiated) {
                await daemon.run()
                await MainActor.run { vmRef.isRunning = false }
            }
        } catch {
            self.lastError = "could not start daemon: \(error)"
        }
    }

    func stop() {
        runTask?.cancel()
        observerTask?.cancel()
        runTask = nil
        observerTask = nil
        daemon = nil
        isRunning = false
    }

    // MARK: - GUI control of the daemon

    func pauseSequence(_ name: String) {
        guard let daemon else { return }
        Task { await daemon.pauseSequence(name) }
    }

    func resumeSequence(_ name: String) {
        guard let daemon else { return }
        Task { await daemon.resumeSequence(name) }
    }

    func pauseVariant(sequenceName: String, variantKey: String) {
        guard let daemon else { return }
        Task { await daemon.pauseVariant(sequenceName: sequenceName, variantKey: variantKey) }
    }

    func resumeVariant(sequenceName: String, variantKey: String) {
        guard let daemon else { return }
        Task { await daemon.resumeVariant(sequenceName: sequenceName, variantKey: variantKey) }
    }

    // MARK: - Per-render pause / resume / stop

    func pauseRender(outputPath: String) {
        guard let daemon else { return }
        Task { await daemon.pauseRenderJob(outputPath: outputPath) }
    }

    func resumeRender(outputPath: String) {
        guard let daemon else { return }
        Task { await daemon.resumeRenderJob(outputPath: outputPath) }
    }

    func stopRender(outputPath: String) {
        guard let daemon else { return }
        Task { await daemon.stopRenderJob(outputPath: outputPath) }
    }

    func pushConfigToDaemon() {
        guard let daemon, let config else { return }
        Task { await daemon.updateConfig(config, persist: true) }
    }
}

