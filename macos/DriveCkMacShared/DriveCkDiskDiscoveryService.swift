import DiskArbitration
import Foundation

struct DriveCkDiskDiscoveryService {
    func loadTargets() throws -> [DriveCkTargetInfo] {
        guard let session = DASessionCreate(kCFAllocatorDefault) else {
            throw DriveCkUserFacingError.from(message: "Unable to create a Disk Arbitration session.")
        }

        let eligibleWholeDisks = try externalPhysicalWholeDisks()
        if eligibleWholeDisks.isEmpty {
            return []
        }
        let targets = eligibleWholeDisks.compactMap { name -> DriveCkTargetInfo? in
            guard let description = description(for: name, session: session),
                  boolValue(for: description, key: kDADiskDescriptionMediaWholeKey as String)
            else {
                return nil
            }

            let isInternal = boolValue(for: description, key: kDADiskDescriptionDeviceInternalKey as String)
            let isRemovable = boolValue(for: description, key: kDADiskDescriptionMediaRemovableKey as String)
                || boolValue(for: description, key: kDADiskDescriptionMediaEjectableKey as String)
            let transport = stringValue(for: description, key: kDADiskDescriptionDeviceProtocolKey as String)
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
            let size = uint64Value(for: description, key: kDADiskDescriptionMediaSizeKey as String)
            guard !isInternal, size > 0 else {
                return nil
            }

            let preferredPath = rawDevicePath(for: name)
            let blockSize = uint64Value(for: description, key: kDADiskDescriptionMediaBlockSizeKey as String)
            let mediaName = stringValue(for: description, key: kDADiskDescriptionMediaNameKey as String)
            let deviceVendor = stringValue(for: description, key: kDADiskDescriptionDeviceVendorKey as String)
            let deviceModel = stringValue(for: description, key: kDADiskDescriptionDeviceModelKey as String)
            let devicePath = stringValue(for: description, key: kDADiskDescriptionDevicePathKey as String)
            let busPath = stringValue(for: description, key: kDADiskDescriptionBusPathKey as String)
            let isUsb = looksLikeUSBStorageDevice(
                transport: transport,
                devicePath: devicePath,
                busPath: busPath
            )
            guard isUsb else {
                return nil
            }

            return DriveCkTargetInfo(
                kind: .BlockDevice,
                path: preferredPath,
                name: name,
                vendor: deviceVendor,
                model: firstNonEmpty([deviceModel, mediaName, name]),
                transport: transport.isEmpty ? (isRemovable ? "removable" : "external") : transport,
                sizeBytes: size,
                logicalBlockSize: blockSize == 0 ? 4096 : UInt32(clamping: blockSize),
                deviceGUID: dataStringValue(for: description, key: kDADiskDescriptionDeviceGUIDKey as String),
                mediaUUID: uuidStringValue(for: description, key: kDADiskDescriptionMediaUUIDKey as String),
                devicePath: devicePath,
                busPath: busPath,
                isBlockDevice: true,
                isRemovable: isRemovable,
                isUsb: isUsb,
                // Mounted state is resolved inside the privileged path right before
                // disk operations so the GUI does not need removable-volume metadata.
                isMounted: false,
                // We only know whether uncached I/O is active after opening the raw device.
                directIo: false
            )
        }

        if !targets.isEmpty {
            return targets.sorted { left, right in
                left.path.localizedStandardCompare(right.path) == .orderedAscending
            }
        }

        return []
    }

    func resolveCurrentTarget(matching expected: DriveCkTargetInfo) throws -> DriveCkTargetInfo {
        let currentTargets = try loadTargets()
        guard let current = currentTargets.first(where: { $0.name == expected.name || $0.path == expected.path }) else {
            throw DriveCkUserFacingError(
                title: "Disk changed before authorization",
                message: "The selected disk is no longer available at its original device node.",
                suggestion: "Refresh the disk list, reselect the disk, and start again.",
                detail: "Expected: \(expected.privilegedIdentitySummary)"
            )
        }

        guard current.matchesPrivilegedIdentity(of: expected) else {
            throw DriveCkUserFacingError(
                title: "Disk changed before authorization",
                message: "The disk at \(expected.name) no longer matches the device you selected before approving administrator access.",
                suggestion: "Refresh the disk list and start the action again so DriveCk can verify the correct disk.",
                detail: """
                Expected: \(expected.privilegedIdentitySummary)
                Current: \(current.privilegedIdentitySummary)
                """
            )
        }

        return current
    }

