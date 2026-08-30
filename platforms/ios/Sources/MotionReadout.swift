import SwiftUI

struct MotionReadout: View {
    let motion: MotionSource
    let statsLine: String

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            row("gravity (g)", motion.gravity.x, motion.gravity.y, motion.gravity.z)
            row("user (g)", motion.userAcceleration.x, motion.userAcceleration.y, motion.userAcceleration.z)
            row("body force (m/s²)", Double(motion.bodyForce.x), Double(motion.bodyForce.y), Double(motion.bodyForce.z))
            Text("samples \(motion.sampleCount)")
            Text(statsLine).font(.system(.caption2, design: .monospaced))
        }
        .font(.system(.body, design: .monospaced))
        .foregroundStyle(.white)
        .padding()
        .background(.black.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))
        .padding()
    }

    private func row(_ label: String, _ x: Double, _ y: Double, _ z: Double) -> some View {
        VStack(alignment: .leading) {
            Text(label).foregroundStyle(.secondary)
            Text(String(format: "%+7.2f %+7.2f %+7.2f", x, y, z))
        }
    }
}
