import Foundation

enum DriveCkTargetKind: String, Codable, Sendable {
    case BlockDevice
}

enum DriveCkSampleStatus: String, Codable, CaseIterable, Sendable {
    case Untested
    case Ok
    case ReadError
    case WriteError
    case VerifyMismatch
    case RestoreError

    var displayName: String {
        switch self {
        case .Untested:
            return "Untested"
        case .Ok:
            return "OK"
        case .ReadError:
            return "Read error"
        case .WriteError:
            return "Write error"
        case .VerifyMismatch:
            return "Verify mismatch"
        case .RestoreError:
            return "Restore error"
        }
    }

    var glyph: String {
        switch self {
        case .Untested:
            return "?"
        case .Ok:
            return "."
        case .ReadError:
            return "R"
        case .WriteError:
            return "W"
        case .VerifyMismatch:
            return "M"
        case .RestoreError:
            return "!"
        }
    }

    static func fromFFICode(_ code: Int32) -> DriveCkSampleStatus? {
        switch code {
        case 0:
            return .Untested
        case 1:
            return .Ok
        case 2:
            return .ReadError
        case 3:
            return .WriteError
        case 4:
            return .VerifyMismatch
        case 5:
            return .RestoreError
        default:
            return nil
        }
    }
}

struct DriveCkValidationOptions: Codable, Hashable, Sendable {
    var seed: UInt64?
}

struct DriveCkTargetInfo: Codable, Hashable, Identifiable, Sendable {
    var kind: DriveCkTargetKind
    var path: String
    var name: String
    var vendor: String
    var model: String
    var transport: String
    var sizeBytes: UInt64
    var logicalBlockSize: UInt32
    var deviceGUID: String?
    var mediaUUID: String?
    var devicePath: String?
    var busPath: String?
    var isBlockDevice: Bool
    var isRemovable: Bool
    var isUsb: Bool
    var isMounted: Bool
    var directIo: Bool

    var id: String { path }

    var displayName: String {
        let modelLine = [vendor, model]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        return modelLine.isEmpty ? name : modelLine
    }

    var subtitle: String {
        var parts = [driveCkFormatBytes(sizeBytes)]
        if !transportLabel.isEmpty {
            parts.append(transportLabel)
        }
        parts.append("Whole disk")
        return parts.joined(separator: " · ")
    }

    var sidebarSubtitle: String {
        [driveCkFormatBytes(sizeBytes), transportLabel]
            .filter { !$0.isEmpty }
            .joined(separator: " · ")
    }

    var shortPath: String {
        path.split(separator: "/").last.map(String.init) ?? path
    }

    var blockSizeLabel: String {
        "\(logicalBlockSize) B"
    }

    var readinessLabel: String {
        isMounted ? "Mounted" : "Ready"
    }

    var transportLabel: String {
        let normalized = transport.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else {
            return isRemovable ? "Removable" : ""
        }
        let acronyms = [
            "sd": "SD",
            "ssd": "SSD",
            "hdd": "HDD",
            "nvme": "NVMe",
            "usb": "USB",
        ]
        return normalized
            .split(separator: "_")
            .map { component in
                let token = component.lowercased()
                return acronyms[token] ?? component.capitalized
            }
            .joined(separator: " ")
    }

    var commandLineAliases: [String] {
        let rawName = name.hasPrefix("r") ? name : "r\(name)"
        return [
            path,
            name,
            rawName,
            "/dev/\(name)",
            "/dev/\(rawName)",
        ]
    }

    var privilegedIdentitySummary: String {
        [
            "name=\(name)",
            "path=\(path)",
            nonEmpty(vendor).map { "vendor=\($0)" },
            nonEmpty(model).map { "model=\($0)" },
            nonEmpty(transport).map { "transport=\($0)" },
            "size_bytes=\(sizeBytes)",
            "logical_block_size=\(logicalBlockSize)",
            nonEmpty(deviceGUID).map { "device_guid=\($0)" },
            nonEmpty(mediaUUID).map { "media_uuid=\($0)" },
            nonEmpty(devicePath).map { "device_path=\($0)" },
            nonEmpty(busPath).map { "bus_path=\($0)" },
        ]
        .compactMap { $0 }
        .joined(separator: ", ")
    }

