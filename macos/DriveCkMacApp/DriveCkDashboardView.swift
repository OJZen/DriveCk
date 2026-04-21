import Charts
import Observation
import SwiftUI

struct DriveCkDashboardView: View {
    @Bindable var viewModel: DriveCkAppViewModel

    var body: some View {
        Group {
            if let target = viewModel.displayedTarget {
                detailContent(for: target)
            } else {
                ContentUnavailableView(
                    "No disk selected",
                    systemImage: "externaldrive.badge.plus",
                    description: Text("Refresh and choose a disk.")
                )
            }
        }
        .background {
            LinearGradient(
                colors: [
                    Color.accentColor.opacity(0.07),
                    Color(nsColor: .windowBackgroundColor),
                    Color(nsColor: .windowBackgroundColor),
                ],
                startPoint: .topTrailing,
                endPoint: .bottomLeading
            )
            .ignoresSafeArea()
        }
    }

    private func detailContent(for target: DriveCkTargetInfo) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                deviceOverviewCard(target: target)

                if let inlineError = viewModel.inlineError {
                    errorBanner(for: inlineError)
                        .transition(.move(edge: .top).combined(with: .opacity))
                }

                if let statuses = viewModel.displayedMapStatuses, !statuses.isEmpty {
                    validationMapCard(
                        statuses: statuses,
                        completed: viewModel.displayedMapCompletedSamples,
                        total: max(viewModel.displayedMapTotalSamples, 1),
                        phase: viewModel.displayedMapPhase,
                        offsets: viewModel.displayedMapOffsets,
                        highlightedIndex: viewModel.displayedMapHighlightedSampleIndex
                    )
                } else {
                    placeholderCard
                }

