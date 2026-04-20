import Observation
import SwiftUI

struct DriveCkAppShellView: View {
    @Bindable var viewModel: DriveCkAppViewModel

    var body: some View {
        NavigationSplitView {
            DriveCkSidebarView(viewModel: viewModel)
        } detail: {
            DriveCkDashboardView(viewModel: viewModel)
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar {
            ToolbarItemGroup {
                Button {
                    Task {
                        await viewModel.refreshTargets()
                    }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help("Refresh disks")
                .keyboardShortcut("r", modifiers: [.command])
                .disabled(viewModel.isRefreshing || viewModel.isRunning)

                Button {
                    viewModel.exportReport()
                } label: {
                    Image(systemName: "square.and.arrow.up")
                }
                .help("Export report")
                .disabled(!viewModel.canExportReport)
            }
        }
        .alert(item: $viewModel.presentedError) { error in
            Alert(
                title: Text(error.title),
                message: Text("\(error.message)\n\n\(error.suggestion)"),
                dismissButton: .default(Text("OK"))
            )
        }
        .animation(.snappy(duration: 0.28), value: viewModel.selectedTargetID)
        .animation(.snappy(duration: 0.28), value: viewModel.statusLine)
    }
}