    func matchesPrivilegedIdentity(of other: DriveCkTargetInfo) -> Bool {
        guard kind == other.kind,
              isBlockDevice == other.isBlockDevice,
              isRemovable == other.isRemovable,
              isUsb == other.isUsb,
              sizeBytes == other.sizeBytes,
              logicalBlockSize == other.logicalBlockSize
        else {
            return false
        }

        guard strongIdentityMatches(deviceGUID, other.deviceGUID),
              strongIdentityMatches(mediaUUID, other.mediaUUID),
              strongIdentityMatches(devicePath, other.devicePath),
              strongIdentityMatches(busPath, other.busPath)
        else {
            return false
        }

        guard softIdentityMatches(vendor, other.vendor),
              softIdentityMatches(model, other.model),
              softIdentityMatches(transport, other.transport)
        else {
            return false
        }

        if hasStrongIdentity || other.hasStrongIdentity {
            return true
        }

        return path == other.path && name == other.name
    }

    private var hasStrongIdentity: Bool {
        nonEmpty(deviceGUID) != nil
            || nonEmpty(mediaUUID) != nil
            || nonEmpty(devicePath) != nil
            || nonEmpty(busPath) != nil
    }

    private func nonEmpty(_ value: String?) -> String? {
        let normalized = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let normalized, !normalized.isEmpty else {
            return nil
        }
        return normalized
    }

    private func strongIdentityMatches(_ lhs: String?, _ rhs: String?) -> Bool {
        let left = normalizedIdentityValue(lhs)
        let right = normalizedIdentityValue(rhs)
        switch (left, right) {
        case (nil, nil):
            return true
        case let (.some(left), .some(right)):
            return left == right
        default:
            return false
        }
    }

    private func softIdentityMatches(_ lhs: String, _ rhs: String) -> Bool {
        let left = normalizedIdentityValue(lhs)
        let right = normalizedIdentityValue(rhs)
        guard let left, let right else {
            return true
        }
        return left == right
    }

    private func normalizedIdentityValue(_ value: String?) -> String? {
        nonEmpty(value)?.lowercased()
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case path
        case name
        case vendor
        case model
        case transport
        case sizeBytes = "size_bytes"
        case logicalBlockSize = "logical_block_size"
        case deviceGUID = "device_guid"
        case mediaUUID = "media_uuid"
        case devicePath = "device_path"
        case busPath = "bus_path"
        case isBlockDevice = "is_block_device"
        case isRemovable = "is_removable"
        case isUsb = "is_usb"
        case isMounted = "is_mounted"
        case directIo = "direct_io"
    }
}

struct DriveCkTimingSeries: Codable, Hashable, Sendable {
    var values: [Double]
}

struct DriveCkValidationReport: Codable, Hashable, Sendable {
    var startedAt: Int64
    var finishedAt: Int64
    var seed: UInt64
    var reportedSizeBytes: UInt64
    var regionSizeBytes: UInt64
    var validatedDriveSizeBytes: UInt64
    var highestValidRegionBytes: UInt64
    var sampleOffsets: [UInt64]
    var sampleStatus: [DriveCkSampleStatus]
    var readTimings: DriveCkTimingSeries
    var writeTimings: DriveCkTimingSeries
    var successCount: Int
    var readErrorCount: Int
    var writeErrorCount: Int
    var mismatchCount: Int
    var restoreErrorCount: Int
    var completedSamples: Int
    var cancelled: Bool
    var completedAllSamples: Bool

    var verdict: String {
        if restoreErrorCount != 0 {
            return "Critical restore failure"
        }
        if cancelled {
            return "Validation cancelled"
        }
        if mismatchCount != 0 {
            return "Missing or spoofed storage detected"
        }
        if readErrorCount != 0 || writeErrorCount != 0 {
            return "I/O errors detected"
        }
        if !completedAllSamples {
            return "Validation incomplete"
        }
        return "All sampled regions validated"
    }

    var hasFailures: Bool {
        restoreErrorCount != 0
            || mismatchCount != 0
            || readErrorCount != 0
            || writeErrorCount != 0
            || !completedAllSamples
    }

