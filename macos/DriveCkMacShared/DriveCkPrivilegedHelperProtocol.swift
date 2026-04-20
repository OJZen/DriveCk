import Darwin
import Foundation

enum DriveCkPrivilegedHelperAction: String, Codable, Sendable {
    case validate
}

struct DriveCkPrivilegedHelperRequest: Codable, Sendable {
    var action: DriveCkPrivilegedHelperAction
    var validationRequest: DriveCkValidationRequest?

    static func validate(request: DriveCkValidationRequest) -> DriveCkPrivilegedHelperRequest {
        DriveCkPrivilegedHelperRequest(action: .validate, validationRequest: request)
    }
}

struct DriveCkPrivilegedHelperResponse: Codable, Sendable {
    var validationResult: DriveCkValidationExecutionResult?
    var userFacingError: DriveCkUserFacingError?

    static func validation(_ result: DriveCkValidationExecutionResult) -> DriveCkPrivilegedHelperResponse {
        DriveCkPrivilegedHelperResponse(validationResult: result, userFacingError: nil)
    }

    static func failure(_ error: DriveCkUserFacingError) -> DriveCkPrivilegedHelperResponse {
        DriveCkPrivilegedHelperResponse(validationResult: nil, userFacingError: error)
    }
}

enum DriveCkPrivilegedHelperIPC {
    private static let userInputFileMode = mode_t(S_IRUSR | S_IWUSR)
    private static let helperOutputFileMode = mode_t(S_IRUSR | S_IRGRP | S_IROTH)

    final class JSONLineWriter: @unchecked Sendable {
        private let handle: FileHandle
        private let descriptor: Int32

        init(url: URL, mode: mode_t = helperOutputFileMode) throws {
            try DriveCkPrivilegedHelperIPC.validateSecureDirectory(at: url.deletingLastPathComponent())

            let descriptor = open(
                url.path,
                O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC | O_NOFOLLOW,
                mode
            )
            guard descriptor >= 0 else {
                throw DriveCkPrivilegedHelperIPC.ipcError(
                    "DriveCk could not append helper IPC output.",
                    errno: errno
                )
            }

            do {
                guard fchmod(descriptor, mode) == 0 else {
                    throw DriveCkPrivilegedHelperIPC.ipcError(
                        "DriveCk could not update helper IPC permissions.",
                        errno: errno
                    )
                }
                self.descriptor = descriptor
                handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            } catch {
                Darwin.close(descriptor)
                throw error
            }
        }

        func append<T: Encodable>(_ value: T) throws {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            var data = try encoder.encode(value)
            data.append(0x0A)
            try handle.write(contentsOf: data)
        }

        func close() {
            guard descriptor >= 0 else {
                return
            }
            try? handle.close()
        }
    }

    struct ContextURLs {
        var directoryURL: URL
        var responseURL: URL
        var progressURL: URL
        var cancelURL: URL
    }

    static func createSecureDirectory() throws -> URL {
        let basePath = FileManager.default.temporaryDirectory.path
        var template = Array("\(basePath)/DriveCkPrivileged.XXXXXX".utf8CString)
        guard let created = mkdtemp(&template) else {
            throw ipcError("DriveCk could not create a secure IPC directory.", errno: errno)
        }

        let path = String(cString: created)
        guard chmod(path, mode_t(S_IRUSR | S_IWUSR | S_IXUSR)) == 0 else {
            let failure = ipcError("DriveCk could not lock down the helper IPC directory.", errno: errno)
            try? FileManager.default.removeItem(atPath: path)
            throw failure
        }

        return URL(fileURLWithPath: path, isDirectory: true)
    }

    static func contextURLs(for directoryURL: URL) -> ContextURLs {
        ContextURLs(
            directoryURL: directoryURL,
            responseURL: directoryURL.appendingPathComponent("response.json"),
            progressURL: directoryURL.appendingPathComponent("progress.json"),
            cancelURL: directoryURL.appendingPathComponent("cancel")
        )
    }

