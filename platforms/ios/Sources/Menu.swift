import SwiftUI

/// The menu's choices live in UserDefaults under these keys: the views
/// bind to them, and the driver reads them once at attach.
enum Settings {
    static let particleScaleKey = "particleScale"
    static let flatKey = "flat"
    static let flatColourKey = "flatColour"
    static let showRateKey = "showRate"
    static let showThermalKey = "showThermal"
    static let showCostKey = "showCost"
    static let defaultScale = 1.0
    static let hotPink = "FF69B4"

    @MainActor static var particleScale: Double {
        UserDefaults.standard.object(forKey: particleScaleKey) as? Double ?? defaultScale
    }

    @MainActor static var flat: Bool { UserDefaults.standard.bool(forKey: flatKey) }

    @MainActor static var flatColour: FluidVec3 {
        FluidVec3(hex: UserDefaults.standard.string(forKey: flatColourKey) ?? hotPink)
    }
}

/// The four scales, and how the reference device runs each: the M5
/// record's ladder, measured 2026-09-02.
struct Level: Identifiable {
    enum Rating {
        case good, borderline, bad

        var colour: Color {
            switch self {
            case .good: .green
            case .borderline: .yellow
            case .bad: .red
            }
        }

        var word: String {
            switch self {
            case .good: "good"
            case .borderline: "borderline"
            case .bad: "bad"
            }
        }
    }

    let scale: Double
    let label: String
    let rating: Rating
    var id: Double { scale }

    static let ladder = [
        Level(scale: 0.25, label: "0.25x", rating: .good),
        Level(scale: 1, label: "1x", rating: .good),
        Level(scale: 4, label: "4x", rating: .borderline),
        Level(scale: 16, label: "16x", rating: .bad),
    ]
}

struct MenuSheet: View {
    @Binding var particleScale: Double
    @Binding var flat: Bool
    @Binding var flatColour: String
    @Binding var showRate: Bool
    @Binding var showThermal: Bool
    @Binding var showCost: Bool
    /// The count the core seeds at a scale, for the labels.
    let particles: (Double) -> UInt32

    private var colour: Binding<Color> {
        Binding(get: { Color(hex: flatColour) }, set: { flatColour = $0.hex })
    }

    var body: some View {
        NavigationStack {
            List {
                Section("Particles") {
                    ForEach(Level.ladder) { level in
                        Button { particleScale = level.scale } label: {
                            HStack(spacing: 12) {
                                Text(level.label).frame(width: 56, alignment: .leading)
                                Text(particles(level.scale), format: .number)
                                    .foregroundStyle(.secondary)
                                Spacer()
                                Circle().fill(level.rating.colour).frame(width: 9, height: 9)
                                Text(level.rating.word).foregroundStyle(.secondary)
                                Image(systemName: "checkmark")
                                    .opacity(level.scale == particleScale ? 1 : 0)
                            }
                        }
                        .tint(.primary)
                    }
                }
                Section("Look") {
                    Toggle("Flat colour", isOn: $flat)
                    ColorPicker("Colour", selection: colour, supportsOpacity: false)
                        .disabled(!flat)
                }
                Section("Readout") {
                    Toggle("Frame rate", isOn: $showRate)
                    Toggle("Temperature", isOn: $showThermal)
                    Toggle("Frame cost", isOn: $showCost)
                }
            }
            .navigationTitle("Render")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

extension FluidVec3 {
    /// "RRGGBB" to components 0 to 1; the colour is display bytes, so no
    /// transfer function applies.
    init(hex: String) {
        let v = UInt32(hex, radix: 16) ?? 0
        self.init(
            x: Float(v >> 16 & 0xFF) / 255,
            y: Float(v >> 8 & 0xFF) / 255,
            z: Float(v & 0xFF) / 255)
    }
}

extension Color {
    init(hex: String) {
        let c = FluidVec3(hex: hex)
        self.init(red: Double(c.x), green: Double(c.y), blue: Double(c.z))
    }

    var hex: String {
        var (r, g, b, a) = (CGFloat(0), CGFloat(0), CGFloat(0), CGFloat(0))
        UIColor(self).getRed(&r, green: &g, blue: &b, alpha: &a)
        let byte = { (v: CGFloat) in Int((min(max(v, 0), 1) * 255).rounded()) }
        return String(format: "%02X%02X%02X", byte(r), byte(g), byte(b))
    }
}
