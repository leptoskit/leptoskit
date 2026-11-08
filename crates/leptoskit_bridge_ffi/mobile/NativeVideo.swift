import AVFoundation
import LeptosKitBridge
#if os(macOS)
import AppKit
typealias NativeVideoBase = NSView
#else
import UIKit
typealias NativeVideoBase = UIView
#endif

/// A `Video` element: an `AVPlayerLayer` host bound to a player id from the
/// `video` plugin. The plugin owns transport; this view only attaches the layer.
final class NativeVideoView: NativeVideoBase {
    #if os(macOS)
    override var wantsLayer: Bool {
        get { true }
        set {}
    }
    #endif

    private var playerLayer: AVPlayerLayer {
        #if os(macOS)
        if let layer = layer as? AVPlayerLayer { return layer }
        let created = AVPlayerLayer()
        created.videoGravity = .resizeAspect
        self.layer = created
        return created
        #else
        return layer as! AVPlayerLayer
        #endif
    }

    #if !os(macOS)
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    #endif

    private var playerId: UInt32?

    #if os(macOS)
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        _ = playerLayer
    }
    #else
    override init(frame: CGRect) {
        super.init(frame: frame)
        playerLayer.videoGravity = .resizeAspect
    }
    #endif

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(player id: UInt32?) {
        playerId = id
        playerLayer.player = id.flatMap { NativeVideoBinding.player?($0) }
    }

    func close() {
        playerLayer.player = nil
        playerId = nil
    }

    #if os(macOS)
    override func layout() {
        super.layout()
        playerLayer.frame = bounds
    }
    #else
    override func layoutSubviews() {
        super.layoutSubviews()
        playerLayer.frame = bounds
    }
    #endif
}
