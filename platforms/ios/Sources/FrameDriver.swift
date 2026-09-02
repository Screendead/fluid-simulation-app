import QuartzCore
import UIKit

/// Owns the render loop: a CADisplayLink at 120 Hz feeds the newest motion
/// sample to the core each frame, and prints one stats line per second for
/// `devicectl` console capture.
@Observable
@MainActor
final class FrameDriver {
    private(set) var readout = Readout()
    var paused = false { didSet { link?.isPaused = paused } }

    @ObservationIgnored private let motion = MotionSource()
    @ObservationIgnored private var renderer: OpaquePointer?
    @ObservationIgnored private var link: CADisplayLink?
    @ObservationIgnored private var ticks = 0
    @ObservationIgnored private var active = true
    @ObservationIgnored private var lastCpuSeconds = 0.0
    @ObservationIgnored private var lastReportTime = CACurrentMediaTime()
    @ObservationIgnored private var lastFrames: UInt64 = 0
    @ObservationIgnored private var look = "glass"

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
        // 0.0006 times the core's WORLD_SCALE (4): demo sprites keep
        // their relative size in the scaled tank.
        let radius = env["FLUID_RADIUS"].flatMap(Float.init) ?? 0.0024
        let bench = env["FLUID_BENCH"].flatMap(UInt32.init) ?? 0
        let spacing = env["FLUID_SPACING"].flatMap(Float.init) ?? 0
        let sim = env["FLUID_SIM"].flatMap(UInt32.init) ?? 16
        let tracers = env["FLUID_TRACERS"].flatMap(UInt32.init) ?? 131_072
        renderer = fluid_renderer_create(
            Unmanaged.passUnretained(layer).toOpaque(), width, height, count, radius,
            bench, spacing, sim, tracers, Float(Settings.particleScale))
        guard renderer != nil else {
            print("fluid_renderer_create failed")
            return
        }
        setLook(flat: Settings.flat, colour: Settings.flatColour)
        let link = CADisplayLink(target: self, selector: #selector(tick))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        link.isPaused = paused
        self.link = link
    }

    func setParticles(_ scale: Double) {
        guard let renderer else { return }
        fluid_renderer_set_particles(renderer, Float(scale))
    }

    func particles(at scale: Double) -> UInt32 {
        guard let renderer else { return 0 }
        return fluid_renderer_particles_at(renderer, Float(scale))
    }

    func setLook(flat: Bool, colour: FluidVec3) {
        guard let renderer else { return }
        fluid_renderer_set_look(renderer, flat, colour)
        look = flat ? "flat" : "glass"
    }

    @objc private func tick(_ link: CADisplayLink) {
        guard let renderer else { return }
        let stepped = fluid_renderer_frame(
            renderer,
            FluidVec3(motion.gravity),
            FluidVec3(motion.userAcceleration),
            FluidVec3(motion.rotationRate),
            link.timestamp * 1000.0) != 0
        if stepped != active {
            active = stepped
            // Asleep, the tick survives only to feed the wake test; 30 Hz
            // holds the wake latency to two visually still frames.
            link.preferredFrameRateRange = stepped
                ? CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
                : CAFrameRateRange(minimum: 10, maximum: 30, preferred: 30)
        }
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
        // Process CPU time over the report interval: the one number that
        // says how hard the phone works for the frame, GPU aside.
        var usage = rusage()
        getrusage(RUSAGE_SELF, &usage)
        let cpuSeconds = Double(usage.ru_utime.tv_sec) + Double(usage.ru_utime.tv_usec) / 1e6
            + Double(usage.ru_stime.tv_sec) + Double(usage.ru_stime.tv_usec) / 1e6
        let now = CACurrentMediaTime()
        let elapsed = max(now - lastReportTime, 1e-3)
        let cpuPercent = (cpuSeconds - lastCpuSeconds) / elapsed * 100
        readout = Readout(
            stepped: Double(stats.frames - lastFrames) / elapsed,
            thermal: ProcessInfo.processInfo.thermalState,
            gpuMs: Double(stats.gpu_p50_us) / 1000)
        lastFrames = stats.frames
        lastCpuSeconds = cpuSeconds
        lastReportTime = now
        let line = String(
            format: "frames %llu | interval µs p50 %.0f p99 %.0f max %.0f | acq µs p50 %.0f p99 %.0f | cpu µs p50 %.0f p99 %.0f | gpu µs p50 %.0f p99 %.0f | compr %% avg %.3f max %.3f | rho %.0f..%.0f | p %.0f..%.0f Pa | dT µK %.1f..%.1f | v %.2f n %u clamp %u nbr %u | idle %llu | mem %.1f MB | batt %.0f%% %@ | cpu%% %.1f | look %@ | stats µs %.0f",
            stats.frames,
            stats.interval_p50_us, stats.interval_p99_us, stats.interval_max_us,
            stats.acquire_p50_us, stats.acquire_p99_us,
            stats.encode_p50_us, stats.encode_p99_us,
            stats.gpu_p50_us, stats.gpu_p99_us,
            stats.compression_avg * 100, stats.compression_max * 100,
            stats.density_min, stats.density_max,
            stats.pressure_min, stats.pressure_max,
            (stats.temperature_min - 293.15) * 1_000_000,
            (stats.temperature_max - 293.15) * 1_000_000,
            stats.v_max, stats.substeps, stats.clamp_count, stats.neighbour_overflow, stats.idle_frames,
            memory, battery, thermal, cpuPercent, look, statsUs)
        print(line)
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