                if let response = viewModel.currentResponse {
                    timingCard(report: response.report)
                    reportCard
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(20)
        }
    }

    private func deviceOverviewCard(target: DriveCkTargetInfo) -> some View {
        DriveCkCard {
            VStack(alignment: .leading, spacing: 18) {
                HStack(alignment: .top, spacing: 16) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .fill(Color.accentColor.opacity(0.12))
                        Image(systemName: target.isUsb ? "externaldrive.badge.connected.to.line.below" : "externaldrive")
                            .font(.system(size: 28, weight: .medium))
                            .foregroundStyle(Color.accentColor)
                    }
                    .frame(width: 56, height: 56)

                    VStack(alignment: .leading, spacing: 8) {
                        Text(target.displayName)
                            .font(.title2.weight(.semibold))
                        Text(target.subtitle)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        HStack(spacing: 8) {
                            DriveCkStatusBadge(
                                text: target.transportLabel.isEmpty ? "External" : target.transportLabel,
                                tint: .blue
                            )
                            if target.isRemovable {
                                DriveCkStatusBadge(text: "Removable", tint: .purple)
                            }
                            DriveCkStatusBadge(text: target.readinessLabel, tint: target.isMounted ? .orange : .green)
                        }
                    }

                    Spacer(minLength: 12)

                    VStack(alignment: .trailing, spacing: 4) {
                        Text(target.shortPath)
                            .font(.headline)
                            .fontDesign(.monospaced)
                            .textSelection(.enabled)
                        Text(target.path)
                            .font(.caption)
                            .fontDesign(.monospaced)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }

                adaptiveMetricGrid(minimum: 160, maximum: 240) {
                    DriveCkMetricTile(title: "Capacity", value: driveCkFormatBytes(target.sizeBytes))
                    DriveCkMetricTile(title: "Block", value: target.blockSizeLabel)
                }

                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .center, spacing: 12) {
                        statusStrip
                        validationActionButton
                            .frame(width: 220)
                    }

                    VStack(alignment: .leading, spacing: 12) {
                        statusStrip
                        validationActionButton
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var statusStrip: some View {
        HStack(spacing: 10) {
            Image(systemName: statusSymbolName)
                .foregroundStyle(statusTint)
            Text(viewModel.statusLine)
                .font(.callout)
                .foregroundStyle(.primary)
                .lineLimit(2)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(statusTint.opacity(0.12), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private var validationActionButton: some View {
        Button {
            if viewModel.isRunning {
                viewModel.cancelValidation()
            } else {
                viewModel.startValidation()
            }
        } label: {
            if viewModel.isCancelling {
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Stopping…")
                }
                .frame(maxWidth: .infinity)
            } else {
                Label(
                    viewModel.isRunning ? "Stop Validation" : "Start Validation",
                    systemImage: viewModel.isRunning ? "stop.fill" : "play.fill"
                )
                .frame(maxWidth: .infinity)
            }
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .disabled(
            viewModel.isCancelling
                || (!viewModel.isRunning && !viewModel.canStartValidation)
        )
    }

    private func errorBanner(for error: DriveCkUserFacingError) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: error.title == "Validation cancelled" ? "pause.circle.fill" : "exclamationmark.triangle.fill")
                .font(.title3)
                .foregroundStyle(error.title == "Validation cancelled" ? .yellow : .orange)
            VStack(alignment: .leading, spacing: 4) {
                Text(error.title)
                    .font(.headline)
                Text(error.message)
                    .font(.callout)
                Text(error.suggestion)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(16)
        .background(.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(.orange.opacity(0.22), lineWidth: 1)
        }
    }

    private func validationMapCard(
        statuses: [DriveCkSampleStatus],
        completed: Int,
        total: Int,
        phase: String,
        offsets: [UInt64]?,
        highlightedIndex: Int?
    ) -> some View {
        let columns = Array(repeating: GridItem(.fixed(11), spacing: 4), count: driveCkMapColumnCount)
        let entries = mapEntries(for: statuses, offsets: offsets)
        return DriveCkCard {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Validation Map")
                        .font(.headline)
                    Spacer()
                    DriveCkStatusBadge(
                        text: phase,
                        tint: phaseTint(for: phase)
                    )
                    Text("\(completed) / \(total)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }

                let showsLiveMap = offsets == nil

                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 18) {
                        if showsLiveMap {
                            DriveCkLiveMapView(
                                statuses: statuses,
                                highlightedIndex: highlightedIndex,
                                colorProvider: color(for:)
                            )
                        } else {
                            DriveCkMapGridView(
                                columns: columns,
                                entries: entries,
                                colorProvider: color(for:),
                                helpProvider: mapHelp(for:)
                            )
                        }
                        legend(statuses: statuses)
                    }
                    VStack(alignment: .leading, spacing: 14) {
                        if showsLiveMap {
                            DriveCkLiveMapView(
                                statuses: statuses,
                                highlightedIndex: highlightedIndex,
                                colorProvider: color(for:)
                            )
                        } else {
                            DriveCkMapGridView(
                                columns: columns,
                                entries: entries,
                                colorProvider: color(for:),
                                helpProvider: mapHelp(for:)
                            )
                        }
                        legend(statuses: statuses)
                    }
                }
            }
        }
    }

    private func legend(statuses: [DriveCkSampleStatus]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            legendItem(label: "OK", count: count(of: .Ok, in: statuses), status: .Ok)
            legendItem(label: "Read", count: count(of: .ReadError, in: statuses), status: .ReadError)
            legendItem(label: "Write", count: count(of: .WriteError, in: statuses), status: .WriteError)
            legendItem(label: "Mismatch", count: count(of: .VerifyMismatch, in: statuses), status: .VerifyMismatch)
            legendItem(label: "Restore", count: count(of: .RestoreError, in: statuses), status: .RestoreError)
            legendItem(label: "Untested", count: count(of: .Untested, in: statuses), status: .Untested)
        }
    }

    private func legendItem(label: String, count: Int, status: DriveCkSampleStatus) -> some View {
        HStack(spacing: 8) {
            RoundedRectangle(cornerRadius: 3, style: .continuous)
                .fill(color(for: status))
                .frame(width: 10, height: 10)
            Text(label)
                .font(.caption)
            Spacer()
            Text("\(count)")
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: 150)
    }

    private func timingCard(report: DriveCkValidationReport) -> some View {
        let readPoints = report.readTimings.values.enumerated().map { (index: $0.offset, value: $0.element) }
        let writePoints = report.writeTimings.values.enumerated().map { (index: $0.offset, value: $0.element) }
        return DriveCkCard {
            VStack(alignment: .leading, spacing: 16) {
                Text("Timing")
                    .font(.headline)

                Chart {
                    ForEach(readPoints, id: \.index) { point in
                        LineMark(
                            x: .value("Sample", point.index),
                            y: .value("Milliseconds", point.value)
                        )
                        .foregroundStyle(.mint)
                        .interpolationMethod(.catmullRom)
                    }

                    ForEach(writePoints, id: \.index) { point in
                        LineMark(
                            x: .value("Sample", point.index),
                            y: .value("Milliseconds", point.value)
                        )
                        .foregroundStyle(.blue)
                        .interpolationMethod(.catmullRom)
                    }
                }
                .frame(height: 200)
                .chartLegend(position: .top, spacing: 12)

                adaptiveMetricGrid(minimum: 240, maximum: 360) {
                    timingSummary(label: "Read", summary: report.readSummary, tint: .mint)
                    timingSummary(label: "Write", summary: report.writeSummary, tint: .blue)
                }
            }
        }
    }

    private func timingSummary(label: String, summary: DriveCkTimingSummary, tint: Color) -> some View {
        DriveCkMetricTile(
            title: label,
            value: String(format: "%.3f ms", summary.meanMs),
            secondary: "\(String(format: "%.2f MiB/s", summary.throughputMiBS)) · \(summary.count) ops",
            tint: tint,
            monospaced: true
        )
    }

    private var reportCard: some View {
        DriveCkCard {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("Text Report")
                        .font(.headline)
                    Spacer()
                    ControlGroup {
                        Button {
                            viewModel.copyReportToPasteboard()
                        } label: {
                            Image(systemName: "doc.on.doc")
                        }
                        .help("Copy report")
                        .disabled(!viewModel.canCopyReport)

                        Button {
                            viewModel.exportReport()
                        } label: {
                            Image(systemName: "square.and.arrow.up")
                        }
                        .help("Export report")
                        .disabled(!viewModel.canExportReport)
                    }
                }

                DriveCkReportTextView(text: viewModel.reportText)
                    .frame(minHeight: 250)
                    .background(
                        RoundedRectangle(cornerRadius: 16, style: .continuous)
                            .fill(Color.primary.opacity(0.05))
                    )
            }
        }
    }

    private var placeholderCard: some View {
        DriveCkCard {
            ContentUnavailableView(
                "No validation report",
                systemImage: "waveform.path.ecg.rectangle",
                description: Text("Start a run to populate the dashboard.")
            )
            .frame(maxWidth: .infinity, minHeight: 240)
        }
    }

    private var statusSymbolName: String {
        switch viewModel.validationState {
        case .idle:
            return viewModel.selectedTargetID == nil ? "externaldrive.badge.plus" : "checkmark.circle.fill"
        case .preparing:
            return "gearshape.2.fill"
        case .running:
            return viewModel.isCancelling ? "stop.circle.fill" : "bolt.horizontal.circle.fill"
        case let .finished(result):
            return result.error == nil ? "checkmark.circle.fill" : "exclamationmark.triangle.fill"
        }
    }

    private var statusTint: Color {
        switch viewModel.validationState {
        case .idle:
            return viewModel.selectedTargetID == nil ? .secondary : .green
        case .preparing:
            return .blue
        case .running:
            return viewModel.isCancelling ? .orange : .accentColor
        case let .finished(result):
            return result.error == nil ? .green : .orange
        }
    }

    private func color(for status: DriveCkSampleStatus) -> Color {
        switch status {
        case .Ok:
            return .green
        case .ReadError:
            return .orange
        case .WriteError:
            return .purple
        case .VerifyMismatch:
            return .red
        case .RestoreError:
            return .red.opacity(0.65)
        case .Untested:
            return .gray.opacity(0.45)
        }
    }

    private func count(of status: DriveCkSampleStatus, in statuses: [DriveCkSampleStatus]) -> Int {
        statuses.reduce(into: 0) { partial, current in
            if current == status {
                partial += 1
            }
        }
    }

    private func mapEntries(for statuses: [DriveCkSampleStatus], offsets: [UInt64]?) -> [DriveCkMapEntry] {
        statuses.enumerated().map { index, status in
            DriveCkMapEntry(
                id: index,
                index: index,
                offset: offsets?[safe: index] ?? 0,
                status: status
            )
        }
    }

    private func mapHelp(for entry: DriveCkMapEntry) -> String {
        if entry.offset == 0 {
            return "Region \(entry.index) — \(entry.status.displayName)"
        }
        return "Region \(entry.index) @ \(driveCkFormatBytes(entry.offset)) — \(entry.status.displayName)"
    }

    private func phaseTint(for phase: String) -> Color {
        switch phase {
        case "Preparing":
            return .blue
        case "Validating":
            return viewModel.isCancelling ? .orange : .accentColor
        case "Completed":
            return .green
        case "Finished with issues":
            return .orange
        default:
            return .secondary
        }
    }

    private func adaptiveMetricGrid<Content: View>(
        minimum: CGFloat,
        maximum: CGFloat,
        @ViewBuilder content: () -> Content
    ) -> some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: minimum, maximum: maximum), spacing: 10, alignment: .top)],
            alignment: .leading,
            spacing: 10,
            content: content
        )
    }
}

