import SwiftUI

final class MetalLayerView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
    var onLayout: ((CAMetalLayer, CGSize) -> Void)?
    /// A finger: its slot, where it presses normalised over the view,
    /// and whether it is still down. The slot names one finger for as
    /// long as it stays on the glass.
    var onTouch: ((UInt32, CGPoint, Bool) -> Void)?
    /// A touch that went nowhere. The menu button rides on this, so a
    /// stroke through the water can never open the menu.
    var onTap: (() -> Void)?

    private static let tapSlop: CGFloat = 10
    private var slots = [UITouch?](repeating: nil, count: Int(FLUID_TOUCH_SLOTS))
    private var starts = [CGPoint](repeating: .zero, count: Int(FLUID_TOUCH_SLOTS))

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
    }

    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = traitCollection.displayScale
        let metalLayer = layer as! CAMetalLayer
        metalLayer.contentsScale = scale
        let size = CGSize(width: bounds.width * scale, height: bounds.height * scale)
        guard size.width > 0, size.height > 0 else { return }
        onLayout?(metalLayer, size)
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            guard let slot = slots.firstIndex(where: { $0 == nil }) else { continue }
            slots[slot] = touch
            starts[slot] = touch.location(in: self)
            report(slot, touch, down: true)
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            guard let slot = slots.firstIndex(of: touch) else { continue }
            report(slot, touch, down: true)
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            guard let slot = slots.firstIndex(of: touch) else { continue }
            let end = touch.location(in: self)
            let moved = hypot(end.x - starts[slot].x, end.y - starts[slot].y)
            report(slot, touch, down: false)
            slots[slot] = nil
            if moved < Self.tapSlop { onTap?() }
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches {
            guard let slot = slots.firstIndex(of: touch) else { continue }
            report(slot, touch, down: false)
            slots[slot] = nil
        }
    }

    private func report(_ slot: Int, _ touch: UITouch, down: Bool) {
        let p = touch.location(in: self)
        guard bounds.width > 0, bounds.height > 0 else { return }
        onTouch?(
            UInt32(slot),
            CGPoint(x: p.x / bounds.width, y: p.y / bounds.height),
            down)
    }
}

struct FluidSurface: UIViewRepresentable {
    let driver: FrameDriver
    let onTap: () -> Void

    func makeUIView(context: Context) -> MetalLayerView {
        let view = MetalLayerView()
        view.onLayout = { layer, size in driver.attach(layer: layer, pixelSize: size) }
        view.onTouch = { slot, at, down in driver.touch(slot, at, down: down) }
        view.onTap = onTap
        return view
    }

    func updateUIView(_ view: MetalLayerView, context: Context) {}
}
