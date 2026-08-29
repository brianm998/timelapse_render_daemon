import SwiftUI
import AppKit
import TimelapseRenderCore

struct ContentView: View {
    @Environment(AppViewModel.self) var viewModel
    @AppStorage(UserPreferences.Key.selectedTab, store: UserPreferences.defaults)
    private var selectedTab: Tab = .status

    enum Tab: String, Hashable { case status, config }

    var body: some View {
        @Bindable var vm = viewModel
        VStack(spacing: 0) {
            HeaderBar()
                .padding()
                .background(.thickMaterial)

            Picker("", selection: $selectedTab) {
                Text("Status").tag(Tab.status)
                Text("Config").tag(Tab.config)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal)
            .padding(.top, 4)

            Divider()

            WarningBanner()

            switch selectedTab {
            case .status:
                StatusView()
            case .config:
                ConfigEditorView()
            }
        }
        .onAppear {
            // Defer bootstrap to the next runloop tick.  Running it
            // synchronously during view appearance causes the cascade of
            // @Observable mutations (configPath / config / isRunning) to
            // land while the List is still doing its initial NSTableView
            // layout, triggering a "reentrant operation in NSTableView
            // delegate" warning.
            DispatchQueue.main.async {
                viewModel.bootstrap()
            }
        }
    }
}

struct HeaderBar: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading) {
                Text("Timelapse Render Daemon")
                    .font(.headline)
                if let path = viewModel.configPath {
                    Text(path).font(.caption).foregroundStyle(.secondary)
                } else {
                    Text("No config loaded").font(.caption).foregroundStyle(.orange)
                }
            }
            Spacer()
            HStack(spacing: 12) {
                StatusPill(snapshot: viewModel.snapshot, isRunning: viewModel.isRunning)
                Button(viewModel.isRunning ? "Stop" : "Start") {
                    viewModel.toggleRunning()
                }
                .disabled(viewModel.configPath == nil)
            }
        }
    }
}

/// Surfaces problems that previously went only to the log file: the daemon
/// dropping render variants whose codec isn't in the codec table, and the
/// view-model's lastError (config load/save failures etc.).
struct WarningBanner: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        let unknown = viewModel.isRunning ? viewModel.snapshot.unknownCodecs : []
        if !unknown.isEmpty {
            banner(
                "Unknown codec\(unknown.count == 1 ? "" : "s") \(unknown.joined(separator: ", ")) in rendering config — those variants are being skipped. Fix them in the Config tab.",
                color: .orange
            )
        }
        let failures = viewModel.failures
        if !failures.isEmpty {
            // Visible from the Config tab too, and regardless of how the
            // status list is filtered.
            let newest = failures[0]
            let others = failures.count - 1
            banner(
                "\(failures.count) render\(failures.count == 1 ? "" : "s") failing — "
                + "\(newest.outputFilename): \(newest.summary)"
                + (others > 0 ? " (and \(others) more)" : "")
                + "  ·  see the Status tab, the console, or \(FileLogger.defaultURL().path)",
                color: .red
            )
        }
        if let error = viewModel.lastError {
            banner(error, color: .red)
        }
    }

    private func banner(_ text: String, color: Color) -> some View {
        Label(text, systemImage: "exclamationmark.triangle.fill")
            .font(.caption)
            .foregroundStyle(color)
            .padding(.horizontal)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(color.opacity(0.1))
    }
}

struct StatusPill: View {
    let snapshot: DaemonStateSnapshot
    let isRunning: Bool

    var body: some View {
        let label: String
        let color: Color
        if !isRunning {
            label = "stopped"
            color = .gray
        } else {
            switch snapshot.phase {
            case .startingUp: label = "starting"; color = .blue
            case .scanning:   label = "scanning"; color = .blue
            case .rendering:  label = "rendering"; color = .green
            case .idle:       label = "idle"; color = .gray
            case .stopped:    label = "stopped"; color = .gray
            }
        }
        return Text(label)
            .font(.caption.bold())
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(color.opacity(0.2))
            .foregroundColor(color)
            .clipShape(Capsule())
    }
}
