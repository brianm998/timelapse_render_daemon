import Foundation

/// Expands `$YEAR` placeholders in config paths.
///
/// A single path containing `$YEAR` is expanded to one path per year from the
/// current year back to 1970, keeping only the expansions that resolve to a
/// real directory.  Paths without `$YEAR` are returned unchanged.
///
/// Use case: instead of listing every year explicitly in the config:
///
///     "backup_paths": [
///       "/mammoth/Photos/timelapses/2026",
///       "/mammoth/Photos/timelapses/2025",
///       ...
///     ]
///
/// you can write:
///
///     "backup_paths": ["/mammoth/Photos/timelapses/$YEAR"]
///
/// and let the daemon discover which years actually exist on disk.
public enum PathExpander {

    public static let placeholder = "$YEAR"

    public static func expandYears(_ paths: [String], now: Date = Date()) -> [String] {
        let currentYear = Calendar(identifier: .gregorian).component(.year, from: now)
        var out: [String] = []
        for path in paths {
            guard path.contains(placeholder) else {
                out.append(path)
                continue
            }
            // Most-recent first so callers that short-circuit on first match
            // (e.g. existence checks) hit the most likely candidate sooner.
            for year in stride(from: currentYear, through: 1970, by: -1) {
                let expanded = path.replacingOccurrences(
                    of: placeholder,
                    with: String(year)
                )
                var isDir: ObjCBool = false
                if FileManager.default.fileExists(atPath: expanded, isDirectory: &isDir),
                   isDir.boolValue {
                    out.append(expanded)
                }
            }
        }
        return out
    }
}

public extension DaemonConfig {
    /// `sourcePaths` with any `$YEAR` placeholders expanded against the
    /// filesystem.  Use this anywhere you'd actually walk source paths.
    var expandedSourcePaths: [String] { PathExpander.expandYears(sourcePaths) }

    /// `backupPaths` with `$YEAR` expanded.
    var expandedBackupPaths: [String] { PathExpander.expandYears(backupPaths) }

    /// `exportPaths` with `$YEAR` expanded.
    var expandedExportPaths: [String] { PathExpander.expandYears(exportPaths) }

    /// `masterVideos` with `$YEAR` expanded.
    var expandedMasterVideos: [String] { PathExpander.expandYears(masterVideos) }
}
