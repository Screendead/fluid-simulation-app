import QuartzCore
import UIKit
import Observation

/// Owns the render loop: a CADisplayLink at 120 Hz feeds the newest motion
/// sample to the core each frame, and prints one stats line per second for
/// `devicectl` console capture.
@MainActor
@Observable
final class FrameDriver {
    let motion = MotionSource()
    private(set) var statsLine = ""
    var paused = false { didSet { link?.isPaused = paused } }

    private var renderer: OpaquePointer?
    private var link: CADisplayLink?
    private var ticks = 0

    init() {
        UIDevice.current.isBatteryMonitoringEnabled = true
    }

    func attach(layer: CAMetalLayer, pixelSize: CGSize) {
        let width = UInt32(pixelSize.width)
        let height = UInt32(pixelSize.height)
        if let renderer {
            fluid_renderer_resize(renderer, width, height)
            return
        }
        let env = ProcessInfo.processInfo.environment
        let count = env["FLUID_PARTICLES"].flatMap(UInt32.init) ?? 50_000
        let radius = env["FLUID_RADIUS"].flatMap(Float.init) ?? 0.0006
        let bench = env["FLUID_BENCH"].flatMap(UInt32.init) ?? 0
        let spacing = env["FLUID_SPACING"].flatMap(Float.init) ?? 0.002
        let sim = env["FLUID_SIM"].flatMap(UInt32.init) ?? 0
        renderer = fluid_renderer_create(
            Unmanaged.passUnretained(layer).toOpaque(), width, height, count, radius,
            bench, spacing, sim)
        guard renderer != nil else {
            statsLine = "renderer failed; see the console"
            return
        }
        let link = CADisplayLink(target: self, selector: #selector(tick))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        link.isPaused = paused
        self.link = link
    }

    @objc private func tick(_ link: CADisplayLink) {
        guard let renderer else { return }
        fluid_renderer_frame(
            renderer,
            FluidVec3(motion.gravity),
            FluidVec3(motion.userAcceleration),
            link.timestamp * 1000.0)
        ticks += 1
        if ticks % 120 == 0 {
            let before = CACurrentMediaTime()
            let stats = fluid_renderer_stats(renderer)
            report(stats, statsUs: (CACurrentMediaTime() - before) * 1_000_000)
        }
    }

    private func report(_ stats: FluidRenderStats, statsUs: Double) {
        let memory = Double(physFootprint()) / 1_048_576.0
        let battery = UIDevice.current.batteryLevel * 100
        let thermal = ["nominal", "fair", "serious", "critical"][ProcessInfo.processInfo.thermalState.rawValue]
        statsLine = String(
            format: "frames %llu | interval µs p50 %.0f p99 %.0f max %.0f | cpu µs p50 %.0f p99 %.0f | gpu µs p50 %.0f p99 %.0f | compr %% avg %.3f max %.3f | rho %.0f..%.0f | p %.0f..%.0f Pa | dT µK %.1f..%.1f | v %.2f n %u clamp %u | mem %.1f MB | batt %.0f%% %@ | stats µs %.0f",
            stats.frames,
            stats.interval_p50_us, stats.interval_p99_us, stats.interval_max_us,
            stats.encode_p50_us, stats.encode_p99_us,
            stats.gpu_p50_us, stats.gpu_p99_us,
            stats.compression_avg * 100, stats.compression_max * 100,
            stats.density_min, stats.density_max,
            stats.pressure_min, stats.pressure_max,
            (stats.temperature_min - 293.15) * 1_000_000,
            (stats.temperature_max - 293.15) * 1_000_000,
            stats.v_max, stats.substeps, stats.clamp_count,
            memory, battery, thermal, statsUs)
        print(statsLine)
    }

    private func physFootprint() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(
            MemoryLayout<task_vm_info_data_t>.stride / MemoryLayout<integer_t>.stride)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.phys_footprint : 0
    }
}
