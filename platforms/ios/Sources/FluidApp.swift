import SwiftUI

@main
struct FluidApp: App {
    var body: some Scene {
        WindowGroup { ContentView() }
    }
}

struct ContentView: View {
    @State private var driver = FrameDriver()
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        FluidSurface(driver: driver)
            .ignoresSafeArea()
            .statusBarHidden()
            .persistentSystemOverlays(.hidden)
            .onChange(of: scenePhase) { _, phase in driver.paused = phase != .active }
    }
}