    static func validateSecureDirectory(at url: URL) throws {
        var fileStatus = stat()
        guard lstat(url.path, &fileStatus) == 0 else {
            throw ipcError("DriveCk could not inspect the helper IPC directory.", errno: errno)
        }
        guard (fileStatus.st_mode & S_IFMT) == S_IFDIR else {
            throw DriveCkUserFacingError.from(message: "The helper IPC directory is not a directory.")
        }

        let disallowedBits = mode_t(S_IRWXG | S_IRWXO)
        guard fileStatus.st_mode & disallowedBits == 0 else {
            throw DriveCkUserFacingError.from(
                message: "The helper IPC directory permissions are too broad for privileged use."
            )
        }
    }

    static func readJSONIfPresent<T: Decodable>(_ type: T.Type, from url: URL) throws -> T? {
        guard let data = try readDataIfPresent(from: url) else {
            return nil
        }
        guard !data.isEmpty else {
            return nil
        }
        return try JSONDecoder().decode(type, from: data)
    }

    static func readJSONLinesIfPresent<T: Decodable>(
        _ type: T.Type,
        from url: URL,
        offset: inout UInt64
    ) throws -> [T] {
        guard let data = try readDataIfPresent(from: url, startingAt: offset) else {
            return []
        }
        guard !data.isEmpty else {
            return []
        }

        var decoded = [T]()
        let decoder = JSONDecoder()
        var nextLineStart = data.startIndex

        while let lineEnd = data[nextLineStart...].firstIndex(of: 0x0A) {
            let line = data[nextLineStart..<lineEnd]
            if !line.isEmpty {
                decoded.append(try decoder.decode(type, from: Data(line)))
            }
            nextLineStart = data.index(after: lineEnd)
        }

        offset += UInt64(nextLineStart)
        return decoded
    }

