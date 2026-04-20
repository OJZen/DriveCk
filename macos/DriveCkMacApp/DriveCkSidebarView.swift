import Observation
import SwiftUI

struct DriveCkSidebarView: View {
    @Bindable var viewModel: DriveCkAppViewModel

    var body: some View {
        List(selection: selection) {
            Section {
                if viewModel.targets.isEmpty {
                    ContentUnavailableView(
                        "No USB disks",
                        systemImage: "externaldrive.badge.questionmark",
                        description: Text("Attach a removable disk and refresh.")
                    )
                    .frame(maxWidth: .infinity, minHeight: 220)
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
                } else {
                    ForEach(viewModel.targets) { target in
                        DriveCkSidebarRow(target: target)
                        .tag(Optional.some(target.id))
                    }
                }
            } header: {
                HStack(spacing: 8) {
                    Text("Eligible Disks")
                    Spacer()
                    DriveCkCountBadge(text: "\(viewModel.targets.count)")
                }
            }
        }
        .listStyle(.sidebar)
        .disabled(viewModel.isRunning)
        .safeAreaInset(edge: .top, spacing: 0) {
            sidebarHeader
        }
        .navigationSplitViewColumnWidth(min: 220, ideal: 260, max: 320)
    }

    private var selection: Binding<String?> {
        Binding(
            get: { viewModel.selectedTargetID },
            set: { viewModel.selectTarget($0) }
        )
    }

    private var sidebarHeader: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("DriveCk")
                .font(.title3.weight(.semibold))
            Text(headerSubtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 12)
        .background(.bar)
    }

    private var headerSubtitle: String {
        if viewModel.isRunning {
            return "Selection is locked during validation."
        }
        if viewModel.targets.isEmpty {
            return "Connect a removable USB disk to begin."
        }
        return "\(viewModel.targets.count) disk\(viewModel.targets.count == 1 ? "" : "s") available"
    }
}

private struct DriveCkSidebarRow: View {
    var target: DriveCkTargetInfo

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: target.isUsb ? "externaldrive.badge.connected.to.line.below" : "externaldrive")
                .font(.body.weight(.medium))
                .foregroundStyle(.secondary)
                .frame(width: 18)

            VStack(alignment: .leading, spacing: 2) {
                Text(target.displayName)
                    .font(.callout.weight(.semibold))
                    .lineLimit(1)
                Text(target.sidebarSubtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 2)
    }
}
