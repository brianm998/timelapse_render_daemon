import SwiftUI
import TimelapseRenderCore

struct ConfigEditorView: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        @Bindable var vm = viewModel
        if vm.config == nil {
            VStack {
                Spacer()
                Text("Open a config file to edit it")
                    .foregroundStyle(.secondary)
                Button("Open Config…") { vm.pickConfig() }
                Spacer()
            }
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    pathListSection(title: "Source paths", keyPath: \.sourcePaths)
                    pathListSection(title: "Backup paths", keyPath: \.backupPaths)
                    pathListSection(title: "Export paths", keyPath: \.exportPaths)
                    pathListSection(title: "Master video paths", keyPath: \.masterVideos)

                    GroupBox("Naming") {
                        VStack(alignment: .leading) {
                            HStack {
                                Text("Image name prefix").frame(width: 220, alignment: .leading)
                                TextField("LRT_", text: bind(\.imageNamePrefix, default: ""))
                            }
                            HStack {
                                Text("Image sequence dirname prefix").frame(width: 220, alignment: .leading)
                                TextField("LRT_", text: bind(\.imageSequenceDirnamePrefix, default: ""))
                            }
                        }
                    }

                    GroupBox("Concurrency") {
                        VStack(alignment: .leading) {
                            HStack {
                                Text("Max ffmpeg processes").frame(width: 220, alignment: .leading)
                                Stepper(value: bind(\.maxProcesses.ffmpeg, default: 1), in: 1...64) {
                                    Text("\(viewModel.config?.maxProcesses.ffmpeg ?? 1)")
                                }
                            }
                            HStack {
                                Text("Max exiftool processes").frame(width: 220, alignment: .leading)
                                Stepper(value: bind(\.maxProcesses.exiftool, default: 10), in: 1...256) {
                                    Text("\(viewModel.config?.maxProcesses.exiftool ?? 10)")
                                }
                            }
                            HStack {
                                Text("Max concurrent renders").frame(width: 220, alignment: .leading)
                                Stepper(value: bind(\.maxConcurrentRenders, default: 1), in: 1...16) {
                                    Text("\(viewModel.config?.maxConcurrentRenders ?? 1)")
                                }
                            }
                            HStack {
                                Text("Scan interval (seconds)").frame(width: 220, alignment: .leading)
                                TextField("2", value: bind(\.scanIntervalSeconds, default: 2), format: .number)
                                    .frame(width: 80)
                            }
                        }
                    }

                    RawTypesEditor()
                    RenderingEditor()
                    RegexListEditor(title: "Ignore sequence regexes", keyPath: \.ignoreSequenceRegexes)
                    RegexListEditor(title: "Delete sequence after render regexes", keyPath: \.deleteSequenceAfterRenderRegexes)

                    HStack {
                        Spacer()
                        Button("Reload from disk") {
                            if let path = vm.configPath { vm.loadConfig(at: path) }
                        }
                        Button("Save & Apply") {
                            vm.saveConfig()
                        }
                        .keyboardShortcut("s")
                        .buttonStyle(.borderedProminent)
                    }
                }
                .padding()
            }
        }
    }

    @ViewBuilder
    private func pathListSection(
        title: String,
        keyPath: WritableKeyPath<DaemonConfig, [String]>
    ) -> some View {
        GroupBox(title) {
            PathListEditor(keyPath: keyPath)
        }
    }

    private func bind<Value>(
        _ keyPath: WritableKeyPath<DaemonConfig, Value>,
        default defaultValue: Value
    ) -> Binding<Value> {
        Binding(
            get: { viewModel.config?[keyPath: keyPath] ?? defaultValue },
            set: { newValue in
                guard var c = viewModel.config else { return }
                c[keyPath: keyPath] = newValue
                viewModel.config = c
            }
        )
    }
}