    private func description(for bsdName: String, session: DASession) -> [String: Any]? {
        bsdName.withCString { pointer in
            guard let disk = DADiskCreateFromBSDName(kCFAllocatorDefault, session, pointer) else {
                return nil
            }
            guard let description = DADiskCopyDescription(disk) else {
                return nil
            }
            return description as? [String: Any]
        }
    }

    private func externalPhysicalWholeDisks() throws -> [String] {
        let plist = try runDiskutilPlist(arguments: ["list", "-plist", "external", "physical"])
        if let wholeDisks = plist["WholeDisks"] as? [String] {
            return wholeDisks
        }
        if let allDisks = plist["AllDisks"] as? [String] {
            return allDisks.compactMap(rootDiskName(from:))
        }
        return []
    }

    private func runDiskutilPlist(arguments: [String]) throws -> [String: Any] {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/diskutil")
        process.arguments = arguments

        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        try process.run()
        process.waitUntilExit()

        let stdoutData = stdout.fileHandleForReading.readDataToEndOfFile()
        let stderrData = stderr.fileHandleForReading.readDataToEndOfFile()
        guard process.terminationStatus == 0 else {
            let detail = String(data: stderrData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
            throw DriveCkUserFacingError.from(
                message: "diskutil failed while enumerating disks.",
                detail: detail?.isEmpty == false ? detail : nil
            )
        }

        let plist = try PropertyListSerialization.propertyList(from: stdoutData, options: [], format: nil)
        guard let dictionary = plist as? [String: Any] else {
            throw DriveCkUserFacingError.from(message: "diskutil returned an unexpected property list.")
        }
        return dictionary
    }

    private func looksLikeUSBStorageDevice(
        transport: String,
        devicePath: String,
        busPath: String
    ) -> Bool {
        let normalizedTransport = transport.lowercased()
        if normalizedTransport.contains("usb") {
            return true
        }

        let normalizedDevicePath = devicePath.lowercased()
        if normalizedDevicePath.contains("usb") {
            return true
        }

        let normalizedBusPath = busPath.lowercased()
        if normalizedBusPath.contains("usb") {
            return true
        }

        return false
    }

    private func rootDiskName(from name: String) -> String? {
        guard name.hasPrefix("disk") else {
            return nil
        }
        let suffix = String(name.dropFirst("disk".count))
        guard !suffix.isEmpty else {
            return nil
        }
        let rootDigits = suffix.prefix { $0.isWholeNumber }
        guard !rootDigits.isEmpty else {
            return nil
        }
        let remainder = suffix.dropFirst(rootDigits.count)
        guard remainder.isEmpty || (remainder.first == "s" && remainder.dropFirst().allSatisfy { $0.isWholeNumber }) else {
            return nil
        }
        return "disk\(rootDigits)"
    }

    private func rawDevicePath(for diskName: String) -> String {
        let rawPath = "/dev/r\(diskName)"
        return FileManager.default.fileExists(atPath: rawPath) ? rawPath : "/dev/\(diskName)"
    }

    private func stringValue(for description: [String: Any], key: String) -> String {
        (description[key] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    private func boolValue(for description: [String: Any], key: String) -> Bool {
        if let value = description[key] as? Bool {
            return value
        }
        if let number = description[key] as? NSNumber {
            return number.boolValue
        }
        return false
    }

    private func uint64Value(for description: [String: Any], key: String) -> UInt64 {
        if let value = description[key] as? UInt64 {
            return value
        }
        if let number = description[key] as? NSNumber {
            return number.uint64Value
        }
        return 0
    }

    private func uuidStringValue(for description: [String: Any], key: String) -> String? {
        if let value = description[key] as? UUID {
            return value.uuidString.lowercased()
        }
        return nil
    }

    private func dataStringValue(for description: [String: Any], key: String) -> String? {
        guard let data = description[key] as? Data, !data.isEmpty else {
            return nil
        }
        return data.map { String(format: "%02x", $0) }.joined()
    }

    private func firstNonEmpty(_ values: [String]) -> String {
        values
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first(where: { !$0.isEmpty }) ?? ""
    }
}