    var failureCount: Int {
        readErrorCount + writeErrorCount + mismatchCount + restoreErrorCount
    }

    var completionFraction: Double {
        guard !sampleStatus.isEmpty else {
            return 0
        }
        return min(max(Double(completedSamples) / Double(sampleStatus.count), 0), 1)
    }

    var readSummary: DriveCkTimingSummary {
        DriveCkTimingSummary.summarize(readTimings, regionSizeBytes: regionSizeBytes)
    }

    var writeSummary: DriveCkTimingSummary {
        DriveCkTimingSummary.summarize(writeTimings, regionSizeBytes: regionSizeBytes)
    }

    var mapEntries: [DriveCkMapEntry] {
        sampleStatus.enumerated().map { index, status in
            DriveCkMapEntry(
                id: index,
                index: index,
                offset: sampleOffsets.indices.contains(index) ? sampleOffsets[index] : 0,
                status: status
            )
        }
    }

    enum CodingKeys: String, CodingKey {
        case startedAt = "started_at"
        case finishedAt = "finished_at"
        case seed
        case reportedSizeBytes = "reported_size_bytes"
        case regionSizeBytes = "region_size_bytes"
        case validatedDriveSizeBytes = "validated_drive_size_bytes"
        case highestValidRegionBytes = "highest_valid_region_bytes"
        case sampleOffsets = "sample_offsets"
        case sampleStatus = "sample_status"
        case readTimings = "read_timings"
        case writeTimings = "write_timings"
        case successCount = "success_count"
        case readErrorCount = "read_error_count"
        case writeErrorCount = "write_error_count"
        case mismatchCount = "mismatch_count"
        case restoreErrorCount = "restore_error_count"
        case completedSamples = "completed_samples"
        case cancelled
        case completedAllSamples = "completed_all_samples"
    }
}

struct DriveCkValidationResponse: Codable, Hashable, Sendable {
    var target: DriveCkTargetInfo
    var report: DriveCkValidationReport
}

struct DriveCkValidationRequest: Codable, Hashable, Sendable {
    var target: DriveCkTargetInfo
    var options: DriveCkValidationOptions
}

struct DriveCkValidationExecutionResult: Codable, Hashable, Sendable {
    var response: DriveCkValidationResponse?
    var error: String?
}

struct DriveCkFFIEnvelope<T: Decodable & Sendable>: Decodable, Sendable {
    var ok: Bool
    var data: T?
    var error: String?
}

struct DriveCkProgressSnapshot: Codable, Hashable, Sendable {
    var phase: String
    var current: Int
    var total: Int
    var finalUpdate: Bool
    var sampleIndex: Int? = nil
    var sampleStatus: DriveCkSampleStatus? = nil

    var fraction: Double {
        guard total > 0 else {
            return 0
        }
        return min(max(Double(current) / Double(total), 0), 1)
    }
}

let driveCkMapRowCount = 18
let driveCkMapColumnCount = 32
let driveCkDefaultMapCellCount = driveCkMapRowCount * driveCkMapColumnCount

struct DriveCkMapEntry: Identifiable, Hashable, Sendable {
    var id: Int
    var index: Int
    var offset: UInt64
    var status: DriveCkSampleStatus
}

struct DriveCkTimingSummary: Hashable, Sendable {
    var count: Int
    var minimumMs: Double
    var medianMs: Double
    var meanMs: Double
    var maximumMs: Double
    var stddevMs: Double
    var totalMs: Double
    var variation: Double
    var throughputMiBS: Double