struct PathListEditor: View {
    @Environment(AppViewModel.self) var viewModel
    let keyPath: WritableKeyPath<DaemonConfig, [String]>

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(values.enumerated()), id: \.offset) { idx, _ in
                HStack {
                    TextField("path", text: pathBinding(at: idx))
                        .textFieldStyle(.roundedBorder)
                    Button {
                        var v = values
                        v.remove(at: idx)
                        write(v)
                    } label: {
                        Image(systemName: "minus.circle")
                    }
                    .buttonStyle(.borderless)
                }
            }
            HStack {
                Spacer()
                Button {
                    var v = values
                    v.append("")
                    write(v)
                } label: {
                    Label("Add path", systemImage: "plus")
                }
                .buttonStyle(.borderless)
            }
        }
    }

    private var values: [String] { viewModel.config?[keyPath: keyPath] ?? [] }

    private func write(_ v: [String]) {
        guard var c = viewModel.config else { return }
        c[keyPath: keyPath] = v
        viewModel.config = c
    }

    private func pathBinding(at idx: Int) -> Binding<String> {
        Binding(
            get: { values.indices.contains(idx) ? values[idx] : "" },
            set: { newValue in
                var v = values
                if v.indices.contains(idx) { v[idx] = newValue; write(v) }
            }
        )
    }
}

struct RawTypesEditor: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        GroupBox("Raw image types") {
            VStack(alignment: .leading) {
                Text(viewModel.config?.rawTypes.joined(separator: ", ") ?? "")
                    .font(.body.monospaced())
                TextField("comma-separated extensions (ARW, ORF, DNG)", text: rawTypesString)
                    .textFieldStyle(.roundedBorder)
            }
        }
    }

    private var rawTypesString: Binding<String> {
        Binding(
            get: { viewModel.config?.rawTypes.joined(separator: ", ") ?? "" },
            set: { newValue in
                guard var c = viewModel.config else { return }
                c.rawTypes = newValue
                    .split(separator: ",")
                    .map { $0.trimmingCharacters(in: .whitespaces) }
                    .filter { !$0.isEmpty }
                viewModel.config = c
            }
        )
    }
}

struct RenderingEditor: View {
    @Environment(AppViewModel.self) var viewModel

