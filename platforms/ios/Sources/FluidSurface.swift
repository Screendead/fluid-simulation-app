import SwiftUI

final class MetalLayerView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
    var onLayout: ((CAMetalLayer, CGSize) -> Void)?

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = traitCollection.displayScale
        let metalLayer = layer as! CAMetalLayer
        metalLayer.contentsScale = scale
        let size = CGSize(width: bounds.width * scale, height: bounds.height * scale)
        guard size.width > 0, size.height > 0 else { return }
        onLayout?(metalLayer, size)
    }
}

struct FluidSurface: UIViewRepresentable {
    let driver: FrameDriver

    func makeUIView(context: Context) -> MetalLayerView {
        let view = MetalLayerView()
        view.onLayout = { layer, size in driver.attach(layer: layer, pixelSize: size) }
        return view
    }

    func updateUIView(_ view: MetalLayerView, context: Context) {}
}
