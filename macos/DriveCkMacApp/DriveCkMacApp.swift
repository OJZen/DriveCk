import SwiftUI

@main
struct DriveCkMacApp: App {
    @State private var viewModel = DriveCkAppViewModel()

    var body: some Scene {
        WindowGroup("DriveCk") {
            DriveCkAppShellView(viewModel: viewModel)
                .task {
                    viewModel.loadIfNeeded()
                }
        }
        .defaultSize(width: 1240, height: 860)
    }
}
