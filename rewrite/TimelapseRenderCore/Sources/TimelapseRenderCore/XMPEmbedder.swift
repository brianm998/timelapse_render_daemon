import Foundation

/// Outcome of an XMP embed attempt.  Distinguishes "fine — just no metadata
/// to write" from "exiftool detected the video itself is structurally
/// broken" from "exiftool failed for some other reason".  The render daemon
/// uses this distinction to decide whether to delete + re-queue the output.
public enum XMPEmbedResult: Sendable, Equatable {
    /// exiftool exited 0 and wrote the XMP block.
    case succeeded
    /// Sidecar missing, unparseable, or empty after mapping — no XMP to write,
    /// not a structural problem with the video.
    case skipped(reason: String)
    /// exiftool refused to write because the container is corrupt (e.g.,
    /// `Truncated mdat atom`, moov references past EOF).  The video itself
    /// needs to be re-rendered.
    case structuralCorruption(reason: String, output: String)
    /// exiftool failed for some reason other than container corruption
    /// (permissions, sidecar issue, disk full, etc.).  The video is presumed
    /// usable; the failure is just about embedding metadata into it.
    case toolError(stderr: String, exitCode: Int32)
}

/// Embeds an XMP packet (canonical XMP-EXIF schema) into an encoded video by
/// shelling out to exiftool.  ExifTool picks the correct atom location based
/// on extension: moov/udta/XMP_ for .mov, the Adobe-UUID atom for .mp4.
///
/// The metadata source is the JSON sidecar produced by `collectExifTotals` —
/// a `{tag-value: frame-count}` histogram per EXIF field.  Each XMP field
/// gets the mode (most frequent value); `DateTimeOriginal` is special-cased
/// to the earliest timestamp so the embedded date marks the start of the
/// timelapse, not its most common frame.
public struct XMPEmbedder: Sendable {

    public let videoPath: String
    public let sidecarPath: String

    public init(videoPath: String, sidecarPath: String) {
        self.videoPath = videoPath
        self.sidecarPath = sidecarPath
    }

    /// Read the sidecar, build exiftool args, run exiftool.  Returns a
    /// `XMPEmbedResult` describing the outcome.  The exiftool write path is
    /// also our render-integrity check: corrupt MOV containers (truncated
    /// mdat atom, sample-table inconsistencies) fail here even when ffprobe
    /// and `exiftool -validate` would say the file is fine.
    public func embed() async throws -> XMPEmbedResult {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: sidecarPath)) else {
            return .skipped(reason: "sidecar not readable: \(sidecarPath)")
        }
        guard let histogram = Self.parseHistogram(data: data) else {
            return .skipped(reason: "sidecar not parseable: \(sidecarPath)")
        }

        let assignments = XMPFieldMapper.buildExiftoolAssignments(from: histogram)
        guard !assignments.isEmpty else {
            return .skipped(reason: "no tags resolved from sidecar")
        }

        var args = ["-overwrite_original", "-q"]
        args.append(contentsOf: assignments)
        args.append(videoPath)

        let runner = ProcessRunner(executable: "exiftool", arguments: args)
        let lines = LineCollector()
        let output = try await runner.run { line in lines.append(line) }
        let combined = lines.joined()
        if output.exitCode == 0 {
            return .succeeded
        }
        if let reason = Self.detectStructuralCorruption(in: combined) {
            return .structuralCorruption(reason: reason, output: combined)
        }
        return .toolError(stderr: combined, exitCode: output.exitCode)
    }

    /// Returns the matched substring describing the container corruption if
    /// the supplied exiftool output indicates a structurally bad video file;
    /// nil otherwise.  Public so the daemon (and tests) can use the same
    /// classifier.
    public static func detectStructuralCorruption(in output: String) -> String? {
        // Exact strings emitted by ExifTool's QuickTime writer when the moov
        // sample table is internally inconsistent or the mdat is short of
        // what stsz claims.
        let needles = [
            "Truncated mdat atom",
            "Bad atom size",
            "Invalid mdat",
            "Could not parse moov",
        ]
        for needle in needles {
            if output.contains(needle) {
                return needle
            }
        }
        return nil
    }

    private final class LineCollector: @unchecked Sendable {
        private let lock = NSLock()
        private var lines: [String] = []
        func append(_ line: String) { lock.lock(); lines.append(line); lock.unlock() }
        func joined() -> String {
            lock.lock(); defer { lock.unlock() }
            return lines.joined(separator: "\n")
        }
    }

    static func parseHistogram(data: Data) -> [String: [String: Int]]? {
        guard let any = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        var out: [String: [String: Int]] = [:]
        for (key, value) in any {
            guard let dict = value as? [String: Any] else { continue }
            var inner: [String: Int] = [:]
            for (k, v) in dict {
                if let n = v as? Int { inner[k] = n }
                else if let n = v as? NSNumber { inner[k] = n.intValue }
            }
            if !inner.isEmpty { out[key] = inner }
        }
        return out
    }
}

/// Pure transformation from a sidecar histogram to a list of `-flag=value`
/// arguments for exiftool.  Public so the mapping is testable without
/// invoking exiftool.
public enum XMPFieldMapper {

