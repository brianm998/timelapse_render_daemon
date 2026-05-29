import Foundation

/// The main orchestrator. Cross-platform; no UI dependencies.
///
/// The daemon:
///   * loads the config and watches it for changes (mtime poll).
///   * scans source/export paths concurrently and quickly,
///   * launches ffmpeg / exiftool tasks subject to per-type concurrency caps,
///   * exposes a `stateStream` for GUI / CLI consumption,
///   * supports pausing whole sequences or specific render variants.
public actor RenderDaemon {

    // Configuration
    private var configPath: String
    private var config: DaemonConfig
    private var codecs: VideoCodecs
    private var configMTime: TimeInterval = 0

    // Collaborators
    private let scanner: SequenceScanner
    private let logger: DaemonLogger

    // State
    private var pausedSequenceNames: Set<String> = []
    private var pausedVariants: Set<PausedVariant> = []
    private var renders: [String: ActiveRender] = [:]    // outputPath -> render
    private var skippedExports: Set<String> = []         // export full paths that finished
    private var completedRenderResults: [CompletedRenderResult] = []
    private let maxCompletedRenderResults = 200
    /// Sequences whose configured variants are all already rendered.  Set by
    /// processDiscovered; surfaced to the GUI via SequenceState.allVariantsRendered.
    private var fullyRenderedSequences: Set<String> = []   // exportPath/dirname
    private var exifKnownSet: Set<String> = []           // exif json files already produced
    private var exifInFlight: Set<String> = []           // exif jobs running for this raw sequence
    private var lastSnapshot: DaemonStateSnapshot
    private var phase: DaemonStateSnapshot.Phase = .startingUp
    private var stateContinuation: AsyncStream<DaemonStateSnapshot>.Continuation?

    /// AsyncStream of snapshots, every time state changes.
    public nonisolated let stateStream: AsyncStream<DaemonStateSnapshot>

    private struct ActiveRender {
        let sequenceName: String
        let exportPath: String
        let outputFilename: String
        let outputPath: String
        let variantKey: String
        let totalFrames: Int
        /// Set once the render is promoted from `.queued` to `.running`.
        /// `nil` while the render is still queued, or for adopted orphans
        /// we don't manage as a Task.
        var task: Task<Void, Never>?
        /// PID of the running ffmpeg child, set when the task launches the
        /// process (or filled in for adopted renders).  Used by pause /
        /// resume / stop to send signals.
        var pid: pid_t?
        /// Build plan for ffmpeg.  `nil` for adopted renders since we can't
        /// reconstruct the original plan from a running process.
        let plan: FFmpegRunner.Plan?
        /// Path to the EXIF histogram sidecar JSON, used after the render
        /// completes to embed a canonical XMP-EXIF packet into the video.
        /// `nil` for adopted renders.
        let sidecarJSONPath: String?
        var frame: Int
        var fps: Double
        var status: RenderJobState.Status
        var message: String?
        /// True for orphans we adopted at startup.
        var isAdopted: Bool = false
    }

    // MARK: - Init

    /// Where the daemon persists the render-history file.  Resolved at init
    /// from (in priority order): explicit `historyURL` argument, the config's
    /// `render_history_path`, or `nil` (no persistence).
    private let historyURL: URL?

    public init(
        configPath: String,
        codecsPath: String? = nil,
        historyURL: URL? = nil,
        logger: DaemonLogger = PlainLogger()
    ) throws {
        self.configPath = configPath
        self.config = try ConfigLoader.load(at: configPath)
        if let attrs = try? FileManager.default.attributesOfItem(atPath: configPath),
           let mod = attrs[.modificationDate] as? Date {
            self.configMTime = mod.timeIntervalSince1970
        }
        if let codecsPath, let codecs = try? ConfigLoader.loadCodecs(at: codecsPath) {
            self.codecs = codecs
        } else {
            self.codecs = ConfigLoader.defaultCodecs
        }
        self.logger = logger
        self.scanner = SequenceScanner()
        self.pausedSequenceNames = Set(self.config.pausedSequenceNames)
        self.pausedVariants = Set(self.config.pausedVariants)

        // Resolve the history file location.
        if let url = historyURL {
            self.historyURL = url
        } else if let p = self.config.renderHistoryPath, !p.isEmpty {
            self.historyURL = URL(fileURLWithPath: p)
        } else {
            self.historyURL = nil
        }
        // Load any existing history, prune entries older than the retention
        // window, and seed `completedRenderResults`.
        if let url = self.historyURL,
           let data = try? Data(contentsOf: url) {
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            if let list = try? decoder.decode([CompletedRenderResult].self, from: data) {
                let cutoff = Date().addingTimeInterval(-Double(self.config.recentRenderHistoryDays) * 86400)
                self.completedRenderResults = list
                    .filter { $0.timestamp > cutoff }
                    .sorted { $0.timestamp > $1.timestamp }
            }
        }

        var continuation: AsyncStream<DaemonStateSnapshot>.Continuation!
        self.stateStream = AsyncStream { c in continuation = c }
        self.stateContinuation = continuation

        self.lastSnapshot = DaemonStateSnapshot(
            phase: .startingUp,
            configPath: configPath,
            completedRenders: self.completedRenderResults,
            pausedSequenceNames: self.pausedSequenceNames,
            pausedVariants: self.pausedVariants
        )
    }

    /// Default history-file location: `~/Library/Application Support/TimelapseRenderDaemon/render_history.json`
    /// on Apple platforms, or the equivalent application-support dir on Linux.
    public static func defaultHistoryURL() -> URL {
        let fm = FileManager.default
        if let appSupport = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first {
            return appSupport
                .appendingPathComponent("TimelapseRenderDaemon")
                .appendingPathComponent("render_history.json")
        }
        return URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("timelapse_render_history.json")
    }

    public func currentConfig() -> DaemonConfig { config }
    public func currentSnapshot() -> DaemonStateSnapshot { lastSnapshot }

    // MARK: - Public control surface (GUI / CLI)

    public func updateConfig(_ newConfig: DaemonConfig, persist: Bool = true) async {
        self.config = newConfig
        if persist {
            do {
                try ConfigLoader.write(newConfig, to: configPath)
                if let attrs = try? FileManager.default.attributesOfItem(atPath: configPath),
                   let mod = attrs[.modificationDate] as? Date {
                    self.configMTime = mod.timeIntervalSince1970
                }
            } catch {
                logger.warn("could not persist config: \(error)")
            }
        }
        self.pausedSequenceNames = Set(newConfig.pausedSequenceNames)
        self.pausedVariants = Set(newConfig.pausedVariants)
        publishSnapshot()
    }

    public func pauseSequence(_ name: String) {
        pausedSequenceNames.insert(name)
        cancelRenders(matching: { $0.sequenceName == name })
        publishSnapshot()
    }

    public func resumeSequence(_ name: String) {
        pausedSequenceNames.remove(name)
        publishSnapshot()
    }

    public func pauseVariant(sequenceName: String, variantKey: String) {
        let v = PausedVariant(sequenceName: sequenceName, variantKey: variantKey)
        pausedVariants.insert(v)
        cancelRenders(matching: { $0.sequenceName == sequenceName && $0.variantKey == variantKey })
        publishSnapshot()
    }

    public func resumeVariant(sequenceName: String, variantKey: String) {
        let v = PausedVariant(sequenceName: sequenceName, variantKey: variantKey)
        pausedVariants.remove(v)
        publishSnapshot()
    }

    // MARK: - Per-render pause / resume / stop
    //
    // These act on individual renders identified by their output path.  Pause
    // suspends the running ffmpeg via SIGSTOP (or just refuses to promote a
    // queued render).  Resume sends SIGCONT (or marks the render queued
    // again).  Stop cancels the render permanently AND adds the variant to
    // the "paused variants" set so it isn't queued again on the next scan.

    public func pauseRenderJob(outputPath: String) {
        guard var r = renders[outputPath] else { return }
        if r.status == .running, let pid = r.pid, pid > 0 {
            _ = kill(pid, SIGSTOP)
            logger.timeLog(name: r.outputFilename,
                           message: "paused (SIGSTOP) \(outputPath)", value: -1)
        }
        r.status = .paused
        renders[outputPath] = r
        publishSnapshot()
    }

    public func resumeRenderJob(outputPath: String) {
        guard var r = renders[outputPath] else { return }
        guard r.status == .paused else { return }
        if let pid = r.pid, pid > 0, r.task != nil {
            _ = kill(pid, SIGCONT)
            r.status = .running
            logger.timeLog(name: r.outputFilename,
                           message: "resumed (SIGCONT) \(outputPath)", value: -1)
        } else {
            // Was paused while still queued — drop back to queued so the
            // promoter picks it up next time a slot frees.
            r.status = .queued
        }
        renders[outputPath] = r
        publishSnapshot()
        promoteQueuedRenders()
    }

    public func stopRenderJob(outputPath: String) {
        guard let r = renders[outputPath] else { return }
        // Mirror the original "pause variant" semantics: persist the variant
        // as paused so the next scan tick won't immediately re-queue it.
        let pv = PausedVariant(sequenceName: r.sequenceName,
                               variantKey: r.variantKey)
        pausedVariants.insert(pv)
        cancelRenders(matching: { $0.outputPath == outputPath })
        publishSnapshot()
    }

    private func cancelRenders(matching predicate: (ActiveRender) -> Bool) {
        for (key, render) in renders where predicate(render) {
            // If suspended, wake the child so SIGTERM can be delivered.
            if render.status == .paused, let pid = render.pid, pid > 0 {
                _ = kill(pid, SIGCONT)
            }
            if render.isAdopted, let pid = render.pid, pid > 0 {
                // No Task to cancel — signal the orphan directly.
                _ = kill(pid, SIGTERM)
            } else {
                render.task?.cancel()
            }
            // Remove the partial output so a future un-pause of the variant
            // doesn't see a half-written file and skip the render.
            try? FileManager.default.removeItem(atPath: render.outputPath)
            var copy = render
            copy.status = .cancelled
            copy.message = "cancelled by user"
            renders[key] = copy
        }
    }

    private func recordPID(outputPath: String, pid: pid_t) {
        guard var r = renders[outputPath] else { return }
        r.pid = pid
        renders[outputPath] = r
    }

    // MARK: - Orphan adoption

    /// Walks `ps` for ffmpeg processes that look like our renders and adopts
    /// any whose output paths we don't already track.  Called once at the
    /// start of `run()`.
    private func adoptOrphanedFFmpegProcesses() async {
        let orphans = await OrphanScanner.scan(logger: logger)
        for orphan in orphans {
            // Already managing this output (managed render from this session)?
            if renders[orphan.outputPath] != nil { continue }
            let render = ActiveRender(
                sequenceName: orphan.sequenceName,
                exportPath: orphan.exportPath,
                outputFilename: orphan.outputFilename,
                outputPath: orphan.outputPath,
                variantKey: "adopted",
                totalFrames: 0,
                task: nil,
                pid: orphan.pid,
                plan: nil,
                sidecarJSONPath: nil,
                frame: 0,
                fps: 0,
                status: .running,
                message: "adopted from previous session",
                isAdopted: true
            )
            renders[orphan.outputPath] = render
            logger.timeLog(
                name: orphan.outputFilename,
                message: "adopted ffmpeg pid=\(orphan.pid) → \(orphan.outputPath)",
                value: -1
            )
        }
    }

    /// Checks each adopted render's PID; transitions to .finished or .failed
    /// once the orphan exits.  Cheap (one `kill(pid, 0)` per adopted entry).
    private func pollAdoptedRenders() {
        var changed = false
        for (key, render) in renders where render.isAdopted {
            guard let pid = render.pid else { continue }
            if Self.processExists(pid: pid) { continue }
            let success = FileManager.default.fileExists(atPath: render.outputPath)
            var copy = render
            copy.status = success ? .finished : .failed
            copy.message = success
                ? "adopted ffmpeg finished"
                : "adopted ffmpeg ended with no output file"
            renders[key] = copy
            if success {
                // Add to the persistent history just like a managed render.
                recordCompletedRender(render)
            }
            logger.timeLog(
                name: render.outputFilename,
                message: success
                    ? "adopted render finished: \(render.outputPath)"
                    : "adopted render ended without output: \(render.outputPath)",
                value: -1
            )
            changed = true
        }
        if changed { publishSnapshot() }
    }

    private static func processExists(pid: pid_t) -> Bool {
        // kill(pid, 0) doesn't actually signal — it just checks if we could.
        // 0 == still there; ESRCH == gone; EPERM == there but not ours.
        if kill(pid, 0) == 0 { return true }
        return errno != ESRCH
    }

    // MARK: - Run loop

    /// Runs forever (or until the parent task is cancelled).
    public func run() async {
        // Validate external tools but don't crash if missing - log a warning so
        // the GUI can surface it to the user.
        if !ToolValidator.isAvailable("ffmpeg") {
            logger.warn("ffmpeg not found in PATH; renders will fail until installed")
        }
        if !ToolValidator.isAvailable("exiftool") {
            logger.warn("exiftool not found in PATH; exif extraction will fail until installed")
        }

        // Kick off backup loading in the background. This is what makes the
        // original Perl daemon slow to start - we don't block on it.
        Task.detached(priority: .background) { [config, scanner, logger] in
            await scanner.loadBackupSequenceCountsLazily(
                backupPaths: config.expandedBackupPaths,
                logger: logger,
                onEntry: { [scanner] parent, name, count in
                    Task { await scanner.mergeBackupEntry(parentPath: parent, name: name, count: count) }
                }
            )
            logger.info("backup path scan complete")
        }

        // Pick up any ffmpeg processes that survived a previous TimelapseRenderGUI
        // session (process killed mid-render).  We can't read their stdout/stderr,
        // but we know the PID and can pause / resume / stop them via signals.
        await adoptOrphanedFFmpegProcesses()

        phase = .scanning
        publishSnapshot()

        while !Task.isCancelled {
            await reloadConfigIfChanged()
            pollAdoptedRenders()
            await scanner.refreshRawSequenceCounts(config: config, logger: logger)
            let exports = await scanner.discoverExportSequences(config: config, logger: logger)
            let raws = await scanner.currentRawSequences()

            await processDiscovered(sequences: exports)
            cleanupFinishedRenders()
            publishSnapshot(rawSequenceCount: raws.count, sequences: exports)

            // Sleep only briefly. The user complained the original took a long
            // time to notice changes; the default interval here is 2s.
            let sleep = max(0.1, config.scanIntervalSeconds)
            try? await Task.sleep(nanoseconds: UInt64(sleep * 1_000_000_000))
        }
        phase = .stopped
        publishSnapshot()
        stateContinuation?.finish()
    }

    // MARK: - Config reload

    private func reloadConfigIfChanged() async {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: configPath),
              let mod = attrs[.modificationDate] as? Date else { return }
        let mtime = mod.timeIntervalSince1970
        if mtime > configMTime {
            do {
                let new = try ConfigLoader.load(at: configPath)
                self.config = new
                self.configMTime = mtime
                self.pausedSequenceNames.formUnion(Set(new.pausedSequenceNames))
                self.pausedVariants.formUnion(Set(new.pausedVariants))
                logger.info("reloaded config")
            } catch {
                logger.warn("could not reload config: \(error)")
            }
        }
    }

    // MARK: - Per-tick logic

    private func processDiscovered(sequences: [ExportSequence]) async {
        let progressBarColor = config.logInColor
        for seq in sequences {
            if pausedSequenceNames.contains(seq.dirname) { continue }
            if !seq.isComplete {
                if seq.rawSequenceLength > 0 && seq.generatedImageCount > 0
                   && !skippedExports.contains(seq.fullPath) {
                    let bar = ProgressBar.render(
                        width: config.progressBarWidth,
                        percentage: seq.progress,
                        color: progressBarColor
                    )
                    let frames = seq.rawSequenceLength - seq.generatedImageCount
                    logger.log(
                        name: seq.dirname,
                        message: "\(bar) waiting for \(frames) more frames (\(seq.generatedImageCount)/\(seq.rawSequenceLength)) in \(seq.fullPath)",
                        value: seq.progress
                    )
                }
                continue
            }

            // The sequence is complete; figure out which variants need rendering.
            let allFilenames = config.rendering.map {
                outputVideoFilename(for: $0, sequenceName: seq.dirname)
            }
            // edge case: every variant already exists -> log and skip
            let variantsToRender = config.rendering.filter { renderType in
                let filename = outputVideoFilename(for: renderType, sequenceName: seq.dirname)
                return shouldRender(outputFilename: filename, exportPath: seq.exportPath)
                    && !pausedVariants.contains(PausedVariant(sequenceName: seq.dirname, variantKey: renderType.variantKey))
            }
            if variantsToRender.isEmpty {
                fullyRenderedSequences.insert(seq.fullPath)
                if !skippedExports.contains(seq.fullPath) {
                    logger.timeLog(
                        name: seq.dirname,
                        message: "all \(allFilenames.count) videos already rendered for \(seq.fullPath)",
                        value: -1
                    )
                    skippedExports.insert(seq.fullPath)
                }
                continue
            } else {
                fullyRenderedSequences.remove(seq.fullPath)
            }

            // Build / read exif metadata; once available, kick off all renders.
            let exifJsonFilename = "\(seq.rawSequenceParent)/\(seq.rawSequenceName).json"
            let metadata: [String: String]? = {
                if FileManager.default.fileExists(atPath: exifJsonFilename) {
                    return readExifMetadata(at: exifJsonFilename)
                }
                return nil
            }()

            if metadata == nil {
                await maybeStartExifJob(
                    rawSequenceParent: seq.rawSequenceParent,
                    rawSequenceName: seq.rawSequenceName,
                    exifJsonFilename: exifJsonFilename
                )
                continue
            }

            // Queue every variant up front so the user can see all pending
            // renders in the UI (and pause individual ones).  The actual
            // ffmpeg processes get started later by promoteQueuedRenders(),
            // subject to maxConcurrentRenders / maxProcesses.ffmpeg limits.
            for renderType in variantsToRender {
                await queueRender(for: renderType, sequence: seq, metadata: metadata!)
            }
        }

        promoteQueuedRenders()
    }

    private func currentRunningCount() -> Int {
        renders.values.filter { $0.status == .running }.count
    }

    private func currentRunningForSequence(_ name: String) -> Int {
        renders.values.filter { $0.sequenceName == name && $0.status == .running }.count
    }

    private func cleanupFinishedRenders() {
        // Drop renders in terminal states that we no longer need to display.
        renders = renders.filter { _, r in
            switch r.status {
            case .running, .queued, .paused:
                return true
            default:
                return false
            }
        }
    }

    // MARK: - Render execution

    /// Adds a render to the renders dict in `.queued` state.  The actual
    /// ffmpeg process is launched later by `promoteQueuedRenders()`.
    private func queueRender(
        for renderType: RenderType,
        sequence: ExportSequence,
        metadata: [String: String]
    ) async {
        let outputFilename = outputVideoFilename(for: renderType, sequenceName: sequence.dirname)
        let outputPath = "\(sequence.exportPath)/\(outputFilename)"
        if renders[outputPath] != nil { return }   // already queued or running
        guard let codec = codecs[renderType.codec] else {
            logger.warn("unknown codec '\(renderType.codec)' in render config")
            return
        }
        let firstImage = firstImageInSequence(at: sequence.fullPath)
        guard let firstImage else {
            logger.warn("no images in \(sequence.fullPath); cannot queue render")
            return
        }
        // The imageExtension passed to ffmpeg is "everything after the first
        // run of digits in the filename" — including any decoration star
        // adds before the dot extension, e.g. "-severe-noise.tiff".  Mirrors
        // the Perl daemon's `if ($test_image =~ /\d+(.*)$/) { $image_type = $1 }`.
        let imageExtension = Self.imageSuffixAfterDigits(
            in: firstImage,
            fallbackExtension: ".tif"
        )

        // Determine the scale dimensions for this render.
        // Width-only renders probe the first source image so the height
        // inherits the source's aspect ratio rather than assuming 16:9.
        let finalWidth = renderType.width
        var finalHeight = renderType.height

        if let w = renderType.width, renderType.height == nil {
            let imagePath = "\(sequence.fullPath)/\(firstImage)"
            if let dims = await probeImageDimensions(at: imagePath) {
                let h = Int((Double(w) * Double(dims.height) / Double(dims.width)).rounded())
                finalHeight = h.isMultiple(of: 2) ? h : h + 1
            }
        }

        let useResize = finalWidth != nil && finalHeight != nil
        let aspectW = finalWidth ?? 16
        let aspectH = finalHeight ?? 9
        let aspect = AspectRatio.string(width: aspectW, height: aspectH)

        let plan = FFmpegRunner.Plan(
            inputDir: sequence.fullPath,
            imageNamePrefix: config.imageNamePrefix,
            imageExtension: imageExtension,
            outputPath: outputPath,
            frameRate: renderType.frameRate,
            aspect: aspect,
            width: useResize ? finalWidth : nil,
            height: useResize ? finalHeight : nil,
            codec: codec,
            metadata: metadata,
            totalFrames: sequence.rawSequenceLength
        )

        renders[outputPath] = ActiveRender(
            sequenceName: sequence.dirname,
            exportPath: sequence.exportPath,
            outputFilename: outputFilename,
            outputPath: outputPath,
            variantKey: renderType.variantKey,
            totalFrames: sequence.rawSequenceLength,
            task: nil,
            pid: nil,
            plan: plan,
            sidecarJSONPath: "\(sequence.rawSequenceParent)/\(sequence.rawSequenceName).json",
            frame: 0,
            fps: 0,
            status: .queued,
            message: nil,
            isAdopted: false
        )
    }

    /// Walks the queued renders and promotes as many as concurrency limits
    /// allow to `.running`, kicking off a real ffmpeg task for each.
    private func promoteQueuedRenders() {
        // Stable order: by output path so the same variant always wins ties.
        let queued = renders
            .filter { $0.value.status == .queued }
            .sorted { $0.key < $1.key }

        for (path, _) in queued {
            if currentRunningCount() >= config.maxProcesses.ffmpeg { break }
            guard var render = renders[path], render.status == .queued else { continue }
            if currentRunningForSequence(render.sequenceName) >= config.maxConcurrentRenders {
                continue
            }
            render.status = .running
            render.task = launchRender(for: render)
            renders[path] = render
        }
        publishSnapshot()
    }

    private func launchRender(for render: ActiveRender) -> Task<Void, Never> {
        // Adopted renders never go through here — they have no plan because
        // we don't manage them as a Task.  promoteQueuedRenders() only
        // promotes .queued status, and adopted renders are .running already.
        guard let plan = render.plan else {
            return Task { /* no-op */ }
        }
        let runner = FFmpegRunner(plan: plan)
        let outputPath = render.outputPath
        let outputFilename = render.outputFilename
        let sidecarPath = render.sidecarJSONPath
        let argv = plan.arguments
        let daemonRef = self
        let outputBuffer = OutputLineBuffer(capacity: 200)
        return Task.detached(priority: .userInitiated) { [logger] in
            logger.timeLog(
                name: outputFilename,
                message: "starting ffmpeg: ffmpeg \(argv.joined(separator: " "))",
                value: -1
            )
            do {
                let result = try await runner.run(
                    onLaunched: { pid in
                        Task { await daemonRef.recordPID(outputPath: outputPath, pid: pid) }
                    },
                    onProgress: { progress in
                        Task { await daemonRef.recordProgress(outputPath: outputPath, frame: progress.frame, fps: progress.fps) }
                    },
                    onOutputLine: { line in
                        outputBuffer.append(line)
                    }
                )
                if result.success, let sidecarPath {
                    do {
                        try await XMPEmbedder(
                            videoPath: outputPath,
                            sidecarPath: sidecarPath
                        ).embed()
                        logger.timeLog(
                            name: outputFilename,
                            message: "embedded XMP metadata in \(outputPath)",
                            value: -1
                        )
                    } catch {
                        logger.warn("XMP embed failed for \(outputFilename): \(error)")
                    }
                }
                await daemonRef.markRenderFinished(outputPath: outputPath, success: result.success)
                if result.success {
                    logger.timeLog(name: outputFilename, message: "rendered \(outputPath)", value: -1)
                } else {
                    let tail = outputBuffer.lines().joined(separator: "\n    | ")
                    logger.timeLog(
                        name: outputFilename,
                        message: "render FAILED for \(outputPath) (ffmpeg exit code \(result.exitCode)). last \(outputBuffer.count) line(s) of ffmpeg output:\n    | \(tail)",
                        value: -1
                    )
                }
            } catch is CancellationError {
                await daemonRef.markRenderFinished(outputPath: outputPath, success: false, cancelled: true)
            } catch {
                let tail = outputBuffer.lines().joined(separator: "\n    | ")
                logger.warn("render error for \(outputFilename): \(error)\nffmpeg output:\n    | \(tail)")
                await daemonRef.markRenderFinished(outputPath: outputPath, success: false)
            }
        }
    }

    private func recordProgress(outputPath: String, frame: Int, fps: Double) {
        guard var r = renders[outputPath] else { return }
        r.frame = frame
        r.fps = fps
        renders[outputPath] = r
        publishSnapshot()
    }

    private func markRenderFinished(outputPath: String, success: Bool, cancelled: Bool = false) {
        guard var r = renders[outputPath] else { return }
        if cancelled {
            r.status = .cancelled
        } else if success {
            r.status = .finished
            recordCompletedRender(r)
            // Possibly delete the source sequence if matching delete regex.
            for pattern in config.deleteSequenceAfterRenderRegexes {
                if let regex = try? NSRegularExpression(pattern: pattern) {
                    let candidatePath = "\(r.exportPath)/\(r.sequenceName)"
                    let range = NSRange(candidatePath.startIndex..., in: candidatePath)
                    if regex.firstMatch(in: candidatePath, options: [], range: range) != nil {
                        // ensure all renders for this sequence are done first
                        let stillBusy = renders.values.contains {
                            $0.sequenceName == r.sequenceName &&
                            ($0.status == .running || $0.status == .queued)
                            && $0.outputFilename != r.outputFilename
                        }
                        if !stillBusy {
                            try? FileManager.default.removeItem(atPath: candidatePath)
                            logger.timeLog(name: r.sequenceName, message: "removed \(candidatePath)", value: -1)
                        }
                    }
                }
            }
        } else {
            r.status = .failed
        }
        renders[outputPath] = r
        publishSnapshot()
        // A slot just opened; promote the next queued variant of this sequence
        // (or any other sequence) without waiting for the next scan tick.
        promoteQueuedRenders()
    }

    /// Record a successful render in the completed-results log and persist
    /// it to disk so the entry survives an app restart.
    private func recordCompletedRender(_ r: ActiveRender) {
        let size: Int64
        if let attrs = try? FileManager.default.attributesOfItem(atPath: r.outputPath),
           let n = attrs[.size] as? NSNumber {
            size = n.int64Value
        } else {
            size = 0
        }
        let result = CompletedRenderResult(
            timestamp: Date(),
            sequenceName: r.sequenceName,
            exportPath: r.exportPath,
            outputFilename: r.outputFilename,
            outputPath: r.outputPath,
            frameCount: r.totalFrames,
            fileSize: size
        )
        // De-dup by output path: an adopted render that completes might also
        // get an entry from elsewhere; just keep the most recent.
        completedRenderResults.removeAll { $0.outputPath == result.outputPath }
        completedRenderResults.insert(result, at: 0)
        pruneAndSaveHistory()
    }

    /// Prune entries older than `recent_render_history_days` and write the
    /// result to disk if a history URL is configured.  No-op if persistence
    /// is disabled.
    private func pruneAndSaveHistory() {
        let cutoff = Date().addingTimeInterval(
            -Double(config.recentRenderHistoryDays) * 86400
        )
        completedRenderResults.removeAll { $0.timestamp <= cutoff }
        if completedRenderResults.count > maxCompletedRenderResults {
            completedRenderResults.removeLast(
                completedRenderResults.count - maxCompletedRenderResults
            )
        }
        guard let url = historyURL else { return }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        guard let data = try? encoder.encode(completedRenderResults) else { return }
        let dir = url.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        do {
            try data.write(to: url, options: .atomic)
        } catch {
            logger.warn("could not write render history to \(url.path): \(error)")
        }
    }

    // MARK: - Exif extraction

    private func maybeStartExifJob(
        rawSequenceParent: String,
        rawSequenceName: String,
        exifJsonFilename: String
    ) async {
        if exifInFlight.contains(exifJsonFilename) { return }
        if exifKnownSet.contains(exifJsonFilename) { return }
        exifInFlight.insert(exifJsonFilename)

        let dir = "\(rawSequenceParent)/\(rawSequenceName)"
        let rawTypes = config.rawTypes
        let logger = self.logger

        Task.detached(priority: .background) { [weak self] in
            guard let self else { return }
            let totals = await Self.collectExifTotals(in: dir, rawTypes: rawTypes, logger: logger)
            let single = ExifAccumulator.collapseToSingleValues(totals)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            // Encode totals as Codable: keys -> dicts of keys -> int
            let totalsCodable = totals.mapValues { $0.mapValues { $0 } }
            if let data = try? encoder.encode(totalsCodable) {
                try? data.write(to: URL(fileURLWithPath: exifJsonFilename), options: .atomic)
                logger.info("wrote exif json \(exifJsonFilename)")
            }
            await self.markExifFinished(filename: exifJsonFilename)
            _ = single
        }
    }

    private func markExifFinished(filename: String) {
        exifInFlight.remove(filename)
        exifKnownSet.insert(filename)
    }

    private static func collectExifTotals(
        in dir: String,
        rawTypes: [String],
        logger: DaemonLogger
    ) async -> [String: [String: Int]] {
        guard let entries = try? FileManager.default.contentsOfDirectory(atPath: dir) else {
            logger.warn("cannot list raw dir \(dir)")
            return [:]
        }
        let types = Set(rawTypes.map { $0.lowercased() })
        let raws = entries.filter { f in
            guard let dot = f.lastIndex(of: ".") else { return false }
            return types.contains(String(f[f.index(after: dot)...]).lowercased())
        }
        var totals: [String: [String: Int]] = [:]
        // Run a few exiftool processes in parallel; cap to keep load reasonable.
        let parallelism = 8
        var index = 0
        while index < raws.count {
            let batch = Array(raws[index..<min(index + parallelism, raws.count)])
            await withTaskGroup(of: [String: String].self) { group in
                for f in batch {
                    group.addTask {
                        let runner = ExiftoolRunner(filename: "\(dir)/\(f)")
                        return (try? await runner.run()) ?? [:]
                    }
                }
                for await map in group {
                    ExifAccumulator.accumulate(into: &totals, from: map)
                }
            }
            index += parallelism
        }
        return totals
    }

    private func readExifMetadata(at path: String) -> [String: String]? {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)) else { return nil }
        guard let any = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        var out: [String: String] = [:]
        for (key, value) in any {
            if let dict = value as? [String: Int], dict.count == 1, let v = dict.first?.key {
                out[key] = v
            } else if let dict = value as? [String: Any], dict.count == 1, let v = dict.first?.key {
                out[key] = v
            }
        }
        return out
    }

    // MARK: - Naming / dedup

    private func outputVideoFilename(for renderType: RenderType, sequenceName: String) -> String {
        var name = sequenceName
        let prefix = config.imageSequenceDirnamePrefix
        if !prefix.isEmpty {
            while name.contains(prefix) { name = name.replacingOccurrences(of: prefix, with: "") }
        }
        let nameOnlyPrefix = config.imageNamePrefix
        if !nameOnlyPrefix.isEmpty {
            while name.contains(nameOnlyPrefix) { name = name.replacingOccurrences(of: nameOnlyPrefix, with: "") }
        }
        guard let codec = codecs[renderType.codec] else {
            return "\(name)_\(renderType.codec)_\(renderType.frameRate).mov"
        }
        let resStr: String
        if let h = renderType.height {
            resStr = "\(h)p"
        } else if let w = renderType.width {
            resStr = "\(w)w"
        } else {
            resStr = "OriRes"
        }
        return "\(name)_\(codec.filename.codec)-\(codec.filename.pixelFormat)_Rec.709F_\(resStr)_\(renderType.frameRate)_\(codec.filename.quality).mov"
    }

    private func shouldRender(outputFilename: String, exportPath: String) -> Bool {
        // Walk every export path and master_videos location looking for the file.
        for dir in config.expandedMasterVideos {
            if FileManager.default.fileExists(atPath: "\(dir)/\(outputFilename)") { return false }
        }
        for dir in config.expandedExportPaths {
            if FileManager.default.fileExists(atPath: "\(dir)/\(outputFilename)") { return false }
        }
        // Also short-circuit if a render is already in flight for this output.
        let target = "\(exportPath)/\(outputFilename)"
        if let existing = renders[target], existing.status == .running || existing.status == .queued { return false }
        return true
    }

    /// Runs ffprobe to get pixel dimensions of an image or video file.
    /// Returns nil if the probe fails or the file has no video stream.
    private func probeImageDimensions(at imagePath: String) async -> (width: Int, height: Int)? {
        let runner = ProcessRunner(
            executable: "ffprobe",
            arguments: ["-v", "error", "-select_streams", "v:0",
                        "-show_entries", "stream=width,height",
                        "-of", "default=noprint_wrappers=1", imagePath]
        )
        let output = OutputLineBuffer(capacity: 10)
        guard (try? await runner.run { output.append($0) }) != nil else { return nil }
        var w: Int?
        var h: Int?
        for line in output.lines() {
            let parts = line.split(separator: "=", maxSplits: 1)
            guard parts.count == 2 else { continue }
            let key = String(parts[0]).trimmingCharacters(in: .whitespaces)
            let val = String(parts[1]).trimmingCharacters(in: .whitespaces)
            if key == "width" { w = Int(val) }
            else if key == "height" { h = Int(val) }
        }
        guard let w, let h, w > 0, h > 0 else { return nil }
        return (width: w, height: h)
    }

    private func firstImageInSequence(at dir: String) -> String? {
        guard let entries = try? FileManager.default.contentsOfDirectory(atPath: dir) else { return nil }
        let exts = Set(config.imageExtensions.map { $0.lowercased() })
        for f in entries.sorted() {
            if f.hasPrefix(".") { continue }
            guard let dot = f.lastIndex(of: ".") else { continue }
            let ext = String(f[f.index(after: dot)...]).lowercased()
            if exts.contains(ext) { return f }
        }
        return nil
    }

    /// Given a filename like `LRT_00001-severe-noise.tiff`, returns
    /// `-severe-noise.tiff` — everything after the first run of digits.  This
    /// is what ffmpeg's image2 demuxer needs as the suffix for a
    /// `prefix%05dsuffix` glob.  Uses the same regex as the Perl daemon
    /// (`\d+(.*)$`) so behaviour matches across edge cases.
    static func imageSuffixAfterDigits(
        in filename: String,
        fallbackExtension: String
    ) -> String {
        if let match = filename.firstMatch(of: #/\d+(.*)$/#) {
            return String(match.output.1)
        }
        return fallbackExtension
    }

    // MARK: - Snapshot publishing

    private func publishSnapshot(
        rawSequenceCount: Int? = nil,
        sequences: [ExportSequence]? = nil
    ) {
        let rawCount = rawSequenceCount ?? lastSnapshot.rawSequenceCount
        let seqStates: [SequenceState]
        if let sequences {
            seqStates = sequences.map { s in
                SequenceState(
                    exportPath: s.exportPath,
                    dirname: s.dirname,
                    rawSequenceLength: s.rawSequenceLength,
                    generatedImageCount: s.generatedImageCount,
                    isComplete: s.isComplete,
                    allVariantsRendered: fullyRenderedSequences.contains(s.fullPath)
                )
            }
        } else {
            seqStates = lastSnapshot.sequences
        }
        let renderStates: [RenderJobState] = renders.values.map { r in
            RenderJobState(
                sequenceName: r.sequenceName,
                exportPath: r.exportPath,
                outputFilename: r.outputFilename,
                variantKey: r.variantKey,
                totalFrames: r.totalFrames,
                frame: r.frame,
                fps: r.fps,
                status: r.status,
                message: r.message,
                isAdopted: r.isAdopted
            )
        }.sorted { $0.id < $1.id }

        let snap = DaemonStateSnapshot(
            phase: phase,
            configPath: configPath,
            rawSequenceCount: rawCount,
            sequences: seqStates,
            renders: renderStates,
            completedRenders: completedRenderResults,
            pausedSequenceNames: pausedSequenceNames,
            pausedVariants: pausedVariants,
            lastError: lastSnapshot.lastError
        )
        lastSnapshot = snap
        stateContinuation?.yield(snap)
    }
}

/// Thread-safe ring buffer of strings.  Used to retain the last N output
/// lines from a child process so we can include them in failure logs.
private final class OutputLineBuffer: @unchecked Sendable {
    private let lock = NSLock()
    private var buffer: [String] = []
    private let capacity: Int

    init(capacity: Int) { self.capacity = max(1, capacity) }

    func append(_ line: String) {
        lock.lock(); defer { lock.unlock() }
        buffer.append(line)
        if buffer.count > capacity {
            buffer.removeFirst(buffer.count - capacity)
        }
    }

    func lines() -> [String] {
        lock.lock(); defer { lock.unlock() }
        return buffer
    }

    var count: Int {
        lock.lock(); defer { lock.unlock() }
        return buffer.count
    }
}
