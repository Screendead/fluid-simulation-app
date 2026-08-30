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
        ZStack(alignment: .topLeading) {
            FluidSurface(driver: driver).ignoresSafeArea()
            MotionReadout(motion: driver.motion, statsLine: driver.statsLine)
        }
        .onChange(of: scenePhase) { _, phase in driver.paused = phase != .active }
    }
}
