import AVFoundation
import LeptosKitBridge
#if os(macOS)
import AppKit
typealias NativeCameraBase = NSView
#else
import UIKit
typealias NativeCameraBase = UIView
#endif

/// A `Camera` element: an `AVCaptureVideoPreviewLayer` host bound to a session
/// id from the `camera` plugin. The plugin owns capture; this view only attaches
/// the preview layer.
final class NativeCameraView: NativeCameraBase {
    #if os(macOS)
    override var wantsLayer: Bool {
        get { true }
        set {}
    }
    #endif

    private var previewLayer: AVCaptureVideoPreviewLayer {
        #if os(macOS)
        if let layer = layer as? AVCaptureVideoPreviewLayer { return layer }
        let created = AVCaptureVideoPreviewLayer()
        created.videoGravity = .resizeAspectFill
        self.layer = created
        return created
        #else
        guard let preview = layer as? AVCaptureVideoPreviewLayer else {
            preconditionFailure("NativeCameraView.layerClass is AVCaptureVideoPreviewLayer")
        }
        return preview
        #endif
    }

    #if !os(macOS)
    override static var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    #endif

    private var sessionId: UInt32?

    #if os(macOS)
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        _ = previewLayer
    }
    #else
    override init(frame: CGRect) {
        super.init(frame: frame)
        previewLayer.videoGravity = .resizeAspectFill
    }
    #endif

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(session id: UInt32?) {
        sessionId = id
        previewLayer.session = id.flatMap { NativeCameraBinding.session?($0) }
    }

    func close() {
        previewLayer.session = nil
        sessionId = nil
    }

    #if os(macOS)
    override func layout() {
        super.layout()
        previewLayer.frame = bounds
    }
    #else
    override func layoutSubviews() {
        super.layoutSubviews()
        previewLayer.frame = bounds
    }
    #endif
}
