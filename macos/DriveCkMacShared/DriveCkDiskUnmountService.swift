import Foundation

enum DriveCkDiskUnmountService {
    static func unmount(target: DriveCkTargetInfo) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/diskutil")
        process.arguments = ["unmountDisk", diskReference(for: target)]

        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        try process.run()
        process.waitUntilExit()

        let stdoutData = stdout.fileHandleForReading.readDataToEndOfFile()
        let stderrData = stderr.fileHandleForReading.readDataToEndOfFile()
        guard process.terminationStatus == 0 else {
            let stdoutText = String(data: stdoutData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let stderrText = String(data: stderrData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let detail = [stderrText, stdoutText]
                .compactMap { text in
                    guard let text, !text.isEmpty else {
                        return nil
                    }
                    return text
                }
                .joined(separator: "\n")
            let normalizedDetail = detail.isEmpty ? nil : detail
            if isAlreadyUnmounted(detail: normalizedDetail) {
                return
            }
            throw errorForUnmountFailure(target: target, detail: normalizedDetail)
        }
    }

    private static func diskReference(for target: DriveCkTargetInfo) -> String {
        if target.name.hasPrefix("disk") {
            return "/dev/\(target.name)"
        }
        if target.name.hasPrefix("rdisk") {
            return "/dev/\(target.name.dropFirst())"
        }
        if target.path.hasPrefix("/dev/rdisk") {
            return "/dev/disk\(target.path.dropFirst("/dev/rdisk".count))"
        }
        return target.path
    }

    private static func errorForUnmountFailure(
        target: DriveCkTargetInfo,
        detail: String?
    ) -> DriveCkUserFacingError {
        let normalized = detail?.lowercased() ?? ""
        if normalized.contains("busy") || normalized.contains("in use") {
            return .init(
                title: "Disk is in use",
                message: "DriveCk could not unmount \(target.name) because one or more apps are still using it.",
                suggestion: "Close files and apps using that disk, then try unmounting again.",
                detail: detail
            )
        }
        if normalized.contains("not mounted") {
            return .init(
                title: "Disk already unmounted",
                message: "\(target.name) is no longer mounted.",
                suggestion: "Start validation again if you still want DriveCk to test that disk.",
                detail: detail
            )
        }
        return .init(
            title: "Could not unmount disk",
            message: "DriveCk could not unmount \(target.name).",
            suggestion: "Close apps using the disk or unmount it in Disk Utility, then start validation again.",
            detail: detail
        )
    }

    private static func isAlreadyUnmounted(detail: String?) -> Bool {
        let normalized = detail?.lowercased() ?? ""
        return normalized.contains("not mounted")
            || normalized.contains("already unmounted")
            || normalized.contains("no mount point")
    }
}
