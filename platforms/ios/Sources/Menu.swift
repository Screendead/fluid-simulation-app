import SwiftUI

/// The menu's choices live in UserDefaults under these keys: the views
/// bind to them, and the driver reads them once at attach.
enum Settings {
    static let particleScaleKey = "particleScale"
    static let lookKey = "look"
    static let flatColourKey = "flatColour"
    static let highColourKey = "highColour"
    static let lensKey = "lens"
    static let showRateKey = "showRate"
    static let showThermalKey = "showThermal"
    static let showCostKey = "showCost"
    static let defaultScale = 1.0
    static let hotPink = "FF69B4"
    static let paleGold = "FFE9A8"

    @MainActor static var particleScale: Double {
        UserDefaults.standard.object(forKey: particleScaleKey) as? Double ?? defaultScale
    }

    @MainActor static var look: Look {
        Look(rawValue: UserDefaults.standard.integer(forKey: lookKey)) ?? .glass
    }

    /// The looks were three booleans and the lens had a `gradient`
    /// toggle beside it until 2026-09-03. Carry a phone's stored
    /// choices over once, so an update does not reset the look.
    @MainActor static func migrate() {
        let store = UserDefaults.standard
        guard store.object(forKey: lookKey) == nil, store.object(forKey: "flat") != nil else {
            return
        }
        let look: Look =
            !store.bool(forKey: "flat")
            ? .glass
            : store.bool(forKey: "particleView")
                ? .particles : store.bool(forKey: "dapple") ? .dapple : .flat
        store.set(look.rawValue, forKey: lookKey)
        store.set(store.bool(forKey: "gradient") ? store.integer(forKey: lensKey) + 1 : 0,
                  forKey: lensKey)
        for old in ["flat", "particleView", "dapple", "gradient"] {
            store.removeObject(forKey: old)
        }
    }

    @MainActor static var flatColour: FluidVec3 {
        FluidVec3(hex: UserDefaults.standard.string(forKey: flatColourKey) ?? hotPink)
    }

    @MainActor static var highColour: FluidVec3 {
        FluidVec3(hex: UserDefaults.standard.string(forKey: highColourKey) ?? paleGold)
    }

    @MainActor static var lens: UInt32 {
        UInt32(UserDefaults.standard.integer(forKey: lensKey))
    }
}

/// What the colour runs across, `Solid` for one colour and no run at
/// all. `code` is the numbering `fluid_renderer_set_look` documents.
/// The note says what the colour means, since none of these is obvious
/// from its name alone. `wheel` picks its own colours and leaves the
/// high one unread.
struct LensChoice: Identifiable {
    let code: Int
    let label: String
    let note: String
    var wheel: Bool = false
    var id: Int { code }

    static let all = [
        LensChoice(code: 0, label: "Solid", note: ""),
        LensChoice(code: 1, label: "Velocity", note: "How fast the water moves"),
        LensChoice(code: 2, label: "Acceleration", note: "How hard it is thrown about"),
        LensChoice(code: 3, label: "Pressure", note: "How hard it is squeezed"),
        LensChoice(code: 4, label: "Proximity", note: "How crowded each drop's neighbours are"),
        LensChoice(
            code: 5, label: "Direction",
            note: "Which way the water goes, around the colour wheel. Your colour "
                + "holds where it barely moves, and the wheel takes over as it speeds up.",
            wheel: true),
    ]

    /// One colour, and no second one to pick.
    var solid: Bool { code == 0 }
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
        Level(scale: 4, label: "4x", rating: .good),
        Level(scale: 16, label: "16x", rating: .bad),
    ]
}

struct MenuSheet: View {
    @Binding var particleScale: Double
    @Binding var look: Int
    @Binding var flatColour: String
    @Binding var highColour: String
    @Binding var lens: Int
    @Binding var showRate: Bool
    @Binding var showThermal: Bool
    @Binding var showCost: Bool
    /// The count the core seeds at a scale, for the labels.
    let particles: (Double) -> UInt32

    private var colour: Binding<Color> {
        Binding(get: { Color(hex: flatColour) }, set: { flatColour = $0.hex })
    }

    private var high: Binding<Color> {
        Binding(get: { Color(hex: highColour) }, set: { highColour = $0.hex })
    }

    private var choice: LensChoice {
        LensChoice.all.first { $0.code == lens } ?? LensChoice.all[0]
    }

    private var painted: Bool {
        (Look(rawValue: look) ?? .glass).painted
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
                Section {
                    Picker("Look", selection: $look) {
                        ForEach(Look.allCases) { look in
                            Text(look.label).tag(look.rawValue)
                        }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    // The glass paints the water itself, so a colour
                    // would have nothing to colour.
                    if painted {
                        ColorPicker(
                            choice.solid || choice.wheel ? "Colour" : "Low",
                            selection: colour,
                            supportsOpacity: false
                        )
                        Picker("Colour by", selection: $lens) {
                            ForEach(LensChoice.all) { choice in
                                Text(choice.label).tag(choice.code)
                            }
                        }
                        if !choice.solid && !choice.wheel {
                            ColorPicker("High", selection: high, supportsOpacity: false)
                        }
                    }
                } header: {
                    Text("Look")
                } footer: {
                    if painted && !choice.solid {
                        Text(choice.note)
                    }
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
    /// "RRGGBB" to components 0 to 1, the picker's own sRGB bytes; the
    /// core linearises them for the surface.
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

/// Which look a run draws, and the numbering
/// `fluid_renderer_set_look` documents. The menu writes the stored
/// one; a measurement run over the console cannot reach the menu, so
/// `FLUID_LOOK` names one instead, with the lens after a colon where a
/// run wants the colour to run ("flat:direction").
enum Look: Int, CaseIterable, Identifiable {
    case glass, flat, dapple, particles

    var id: Int { rawValue }

    var label: String {
        switch self {
        case .glass: "Glass"
        case .flat: "Flat"
        case .dapple: "Dapple"
        case .particles: "Particles"
        }
    }

    /// The glass paints the water itself and reads no colour.
    var painted: Bool { self != .glass }

    @MainActor static func spec(_ spec: String?) -> (look: Look, lens: UInt32) {
        guard let parts = spec?.split(separator: ":"), let named = parts.first else {
            return (Settings.look, Settings.lens)
        }
        let look = Look.allCases.first { $0.label.lowercased() == named.lowercased() } ?? .glass
        let lens = parts.dropFirst().first.flatMap { name in
            LensChoice.all.first { $0.label.lowercased() == name.lowercased() }
        }
        return (look, UInt32(lens?.code ?? 0))
    }
}

/// What the log line reports, so a number carries the look it was
/// taken in.
@MainActor func lookName(_ look: Look, _ lens: UInt32) -> String {
    guard let choice = LensChoice.all.first(where: { $0.code == Int(lens) }), !choice.solid else {
        return look.label.lowercased()
    }
    return "\(look.label.lowercased())+\(choice.label.lowercased())"
}