private struct DriveCkMapCell: View {
    var status: DriveCkSampleStatus
    var color: Color

    var body: some View {
        RoundedRectangle(cornerRadius: 4, style: .continuous)
            .fill(color)
            .opacity(status == .Untested ? 0.72 : 1.0)
            .scaleEffect(status == .Untested ? 0.96 : 1.0)
            .frame(width: 11, height: 11)
    }
}

private struct DriveCkLiveMapView: View {
    private let columns = driveCkMapColumnCount
    private let cellSize: CGFloat = 11
    private let cellSpacing: CGFloat = 4

    var statuses: [DriveCkSampleStatus]
    var highlightedIndex: Int?
    var colorProvider: (DriveCkSampleStatus) -> Color

    @State private var activePulseIndex: Int?
    @State private var pulseProgress = 1.0
    @State private var clearPulseTask: Task<Void, Never>?

    var body: some View {
        Canvas(opaque: false, colorMode: .linear, rendersAsynchronously: true) { context, _ in
            for (index, status) in statuses.enumerated() {
                let rect = rect(for: index)
                let fillColor = colorProvider(status).opacity(status == .Untested ? 0.72 : 1.0)
                context.fill(
                    Path(roundedRect: rect, cornerRadius: status == .Untested ? 3.8 : 4),
                    with: .color(fillColor)
                )
            }

            if let activePulseIndex,
               statuses.indices.contains(activePulseIndex)
            {
                let pulseRect = rect(for: activePulseIndex)
                let inset = (cellSize * 0.42 * pulseProgress) / 2
                let expandedRect = pulseRect.insetBy(dx: -inset, dy: -inset)
                let pulseColor = colorProvider(statuses[activePulseIndex]).opacity((1 - pulseProgress) * 0.85)
                context.stroke(
                    Path(roundedRect: expandedRect, cornerRadius: 4 + (2 * pulseProgress)),
                    with: .color(pulseColor),
                    lineWidth: 1.2
                )
            }
        }
        .frame(width: canvasWidth, height: canvasHeight, alignment: .topLeading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilitySummary)
        .onChange(of: highlightedIndex, initial: true) { _, newValue in
            triggerPulse(for: newValue)
        }
        .onDisappear {
            clearPulseTask?.cancel()
        }
    }

