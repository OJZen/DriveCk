import Darwin
import Foundation

private struct CLIOptions {
    var showHelp = false
    var listOnly = false
    var assumeYes = false
    var seed: UInt64?
    var outputPath: String?
    var targetQuery: String?
}

private struct PrivilegedHelperOptions {
    var directoryPath: String
    var requestPayload: String
}

private struct CLIError: Error {
    var message: String
    var exitCode: Int32
}

private final class CLIProgressState: @unchecked Sendable {
    private let lock = NSLock()
    private var lastProgressLength = 0

    func update(with lineLength: Int, finalUpdate: Bool) -> Int {
        lock.lock()
        defer { lock.unlock() }
        let previousLength = lastProgressLength
        lastProgressLength = finalUpdate ? 0 : lineLength
        return previousLength
    }

    func currentLength() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return lastProgressLength
    }
}

struct DriveCkMacCLI {
    static func run() -> Int32 {
        do {
            if let helperOptions = try parsePrivilegedHelperOptions(arguments: CommandLine.arguments) {
                return runPrivilegedHelper(options: helperOptions)
            }

            let options = try parseOptions(arguments: CommandLine.arguments)
            if options.showHelp {
                printUsage(program: CommandLine.arguments.first ?? "driveck-mac")
                return 0
            }

            let discovery = DriveCkDiskDiscoveryService()
            if options.listOnly {
                let targets = try discovery.loadTargets()
                printTargets(targets)
                return 0
            }

            guard let query = options.targetQuery else {
                throw CLIError(
                    message: "A device path or disk identifier is required unless --list is used.",
                    exitCode: 2
                )
            }

            let targets = try discovery.loadTargets()
            let target = try resolveTarget(query: query, targets: targets)
            try confirmValidation(target: target, assumeYes: options.assumeYes)
            try DriveCkDiskUnmountService.unmount(target: target)

            let isInteractive = isatty(fileno(stderr)) == 1
            let progressState = CLIProgressState()
            let result = try DriveCkValidationCoordinator.validateSync(
                request: DriveCkValidationRequest(
                    target: target,
                    options: DriveCkValidationOptions(seed: options.seed)
                ),
                onProgress: { snapshot in
                    guard isInteractive else {
                        return
                    }
                    let phase = pad(snapshot.phase, width: 12)
                    let line = "\r\(phase) \(String(format: "%3d", snapshot.current))/\(snapshot.total)"
                    let previousLength = progressState.update(with: line.count, finalUpdate: snapshot.finalUpdate)
                    let padding = max(0, previousLength - line.count)
                    fputs(line + String(repeating: " ", count: padding), stderr)
                    fflush(stderr)
                    if snapshot.finalUpdate {
                        fputs("\n", stderr)
                    }
                },
                isCancelled: { false }
            )

            if isInteractive, progressState.currentLength() > 0 {
                fputs("\n", stderr)
            }

            guard let response = result.response else {
                throw CLIError(
                    message: result.error ?? "Validation failed before a report could be produced.",
                    exitCode: 2
                )
            }

            let reportText = try DriveCkFFIBridge.renderReport(response: response)
            print(reportText)

            if let outputPath = options.outputPath {
                try DriveCkReportExportService.writeReport(reportText, to: URL(fileURLWithPath: outputPath))
            }

            if let error = result.error, !error.localizedCaseInsensitiveContains("cancelled") {
                fputs("\(error)\n", stderr)
            }

            if result.error != nil || response.report.hasFailures {
                return 1
            }
            return 0
        } catch let error as CLIError {
            fputs("\(error.message)\n", stderr)
            return error.exitCode
        } catch let error as DriveCkUserFacingError {
            fputs("\(error.message)\n", stderr)
            if let detail = error.detail {
                fputs("\(detail)\n", stderr)
            }
            return 2
        } catch {
            fputs("\(error.localizedDescription)\n", stderr)
            return 2
        }
    }

