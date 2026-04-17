import Foundation

enum DriveCkValidationCoordinator {
    static func validateSync(
        request: DriveCkValidationRequest,
        onProgress: @escaping @Sendable (DriveCkProgressSnapshot) -> Void,
        isCancelled: @escaping @Sendable () -> Bool
    ) throws -> DriveCkValidationExecutionResult {
        try DriveCkFFIBridge.validate(
            request: request,
            onProgress: onProgress,
            isCancelled: isCancelled
        )
    }

    static func validate(
        request: DriveCkValidationRequest,
        onProgress: @escaping @Sendable (DriveCkProgressSnapshot) -> Void,
        isCancelled: @escaping @Sendable () -> Bool
    ) async throws -> DriveCkValidationExecutionResult {
        try await Task.detached(priority: .userInitiated) {
            try validateSync(
                request: request,
                onProgress: onProgress,
                isCancelled: isCancelled
            )
        }
        .value
    }
}
