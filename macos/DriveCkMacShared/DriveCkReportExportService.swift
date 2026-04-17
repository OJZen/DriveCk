import AppKit
import Foundation

enum DriveCkReportExportService {
    static func suggestedFilename(for target: DriveCkTargetInfo) -> String {
        let sanitized = target.name.replacingOccurrences(of: "/", with: "-")
        return "driveck-\(sanitized)-report.txt"
    }

    @MainActor
    static func chooseDestination(for target: DriveCkTargetInfo) -> URL? {
        let panel = NSSavePanel()
        panel.canCreateDirectories = true
        panel.allowedContentTypes = [.plainText]
        panel.nameFieldStringValue = suggestedFilename(for: target)
        panel.title = "Export DriveCk Report"
        panel.message = "Save the human-readable validation report."
        return panel.runModal() == .OK ? panel.url : nil
    }

    static func writeReport(_ text: String, to url: URL) throws {
        try text.write(to: url, atomically: true, encoding: .utf8)
    }
}
