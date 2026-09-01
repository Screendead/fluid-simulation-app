import CoreMotion

@MainActor
final class MotionSource {
    private(set) var gravity = CMAcceleration()
    private(set) var userAcceleration = CMAcceleration()
    private(set) var rotationRate = CMRotationRate()
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
        rotationRate = motion.rotationRate
    }
}

extension FluidVec3 {
    init(_ a: CMAcceleration) {
        self.init(x: Float(a.x), y: Float(a.y), z: Float(a.z))
    }

    init(_ r: CMRotationRate) {
        self.init(x: Float(r.x), y: Float(r.y), z: Float(r.z))
    }
}
