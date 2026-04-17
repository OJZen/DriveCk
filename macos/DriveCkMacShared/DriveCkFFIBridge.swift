import Foundation

private typealias DriveCkFFIProgressCallback =
    @convention(c) (UnsafePointer<CChar>?, Int, Int, Bool, UnsafeMutableRawPointer?) -> Void
private typealias DriveCkFFICancelCallback =
    @convention(c) (UnsafeMutableRawPointer?) -> Bool

@_silgen_name("driveck_ffi_free_string")
private func driveck_ffi_free_string(_ value: UnsafeMutablePointer<CChar>?)

@_silgen_name("driveck_ffi_validate_target_json")
private func driveck_ffi_validate_target_json(
    _ targetJSON: UnsafePointer<CChar>?,
    _ seedSet: Bool,
    _ seed: UInt64,
    _ progressCallback: DriveCkFFIProgressCallback?,
    _ cancelCallback: DriveCkFFICancelCallback?,
    _ userData: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("driveck_ffi_format_report_text_json")
private func driveck_ffi_format_report_text_json(
    _ validationJSON: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

private final class DriveCkFFIContext: @unchecked Sendable {
    let onProgress: @Sendable (DriveCkProgressSnapshot) -> Void
    let isCancelled: @Sendable () -> Bool

    init(
        onProgress: @escaping @Sendable (DriveCkProgressSnapshot) -> Void,
        isCancelled: @escaping @Sendable () -> Bool
    ) {
        self.onProgress = onProgress
        self.isCancelled = isCancelled
    }
}

enum DriveCkFFIBridge {
    static func validate(
        request: DriveCkValidationRequest,
        onProgress: @escaping @Sendable (DriveCkProgressSnapshot) -> Void,
        isCancelled: @escaping @Sendable () -> Bool
    ) throws -> DriveCkValidationExecutionResult {
        let payload = try JSONEncoder().encode(request)
        let json = String(decoding: payload, as: UTF8.self)
        let context = DriveCkFFIContext(onProgress: onProgress, isCancelled: isCancelled)
        let unmanaged = Unmanaged.passRetained(context)
        defer {
            unmanaged.release()
        }

        let rawResponse = json.withCString { pointer in
            driveck_ffi_validate_target_json(
                pointer,
                request.options.seed != nil,
                request.options.seed ?? 0,
                progressCallback,
                cancelCallback,
                unmanaged.toOpaque()
            )
        }

        let responseJSON = try takeJSONString(from: rawResponse)
        let envelope = try JSONDecoder().decode(
            DriveCkFFIEnvelope<DriveCkValidationExecutionResult>.self,
            from: Data(responseJSON.utf8)
        )
        guard envelope.ok, let data = envelope.data else {
            throw DriveCkUserFacingError.from(
                message: envelope.error ?? "DriveCk FFI returned an empty validation response."
            )
        }
        return data
    }

    static func renderReport(response: DriveCkValidationResponse) throws -> String {
        let payload = try JSONEncoder().encode(response)
        let json = String(decoding: payload, as: UTF8.self)
        let rawResponse = json.withCString { pointer in
            driveck_ffi_format_report_text_json(pointer)
        }
        let responseJSON = try takeJSONString(from: rawResponse)
        let envelope = try JSONDecoder().decode(
            DriveCkFFIEnvelope<String>.self,
            from: Data(responseJSON.utf8)
        )
        guard envelope.ok, let text = envelope.data else {
            throw DriveCkUserFacingError.from(
                message: envelope.error ?? "DriveCk FFI returned an empty report payload."
            )
        }
        return text
    }

    private static func takeJSONString(from pointer: UnsafeMutablePointer<CChar>?) throws -> String {
        guard let pointer else {
            throw DriveCkUserFacingError.from(message: "DriveCk FFI returned a null string pointer.")
        }
        defer {
            driveck_ffi_free_string(pointer)
        }
        return String(cString: pointer)
    }

    private static let progressCallback: DriveCkFFIProgressCallback = { phase, current, total, finalUpdate, userData in
        guard let userData else {
            return
        }
        let context = Unmanaged<DriveCkFFIContext>.fromOpaque(userData).takeUnretainedValue()
        let phaseText = phase.map(String.init(cString:)) ?? "Working"
        context.onProgress(
            DriveCkProgressSnapshot(
                phase: phaseText,
                current: current,
                total: total,
                finalUpdate: finalUpdate
            )
        )
    }

    private static let cancelCallback: DriveCkFFICancelCallback = { userData in
        guard let userData else {
            return false
        }
        let context = Unmanaged<DriveCkFFIContext>.fromOpaque(userData).takeUnretainedValue()
        return context.isCancelled()
    }
}