    private static func parsePrivilegedHelperOptions(arguments: [String]) throws -> PrivilegedHelperOptions? {
        guard arguments.contains("--privileged-helper") else {
            return nil
        }

        var directoryPath: String?
        var requestPayload: String?
        var index = 1

        while index < arguments.count {
            let argument = arguments[index]
            switch argument {
            case "--privileged-helper":
                break
            case "--privileged-helper-directory":
                index += 1
                guard index < arguments.count else {
                    throw CLIError(message: "--privileged-helper-directory requires a path.", exitCode: 2)
                }
                directoryPath = arguments[index]
            case "--privileged-helper-request":
                index += 1
                guard index < arguments.count else {
                    throw CLIError(message: "--privileged-helper-request requires a payload.", exitCode: 2)
                }
                requestPayload = arguments[index]
            default:
                throw CLIError(message: "Unknown privileged helper option: \(argument)", exitCode: 2)
            }
            index += 1
        }

        guard let directoryPath else {
            throw CLIError(
                message: "Privileged helper mode requires a secure IPC directory.",
                exitCode: 2
            )
        }
        guard let requestPayload else {
            throw CLIError(
                message: "Privileged helper mode requires an embedded request payload.",
                exitCode: 2
            )
        }

        return PrivilegedHelperOptions(directoryPath: directoryPath, requestPayload: requestPayload)
    }

    private static func parseOptions(arguments: [String]) throws -> CLIOptions {
        var options = CLIOptions()
        var index = 1

        while index < arguments.count {
            let argument = arguments[index]
            switch argument {
            case "--list", "-l":
                options.listOnly = true
            case "--yes", "-y":
                options.assumeYes = true
            case "--help", "-h":
                options.showHelp = true
            case "--output", "-o":
                index += 1
                guard index < arguments.count else {
                    throw CLIError(message: "--output requires a path.", exitCode: 2)
                }
                options.outputPath = arguments[index]
            case "--seed":
                index += 1
                guard index < arguments.count else {
                    throw CLIError(message: "--seed requires a value.", exitCode: 2)
                }
                options.seed = try driveCkParseSeed(arguments[index])
            default:
                guard !argument.hasPrefix("-") else {
                    throw CLIError(message: "Unknown option: \(argument)", exitCode: 2)
                }
                guard options.targetQuery == nil else {
                    throw CLIError(message: "Only one target may be provided.", exitCode: 2)
                }
                options.targetQuery = argument
            }
            index += 1
        }

        if !options.showHelp, !options.listOnly, options.targetQuery == nil {
            throw CLIError(
                message: "A device path or disk identifier is required unless --list is used.",
                exitCode: 2
            )
        }

        return options
    }

    private static func resolveTarget(query: String, targets: [DriveCkTargetInfo]) throws -> DriveCkTargetInfo {
        guard let target = targets.first(where: { $0.commandLineAliases.contains(query) }) else {
            throw CLIError(
                message: "Could not find a USB whole-disk target matching \(query). Use --list to inspect available devices.",
                exitCode: 2
            )
        }
        return target
    }