    public static func buildExiftoolAssignments(
        from histogram: [String: [String: Int]]
    ) -> [String] {
        var out: [String] = []

        if let v = mode(histogram["Make"]) {
            out.append("-XMP-tiff:Make=\(v)")
        }
        if let v = mode(histogram["Model"]) {
            out.append("-XMP-tiff:Model=\(v)")
        }
        if let v = mode(histogram["LensModel"]) {
            out.append("-XMP-aux:Lens=\(v)")
        }
        // ExifTool exposes the XMP exif:ISOSpeedRatings tag under the name
        // `ISO`; writing it produces the canonical <exif:ISOSpeedRatings>
        // element in the XMP packet.
        if let v = mode(histogram["ISO"]) {
            out.append("-XMP-exif:ISO=\(v)")
        }
        if let v = mode(histogram["FNumber"]) {
            out.append("-XMP-exif:FNumber=\(v)")
        }
        if let v = mode(histogram["ExposureTime"]) {
            out.append("-XMP-exif:ExposureTime=\(v)")
        }
        if let v = mode(histogram["FocalLength"]) {
            out.append("-XMP-exif:FocalLength=\(stripFocalLengthSuffix(v))")
        }
        // The three EXIF enum tags below are stored as integers in XMP but
        // exiftool's default PrintConv expects the text form ("Manual",
        // "Auto", etc.).  The `#` suffix tells exiftool to treat the value
        // as raw and skip PrintConv, so the integer we mapped to lands
        // unchanged in the XMP packet.
        if let v = mode(histogram["ExposureProgram"]) {
            out.append("-XMP-exif:ExposureProgram#=\(exposureProgramToInt(v))")
        }
        if let v = mode(histogram["ExposureMode"]) {
            out.append("-XMP-exif:ExposureMode#=\(exposureModeToInt(v))")
        }
        if let v = mode(histogram["WhiteBalance"]) {
            out.append("-XMP-exif:WhiteBalance#=\(whiteBalanceToInt(v))")
        }
        if let dto = earliestKey(histogram["DateTimeOriginal"]) {
            out.append("-XMP-exif:DateTimeOriginal=\(toISO8601(dto))")
        }
        return out
    }

    /// Returns the most-frequent key.  Ties break by key descending so the
    /// choice is deterministic across runs.
    static func mode(_ counts: [String: Int]?) -> String? {
        guard let counts, !counts.isEmpty else { return nil }
        return counts.max { a, b in
            if a.value != b.value { return a.value < b.value }
            return a.key > b.key
        }?.key
    }

    /// Returns the lexicographically-earliest key.  For EXIF-format dates
    /// (`YYYY:MM:DD HH:MM:SS`) lex order matches chronological order because
    /// every component is fixed-width and zero-padded.
    static func earliestKey(_ counts: [String: Int]?) -> String? {
        guard let counts, !counts.isEmpty else { return nil }
        return counts.keys.min()
    }

    /// `"14.0 mm"` → `"14.0"`, `"85mm"` → `"85"`, `"85"` → `"85"`.
    static func stripFocalLengthSuffix(_ s: String) -> String {
        let trimmed = s.trimmingCharacters(in: .whitespaces)
        if trimmed.lowercased().hasSuffix("mm") {
            let cut = trimmed.index(trimmed.endIndex, offsetBy: -2)
            return String(trimmed[..<cut]).trimmingCharacters(in: .whitespaces)
        }
        return trimmed
    }

    /// EXIF ExposureProgram PrintConv string → standard EXIF integer.
    /// Unknown values map to 0 (Not defined).
    static func exposureProgramToInt(_ s: String) -> Int {
        let lower = s.lowercased()
        if lower == "manual" { return 1 }
        if lower == "program ae" || lower == "normal program" { return 2 }
        if lower.contains("aperture") { return 3 }
        if lower.contains("shutter") { return 4 }
        if lower.contains("creative") { return 5 }
        if lower.contains("action") { return 6 }
        if lower == "portrait" { return 7 }
        if lower == "landscape" { return 8 }
        return 0
    }

    /// EXIF ExposureMode: Auto=0, Manual=1, Auto-bracket=2.
    static func exposureModeToInt(_ s: String) -> Int {
        let lower = s.lowercased()
        if lower == "manual" { return 1 }
        if lower == "auto bracket" || lower == "auto-bracket" { return 2 }
        return 0
    }

    /// EXIF WhiteBalance only distinguishes Auto (0) vs Manual / preset (1).
    static func whiteBalanceToInt(_ s: String) -> Int {
        return s.lowercased() == "auto" ? 0 : 1
    }

    /// `YYYY:MM:DD HH:MM:SS` → `YYYY-MM-DDTHH:MM:SS`.  Pass-through if the
    /// input doesn't match the EXIF shape — exiftool accepts both forms.
    static func toISO8601(_ s: String) -> String {
        let parts = s.split(separator: " ", maxSplits: 1)
        guard parts.count == 2 else { return s }
        let date = parts[0].replacingOccurrences(of: ":", with: "-")
        let time = String(parts[1])
        let dateParts = date.split(separator: "-")
        let timeParts = time.split(separator: ":")
        guard dateParts.count == 3, timeParts.count == 3 else { return s }
        return "\(date)T\(time)"
    }
}
