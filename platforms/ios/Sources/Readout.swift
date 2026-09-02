import SwiftUI

/// What the readout shows, refreshed once a second with the stats line.
struct Readout {
    /// Frames stepped per second; zero while the settled sim sleeps.
    var stepped = 0.0
    var thermal = ProcessInfo.ThermalState.nominal
    var gpuMs = 0.0
}

struct ReadoutView: View {
    let readout: Readout
    let rate: Bool
    let thermal: Bool
    let cost: Bool

    /// One 120 Hz frame.
    static let budgetMs = 1000.0 / 120.0

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            if rate {
                if readout.stepped > 0 {
                    line(String(format: "%.0f fps", readout.stepped), rateColour)
                } else {
                    line("idle", .gray)
                }
            }
            if thermal {
                line("temperature \(thermalWord)", thermalColour)
            }
            if cost {
                line(String(format: "%.1f / %.1f ms", readout.gpuMs, Self.budgetMs), costColour)
            }
        }
        .font(.system(.footnote, design: .monospaced))
        .padding(8)
        .background(.black.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
        .padding()
    }

    private func line(_ text: String, _ colour: Color) -> some View {
        HStack(spacing: 6) {
            Circle().fill(colour).frame(width: 7, height: 7)
            Text(text).foregroundStyle(.white)
        }
    }

    private var rateColour: Color {
        readout.stepped >= 110 ? .green : readout.stepped >= 55 ? .yellow : .red
    }

    private var thermalWord: String {
        switch readout.thermal {
        case .nominal: "nominal"
        case .fair: "fair"
        case .serious: "serious"
        case .critical: "critical"
        @unknown default: "unknown"
        }
    }

    private var thermalColour: Color {
        switch readout.thermal {
        case .nominal: .green
        case .fair: .yellow
        case .serious: .orange
        default: .red
        }
    }

    private var costColour: Color {
        readout.gpuMs < 0.8 * Self.budgetMs ? .green : readout.gpuMs < Self.budgetMs ? .yellow : .red
    }
}
