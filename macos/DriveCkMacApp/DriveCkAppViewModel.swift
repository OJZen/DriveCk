import AppKit
import Foundation
import Observation

enum DriveCkValidationState {
    case idle
    case preparing
    case running(DriveCkProgressSnapshot)
    case finished(DriveCkValidationExecutionResult)
}

final class DriveCkCancellationFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false

    func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
    }

    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }
}

@MainActor
@Observable
final class DriveCkAppViewModel {
    var targets: [DriveCkTargetInfo] = []
    var selectedTargetID: String?
    var useCustomSeed = false
    var seedText = ""
    var isRefreshing = false
    var validationState: DriveCkValidationState = .idle
    var latestResult: DriveCkValidationExecutionResult?
    var reportText = ""
    var presentedError: DriveCkUserFacingError?
    var isCancelling = false
    var liveSampleStatus: [DriveCkSampleStatus] = []

    private var hasLoaded = false
    private var validationTask: Task<Void, Never>?
    private var cancellationFlag: DriveCkCancellationFlag?
    private var activeTarget: DriveCkTargetInfo?
    private var workspaceObservers: [NSObjectProtocol] = []

    init() {
        installWorkspaceObservers()
    }

    var selectedTarget: DriveCkTargetInfo? {
        targets.first(where: { $0.id == selectedTargetID })
    }

    var displayedTarget: DriveCkTargetInfo? {
        if let activeTarget {
            return activeTarget
        }
        return selectedTarget
    }

    var currentResponse: DriveCkValidationResponse? {
        latestResult?.response
    }

    var isRunning: Bool {
        switch validationState {
        case .idle, .finished:
            return false
        case .preparing, .running:
            return true
        }
    }

    var currentProgress: DriveCkProgressSnapshot {
        switch validationState {
        case .idle:
            return DriveCkProgressSnapshot(phase: "Idle", current: 0, total: 1, finalUpdate: false, sampleIndex: nil, sampleStatus: nil)
        case .preparing:
            return DriveCkProgressSnapshot(phase: "Preparing", current: 0, total: 1, finalUpdate: false, sampleIndex: nil, sampleStatus: nil)
        case let .running(snapshot):
            return snapshot
        case let .finished(result):
            if let response = result.response {
                return DriveCkProgressSnapshot(
                    phase: result.error == nil ? "Finished" : "Finished with issues",
                    current: response.report.completedSamples,
                    total: max(response.report.sampleStatus.count, 1),
                    finalUpdate: true,
                    sampleIndex: nil,
                    sampleStatus: nil
                )
            }
            return DriveCkProgressSnapshot(phase: "Finished", current: 1, total: 1, finalUpdate: true, sampleIndex: nil, sampleStatus: nil)
        }
    }

    var statusLine: String {
        switch validationState {
        case .idle:
            return selectedTarget == nil
                ? "Choose a USB whole-disk target."
                : "Ready. Admin approval is requested when validation starts."
        case .preparing:
            if isCancelling {
                return "Stopping validation…"
            }
            return "Preparing validation and unmounting if needed…"
        case let .running(snapshot):
            if isCancelling {
                return "Stopping validation… \(snapshot.current)/\(snapshot.total)"
            }
            return "\(snapshot.phase) \(snapshot.current)/\(snapshot.total)"
        case let .finished(result):
            if let response = result.response {
                if let error = result.error {
                    return "\(response.report.verdict) · \(error)"
                }
                return response.report.verdict
            }
            return result.error ?? "Validation finished."
        }
    }

    var canStartValidation: Bool {
        !isRunning && selectedTarget != nil
    }

    var canCancelValidation: Bool {
        isRunning && !isCancelling
    }

    var canExportReport: Bool {
        !isRunning && currentResponse != nil && !reportText.isEmpty
    }

    var canCopyReport: Bool {
        !reportText.isEmpty
    }

    var displayedMapStatuses: [DriveCkSampleStatus]? {
        if let response = currentResponse {
            return response.report.sampleStatus
        }
        return liveSampleStatus.isEmpty ? nil : liveSampleStatus
    }

    var displayedMapOffsets: [UInt64]? {
        currentResponse?.report.sampleOffsets
    }

    var displayedMapCompletedSamples: Int {
        if let response = currentResponse {
            return response.report.completedSamples
        }
        return liveSampleStatus.reduce(into: 0) { partial, status in
            if status != .Untested {
                partial += 1
            }
        }
    }

    var displayedMapTotalSamples: Int {
        if let response = currentResponse {
            return response.report.sampleStatus.count
        }
        return liveSampleStatus.count
    }

    var displayedMapPhase: String {
        if currentResponse != nil {
            return latestResult?.error == nil ? "Completed" : "Finished with issues"
        }
        return currentProgress.phase
    }

    var displayedMapHighlightedSampleIndex: Int? {
        guard currentResponse == nil else {
            return nil
        }
        guard case let .running(snapshot) = validationState,
              snapshot.phase == "Validating"
        else {
            return nil
        }
        return snapshot.sampleIndex
    }

    var inlineError: DriveCkUserFacingError? {
        if let error = latestResult?.error {
            return DriveCkUserFacingError.from(message: error)
        }
        return nil
    }

    func loadIfNeeded() {
        guard !hasLoaded else {
            return
        }
        hasLoaded = true
        Task {
            await refreshTargets()
        }
    }

