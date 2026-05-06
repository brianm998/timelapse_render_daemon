import SwiftUI
import TimelapseRenderCore

struct StatusView: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        HSplitView {
            SequenceListView()
                .frame(minWidth: 320)
            RenderListView()
                .frame(minWidth: 360)
        }
    }
}

struct SequenceListView: View {
    @Environment(AppViewModel.self) var viewModel
    @State private var filter = SequenceFilter.loadFromDefaults()

    private var filteredSequences: [SequenceState] {
        filter.apply(
            to: viewModel.snapshot.sequences,
            paused: viewModel.snapshot.pausedSequenceNames
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Image Sequences")
                    .font(.title3.bold())
                Spacer()
                Text(countLabel)
                    .foregroundStyle(.secondary)
                    .font(.caption.monospacedDigit())
            }
            .padding(.horizontal)
            .padding(.vertical, 8)

            SequenceFilterBar(
                filter: $filter,
                exportPaths: uniqueExportPaths
            )
            .padding(.horizontal)
            .padding(.bottom, 8)

            Divider()

            content
        }
        .onChange(of: filter) { _, newValue in
            newValue.saveToDefaults()
        }
    }

    @ViewBuilder
    private var content: some View {
        if viewModel.snapshot.sequences.isEmpty {
            Spacer()
            Text("No sequences discovered yet").foregroundStyle(.secondary)
            Spacer()
        } else if filteredSequences.isEmpty {
            Spacer()
            VStack(spacing: 8) {
                Text("No sequences match the current filter")
                    .foregroundStyle(.secondary)
                Button("Clear filter") { filter = SequenceFilter() }
            }
            Spacer()
        } else {
            List {
                ForEach(filteredSequences) { seq in
                    SequenceRow(sequence: seq)
                }
            }
            .listStyle(.inset)
        }
    }

    private var countLabel: String {
        let total = viewModel.snapshot.sequences.count
        if filter.isActive {
            return "\(filteredSequences.count) / \(total)"
        }
        return "\(total)"
    }

    private var uniqueExportPaths: [String] {
        Array(Set(viewModel.snapshot.sequences.map(\.exportPath))).sorted()
    }
}

struct SequenceFilter: Equatable, Codable {
    enum Status: String, CaseIterable, Identifiable, Codable {
        case all = "All"
        case inProgress = "In progress"
        case complete = "Complete"
        var id: String { rawValue }
    }

    enum PauseFilter: String, CaseIterable, Identifiable, Codable {
        case all = "All"
        case onlyActive = "Active"
        case onlyPaused = "Paused"
        var id: String { rawValue }
    }

    enum SortBy: String, CaseIterable, Identifiable, Codable {
        case progressDesc   = "Progress (high → low)"
        case progressAsc    = "Progress (low → high)"
        case nameAsc        = "Name (A → Z)"
        case nameDesc       = "Name (Z → A)"
        case imageCountDesc = "Frames (most first)"
        case imageCountAsc  = "Frames (fewest first)"
        var id: String { rawValue }
    }

    var search: String = ""
    var status: Status = .all
    var pauseFilter: PauseFilter = .all
    var exportPath: String? = nil   // nil == any path
    var sortBy: SortBy = .progressDesc

    /// Whether any filter (not the sort) is in non-default state.
    /// Used to decide whether to show the "Reset" button and the
    /// "filtered / total" count label.
    var isActive: Bool {
        !search.isEmpty || status != .all || pauseFilter != .all || exportPath != nil
    }

    func apply(to sequences: [SequenceState], paused: Set<String>) -> [SequenceState] {
        let needle = search.trimmingCharacters(in: .whitespaces).lowercased()
        let filtered = sequences.filter { seq in
            if !needle.isEmpty && !seq.dirname.lowercased().contains(needle) { return false }
            switch status {
            case .all: break
            case .inProgress: if seq.isComplete { return false }
            case .complete:   if !seq.isComplete { return false }
            }
            let isPaused = paused.contains(seq.dirname)
            switch pauseFilter {
            case .all: break
            case .onlyActive: if isPaused { return false }
            case .onlyPaused: if !isPaused { return false }
            }
            if let exportPath, seq.exportPath != exportPath { return false }
            return true
        }
        return sort(filtered)
    }

