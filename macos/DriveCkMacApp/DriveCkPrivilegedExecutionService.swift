import Darwin
import Foundation
import Security

@_silgen_name("AuthorizationExecuteWithPrivileges")
private func driveCkAuthorizationExecuteWithPrivileges(
    _ authorization: AuthorizationRef,
    _ pathToTool: UnsafePointer<CChar>,
    _ options: AuthorizationFlags,
    _ arguments: UnsafePointer<UnsafeMutablePointer<CChar>>?,
    _ communicationsPipe: UnsafeMutablePointer<UnsafeMutablePointer<FILE>?>
) -> OSStatus

private struct DriveCkPrivilegedInvocationContext {
    let directoryURL: URL
    let responseURL: URL
    let progressURL: URL
    let cancelURL: URL

    init() throws {
        let directoryURL = try DriveCkPrivilegedHelperIPC.createSecureDirectory()
        let ipcURLs = DriveCkPrivilegedHelperIPC.contextURLs(for: directoryURL)
        self.directoryURL = directoryURL
        responseURL = ipcURLs.responseURL
        progressURL = ipcURLs.progressURL
        cancelURL = ipcURLs.cancelURL
    }

    func helperArguments(for request: DriveCkPrivilegedHelperRequest) throws -> [String] {
        [
            "--privileged-helper",
            "--privileged-helper-directory", directoryURL.path,
            "--privileged-helper-request", try DriveCkPrivilegedHelperIPC.encodeRequestPayload(request),
        ]
    }

    func cleanup() {
        try? FileManager.default.removeItem(at: directoryURL)
    }
}

private final class DriveCkPrivilegedPipeMonitor {
    private let stream: UnsafeMutablePointer<FILE>?
    private let descriptor: Int32
    private var isClosed = false

    private(set) var didReachEOF = false
    private(set) var output = Data()

    init(stream: UnsafeMutablePointer<FILE>?) {
        self.stream = stream
        if let stream {
            let descriptor = fileno(stream)
            self.descriptor = descriptor
            let currentFlags = fcntl(descriptor, F_GETFL, 0)
            if currentFlags >= 0 {
                _ = fcntl(descriptor, F_SETFL, currentFlags | O_NONBLOCK)
            }
        } else {
            descriptor = -1
            didReachEOF = true
        }
    }

