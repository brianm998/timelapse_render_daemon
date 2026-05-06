import Foundation

/// Builds a render command line and runs ffmpeg, parsing progress lines.
public struct FFmpegRunner: Sendable {

    public struct Plan: Sendable {
        public let inputDir: String
        public let imageNamePrefix: String
        public let imageExtension: String
        public let outputPath: String
        public let frameRate: Int
        public let aspect: String
        public let width: Int?
        public let height: Int?
        public let codec: VideoCodec
        public let metadata: [String: String]
        public let totalFrames: Int

        public init(
            inputDir: String,
            imageNamePrefix: String,
            imageExtension: String,
            outputPath: String,
            frameRate: Int,
            aspect: String,
            width: Int?,
            height: Int?,
            codec: VideoCodec,
            metadata: [String: String],
            totalFrames: Int
        ) {
            self.inputDir = inputDir
            self.imageNamePrefix = imageNamePrefix
            self.imageExtension = imageExtension
            self.outputPath = outputPath
            self.frameRate = frameRate
            self.aspect = aspect
            self.width = width
            self.height = height
            self.codec = codec
            self.metadata = metadata
            self.totalFrames = totalFrames
        }

        /// Argv for ffmpeg.  Mirrors the original Perl command line.
        public var arguments: [String] {
            var args: [String] = [
                // -nostdin tells ffmpeg not to try reading from stdin.  Even
                // though we point stdin at /dev/null, this avoids any future
                // surprises where ffmpeg blocks on a stdin read.
                "-nostdin",
                "-y",
                "-r", String(frameRate),
                "-i", "\(inputDir)/\(imageNamePrefix)%05d\(imageExtension)",
                "-aspect", aspect,
            ]
            if let w = width, let h = height {
                let filter = "crop=floor(iw/2)*2:floor(ih/2)*2,scale=\(w):\(h):in_range=full:out_range=full,setsar=1/1"
                args += ["-filter_complex", filter]
            }
            args += [
                "-c:v", codec.ffmpeg.codec,
                "-pix_fmt", codec.ffmpeg.pixelFormat,
                "-threads", "0",
                "-profile:v", "1",
                "-movflags", "+write_colr",
                "-an",
                "-color_range", codec.ffmpeg.colorRange,
                "-color_primaries", "bt709",
                "-colorspace", "bt709",
                "-color_trc", "bt709",
                "-timecode", "00:00:00:00",
            ]
            for (k, v) in metadata.sorted(by: { $0.key < $1.key }) {
                args += ["-metadata", "\(k)=\(v)"]
            }
            args.append(outputPath)
            return args
        }
    }

    public struct Progress: Sendable {
        public let frame: Int
        public let fps: Double
    }

    public let plan: Plan

    public init(plan: Plan) {
        self.plan = plan
    }

    public struct Result: Sendable {
        public let success: Bool
        public let exitCode: Int32
    }

    /// Runs ffmpeg.  `onProgress` is called for `frame=… fps=…` lines.
    /// `onOutputLine` is called for every line — useful for capturing
    /// stderr context to log when a render fails.  Returns success +
    /// exit code so callers can distinguish "ffmpeg crashed" from "ffmpeg
    /// finished but produced no output".
    public func run(
        onLaunched: @escaping @Sendable (pid_t) -> Void = { _ in },
        onProgress: @escaping @Sendable (Progress) -> Void = { _ in },
        onOutputLine: @escaping @Sendable (String) -> Void = { _ in }
    ) async throws -> Result {
        let runner = ProcessRunner(executable: "ffmpeg", arguments: plan.arguments)
        let output = try await runner.run(onLaunched: onLaunched) { line in
            onOutputLine(line)
            if let p = Self.parseProgress(line) {
                onProgress(p)
            }
        }
        let fileExists = FileManager.default.fileExists(atPath: plan.outputPath)
        return Result(
            success: fileExists && output.exitCode == 0,
            exitCode: output.exitCode
        )
    }

    /// Parses a single ffmpeg progress line, e.g.
    /// "frame=   30 fps=0.0 q=-0.0 Lsize=       6KiB time=00:00:06.00 ..."
    ///
    /// Uses the same regex as the original Perl daemon
    /// (`^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)`), which correctly handles
    /// fixed-width right-aligned values where multiple spaces separate `=`
    /// from the value (`frame=   30`).
    public static func parseProgress(_ line: String) -> Progress? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        let regex = #/^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)/#
        guard let match = trimmed.firstMatch(of: regex) else { return nil }
        guard let frame = Int(match.output.1), let fps = Double(match.output.2) else { return nil }
        return Progress(frame: frame, fps: fps)
    }
}

public enum AspectRatio {
    public static func string(width: Int, height: Int) -> String {
        guard height > 0 else { return "\(width)/\(height)" }
        if width % height == 0 { return "\(width / height)/1" }
        // Find smallest integer multiplier (1..1000) producing whole-number aspect.
        let w = Double(width) / Double(height)
        for m in 2...1000 {
            let lhs = w * Double(m)
            if abs(lhs - lhs.rounded()) < 1e-9 {
                return "\(Int(lhs.rounded()))/\(m)"
            }
        }
        return "\(width)/\(height)"
    }
}