    var body: some View {
        GroupBox("Rendering variants") {
            VStack(alignment: .leading) {
                ForEach(Array(rendering.enumerated()), id: \.offset) { idx, variant in
                    HStack {
                        Text("\(idx + 1).").frame(width: 24)
                        codecPicker(at: idx, current: variant.codec)
                        TextField("frame rate", value: frameRateBinding(at: idx), format: .number)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 60)
                        TextField("width", value: widthBinding(at: idx), format: .number)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 80)
                        TextField("height", value: heightBinding(at: idx), format: .number)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 80)
                        Button { remove(at: idx) } label: { Image(systemName: "minus.circle") }
                            .buttonStyle(.borderless)
                    }
                }
                if !unknownCodecs.isEmpty {
                    Label(
                        "Unknown codec\(unknownCodecs.count == 1 ? "" : "s") \(unknownCodecs.joined(separator: ", ")) — not in the codec table, so these variants will never render. Pick a known codec or add an entry to video_codecs.json next to the config file.",
                        systemImage: "exclamationmark.triangle.fill"
                    )
                    .font(.caption)
                    .foregroundStyle(.orange)
                }
                Button {
                    var v = rendering
                    v.append(RenderType(codec: viewModel.codecNames.first ?? "ProResMQ", frameRate: 30))
                    write(v)
                } label: {
                    Label("Add variant", systemImage: "plus")
                }
                .buttonStyle(.borderless)
            }
        }
    }

    /// Picker over the known codec-table keys.  A codec name that isn't in
    /// the table (e.g. hand-edited config, or the table changed) is kept as
    /// an extra selectable entry so the current value stays visible, and is
    /// flagged with a warning badge.
    @ViewBuilder
    private func codecPicker(at idx: Int, current: String) -> some View {
        let known = viewModel.codecNames
        let isUnknown = !known.contains(current)
        Picker("codec", selection: codecBinding(at: idx)) {
            ForEach(known, id: \.self) { name in
                Text(name).tag(name)
            }
            if isUnknown {
                Text("\(current) (unknown)").tag(current)
            }
        }
        .labelsHidden()
        .frame(width: 140)
        if isUnknown {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
                .help("Codec '\(current)' is not in the codec table — this variant will not render.")
        }
    }

    /// Codec names used by variants that don't exist in the codec table.
    private var unknownCodecs: [String] {
        let known = Set(viewModel.codecNames)
        return Array(Set(rendering.map(\.codec).filter { !known.contains($0) })).sorted()
    }

    private var rendering: [RenderType] { viewModel.config?.rendering ?? [] }
    private func write(_ v: [RenderType]) {
        guard var c = viewModel.config else { return }
        c.rendering = v
        viewModel.config = c
    }
    private func remove(at idx: Int) {
        var v = rendering
        v.remove(at: idx)
        write(v)
    }
    private func codecBinding(at idx: Int) -> Binding<String> {
        Binding(
            get: { rendering.indices.contains(idx) ? rendering[idx].codec : "" },
            set: { val in
                var v = rendering
                if v.indices.contains(idx) { v[idx].codec = val; write(v) }
            }
        )
    }
    private func frameRateBinding(at idx: Int) -> Binding<Int> {
        Binding(
            get: { rendering.indices.contains(idx) ? rendering[idx].frameRate : 30 },
            set: { val in
                var v = rendering
                if v.indices.contains(idx) { v[idx].frameRate = val; write(v) }
            }
        )
    }
    private func widthBinding(at idx: Int) -> Binding<Int?> {
        Binding(
            get: { rendering.indices.contains(idx) ? rendering[idx].width : nil },
            set: { val in
                var v = rendering
                guard v.indices.contains(idx) else { return }
                if let val {
                    let h = v[idx].height ?? 0
                    v[idx].resolution = [val, h]
                } else {
                    v[idx].resolution = nil
                }
                write(v)
            }
        )
    }
    private func heightBinding(at idx: Int) -> Binding<Int?> {
        Binding(
            get: { rendering.indices.contains(idx) ? rendering[idx].height : nil },
            set: { val in
                var v = rendering
                guard v.indices.contains(idx) else { return }
                if let val {
                    let w = v[idx].width ?? 0
                    v[idx].resolution = [w, val]
                } else {
                    v[idx].resolution = nil
                }
                write(v)
            }
        )
    }
}

struct RegexListEditor: View {
    @Environment(AppViewModel.self) var viewModel
    let title: String
    let keyPath: WritableKeyPath<DaemonConfig, [String]>

    var body: some View {
        GroupBox(title) {
            VStack(alignment: .leading) {
                ForEach(Array(values.enumerated()), id: \.offset) { idx, _ in
                    HStack {
                        TextField("regex", text: rowBinding(at: idx))
                            .textFieldStyle(.roundedBorder)
                            .font(.body.monospaced())
                        Button {
                            var v = values; v.remove(at: idx); write(v)
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.borderless)
                    }
                }
                Button {
                    var v = values; v.append(""); write(v)
                } label: {
                    Label("Add regex", systemImage: "plus")
                }
                .buttonStyle(.borderless)
            }
        }
    }

    private var values: [String] { viewModel.config?[keyPath: keyPath] ?? [] }
    private func write(_ v: [String]) {
        guard var c = viewModel.config else { return }
        c[keyPath: keyPath] = v
        viewModel.config = c
    }
    private func rowBinding(at idx: Int) -> Binding<String> {
        Binding(
            get: { values.indices.contains(idx) ? values[idx] : "" },
            set: { val in
                var v = values
                if v.indices.contains(idx) { v[idx] = val; write(v) }
            }
        )
    }
}
