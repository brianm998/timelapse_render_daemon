import Foundation
import AppKit

/// Persists GUI state to a user-defaults plist at the standard macOS
/// location (`~/Library/Preferences/com.timelapse-render-daemon.gui.plist`).
///
/// We use an explicit suite name rather than `UserDefaults.standard` so
/// persistence works for unbundled SwiftPM builds (which have no
/// `CFBundleIdentifier`) the same way it does for bundled `.app` releases.
///
/// The running/stopped state is intentionally NOT persisted — restarting the
/// daemon is an explicit action, not a UI preference.
enum UserPreferences {

    static let suiteName = "com.timelapse-render-daemon.gui"
    static let defaults: UserDefaults = UserDefaults(suiteName: suiteName) ?? .standard

    enum Key {
        static let selectedTab           = "selectedTab"
        static let lastConfigPath        = "lastConfigPath"
        static let sequenceFilter        = "sequenceFilter"
        static let skipStopRenderConfirm   = "skipStopRenderConfirm"
        static let showCompletedSequences  = "showCompletedSequences"
    }
}

enum ConfirmDialog {
    /// Shows a yes/no confirmation alert with a "don't show this again"
    /// suppression checkbox.  If the user previously checked the box and
    /// confirmed, the alert is skipped and `action` runs straight away.
    @MainActor
    static func confirm(
        suppressKey: String,
        title: String,
        message: String,
        confirmTitle: String,
        action: () -> Void
    ) {
        if UserPreferences.defaults.bool(forKey: suppressKey) {
            action()
            return
        }
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: confirmTitle)
        alert.addButton(withTitle: "Cancel")
        alert.alertStyle = .warning
        alert.showsSuppressionButton = true
        alert.suppressionButton?.title = "Don't ask again"
        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            if alert.suppressionButton?.state == .on {
                UserPreferences.defaults.set(true, forKey: suppressKey)
            }
            action()
        }
    }
}