    func poll() {
        guard descriptor >= 0, !didReachEOF else {
            return
        }

        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let bytesRead = Darwin.read(descriptor, &buffer, buffer.count)
            if bytesRead > 0 {
                output.append(contentsOf: buffer[0..<Int(bytesRead)])
                continue
            }
            if bytesRead == 0 {
                didReachEOF = true
                return
            }
            if errno == EAGAIN || errno == EWOULDBLOCK {
                return
            }
            didReachEOF = true
            return
        }
    }

    var outputText: String? {
        guard !output.isEmpty else {
            return nil
        }
        let text = String(decoding: output, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    func close() {
        guard !isClosed, let stream else {
            return
        }
        isClosed = true
        fclose(stream)
    }

    deinit {
        close()
    }
}

enum DriveCkPrivilegedExecutionService {
    static func validate(
        request: DriveCkValidationRequest,
        onProgress: @escaping @Sendable (DriveCkProgressSnapshot) -> Void,
        isCancelled: @escaping @Sendable () -> Bool
    ) async throws -> DriveCkValidationExecutionResult {
        let response = try await execute(
            request: .validate(request: request),
            onProgress: onProgress,
            isCancelled: isCancelled
        )
        guard let result = response.validationResult else {
            throw DriveCkUserFacingError.from(
                message: "The privileged helper completed without returning a validation result."
            )
        }
        return result
    }

    private static func execute(
        request: DriveCkPrivilegedHelperRequest,
        onProgress: @escaping @Sendable (DriveCkProgressSnapshot) -> Void = { _ in },
        isCancelled: @escaping @Sendable () -> Bool = { false }
    ) async throws -> DriveCkPrivilegedHelperResponse {
        try await Task.detached(priority: .userInitiated) {
            let context = try DriveCkPrivilegedInvocationContext()
            defer {
                context.cleanup()
            }

            let helperArguments = try context.helperArguments(for: request)
            let helperURL = try resolveHelperExecutable()
            let pipeMonitor = DriveCkPrivilegedPipeMonitor(
                stream: try launchPrivilegedHelper(helperURL: helperURL, arguments: helperArguments)
            )
            defer {
                pipeMonitor.close()
            }

            var hasRequestedCancel = false
            var lastProgressSnapshot: DriveCkProgressSnapshot?

            while true {
                pipeMonitor.poll()

                if !hasRequestedCancel && isCancelled() {
                    try? DriveCkPrivilegedHelperIPC.writeMarker(to: context.cancelURL)
                    hasRequestedCancel = true
                }

                if let snapshot = try DriveCkPrivilegedHelperIPC.readJSONIfPresent(
                    DriveCkProgressSnapshot.self,
                    from: context.progressURL
                ),
                   snapshot != lastProgressSnapshot
                {
                    lastProgressSnapshot = snapshot
                    onProgress(snapshot)
                }

                if let response = try DriveCkPrivilegedHelperIPC.readJSONIfPresent(
                    DriveCkPrivilegedHelperResponse.self,
                    from: context.responseURL
                ) {
                    if let error = response.userFacingError {
                        throw error
                    }
                    return response
                }

                if pipeMonitor.didReachEOF {
                    throw DriveCkUserFacingError(
                        title: "Privileged helper stopped unexpectedly",
                        message: "DriveCk lost the elevated helper before the disk operation finished.",
                        suggestion: "Try again and keep the app open during authorization.",
                        detail: pipeMonitor.outputText
                    )
                }

                try await Task.sleep(nanoseconds: 150_000_000)
            }
        }
        .value
    }

    private static func resolveHelperExecutable() throws -> URL {
        let candidates = [
            Bundle.main.bundleURL
                .deletingLastPathComponent()
                .appendingPathComponent("driveck-mac"),
            Bundle.main.bundleURL
                .appendingPathComponent("Contents/Resources/driveck-mac"),
        ]

        if let helperURL = candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0.path) }) {
            return helperURL
        }

        throw DriveCkUserFacingError(
            title: "Privileged helper missing",
            message: "DriveCk could not find the elevated helper executable.",
            suggestion: "Rebuild the macOS app so the CLI helper is available next to the app bundle.",
            detail: candidates.map(\.path).joined(separator: "\n")
        )
    }

    private static func launchPrivilegedHelper(
        helperURL: URL,
        arguments: [String]
    ) throws -> UnsafeMutablePointer<FILE>? {
        let authorizationRef = try makeAuthorization()
        defer {
            _ = AuthorizationFree(authorizationRef, [.destroyRights])
        }

        try authorizeExecution(of: helperURL, authorizationRef: authorizationRef)

        var stream: UnsafeMutablePointer<FILE>?
        let status = try withCStringArray(arguments) { pointer in
            helperURL.path.withCString { helperPath in
                driveCkAuthorizationExecuteWithPrivileges(
                    authorizationRef,
                    helperPath,
                    [],
                    pointer,
                    &stream
                )
            }
        }
        guard status == errAuthorizationSuccess else {
            throw authorizationError(for: status, helperURL: helperURL)
        }
        return stream
    }

    private static func makeAuthorization() throws -> AuthorizationRef {
        var authorizationRef: AuthorizationRef?
        let status = AuthorizationCreate(nil, nil, [], &authorizationRef)
        guard status == errAuthorizationSuccess, let authorizationRef else {
            throw authorizationError(
                for: status,
                helperURL: nil,
                detail: "AuthorizationCreate failed."
            )
        }
        return authorizationRef
    }

    private static func authorizeExecution(
        of helperURL: URL,
        authorizationRef: AuthorizationRef
    ) throws {
        let flags: AuthorizationFlags = [.interactionAllowed, .extendRights, .preAuthorize]

        let status = kAuthorizationRightExecute.withCString { rightName in
            helperURL.path.withCString { helperPath in
                var item = AuthorizationItem(
                    name: rightName,
                    valueLength: strlen(helperPath),
                    value: UnsafeMutableRawPointer(mutating: helperPath),
                    flags: 0
                )
                return withUnsafeMutablePointer(to: &item) { itemPointer in
                    var rights = AuthorizationRights(count: 1, items: itemPointer)
                    return AuthorizationCopyRights(authorizationRef, &rights, nil, flags, nil)
                }
            }
        }

        guard status == errAuthorizationSuccess else {
            throw authorizationError(for: status, helperURL: helperURL)
        }
    }

    private static func withCStringArray<Result>(
        _ arguments: [String],
        _ body: (UnsafePointer<UnsafeMutablePointer<CChar>>?) throws -> Result
    ) throws -> Result {
        if arguments.isEmpty {
            return try body(nil)
        }

        var duplicatedArguments = [UnsafeMutablePointer<CChar>]()
        duplicatedArguments.reserveCapacity(arguments.count)
        do {
            for argument in arguments {
                guard let duplicated = strdup(argument) else {
                    throw DriveCkUserFacingError.from(message: "Failed to allocate helper arguments.")
                }
                duplicatedArguments.append(duplicated)
            }
        } catch {
            duplicatedArguments.forEach { free($0) }
            throw error
        }

        let buffer = UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>.allocate(capacity: duplicatedArguments.count + 1)
        defer {
            buffer.deallocate()
            duplicatedArguments.forEach { free($0) }
        }

        for (index, argument) in duplicatedArguments.enumerated() {
            buffer[index] = argument
        }
        buffer[duplicatedArguments.count] = nil

        return try buffer.withMemoryRebound(
            to: UnsafeMutablePointer<CChar>.self,
            capacity: duplicatedArguments.count + 1
        ) { rebound in
            try body(UnsafePointer(rebound))
        }
    }
    private static func authorizationError(
        for status: OSStatus,
        helperURL: URL?,
        detail: String? = nil
    ) -> DriveCkUserFacingError {
        let helperPath = helperURL?.path
        let suffix = helperPath.map { "\nHelper: \($0)" } ?? ""
        let composedDetail = [
            detail,
            "OSStatus: \(status)\(suffix)",
        ]
        .compactMap { $0 }
        .joined(separator: "\n")

        switch status {
        case errAuthorizationCanceled:
            return DriveCkUserFacingError(
                title: "Authorization cancelled",
                message: "DriveCk needs administrator access before it can touch raw disks.",
                suggestion: "Start the operation again and approve the system password prompt.",
                detail: composedDetail
            )
        case errAuthorizationDenied:
            return DriveCkUserFacingError(
                title: "Authorization denied",
                message: "macOS did not grant DriveCk permission to run the privileged helper.",
                suggestion: "Try again and confirm the administrator password prompt.",
                detail: composedDetail
            )
        case errAuthorizationInteractionNotAllowed:
            return DriveCkUserFacingError(
                title: "Authorization UI unavailable",
                message: "DriveCk could not show the administrator password prompt.",
                suggestion: "Launch the app from the desktop session and try the disk action again.",
                detail: composedDetail
            )
        case errAuthorizationToolExecuteFailure:
            return DriveCkUserFacingError(
                title: "Helper launch failed",
                message: "DriveCk could not start the privileged helper executable.",
                suggestion: "Rebuild the app and confirm the helper binary is present.",
                detail: composedDetail
            )
        default:
            return DriveCkUserFacingError(
                title: "Authorization failed",
                message: "DriveCk could not obtain administrator access for the requested disk operation.",
                suggestion: "Try the operation again and confirm the password prompt appears.",
                detail: composedDetail
            )
        }
    }
}