    static func writeJSON<T: Encodable>(
        _ value: T,
        to url: URL,
        exclusive: Bool = false,
        mode: mode_t = userInputFileMode,
        synchronize: Bool = true
    ) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(value)
        if exclusive {
            try writeNewFile(data, to: url, mode: mode, synchronize: synchronize)
        } else {
            try writeAtomically(data, to: url, mode: mode, synchronize: synchronize)
        }
    }

    static func writeMarker(to url: URL) throws {
        do {
            try writeNewFile(Data(), to: url, mode: userInputFileMode, synchronize: false)
        } catch let error as POSIXError where error.code == .EEXIST {
            return
        }
    }

    static func writeHelperOutputJSON<T: Encodable>(_ value: T, to url: URL) throws {
        try writeJSON(value, to: url, mode: helperOutputFileMode, synchronize: false)
    }

    static func appendHelperOutputJSONLine<T: Encodable>(_ value: T, to url: URL) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        var data = try encoder.encode(value)
        data.append(0x0A)
        try appendData(data, to: url, mode: helperOutputFileMode, synchronize: false)
    }

    static func encodeRequestPayload(_ request: DriveCkPrivilegedHelperRequest) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(request).base64EncodedString()
    }

    static func decodeRequestPayload(_ payload: String) throws -> DriveCkPrivilegedHelperRequest {
        guard let data = Data(base64Encoded: payload) else {
            throw DriveCkUserFacingError.from(message: "DriveCk could not decode the privileged helper request.")
        }
        do {
            return try JSONDecoder().decode(DriveCkPrivilegedHelperRequest.self, from: data)
        } catch {
            throw DriveCkUserFacingError.from(
                message: "DriveCk received an invalid privileged helper request.",
                detail: error.localizedDescription
            )
        }
    }

    private static func readDataIfPresent(from url: URL) throws -> Data? {
        try validateSecureDirectory(at: url.deletingLastPathComponent())

        let descriptor = try openExistingRegularFile(at: url)
        switch descriptor {
        case .none:
            return nil
        case let .some(descriptor):
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            return try handle.readToEnd() ?? Data()
        }
    }

    private static func readDataIfPresent(from url: URL, startingAt offset: UInt64) throws -> Data? {
        try validateSecureDirectory(at: url.deletingLastPathComponent())

        let descriptor = try openExistingRegularFile(at: url)
        switch descriptor {
        case .none:
            return nil
        case let .some(descriptor):
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            try handle.seek(toOffset: offset)
            return try handle.readToEnd() ?? Data()
        }
    }

    private static func writeNewFile(
        _ data: Data,
        to url: URL,
        mode: mode_t,
        synchronize: Bool
    ) throws {
        try validateSecureDirectory(at: url.deletingLastPathComponent())
        let descriptor = try createNewRegularFile(at: url, mode: mode)
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        try handle.write(contentsOf: data)
        if synchronize {
            try handle.synchronize()
        }
    }

    private static func writeAtomically(
        _ data: Data,
        to url: URL,
        mode: mode_t,
        synchronize: Bool
    ) throws {
        let directoryURL = url.deletingLastPathComponent()
        try validateSecureDirectory(at: directoryURL)

        let tempURL = directoryURL.appendingPathComponent(".\(UUID().uuidString).tmp")
        do {
            try writeNewFile(data, to: tempURL, mode: mode, synchronize: synchronize)
            guard rename(tempURL.path, url.path) == 0 else {
                throw ipcError("DriveCk could not update the helper IPC file.", errno: errno)
            }
        } catch {
            try? FileManager.default.removeItem(at: tempURL)
            throw error
        }
    }

    private static func appendData(
        _ data: Data,
        to url: URL,
        mode: mode_t,
        synchronize: Bool
    ) throws {
        try validateSecureDirectory(at: url.deletingLastPathComponent())

        let descriptor = open(
            url.path,
            O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC | O_NOFOLLOW,
            mode
        )
        guard descriptor >= 0 else {
            throw ipcError("DriveCk could not append helper IPC output.", errno: errno)
        }

        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        try handle.write(contentsOf: data)
        if synchronize {
            try handle.synchronize()
        }
        guard fchmod(descriptor, mode) == 0 else {
            throw ipcError("DriveCk could not update helper IPC permissions.", errno: errno)
        }
    }

    private static func openExistingRegularFile(at url: URL) throws -> Int32? {
        let descriptor = open(url.path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW)
        if descriptor < 0 {
            if errno == ENOENT {
                return nil
            }
            throw ipcError("DriveCk could not open a helper IPC file for reading.", errno: errno)
        }

        do {
            try validateRegularFile(descriptor: descriptor, message: "DriveCk expected a regular helper IPC file.")
            return descriptor
        } catch {
            close(descriptor)
            throw error
        }
    }

    private static func createNewRegularFile(at url: URL, mode: mode_t) throws -> Int32 {
        let descriptor = open(
            url.path,
            O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
            mode
        )
        guard descriptor >= 0 else {
            if errno == EEXIST {
                throw POSIXError(.EEXIST)
            }
            throw ipcError("DriveCk could not create a helper IPC file.", errno: errno)
        }

        do {
            try validateRegularFile(descriptor: descriptor, message: "DriveCk created an unexpected helper IPC file type.")
            return descriptor
        } catch {
            close(descriptor)
            throw error
        }
    }

    private static func validateRegularFile(descriptor: Int32, message: String) throws {
        var fileStatus = stat()
        guard fstat(descriptor, &fileStatus) == 0 else {
            throw ipcError(message, errno: errno)
        }
        guard (fileStatus.st_mode & S_IFMT) == S_IFREG else {
            throw DriveCkUserFacingError.from(message: message)
        }
    }

    private static func ipcError(_ message: String, errno: Int32) -> DriveCkUserFacingError {
        DriveCkUserFacingError.from(
            message: message,
            detail: String(cString: strerror(errno))
        )
    }
}
