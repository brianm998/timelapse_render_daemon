import SwiftUI
import AppKit
import TimelapseRenderCore

@main
struct TimelapseRenderApp: App {

    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var viewModel = AppViewModel()

    var body: some Scene {
        WindowGroup("Timelapse Render Daemon") {
            ContentView()
                .environment(viewModel)
                .frame(minWidth: 900, minHeight: 600)
        }
        .commands {
            CommandGroup(replacing: .newItem) { }
            CommandMenu("Daemon") {
                Button("Open Config…") { viewModel.pickConfig() }
                    .keyboardShortcut("o")
                Button(viewModel.isRunning ? "Stop Daemon" : "Start Daemon") {
                    viewModel.toggleRunning()
                }
                .keyboardShortcut("r")
                .disabled(viewModel.configPath == nil)
                Divider()
                Button("Reveal Log in Finder") {
                    let url = FileLogger.defaultURL()
                    NSWorkspace.shared.activateFileViewerSelecting([url])
                }
                .keyboardShortcut("l")
            }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {

    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.activate(ignoringOtherApps: true)
        if let window = NSApp.windows.first {
            window.makeKeyAndOrderFront(nil)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    // We deliberately do NOT implement application(_:openFile:) /
    // application(_:openFiles:) / application(_:open:).  When SwiftUI sees
    // those methods AND AppKit dispatches a launch-time positional argv
    // entry through them, the WindowGroup's primary window is suppressed and
    // never appears.  Instead we accept the config path via a `-c <path>` /
    // `--config <path>` flag — flag-shaped argv entries are routed through
    // NSUserDefaults' command-line override mechanism and never become an
    // "open document" event.
}
