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