    static func summarize(_ series: DriveCkTimingSeries, regionSizeBytes: UInt64) -> DriveCkTimingSummary {
        guard !series.values.isEmpty else {
            return .init(
                count: 0,
                minimumMs: 0,
                medianMs: 0,
                meanMs: 0,
                maximumMs: 0,
                stddevMs: 0,
                totalMs: 0,
                variation: 0,
                throughputMiBS: 0
            )
        }

        let sorted = series.values.sorted()
        let total = series.values.reduce(0, +)
        let mean = total / Double(series.values.count)
        let variance = series.values.reduce(0.0) { partial, value in
            let delta = value - mean
            return partial + delta * delta
        } / Double(series.values.count)
        let median: Double
        if sorted.count.isMultiple(of: 2) {
            let middle = sorted.count / 2
            median = (sorted[middle - 1] + sorted[middle]) / 2
        } else {
            median = sorted[sorted.count / 2]
        }

        let throughput: Double
        if total > 0 {
            let totalBytes = Double(regionSizeBytes) * Double(series.values.count)
            throughput = (totalBytes / (1024 * 1024)) / (total / 1000)
        } else {
            throughput = 0
        }

        return .init(
            count: series.values.count,
            minimumMs: sorted.first ?? 0,
            medianMs: median,
            meanMs: mean,
            maximumMs: sorted.last ?? 0,
            stddevMs: variance.squareRoot(),
            totalMs: total,
            variation: mean == 0 ? 0 : variance.squareRoot() / mean,
            throughputMiBS: throughput
        )
    }
}

struct DriveCkUserFacingError: Codable, Error, Identifiable, Hashable, Sendable {
    var id = UUID()
    var title: String
    var message: String
    var suggestion: String
    var detail: String?

    static func from(message: String, detail: String? = nil) -> DriveCkUserFacingError {
        let normalized = message.lowercased()
        if normalized.contains("permission denied") || normalized.contains("operation not permitted") {
            return .init(
                title: "Permission required",
                message: "DriveCk could not open the device with write access.",
                suggestion: "Run the CLI with administrator privileges, or relaunch the GUI from a privileged environment.",
                detail: detail ?? message
            )
        }
        if normalized.contains("mounted") {
            return .init(
                title: "Disk is still mounted",
                message: "The selected disk or one of its volumes is mounted and cannot be validated safely.",
                suggestion: "Close apps using that disk and try again. DriveCk will unmount it automatically after administrator approval when macOS allows it.",
                detail: detail ?? message
            )
        }
        if normalized.contains("cancelled") {
            return .init(
                title: "Validation cancelled",
                message: "The validation run stopped before all samples completed.",
                suggestion: "Review the partial report or run the validation again when the device is stable.",
                detail: detail ?? message
            )
        }
        return .init(
            title: "DriveCk failed",
            message: message,
            suggestion: "Review the technical detail and confirm the selected disk is ready for validation.",
            detail: detail
        )
    }
}

enum DriveCkFormattingError: LocalizedError {
    case invalidSeed(String)

    var errorDescription: String? {
        switch self {
        case let .invalidSeed(value):
            return "Invalid seed value: \(value)"
        }
    }
}

func driveCkFormatBytes(_ bytes: UInt64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"]
    var value = Double(bytes)
    var unitIndex = 0
    while value >= 1024, unitIndex + 1 < units.count {
        value /= 1024
        unitIndex += 1
    }
    return String(format: "%.2f %@", value, units[unitIndex])
}

func driveCkFormatTimestamp(_ timestamp: Int64) -> String {
    guard timestamp > 0 else {
        return "Unavailable"
    }
    let formatter = DateFormatter()
    formatter.dateStyle = .medium
    formatter.timeStyle = .medium
    return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(timestamp)))
}

func driveCkParseSeed(_ value: String) throws -> UInt64 {
    if let hex = stripKnownPrefix(value, options: ["0x", "0X"]) {
        guard let seed = UInt64(hex, radix: 16) else {
            throw DriveCkFormattingError.invalidSeed(value)
        }
        return seed
    }
    if let binary = stripKnownPrefix(value, options: ["0b", "0B"]) {
        guard let seed = UInt64(binary, radix: 2) else {
            throw DriveCkFormattingError.invalidSeed(value)
        }
        return seed
    }
    if let octal = stripKnownPrefix(value, options: ["0o", "0O"]) {
        guard let seed = UInt64(octal, radix: 8) else {
            throw DriveCkFormattingError.invalidSeed(value)
        }
        return seed
    }
    guard let seed = UInt64(value) else {
        throw DriveCkFormattingError.invalidSeed(value)
    }
    return seed
}

private func stripKnownPrefix(_ value: String, options: [String]) -> String? {
    for prefix in options where value.hasPrefix(prefix) {
        return String(value.dropFirst(prefix.count))
    }
    return nil
}