    func selectTarget(_ id: String?) {
        guard !isRunning else {
            return
        }
        guard selectedTargetID != id else {
            return
        }
        selectedTargetID = id
        if !isRunning {
            clearResultState()
        }
    }

    func refreshTargets() async {
        isRefreshing = true
        let currentSelection = selectedTargetID
        defer {
            isRefreshing = false
        }

        do {
            let loadedTargets = try DriveCkDiskDiscoveryService().loadTargets()
            targets = loadedTargets
            if let currentSelection,
               loadedTargets.contains(where: { $0.id == currentSelection })
            {
                selectedTargetID = currentSelection
            } else if isRunning {
                selectedTargetID = currentSelection
            } else {
                selectedTargetID = loadedTargets.first?.id
                if !isRunning {
                    clearResultState()
                }
            }
        } catch let error as DriveCkUserFacingError {
            presentedError = error
        } catch {
            presentedError = DriveCkUserFacingError.from(message: error.localizedDescription)
        }
    }

    func startValidation() {
        guard let target = selectedTarget else {
            presentedError = DriveCkUserFacingError.from(message: "Choose a USB whole-disk target first.")
            return
        }

        let seed: UInt64?
        do {
            if useCustomSeed {
                let trimmed = seedText.trimmingCharacters(in: .whitespacesAndNewlines)
                seed = trimmed.isEmpty ? nil : try driveCkParseSeed(trimmed)
            } else {
                seed = nil
            }
        } catch {
            presentedError = DriveCkUserFacingError.from(message: error.localizedDescription)
            return
        }

        validationTask?.cancel()
        let cancellationFlag = DriveCkCancellationFlag()
        self.cancellationFlag = cancellationFlag
        clearResultState()
        isCancelling = false
        validationState = .preparing
        liveSampleStatus = Array(repeating: .Untested, count: driveCkDefaultMapCellCount)

        let request = DriveCkValidationRequest(
            target: target,
            options: DriveCkValidationOptions(seed: seed)
        )
        activeTarget = target

        validationTask = Task { [weak self] in
            guard let self else {
                return
            }
            do {
                let result = try await DriveCkPrivilegedExecutionService.validate(
                    request: request,
                    onProgress: { [weak self] snapshot in
                        MainActor.assumeIsolated {
                            guard let self else {
                                return
                            }
                            self.applyProgress(snapshot)
                        }
                    },
                    isCancelled: { cancellationFlag.isCancelled }
                )
                await MainActor.run {
                    self.finishValidation(result)
                }
            } catch {
                await MainActor.run {
                    self.validationTask = nil
                    self.cancellationFlag = nil
                    self.activeTarget = nil
                    self.isCancelling = false
                    self.validationState = .idle
                    self.liveSampleStatus = []
                    if let userFacing = error as? DriveCkUserFacingError {
                        self.presentedError = userFacing
                    } else {
                        self.presentedError = DriveCkUserFacingError.from(message: error.localizedDescription)
                    }
                }
            }
        }
    }

    func cancelValidation() {
        guard canCancelValidation else {
            return
        }
        isCancelling = true
        cancellationFlag?.cancel()
    }

    func exportReport() {
        guard let target = currentResponse?.target else {
            return
        }
        guard let url = DriveCkReportExportService.chooseDestination(for: target) else {
            return
        }

        do {
            try DriveCkReportExportService.writeReport(reportText, to: url)
        } catch {
            presentedError = DriveCkUserFacingError.from(message: "Failed to save report.", detail: error.localizedDescription)
        }
    }

    func copyReportToPasteboard() {
        guard !reportText.isEmpty else {
            return
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(reportText, forType: .string)
    }

    private func finishValidation(_ result: DriveCkValidationExecutionResult) {
        validationTask = nil
        cancellationFlag = nil
        activeTarget = nil
        isCancelling = false
        latestResult = result
        validationState = .finished(result)
        liveSampleStatus = []

        if let response = result.response {
            do {
                reportText = try DriveCkFFIBridge.renderReport(response: response)
            } catch let error as DriveCkUserFacingError {
                presentedError = error
            } catch {
                presentedError = DriveCkUserFacingError.from(message: error.localizedDescription)
            }
        } else if let error = result.error {
            presentedError = DriveCkUserFacingError.from(message: error)
        }
    }

    private func clearResultState() {
        latestResult = nil
        reportText = ""
        liveSampleStatus = []
        if !isRunning {
            activeTarget = nil
            isCancelling = false
            validationState = .idle
        }
    }

    private func applyProgress(_ snapshot: DriveCkProgressSnapshot) {
        if snapshot.phase == "Validating" {
            if liveSampleStatus.isEmpty || liveSampleStatus.count != snapshot.total {
                liveSampleStatus = Array(repeating: .Untested, count: snapshot.total)
            }
            if let sampleIndex = snapshot.sampleIndex,
               let sampleStatus = snapshot.sampleStatus,
               liveSampleStatus.indices.contains(sampleIndex)
            {
                liveSampleStatus[sampleIndex] = sampleStatus
            }
            validationState = .running(snapshot)
            return
        }

        validationState = .preparing
    }

    private func installWorkspaceObservers() {
        let center = NSWorkspace.shared.notificationCenter
        let names: [Notification.Name] = [
            NSWorkspace.didWakeNotification,
        ]
        workspaceObservers = names.map { name in
            center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                guard let self else {
                    return
                }
                Task { @MainActor in
                    await self.refreshTargets()
                }
            }
        }
    }
}
