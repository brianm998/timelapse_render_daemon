import SwiftUI
import AppKit
import TimelapseRenderCore

struct StatusView: View {
    @Environment(AppViewModel.self) var viewModel
    @State private var filter = SequenceFilter.loadFromDefaults()
    @AppStorage(UserPreferences.Key.showCompletedSequences, store: UserPreferences.defaults)
    private var showCompletedSequences: Bool = false
    @State private var highlightedItemID: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            SequenceFilterBar(
                filter: $filter,
                exportPaths: uniqueExportPaths,
                showCompletedSequences: $showCompletedSequences
            )
            .padding(.horizontal)
            .padding(.bottom, 8)
            Divider()
            unifiedList
        }
        .onChange(of: filter) { _, newValue in
            newValue.saveToDefaults()
        }
    }

    private var header: some View {
        HStack {
            Text("Renders & Sequences")
                .font(.title3.bold())
            Spacer()
            Text(headerCount).foregroundStyle(.secondary)
                .font(.caption.monospacedDigit())
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    private var headerCount: String {
        let active = viewModel.snapshot.renders.count
        let inProgress = viewModel.snapshot.sequences.filter { !$0.isComplete }.count
        let done = viewModel.snapshot.completedRenders.count
        var s = "\(active) rendering · \(inProgress) in progress · \(done) finished"
        let failed = viewModel.failures.count
        if failed > 0 { s += " · \(failed) failed" }
        return s
    }

    private var uniqueExportPaths: [String] {
        var s = Set<String>()
        for x in viewModel.snapshot.sequences { s.insert(x.exportPath) }
        for x in viewModel.snapshot.renders { s.insert(x.exportPath) }
        for x in viewModel.snapshot.completedRenders { s.insert(x.exportPath) }
        for x in viewModel.failures { s.insert(x.exportPath) }
        return Array(s).sorted()
    }

    @ViewBuilder
    private var unifiedList: some View {
        let items = buildItems()
        if items.isEmpty {
            Spacer()
            Text("Nothing yet").foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
            Spacer()
        } else {
            ScrollViewReader { proxy in
                List {
                    ForEach(items) { item in
                        UnifiedItemRow(
                            item: item,
                            isHighlighted: highlightedItemID == item.id,
                            onJumpToSequence: { sequenceName, exportPath in
                                jumpToSequence(name: sequenceName, exportPath: exportPath, proxy: proxy)
                            }
                        )
                        .id(item.id)
                    }
                }
                .listStyle(.inset)
            }
        }
    }

    private func jumpToSequence(name: String, exportPath: String, proxy: ScrollViewProxy) {
        // Make sure completed sequences are visible — that's where the user
        // wants to land when they click "go to sequence" on a completed result.
        if !showCompletedSequences {
            showCompletedSequences = true
        }
        // The UnifiedItem.id for a completed sequence is "done:<exportPath>/<dirname>"
        // (see UnifiedItem.id below).  Build the same string directly so we
        // don't have to construct a SequenceState from outside the core.
        let targetID = "done:\(exportPath)/\(name)"
        // SwiftUI needs a moment to rebuild the list with completed sequences
        // before scrollTo can find the new id.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            withAnimation { proxy.scrollTo(targetID, anchor: .center) }
            highlightedItemID = targetID
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                if highlightedItemID == targetID { highlightedItemID = nil }
            }
        }
    }

    /// Build the unified list of items in the order the GUI displays them.
    private func buildItems() -> [UnifiedItem] {
        var items: [UnifiedItem] = []
        let snap = viewModel.snapshot

        // 0. Failures first.  They are the only thing here that needs the
        //    user to do something, and they used to be invisible entirely.
        let failures = filter.matches(failures: viewModel.failures)
        items.append(contentsOf: failures.map { .failure($0) })

        // 1. Active renders — closest to done at the top.
        let activeRenders = filter.matches(renders: snap.renders).sorted {
            if $0.progress != $1.progress { return $0.progress > $1.progress }
            return $0.id < $1.id
        }
        items.append(contentsOf: activeRenders.map { .activeRender($0) })

        // 2. In-progress sequences — most-complete first.
        let inProgress = filter.matches(sequences: snap.sequences,
                                        paused: snap.pausedSequenceNames)
            .filter { !$0.allVariantsRendered }
            .filter { !$0.isComplete }
        items.append(contentsOf: inProgress.map { .inProgressSequence($0) })

        // 3. Bottom: either completed results (default) or completed sequences.
        if showCompletedSequences {
            let completed = filter.matches(sequences: snap.sequences,
                                           paused: snap.pausedSequenceNames)
                .filter { $0.allVariantsRendered }
                .sorted { $0.dirname < $1.dirname }
            items.append(contentsOf: completed.map { .completedSequence($0) })
        } else {
            let results = filter.matches(results: snap.completedRenders)
            items.append(contentsOf: results.map { .completedResult($0) })
        }
        return items
    }
}