    /// Read the persisted filter from UserDefaults, or return defaults.
    static func loadFromDefaults() -> SequenceFilter {
        guard let data = UserPreferences.defaults.data(forKey: UserPreferences.Key.sequenceFilter),
              let decoded = try? JSONDecoder().decode(SequenceFilter.self, from: data)
        else { return SequenceFilter() }
        return decoded
    }

    /// Persist the current filter to UserDefaults.
    func saveToDefaults() {
        guard let data = try? JSONEncoder().encode(self) else { return }
        UserPreferences.defaults.set(data, forKey: UserPreferences.Key.sequenceFilter)
    }

    private func sort(_ sequences: [SequenceState]) -> [SequenceState] {
        // Use dirname as a stable tie-breaker so equal-progress sequences
        // (e.g. all complete at 1.0) don't reshuffle on every snapshot.
        switch sortBy {
        case .progressDesc:
            return sequences.sorted {
                $0.progress == $1.progress
                  ? $0.dirname < $1.dirname
                  : $0.progress > $1.progress
            }
        case .progressAsc:
            return sequences.sorted {
                $0.progress == $1.progress
                  ? $0.dirname < $1.dirname
                  : $0.progress < $1.progress
            }
        case .nameAsc:
            return sequences.sorted { $0.dirname < $1.dirname }
        case .nameDesc:
            return sequences.sorted { $0.dirname > $1.dirname }
        case .imageCountDesc:
            return sequences.sorted {
                $0.rawSequenceLength == $1.rawSequenceLength
                  ? $0.dirname < $1.dirname
                  : $0.rawSequenceLength > $1.rawSequenceLength
            }
        case .imageCountAsc:
            return sequences.sorted {
                $0.rawSequenceLength == $1.rawSequenceLength
                  ? $0.dirname < $1.dirname
                  : $0.rawSequenceLength < $1.rawSequenceLength
            }
        }
    }
}

struct SequenceFilterBar: View {
    @Binding var filter: SequenceFilter
    let exportPaths: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Filter by name", text: $filter.search)
                    .textFieldStyle(.roundedBorder)
                if !filter.search.isEmpty {
                    Button {
                        filter.search = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.borderless)
                }
            }
            HStack(spacing: 6) {
                Picker("Status", selection: $filter.status) {
                    ForEach(SequenceFilter.Status.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .frame(maxWidth: 130)

                Picker("Pause", selection: $filter.pauseFilter) {
                    ForEach(SequenceFilter.PauseFilter.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .frame(maxWidth: 100)

                Picker("Export path", selection: $filter.exportPath) {
                    Text("Any path").tag(String?.none)
                    ForEach(exportPaths, id: \.self) { p in
                        Text(p).tag(String?.some(p))
                    }
                }
                .labelsHidden()
                .pickerStyle(.menu)

                if filter.isActive {
                    Button("Reset") { filter = SequenceFilter() }
                        .buttonStyle(.borderless)
                        .font(.caption)
                }
            }
            HStack(spacing: 6) {
                Image(systemName: "arrow.up.arrow.down")
                    .foregroundStyle(.secondary)
                Text("Sort by")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Picker("Sort by", selection: $filter.sortBy) {
                    ForEach(SequenceFilter.SortBy.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden()
                .pickerStyle(.menu)
            }
        }
    }
}

struct SequenceRow: View {
    @Environment(AppViewModel.self) var viewModel
    let sequence: SequenceState

    var isPaused: Bool { viewModel.snapshot.pausedSequenceNames.contains(sequence.dirname) }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: sequence.isComplete ? "checkmark.circle.fill" : "hourglass")
                    .foregroundStyle(sequence.isComplete ? .green : .orange)
                Text(sequence.dirname).font(.system(.body, design: .monospaced))
                Spacer()
                if isPaused {
                    Text("paused").font(.caption).foregroundStyle(.orange)
                }
                Menu {
                    if isPaused {
                        Button("Resume") { viewModel.resumeSequence(sequence.dirname) }
                    } else {
                        Button("Pause") { viewModel.pauseSequence(sequence.dirname) }
                    }
                    if let config = viewModel.config {
                        ForEach(config.rendering, id: \.variantKey) { rt in
                            Menu(rt.variantKey) {
                                let pv = PausedVariant(sequenceName: sequence.dirname, variantKey: rt.variantKey)
                                if viewModel.snapshot.pausedVariants.contains(pv) {
                                    Button("Resume variant") {
                                        viewModel.resumeVariant(sequenceName: sequence.dirname, variantKey: rt.variantKey)
                                    }
                                } else {
                                    Button("Pause variant") {
                                        viewModel.pauseVariant(sequenceName: sequence.dirname, variantKey: rt.variantKey)
                                    }
                                }
                            }
                        }
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .frame(width: 26)
            }
            HStack(spacing: 8) {
                ProgressView(value: sequence.progress)
                    .progressViewStyle(.linear)
                Text("\(sequence.generatedImageCount)/\(sequence.rawSequenceLength)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Text(sequence.exportPath)
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }
}

struct RenderListView: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Active Renders")
                    .font(.title3.bold())
                Spacer()
                Text("\(viewModel.snapshot.renders.count)")
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal)
            .padding(.vertical, 8)
            Divider()

            if viewModel.snapshot.renders.isEmpty {
                Spacer()
                Text("No active renders")
                    .foregroundStyle(.secondary)
                Spacer()
            } else {
                List {
                    ForEach(viewModel.snapshot.renders) { r in
                        RenderRow(render: r)
                    }
                }
                .listStyle(.inset)
            }
        }
    }
}