    private static func runPrivilegedHelper(options: PrivilegedHelperOptions) -> Int32 {
        let directoryURL = URL(fileURLWithPath: options.directoryPath, isDirectory: true)

        do {
            try DriveCkPrivilegedHelperIPC.validateSecureDirectory(at: directoryURL)
            let ipcURLs = DriveCkPrivilegedHelperIPC.contextURLs(for: directoryURL)
            let request = try DriveCkPrivilegedHelperIPC.decodeRequestPayload(options.requestPayload)
            guard request.action == .validate, var validationRequest = request.validationRequest else {
                throw DriveCkUserFacingError.from(
                    message: "Privileged helper received an unsupported request."
                )
            }

            let discovery = DriveCkDiskDiscoveryService()
            let expectedTarget = validationRequest.target
            var currentTarget = try discovery.resolveCurrentTarget(matching: expectedTarget)
            let progressWriter = try DriveCkPrivilegedHelperIPC.JSONLineWriter(url: ipcURLs.progressURL)
            try? progressWriter.append(
                DriveCkProgressSnapshot(
                    phase: "Preparing",
                    current: 0,
                    total: 1,
                    finalUpdate: false,
                    sampleIndex: nil,
                    sampleStatus: nil
                )
            )
            try DriveCkDiskUnmountService.unmount(target: currentTarget)
            currentTarget = try discovery.resolveCurrentTarget(matching: expectedTarget)
            try? progressWriter.append(
                DriveCkProgressSnapshot(
                    phase: "Preparing",
                    current: 1,
                    total: 1,
                    finalUpdate: true,
                    sampleIndex: nil,
                    sampleStatus: nil
                )
            )

            validationRequest.target = currentTarget
            let result = try DriveCkValidationCoordinator.validateSync(
                request: validationRequest,
                onProgress: { snapshot in
                    try? progressWriter.append(snapshot)
                },
                isCancelled: {
                    FileManager.default.fileExists(atPath: ipcURLs.cancelURL.path)
                }
            )

            try DriveCkPrivilegedHelperIPC.writeHelperOutputJSON(
                DriveCkPrivilegedHelperResponse.validation(result),
                to: ipcURLs.responseURL
            )
            return 0
        } catch let error as DriveCkUserFacingError {
            try? DriveCkPrivilegedHelperIPC.writeHelperOutputJSON(
                DriveCkPrivilegedHelperResponse.failure(error),
                to: DriveCkPrivilegedHelperIPC
                    .contextURLs(for: URL(fileURLWithPath: options.directoryPath, isDirectory: true))
                    .responseURL
            )
            return 1
        } catch {
            let userFacing = DriveCkUserFacingError.from(message: error.localizedDescription)
            try? DriveCkPrivilegedHelperIPC.writeHelperOutputJSON(
                DriveCkPrivilegedHelperResponse.failure(userFacing),
                to: DriveCkPrivilegedHelperIPC
                    .contextURLs(for: URL(fileURLWithPath: options.directoryPath, isDirectory: true))
                    .responseURL
            )
            return 1
        }
    }

    private static func confirmValidation(target: DriveCkTargetInfo, assumeYes: Bool) throws {
        guard !assumeYes else {
            return
        }
        guard isatty(fileno(stdin)) == 1 else {
            throw CLIError(
                message: "Refusing to touch \(target.path) without --yes in a non-interactive session.",
                exitCode: 2
            )
        }

        let summary = [
            target.displayName,
            driveCkFormatBytes(target.sizeBytes),
            target.transportLabel.isEmpty ? nil : target.transportLabel,
            target.isRemovable ? "removable" : nil,
            target.isUsb ? "usb" : nil,
        ]
        .compactMap { $0 }
        .joined(separator: ", ")

        fputs(
            """
            About to validate \(target.path) (\(summary)).
            DriveCk temporarily overwrites sampled regions and restores them afterwards.
            Continue? [y/N]:
            """,
            stderr
        )
        fflush(stderr)

        guard let line = readLine(), line.first.map({ $0 == "y" || $0 == "Y" }) == true else {
            throw CLIError(message: "Validation cancelled.", exitCode: 1)
        }
    }

    private static func printTargets(_ targets: [DriveCkTargetInfo]) {
        print("\(pad("PATH", width: 16)) \(pad("SIZE", width: 12)) \(pad("TRANSPORT", width: 12)) MODEL")
        guard !targets.isEmpty else {
            print("No USB whole-disk devices are currently available.")
            return
        }

        for target in targets {
            print(
                "\(pad(target.path, width: 16)) \(pad(driveCkFormatBytes(target.sizeBytes), width: 12)) \(pad(target.transportLabel, width: 12)) \(target.displayName)"
            )
        }
    }

    private static func printUsage(program: String) {
        print(
            """
            Usage:
              \(program) --list
              \(program) [--yes] [--seed N] [--output FILE] DEVICE

            Examples:
              \(program) --list
              \(program) --yes disk2
              \(program) --yes --output report.txt /dev/rdisk2

            Options:
              -l, --list          List USB whole-disk targets.
              -o, --output FILE   Save the text report to FILE.
              -y, --yes           Skip the destructive-operation confirmation.
                  --seed N        Use a fixed seed for deterministic sample data.
              -h, --help          Show this help text.
            """
        )
    }

    private static func pad(_ text: String, width: Int) -> String {
        if text.count >= width {
            return text
        }
        return text + String(repeating: " ", count: width - text.count)
    }
}

exit(DriveCkMacCLI.run())