    private var canvasWidth: CGFloat {
        guard !statuses.isEmpty else {
            return cellSize
        }
        return (CGFloat(columns) * cellSize) + (CGFloat(columns - 1) * cellSpacing)
    }

    private var canvasHeight: CGFloat {
        let rowCount = max(Int(ceil(Double(max(statuses.count, 1)) / Double(columns))), 1)
        return (CGFloat(rowCount) * cellSize) + (CGFloat(rowCount - 1) * cellSpacing)
    }

    private var accessibilitySummary: String {
        let completed = statuses.reduce(into: 0) { partial, status in
            if status != .Untested {
                partial += 1
            }
        }
        return "Validation map, \(completed) of \(statuses.count) regions completed."
    }

    private func rect(for index: Int) -> CGRect {
        let row = index / columns
        let column = index % columns
        let x = CGFloat(column) * (cellSize + cellSpacing)
        let y = CGFloat(row) * (cellSize + cellSpacing)
        return CGRect(x: x, y: y, width: cellSize, height: cellSize)
    }

    private func triggerPulse(for index: Int?) {
        clearPulseTask?.cancel()

        guard let index, statuses.indices.contains(index) else {
            activePulseIndex = nil
            pulseProgress = 1
            return
        }

        activePulseIndex = index
        pulseProgress = 0

        withAnimation(.easeOut(duration: 0.14)) {
            pulseProgress = 1
        }

        clearPulseTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 140_000_000)
            guard !Task.isCancelled, activePulseIndex == index else {
                return
            }
            activePulseIndex = nil
            pulseProgress = 1
        }
    }
}

private struct DriveCkMapGridView: View {
    var columns: [GridItem]
    var entries: [DriveCkMapEntry]
    var colorProvider: (DriveCkSampleStatus) -> Color
    var helpProvider: (DriveCkMapEntry) -> String

    var body: some View {
        LazyVGrid(columns: columns, spacing: 4) {
            ForEach(entries) { entry in
                let cell = DriveCkMapCell(status: entry.status, color: colorProvider(entry.status))
                .accessibilityHidden(true)

                cell.help(helpProvider(entry))
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilitySummary)
    }

    private var accessibilitySummary: String {
        let completed = entries.reduce(into: 0) { partial, entry in
            if entry.status != .Untested {
                partial += 1
            }
        }
        return "Validation map, \(completed) of \(entries.count) regions completed."
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