struct RenderRow: View {
    @Environment(AppViewModel.self) var viewModel
    let render: RenderJobState

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Circle()
                    .fill(statusColor)
                    .frame(width: 8, height: 8)
                Text(render.outputFilename)
                    .font(.system(.body, design: .monospaced))
                Spacer()
                if render.isAdopted {
                    Text("adopted")
                        .font(.caption.bold())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.purple.opacity(0.2))
                        .foregroundColor(.purple)
                        .clipShape(Capsule())
                }
                Text(render.status.rawValue)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                pauseResumeButton
                stopButton
            }
            HStack(spacing: 8) {
                if render.isAdopted && render.status != .paused {
                    // We have no pipe to the orphan, so progress is unknown.
                    // Use SwiftUI's indeterminate "barber pole" style.
                    ProgressView()
                        .progressViewStyle(.linear)
                } else {
                    ProgressView(value: render.progress)
                        .progressViewStyle(.linear)
                }
                if render.isAdopted {
                    Text("? frames")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                } else {
                    Text("\(render.frame)/\(render.totalFrames)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                    if render.fps > 0 {
                        Text("\(render.fps, specifier: "%.1f") fps")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Text("\(render.exportPath)  ·  \(render.variantKey)")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private var pauseResumeButton: some View {
        switch render.status {
        case .paused:
            Button {
                viewModel.resumeRender(outputPath: render.id)
            } label: {
                Image(systemName: "play.circle")
            }
            .buttonStyle(.borderless)
            .help("Resume (SIGCONT)")
        case .running:
            Button {
                viewModel.pauseRender(outputPath: render.id)
            } label: {
                Image(systemName: "pause.circle")
            }
            .buttonStyle(.borderless)
            .help("Pause (SIGSTOP)")
        case .queued:
            // Queued renders haven't started yet — show the play icon as a
            // "ready to play" cue.  Clicking it marks the render paused so
            // it isn't auto-promoted on the next slot.
            Button {
                viewModel.pauseRender(outputPath: render.id)
            } label: {
                Image(systemName: "play.circle")
            }
            .buttonStyle(.borderless)
            .help("Pause (don't promote)")
        default:
            EmptyView()
        }
    }

    @ViewBuilder
    private var stopButton: some View {
        if render.status == .running || render.status == .paused || render.status == .queued {
            Button {
                ConfirmDialog.confirm(
                    suppressKey: UserPreferences.Key.skipStopRenderConfirm,
                    title: "Stop this render?",
                    message: "ffmpeg will be terminated and the partial output deleted. The variant will be marked paused so it isn't re-rendered automatically.",
                    confirmTitle: "Stop"
                ) {
                    viewModel.stopRender(outputPath: render.id)
                }
            } label: {
                Image(systemName: "xmark.circle")
            }
            .buttonStyle(.borderless)
            .help("Stop and cancel render")
        }
    }

    private var statusColor: Color {
        switch render.status {
        case .running:   return .green
        case .queued:    return .yellow
        case .finished:  return .blue
        case .failed:    return .red
        case .cancelled: return .orange
        case .paused:    return .gray
        case .skipped:   return .gray
        }
    }
}
