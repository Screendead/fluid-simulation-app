import CoreMotion
import Observation

@MainActor
@Observable
final class MotionSource {
    private(set) var gravity = CMAcceleration()
    private(set) var userAcceleration = CMAcceleration()
    private(set) var bodyForce = FluidVec3(x: 0, y: 0, z: 0)
    private(set) var sampleCount = 0
    private let manager = CMMotionManager()

    init() {
        manager.deviceMotionUpdateInterval = 1.0 / 100.0
        manager.startDeviceMotionUpdates(to: .main) { motion, _ in
            guard let motion else { return }
            MainActor.assumeIsolated { self.take(motion) }
        }
    }

    private func take(_ motion: CMDeviceMotion) {
        gravity = motion.gravity
        userAcceleration = motion.userAcceleration
        bodyForce = fluid_body_force(FluidVec3(gravity), FluidVec3(userAcceleration))
        sampleCount += 1
    }
}

extension FluidVec3 {
    init(_ a: CMAcceleration) {
        self.init(x: Float(a.x), y: Float(a.y), z: Float(a.z))
    }
}
