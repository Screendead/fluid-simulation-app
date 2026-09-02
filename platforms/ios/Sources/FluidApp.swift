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
    @AppStorage(Settings.particleScaleKey) private var particleScale = Settings.defaultScale
    @AppStorage(Settings.flatKey) private var flat = false
    @AppStorage(Settings.particleViewKey) private var particleView = false
    @AppStorage(Settings.flatColourKey) private var flatColour = Settings.hotPink
    @AppStorage(Settings.showRateKey) private var showRate = false
    @AppStorage(Settings.showThermalKey) private var showThermal = false
    @AppStorage(Settings.showCostKey) private var showCost = false
    // Each tap restarts the button's four seconds.
    @State private var reveal = 0
    @State private var buttonShown = false
    @State private var menuShown = false

    /// A touch is a drag through the water first. It counts as the tap
    /// that shows the button only if the finger barely moved, so a
    /// stroke never opens the menu on you.
    private static let tapSlop: CGFloat = 10

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            GeometryReader { geo in
                FluidSurface(driver: driver)
                    .gesture(
                        DragGesture(minimumDistance: 0)
                            .onChanged { v in
                                driver.touch(CGPoint(
                                    x: v.location.x / geo.size.width,
                                    y: v.location.y / geo.size.height))
                            }
                            .onEnded { v in
                                driver.touch(nil)
                                let moved = hypot(v.translation.width, v.translation.height)
                                if moved < Self.tapSlop {
                                    reveal += 1
                                    buttonShown = true
                                }
                            })
            }
            .ignoresSafeArea()
            if buttonShown {
                Button { menuShown = true } label: {
                    Image(systemName: "slider.horizontal.3")
                        .font(.title2)
                        .foregroundStyle(.white)
                        .padding(14)
                        .background(.black.opacity(0.4), in: Circle())
                }
                .padding(24)
                .transition(.opacity)
            }
        }
        .overlay(alignment: .topLeading) {
            if showRate || showThermal || showCost {
                ReadoutView(readout: driver.readout, rate: showRate, thermal: showThermal, cost: showCost)
            }
        }
        .animation(.easeInOut(duration: 0.2), value: buttonShown)
        .task(id: reveal) {
            guard reveal > 0 else { return }
            try? await Task.sleep(for: .seconds(4))
            if !menuShown && !Task.isCancelled { buttonShown = false }
        }
        .sheet(isPresented: $menuShown, onDismiss: { buttonShown = false }) {
            MenuSheet(
                particleScale: $particleScale, flat: $flat, particleView: $particleView,
                flatColour: $flatColour,
                showRate: $showRate, showThermal: $showThermal, showCost: $showCost,
                particles: driver.particles(at:))
            .presentationDetents([.medium, .large])
        }
        .statusBarHidden()
        .persistentSystemOverlays(.hidden)
        .onChange(of: scenePhase) { _, phase in driver.paused = phase != .active }
        .onChange(of: particleScale) { _, scale in driver.setParticles(scale) }
        .onChange(of: flat) { setLook() }
        .onChange(of: particleView) { setLook() }
        .onChange(of: flatColour) { setLook() }
    }

    private func setLook() {
        driver.setLook(flat: flat, particles: particleView, colour: FluidVec3(hex: flatColour))
    }
}
