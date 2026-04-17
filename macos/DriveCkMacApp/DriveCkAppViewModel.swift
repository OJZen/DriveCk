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

    private var hasLoaded = false
    private var validationTask: Task<Void, Never>?
    private var cancellationFlag: DriveCkCancellationFlag?
    private var workspaceObservers: [NSObjectProtocol] = []

    init() {
        installWorkspaceObservers()
    }

    var selectedTarget: DriveCkTargetInfo? {
        targets.first(where: { $0.id == selectedTargetID })
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
            return DriveCkProgressSnapshot(phase: "Idle", current: 0, total: 1, finalUpdate: false)
        case .preparing:
            return DriveCkProgressSnapshot(phase: "Preparing", current: 0, total: 1, finalUpdate: false)
        case let .running(snapshot):
            return snapshot
        case let .finished(result):
            if let response = result.response {
                return DriveCkProgressSnapshot(
                    phase: result.error == nil ? "Finished" : "Finished with issues",
                    current: response.report.completedSamples,
                    total: max(response.report.sampleStatus.count, 1),
                    finalUpdate: true
                )
            }
            return DriveCkProgressSnapshot(phase: "Finished", current: 1, total: 1, finalUpdate: true)
        }
    }

    var statusLine: String {
        switch validationState {
        case .idle:
            return "Choose a removable whole-disk target to begin."
        case .preparing:
            return "Preparing validation…"
        case let .running(snapshot):
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
        !isRunning && selectedTarget != nil && !(selectedTarget?.isMounted ?? true)
    }

    var canExportReport: Bool {
        !isRunning && currentResponse != nil && !reportText.isEmpty
    }

    var inlineError: DriveCkUserFacingError? {
        if let error = latestResult?.error {
            return DriveCkUserFacingError.from(message: error)
        }
        if selectedTarget?.isMounted == true {
            return DriveCkUserFacingError.from(message: "mounted")
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
            presentedError = DriveCkUserFacingError.from(message: "Choose a removable whole-disk target first.")
            return
        }
        guard !target.isMounted else {
            presentedError = DriveCkUserFacingError.from(message: "mounted")
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
        validationState = .preparing

        let request = DriveCkValidationRequest(
            target: target,
            options: DriveCkValidationOptions(seed: seed)
        )

        validationTask = Task { [weak self] in
            guard let self else {
                return
            }
            do {
                let result = try await DriveCkValidationCoordinator.validate(
                    request: request,
                    onProgress: { [weak self] snapshot in
                        Task { @MainActor in
                            guard let self else {
                                return
                            }
                            self.validationState = .running(snapshot)
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
                    self.validationState = .idle
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
        latestResult = result
        validationState = .finished(result)

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
        if !isRunning {
            validationState = .idle
        }
    }

    private func installWorkspaceObservers() {
        let center = NSWorkspace.shared.notificationCenter
        let names: [Notification.Name] = [
            NSWorkspace.didMountNotification,
            NSWorkspace.didUnmountNotification,
            NSWorkspace.didRenameVolumeNotification,
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
