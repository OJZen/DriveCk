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
        let apfsPhysicalRoots = try apfsContainerPhysicalRoots()
        let deviceNames = try FileManager.default.contentsOfDirectory(atPath: "/dev")
            .compactMap(canonicalDiskName(from:))
        let uniqueNames = Array(Set(deviceNames)).sorted()

        var descriptions: [String: [String: Any]] = [:]
        var mountedWholeDisks = Set<String>()

        for name in uniqueNames {
            guard let description = description(for: name, session: session) else {
                continue
            }
            descriptions[name] = description
            if !boolValue(for: description, key: kDADiskDescriptionMediaWholeKey as String),
               hasMountedVolume(description),
               let root = rootDiskName(from: name)
            {
                mountedWholeDisks.formUnion(physicalRoots(for: root, apfsPhysicalRoots: apfsPhysicalRoots))
            }
        }

        let targets = eligibleWholeDisks.compactMap { name -> DriveCkTargetInfo? in
            guard let description = descriptions[name],
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
            let mounted = mountedWholeDisks.contains(name) || hasMountedVolume(description)

            return DriveCkTargetInfo(
                kind: .BlockDevice,
                path: preferredPath,
                name: name,
                vendor: "",
                model: mediaName.isEmpty ? name : mediaName,
                transport: transport.isEmpty ? (isRemovable ? "removable" : "external") : transport,
                sizeBytes: size,
                logicalBlockSize: blockSize == 0 ? 4096 : UInt32(clamping: blockSize),
                isBlockDevice: true,
                isRemovable: isRemovable,
                isUsb: transport.contains("usb"),
                isMounted: mounted,
                directIo: true
            )
        }

        if !targets.isEmpty {
            return targets.sorted { left, right in
                left.path.localizedStandardCompare(right.path) == .orderedAscending
            }
        }

        return []
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

    private func apfsContainerPhysicalRoots() throws -> [String: Set<String>] {
        let plist = try runDiskutilPlist(arguments: ["apfs", "list", "-plist"])
        guard let containers = plist["Containers"] as? [[String: Any]] else {
            return [:]
        }

        var mapping: [String: Set<String>] = [:]
        for container in containers {
            guard let containerReference = container["ContainerReference"] as? String else {
                continue
            }

            let physicalRoots = ((container["PhysicalStores"] as? [[String: Any]]) ?? [])
                .compactMap { store in
                    (store["DeviceIdentifier"] as? String).flatMap(rootDiskName(from:))
                }
            if !physicalRoots.isEmpty {
                mapping[containerReference] = Set(physicalRoots)
            }
        }
        return mapping
    }

    private func physicalRoots(
        for diskName: String,
        apfsPhysicalRoots: [String: Set<String>]
    ) -> Set<String> {
        apfsPhysicalRoots[diskName] ?? [diskName]
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

    private func canonicalDiskName(from name: String) -> String? {
        guard name.hasPrefix("disk") || name.hasPrefix("rdisk") else {
            return nil
        }
        let normalized = name.hasPrefix("rdisk") ? String(name.dropFirst()) : name
        guard rootDiskName(from: normalized) != nil else {
            return nil
        }
        return normalized
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

    private func hasMountedVolume(_ description: [String: Any]) -> Bool {
        description[kDADiskDescriptionVolumePathKey as String] != nil
    }
}
