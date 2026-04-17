import Charts
import Observation
import SwiftUI

struct DriveCkAppShellView: View {
    @Bindable var viewModel: DriveCkAppViewModel

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar {
            ToolbarItemGroup {
                Button {
                    Task {
                        await viewModel.refreshTargets()
                    }
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .keyboardShortcut("r", modifiers: [.command])
                .disabled(viewModel.isRefreshing || viewModel.isRunning)

                Button {
                    viewModel.exportReport()
                } label: {
                    Label("Export Report", systemImage: "square.and.arrow.up")
                }
                .disabled(!viewModel.canExportReport)
            }
        }
        .alert(item: $viewModel.presentedError) { error in
            Alert(
                title: Text(error.title),
                message: Text("\(error.message)\n\n\(error.suggestion)"),
                dismissButton: .default(Text("OK"))
            )
        }
        .animation(.snappy(duration: 0.28), value: viewModel.selectedTargetID)
        .animation(.snappy(duration: 0.28), value: viewModel.statusLine)
    }

    private var sidebar: some View {
        List(selection: Binding(
            get: { viewModel.selectedTargetID },
            set: { viewModel.selectTarget($0) }
        )) {
            Section {
                if viewModel.targets.isEmpty {
                    ContentUnavailableView(
                        "No removable disks",
                        systemImage: "externaldrive.badge.questionmark",
                        description: Text("Plug in a removable disk, unmount it if needed, then refresh.")
                    )
                    .frame(maxWidth: .infinity, minHeight: 220)
                    .listRowSeparator(.hidden)
                } else {
                    ForEach(viewModel.targets) { target in
                        DeviceRow(target: target)
                            .tag(Optional.some(target.id))
                            .padding(.vertical, 4)
                    }
                }
            } header: {
                Text("Eligible Disks")
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .top) {
            VStack(alignment: .leading, spacing: 8) {
                Text("DriveCk")
                    .font(.title2.weight(.semibold))
                Text("Validate removable storage without leaving macOS.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 14)
            .padding(.bottom, 10)
            .background(.bar)
        }
    }

    @ViewBuilder
    private var detail: some View {
        if let target = viewModel.selectedTarget {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    heroCard(target: target)
                    if let inlineError = viewModel.inlineError {
                        banner(for: inlineError)
                            .transition(.move(edge: .top).combined(with: .opacity))
                    }
                    controlCard(target: target)
                    if viewModel.isRunning {
                        progressCard
                            .transition(.move(edge: .top).combined(with: .opacity))
                    }
                    if let response = viewModel.currentResponse {
                        summaryCard(response: response)
                            .transition(.scale(scale: 0.98).combined(with: .opacity))
                        mapCard(report: response.report)
                        timingCard(report: response.report)
                        reportCard
                    } else {
                        placeholderCard
                    }
                }
                .padding(24)
            }
            .background(Color(nsColor: .windowBackgroundColor))
        } else {
            ContentUnavailableView(
                "No disk selected",
                systemImage: "externaldrive.badge.plus",
                description: Text("Refresh the disk list and choose a removable whole-disk target.")
            )
        }
    }

    private func heroCard(target: DriveCkTargetInfo) -> some View {
        DriveCkCard {
            HStack(alignment: .top, spacing: 18) {
                Image(systemName: target.isUsb ? "externaldrive.badge.connected.to.line.below" : "externaldrive")
                    .font(.system(size: 38, weight: .medium))
                    .foregroundStyle(target.isMounted ? Color.orange : Color.accentColor)
                    .frame(width: 56, height: 56)
                    .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 18, style: .continuous))

                VStack(alignment: .leading, spacing: 8) {
                    Text(target.displayName)
                        .font(.title2.weight(.semibold))
                    Text(target.subtitle)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    HStack(spacing: 8) {
                        statusBadge(text: target.transportLabel.isEmpty ? "External" : target.transportLabel, tint: .blue)
                        if target.isRemovable {
                            statusBadge(text: "Removable", tint: .purple)
                        }
                        statusBadge(text: target.isMounted ? "Mounted" : "Ready", tint: target.isMounted ? .orange : .green)
                    }
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 6) {
                    Text("Execution Path")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                    Text(target.path)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                }
            }
        }
    }

    private func banner(for error: DriveCkUserFacingError) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: error.title == "Validation cancelled" ? "pause.circle.fill" : "exclamationmark.triangle.fill")
                .foregroundStyle(error.title == "Validation cancelled" ? .yellow : .orange)
                .font(.title3)
            VStack(alignment: .leading, spacing: 4) {
                Text(error.title)
                    .font(.headline)
                Text(error.message)
                    .font(.subheadline)
                Text(error.suggestion)
                    .font(.footnote)
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

    private func controlCard(target: DriveCkTargetInfo) -> some View {
        DriveCkCard {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Validation Controls")
                            .font(.headline)
                        Text("Keep the disk unmounted while DriveCk samples and restores data.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button {
                        viewModel.copyReportToPasteboard()
                    } label: {
                        Label("Copy Report", systemImage: "doc.on.doc")
                    }
                    .disabled(viewModel.reportText.isEmpty)
                }

                HStack(alignment: .center, spacing: 16) {
                    Toggle("Custom seed", isOn: $viewModel.useCustomSeed)
                        .toggleStyle(.switch)
                        .frame(maxWidth: 220, alignment: .leading)
                    TextField("Optional seed", text: $viewModel.seedText)
                        .textFieldStyle(.roundedBorder)
                        .disabled(!viewModel.useCustomSeed || viewModel.isRunning)
                        .frame(maxWidth: 220)
                    Spacer()
                    Button {
                        if viewModel.isRunning {
                            viewModel.cancelValidation()
                        } else {
                            viewModel.startValidation()
                        }
                    } label: {
                        Label(
                            viewModel.isRunning ? "Stop Validation" : "Start Validation",
                            systemImage: viewModel.isRunning ? "stop.fill" : "play.fill"
                        )
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(!viewModel.isRunning && !viewModel.canStartValidation)
                }

                if viewModel.useCustomSeed {
                    Text(viewModel.seedText.isEmpty ? "Leave the field empty to let DriveCk derive a seed from the device and current time." : "Parsed seed will be passed directly to the Rust validator.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }

                Text(viewModel.statusLine)
                    .font(.callout)
                    .foregroundStyle(viewModel.isRunning ? .primary : .secondary)
            }
        }
    }

    private var progressCard: some View {
        let snapshot = viewModel.currentProgress
        return DriveCkCard {
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Validation in Progress")
                            .font(.headline)
                        Text(snapshot.phase)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text("\(snapshot.current)/\(snapshot.total)")
                        .font(.title3.monospacedDigit())
                        .contentTransition(.numericText())
                }

                ProgressView(value: snapshot.fraction)
                    .progressViewStyle(.linear)
                    .tint(.accentColor)
                    .scaleEffect(y: 1.25)

                Text("DriveCk updates phase and sample progress in real time. Cancelling preserves any partial report returned by the core.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func summaryCard(response: DriveCkValidationResponse) -> some View {
        let report = response.report
        return DriveCkCard {
            VStack(alignment: .leading, spacing: 18) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Validation Result")
                            .font(.headline)
                        Text(report.verdict)
                            .font(.title3.weight(.semibold))
                    }
                    Spacer()
                    statusBadge(
                        text: report.hasFailures ? "Needs attention" : "Healthy",
                        tint: report.hasFailures ? .orange : .green
                    )
                }

                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 3), spacing: 12) {
                    summaryMetric(title: "Declared Size", value: driveCkFormatBytes(report.reportedSizeBytes))
                    summaryMetric(title: "Validated Size", value: driveCkFormatBytes(report.validatedDriveSizeBytes))
                    summaryMetric(title: "Highest Valid Region", value: driveCkFormatBytes(report.highestValidRegionBytes))
                    summaryMetric(title: "Samples Completed", value: "\(report.completedSamples)/\(report.sampleStatus.count)")
                    summaryMetric(title: "Failures", value: "\(report.failureCount)")
                    summaryMetric(title: "Seed", value: driveCkFormatSeed(report.seed))
                }

                HStack(spacing: 16) {
                    Text("Started \(driveCkFormatTimestamp(report.startedAt))")
                    Text("Finished \(driveCkFormatTimestamp(report.finishedAt))")
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
            }
        }
    }

    private func summaryMetric(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.title3.monospacedDigit().weight(.semibold))
                .contentTransition(.numericText())
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(.white.opacity(0.04), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private func mapCard(report: DriveCkValidationReport) -> some View {
        let columns = Array(repeating: GridItem(.fixed(12), spacing: 4), count: 24)
        return DriveCkCard {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Validation Map")
                        .font(.headline)
                    Spacer()
                    legend
                }

                LazyVGrid(columns: columns, spacing: 4) {
                    ForEach(report.mapEntries) { entry in
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .fill(color(for: entry.status))
                            .frame(width: 12, height: 12)
                            .help("Region \(entry.index) @ \(driveCkFormatBytes(entry.offset)) — \(entry.status.displayName)")
                    }
                }

                Text("The 24×24 grid mirrors the shared Rust report: green means verified, warm colors indicate failure modes, and gray indicates untested regions.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var legend: some View {
        HStack(spacing: 10) {
            legendItem(label: "OK", status: .Ok)
            legendItem(label: "Read", status: .ReadError)
            legendItem(label: "Write", status: .WriteError)
            legendItem(label: "Mismatch", status: .VerifyMismatch)
            legendItem(label: "Restore", status: .RestoreError)
            legendItem(label: "Untested", status: .Untested)
        }
    }

    private func legendItem(label: String, status: DriveCkSampleStatus) -> some View {
        HStack(spacing: 6) {
            RoundedRectangle(cornerRadius: 3, style: .continuous)
                .fill(color(for: status))
                .frame(width: 10, height: 10)
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func timingCard(report: DriveCkValidationReport) -> some View {
        let readPoints = report.readTimings.values.enumerated().map { (index: $0.offset, value: $0.element, series: "Read") }
        let writePoints = report.writeTimings.values.enumerated().map { (index: $0.offset, value: $0.element, series: "Write") }
        return DriveCkCard {
            VStack(alignment: .leading, spacing: 16) {
                Text("Timing Overview")
                    .font(.headline)

                Chart {
                    ForEach(readPoints, id: \.index) { point in
                        LineMark(
                            x: .value("Sample", point.index),
                            y: .value("Milliseconds", point.value)
                        )
                        .foregroundStyle(.green)
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
                .frame(height: 220)
                .chartLegend(position: .top, spacing: 12)

                HStack(spacing: 16) {
                    timingSummary(label: "Read", summary: report.readSummary)
                    timingSummary(label: "Write", summary: report.writeSummary)
                }
            }
        }
    }

    private func timingSummary(label: String, summary: DriveCkTimingSummary) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.headline)
            Text("ops \(summary.count) · mean \(String(format: "%.3f ms", summary.meanMs))")
                .font(.callout.monospacedDigit())
            Text("median \(String(format: "%.3f ms", summary.medianMs)) · throughput \(String(format: "%.2f MiB/s", summary.throughputMiBS))")
                .font(.footnote.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(.white.opacity(0.04), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private var reportCard: some View {
        DriveCkCard {
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    Text("Text Report")
                        .font(.headline)
                    Spacer()
                    Button {
                        viewModel.copyReportToPasteboard()
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    Button {
                        viewModel.exportReport()
                    } label: {
                        Label("Export", systemImage: "square.and.arrow.up")
                    }
                    .disabled(!viewModel.canExportReport)
                }

                DriveCkReportTextView(text: viewModel.reportText)
                    .frame(minHeight: 280)
                    .background(.black.opacity(0.10), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            }
        }
    }

    private var placeholderCard: some View {
        DriveCkCard {
            ContentUnavailableView(
                "No validation report yet",
                systemImage: "waveform.path.ecg.rectangle",
                description: Text("Pick a removable disk, review the safety state, then start a validation run to populate metrics and the report preview.")
            )
            .frame(maxWidth: .infinity, minHeight: 260)
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

    private func statusBadge(text: String, tint: Color) -> some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(tint.opacity(0.12), in: Capsule())
    }
}

private struct DeviceRow: View {
    var target: DriveCkTargetInfo
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: target.isUsb ? "externaldrive.badge.connected.to.line.below" : "externaldrive")
                .font(.title3)
                .frame(width: 30)
                .foregroundStyle(target.isMounted ? Color.orange : Color.accentColor)
            VStack(alignment: .leading, spacing: 4) {
                Text(target.displayName)
                    .font(.headline)
                Text(target.subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .background {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color.secondary.opacity(isHovering ? 0.12 : 0.0))
        }
        .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .onHover { hovering in
            withAnimation(.easeInOut(duration: 0.15)) {
                isHovering = hovering
            }
        }
    }
}