// MARK: - Item types

enum UnifiedItem: Identifiable, Hashable {
    case failure(RenderFailure)
    case activeRender(RenderJobState)
    case inProgressSequence(SequenceState)
    case completedResult(CompletedRenderResult)
    case completedSequence(SequenceState)

    var id: String {
        switch self {
        case .failure(let f):            return "fail:\(f.id)"
        case .activeRender(let r):       return "render:\(r.id)"
        case .inProgressSequence(let s): return "seq:\(s.id)"
        case .completedResult(let r):    return "result:\(r.id)"
        case .completedSequence(let s):  return "done:\(s.id)"
        }
    }
}

// MARK: - Filter (preserved from earlier; matches multiple item types)

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

    var search: String = ""
    var status: Status = .all
    var pauseFilter: PauseFilter = .all
    var exportPath: String? = nil

    var isActive: Bool {
        !search.isEmpty || status != .all || pauseFilter != .all || exportPath != nil
    }

    static func loadFromDefaults() -> SequenceFilter {
        guard let data = UserPreferences.defaults.data(forKey: UserPreferences.Key.sequenceFilter),
              let decoded = try? JSONDecoder().decode(SequenceFilter.self, from: data)
        else { return SequenceFilter() }
        return decoded
    }
    func saveToDefaults() {
        guard let data = try? JSONEncoder().encode(self) else { return }
        UserPreferences.defaults.set(data, forKey: UserPreferences.Key.sequenceFilter)
    }

    private var needle: String {
        search.trimmingCharacters(in: .whitespaces).lowercased()
    }

    func matches(sequences: [SequenceState], paused: Set<String>) -> [SequenceState] {
        let n = needle
        return sequences.filter { seq in
            if !n.isEmpty && !seq.dirname.lowercased().contains(n) { return false }
            switch status {
            case .all: break
            case .inProgress: if seq.isComplete && seq.allVariantsRendered { return false }
            case .complete:   if !seq.allVariantsRendered { return false }
            }
            let isPaused = paused.contains(seq.dirname)
            switch pauseFilter {
            case .all: break
            case .onlyActive: if isPaused { return false }
            case .onlyPaused: if !isPaused { return false }
            }
            if let exportPath, seq.exportPath != exportPath { return false }
            return true
        }.sorted {
            if $0.progress != $1.progress { return $0.progress > $1.progress }
            return $0.dirname < $1.dirname
        }
    }

    func matches(renders: [RenderJobState]) -> [RenderJobState] {
        let n = needle
        return renders.filter { r in
            if !n.isEmpty,
               !r.outputFilename.lowercased().contains(n),
               !r.sequenceName.lowercased().contains(n) {
                return false
            }
            if let exportPath, r.exportPath != exportPath { return false }
            return true
        }
    }

    func matches(failures: [RenderFailure]) -> [RenderFailure] {
        let n = needle
        return failures.filter { f in
            if !n.isEmpty,
               !f.outputFilename.lowercased().contains(n),
               !f.sequenceName.lowercased().contains(n) {
                return false
            }
            if let exportPath, f.exportPath != exportPath { return false }
            // A failure is never "complete", so hide it when the user has
            // asked to see only completed work.
            if status == .complete { return false }
            return true
        }
    }

    func matches(results: [CompletedRenderResult]) -> [CompletedRenderResult] {
        let n = needle
        return results.filter { r in
            if !n.isEmpty,
               !r.outputFilename.lowercased().contains(n),
               !r.sequenceName.lowercased().contains(n) {
                return false
            }
            if let exportPath, r.exportPath != exportPath { return false }
            return true
        }
    }
}

// MARK: - Filter bar

