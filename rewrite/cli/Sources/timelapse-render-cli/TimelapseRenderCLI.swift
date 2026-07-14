import ArgumentParser
import Foundation
import TimelapseRenderCore

@main
struct TimelapseRenderCLI: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "timelapse-render",
        abstract: "Watch image-sequence directories and render timelapse videos.",
        subcommands: [Run.self, Scan.self, ListSequences.self, Pause.self, Resume.self, ShowConfig.self],
        defaultSubcommand: Run.self
    )
}

// MARK: - Shared options

struct ConfigOption: ParsableArguments {
    @Option(name: [.customShort("c"), .long], help: "Path to config.json")
    var config: String?

    @Option(name: .long, help: "Path to video_codecs.json (defaults to one next to the config file, if present)")
    var codecs: String?

    func resolveCodecsPath(configPath: String) -> String? {
        codecs ?? ConfigLoader.codecsPath(alongsideConfig: configPath)
    }

    func resolveConfigPath() throws -> String {
        if let config { return config }
        let cwd = FileManager.default.currentDirectoryPath
        let candidate = "\(cwd)/config.json"
        if FileManager.default.fileExists(atPath: candidate) { return candidate }
        throw ValidationError("--config not given and ./config.json does not exist")
    }
}

// MARK: - run

struct Run: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "run",
        abstract: "Run the daemon, watching for sequences to render."
    )

    @OptionGroup var configOpt: ConfigOption

    @Flag(name: .long, help: "Disable color in console output")
    var noColor: Bool = false

    @Flag(name: .long, help: "Use a plain (non-updateable) log; useful when output is piped")
    var plainLog: Bool = false

    func run() async throws {
        let configPath = try configOpt.resolveConfigPath()
        let logger: DaemonLogger = plainLog ? PlainLogger() : UpdateableLog(enableColor: !noColor)
        let daemon: RenderDaemon
        do {
            daemon = try RenderDaemon(
                configPath: configPath,
                codecsPath: configOpt.resolveCodecsPath(configPath: configPath),
                logger: logger
            )
        } catch {
            throw CleanExit.message("could not load config: \(error)")
        }

        // Stream snapshots to stdout in plain mode for visibility / piping.
        let snapshots = daemon.stateStream
        let observerTask = Task {
            for await _ in snapshots {
                // The updateable log handles its own rendering; we don't need
                // to do anything here. The stream is consumed so it doesn't
                // back up and stays available for other observers in-process.
            }
        }

        await daemon.run()
        observerTask.cancel()
    }
}

// MARK: - scan

struct Scan: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "scan",
        abstract: "Scan once and print discovered sequences, then exit."
    )

    @OptionGroup var configOpt: ConfigOption

    func run() async throws {
        let configPath = try configOpt.resolveConfigPath()
        let config = try ConfigLoader.load(at: configPath)
        let logger = PlainLogger()

        let scanner = SequenceScanner()
        await scanner.refreshRawSequenceCounts(config: config, logger: logger)
        let exports = await scanner.discoverExportSequences(config: config, logger: logger)
        let raws = await scanner.currentRawSequences()

        print("raw sequences: \(raws.count)")
        for r in raws.sorted(by: { $0.name < $1.name }) {
            print("  \(r.parentPath)/\(r.name)  (\(r.imageCount) images)")
        }
        print()
        print("export sequences: \(exports.count)")
        for e in exports.sorted(by: { $0.dirname < $1.dirname }) {
            let pct = e.rawSequenceLength > 0
                ? "\(e.generatedImageCount)/\(e.rawSequenceLength)"
                : "(unknown)"
            print("  \(e.exportPath)/\(e.dirname)  \(pct)\(e.isComplete ? "  [READY]" : "")")
        }
    }
}

// MARK: - list

struct ListSequences: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "list",
        abstract: "List render variants that would be produced for one sequence."
    )

    @OptionGroup var configOpt: ConfigOption

    @Argument(help: "The exported image sequence dirname")
    var sequenceName: String

    func run() async throws {
        let configPath = try configOpt.resolveConfigPath()
        let config = try ConfigLoader.load(at: configPath)
        for r in config.rendering {
            print("  variant: \(r.variantKey)  codec=\(r.codec) frameRate=\(r.frameRate)")
        }
        print()
        print("(use 'pause' / 'resume' to control rendering of \(sequenceName))")
    }
}

// MARK: - pause / resume

struct Pause: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "pause",
        abstract: "Persistently pause a sequence or one of its variants in the config."
    )

    @OptionGroup var configOpt: ConfigOption

    @Argument(help: "Sequence dirname to pause")
    var sequenceName: String

    @Option(name: .long, help: "Variant key to pause (omit to pause whole sequence)")
    var variant: String?

    func run() throws {
        let configPath = try configOpt.resolveConfigPath()
        var config = try ConfigLoader.load(at: configPath)
        if let variant {
            let v = PausedVariant(sequenceName: sequenceName, variantKey: variant)
            if !config.pausedVariants.contains(v) {
                config.pausedVariants.append(v)
            }
        } else {
            if !config.pausedSequenceNames.contains(sequenceName) {
                config.pausedSequenceNames.append(sequenceName)
            }
        }
        try ConfigLoader.write(config, to: configPath)
        print("paused")
    }
}

struct Resume: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "resume",
        abstract: "Resume a sequence or variant previously paused via pause."
    )

    @OptionGroup var configOpt: ConfigOption

    @Argument(help: "Sequence dirname to resume")
    var sequenceName: String

    @Option(name: .long, help: "Variant key to resume (omit to resume whole sequence)")
    var variant: String?

    func run() throws {
        let configPath = try configOpt.resolveConfigPath()
        var config = try ConfigLoader.load(at: configPath)
        if let variant {
            let v = PausedVariant(sequenceName: sequenceName, variantKey: variant)
            config.pausedVariants.removeAll { $0 == v }
        } else {
            config.pausedSequenceNames.removeAll { $0 == sequenceName }
        }
        try ConfigLoader.write(config, to: configPath)
        print("resumed")
    }
}

// MARK: - show-config

struct ShowConfig: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "show-config",
        abstract: "Print the parsed config to stdout."
    )

    @OptionGroup var configOpt: ConfigOption

    func run() throws {
        let configPath = try configOpt.resolveConfigPath()
        let config = try ConfigLoader.load(at: configPath)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(config)
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }
}