struct SequenceFilterBar: View {
    @Binding var filter: SequenceFilter
    let exportPaths: [String]
    @Binding var showCompletedSequences: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Filter by name", text: $filter.search)
                    .textFieldStyle(.roundedBorder)
                if !filter.search.isEmpty {
                    Button { filter.search = "" } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                    }
                    .buttonStyle(.borderless)
                }
            }
            HStack(spacing: 6) {
                Picker("Status", selection: $filter.status) {
                    ForEach(SequenceFilter.Status.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden().pickerStyle(.menu).frame(maxWidth: 130)

                Picker("Pause", selection: $filter.pauseFilter) {
                    ForEach(SequenceFilter.PauseFilter.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden().pickerStyle(.menu).frame(maxWidth: 100)

                Picker("Export path", selection: $filter.exportPath) {
                    Text("Any path").tag(String?.none)
                    ForEach(exportPaths, id: \.self) { p in
                        Text(p).tag(String?.some(p))
                    }
                }
                .labelsHidden().pickerStyle(.menu)

                Spacer()

                Toggle("Show completed sequences", isOn: $showCompletedSequences)
                    .toggleStyle(.checkbox)
                    .font(.caption)

                if filter.isActive {
                    Button("Reset") { filter = SequenceFilter() }
                        .buttonStyle(.borderless)
                        .font(.caption)
                }
            }
        }
    }
}

// MARK: - One row, dispatches per item type

struct UnifiedItemRow: View {
    let item: UnifiedItem
    let isHighlighted: Bool
    let onJumpToSequence: (_ sequenceName: String, _ exportPath: String) -> Void

    var body: some View {
        Group {
            switch item {
            case .failure(let f):            FailureRow(failure: f)
            case .activeRender(let r):       RenderRow(render: r)
            case .inProgressSequence(let s): SequenceProgressRow(sequence: s)
            case .completedResult(let r):    CompletedResultRow(result: r, onJumpToSequence: onJumpToSequence)
            case .completedSequence(let s):  CompletedSequenceRow(sequence: s)
            }
        }
        .listRowBackground(
            isHighlighted ? Color.yellow.opacity(0.25) : Color.clear
        )
    }
}

// MARK: - In-progress sequence row (was SequenceRow)

struct SequenceProgressRow: View {
    @Environment(AppViewModel.self) var viewModel
    let sequence: SequenceState

    var isPaused: Bool {
        viewModel.snapshot.pausedSequenceNames.contains(sequence.dirname)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: "hourglass").foregroundStyle(.orange)
                Text(sequence.dirname).font(.system(.body, design: .monospaced))
                Spacer()
                if isPaused {
                    Text("paused").font(.caption).foregroundStyle(.orange)
                }
                sequenceMenu
            }
            HStack(spacing: 8) {
                ProgressView(value: sequence.progress).progressViewStyle(.linear)
                Text("\(sequence.generatedImageCount)/\(sequence.rawSequenceLength)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Text(sequence.exportPath).font(.caption2).foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    private var sequenceMenu: some View {
        Menu {
            if isPaused {
                Button("Resume") { viewModel.resumeSequence(sequence.dirname) }
            } else {
                Button("Pause") { viewModel.pauseSequence(sequence.dirname) }
            }
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .frame(width: 26)
    }
}

// MARK: - Completed sequence row (when toggle is on)

struct CompletedSequenceRow: View {
    let sequence: SequenceState

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text(sequence.dirname).font(.system(.body, design: .monospaced))
                Spacer()
                Text("done").font(.caption).foregroundStyle(.secondary)
            }
            Text(sequence.exportPath).font(.caption2).foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Completed individual render row

struct CompletedResultRow: View {
    let result: CompletedRenderResult
    let onJumpToSequence: (_ sequenceName: String, _ exportPath: String) -> Void

    static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "h:mm:ss a"
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Image(systemName: "film").foregroundStyle(.blue)
                Text(Self.timeFormatter.string(from: result.timestamp))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                Text("rendered")
                    .font(.caption)
                    .foregroundStyle(.green)
                Text(result.outputFilename)
                    .font(.system(.body, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                if result.fileSize > 0 {
                    Text(humanSize(result.fileSize))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                Button {
                    onJumpToSequence(result.sequenceName, result.exportPath)
                } label: {
                    Image(systemName: "arrow.up.left.and.arrow.down.right.magnifyingglass")
                }
                .buttonStyle(.borderless)
                .help("Show parent sequence (turns on 'Show completed sequences' if needed)")
            }
            Text("\(result.frameCount) frames · \(result.exportPath)/\(result.sequenceName)")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    private func humanSize(_ bytes: Int64) -> String {
        let units = ["B", "K", "M", "G", "T"]
        var d = Double(bytes)
        var i = 0
        while d >= 1024 && i < units.count - 1 { d /= 1024; i += 1 }
        if d >= 100 { return "\(Int(d))\(units[i])" }
        return String(format: "%.1f%@", d, units[i])
    }
}

// MARK: - Active render row (mostly the same as before)

struct RenderRow: View {
    @Environment(AppViewModel.self) var viewModel
    let render: RenderJobState

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Circle().fill(statusColor).frame(width: 8, height: 8)
                Text(render.outputFilename).font(.system(.body, design: .monospaced))
                Spacer()
                if render.isAdopted {
                    Text("adopted")
                        .font(.caption.bold())
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(.purple.opacity(0.2))
                        .foregroundColor(.purple)
                        .clipShape(Capsule())
                }
                Text(render.status.rawValue).font(.caption).foregroundStyle(.secondary)
                pauseResumeButton
                stopButton
            }
            HStack(spacing: 8) {
                if render.isAdopted && render.status != .paused {
                    ProgressView().progressViewStyle(.linear)
                } else {
                    ProgressView(value: render.progress).progressViewStyle(.linear)
                }
                if render.isAdopted {
                    Text("? frames").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                } else {
                    Text("\(render.frame)/\(render.totalFrames)")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    if render.fps > 0 {
                        Text("\(render.fps, specifier: "%.1f") fps")
                            .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    }
                }
            }
            if let message = render.message, render.status == .failed || render.status == .cancelled {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(render.status == .failed ? Color.red : Color.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text("\(render.exportPath)  ·  \(render.variantKey)")
                .font(.caption2).foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private var pauseResumeButton: some View {
        switch render.status {
        case .paused:
            Button { viewModel.resumeRender(outputPath: render.id) } label: {
                Image(systemName: "play.circle")
            }
            .buttonStyle(.borderless).help("Resume (SIGCONT)")
        case .running:
            Button { viewModel.pauseRender(outputPath: render.id) } label: {
                Image(systemName: "pause.circle")
            }
            .buttonStyle(.borderless).help("Pause (SIGSTOP)")
        case .queued:
            Button { viewModel.pauseRender(outputPath: render.id) } label: {
                Image(systemName: "play.circle")
            }
            .buttonStyle(.borderless).help("Pause (don't promote)")
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
            .buttonStyle(.borderless).help("Stop and cancel render")
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

// MARK: - Failed render row

/// A render that did not produce a video, with the diagnosis inline and the
/// tool output one click away.  Failures used to be visible only as a red dot
/// on a row that `cleanupFinishedRenders()` swept away seconds later, so in
/// practice they were invisible.
struct FailureRow: View {
    @Environment(AppViewModel.self) var viewModel
    let failure: RenderFailure
    @State private var expanded = false

    static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "h:mm:ss a"
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                Text(Self.timeFormatter.string(from: failure.timestamp))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                Text(failure.kind.label)
                    .font(.caption.bold())
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(.red.opacity(0.18))
                    .foregroundColor(.red)
                    .clipShape(Capsule())
                Text(failure.outputFilename)
                    .font(.system(.body, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                if failure.occurrences > 1 {
                    Text("x\(failure.occurrences)")
                        .font(.caption.monospacedDigit().bold())
                        .foregroundStyle(.red)
                        .help("This render has failed the same way \(failure.occurrences) times")
                }
                Button {
                    copyToPasteboard()
                } label: {
                    Image(systemName: "doc.on.doc")
                }
                .buttonStyle(.borderless)
                .help("Copy the full failure report")

                Button {
                    viewModel.clearFailure(outputPath: failure.outputPath)
                } label: {
                    Image(systemName: "xmark.circle")
                }
                .buttonStyle(.borderless)
                .help("Dismiss (it will come back if the render fails again)")
            }

            Text(failure.summary)
                .font(.callout)
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)

            if let detail = failure.detail, !detail.isEmpty {
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if !failure.toolOutput.isEmpty || failure.commandLine != nil {
                DisclosureGroup(isExpanded: $expanded) {
                    VStack(alignment: .leading, spacing: 6) {
                        if !failure.toolOutput.isEmpty {
                            Text(failure.toolOutput.joined(separator: "\n"))
                                .font(.caption2.monospaced())
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if let cmd = failure.commandLine {
                            Text(cmd)
                                .font(.caption2.monospaced())
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(.top, 4)
                } label: {
                    Text(expanded ? "Hide ffmpeg output" : "Show ffmpeg output and command")
                        .font(.caption)
                }
            }

            Text("\(failure.sequenceName)  ·  \(failure.variantKey)")
                .font(.caption2).foregroundStyle(.secondary)
        }
        .padding(.vertical, 6)
    }

    private func copyToPasteboard() {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(failure.consoleReport, forType: .string)
    }
}
