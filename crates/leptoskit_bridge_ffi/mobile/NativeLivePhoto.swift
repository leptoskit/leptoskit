import LeptosKitBridge
import PhotosUI
#if os(macOS)
import AppKit
typealias NativeLivePhotoBase = NSView
#else
import UIKit
typealias NativeLivePhotoBase = UIView
#endif

/// A `LivePhoto` element: hosts a `PHLivePhotoView` bound to a session id from
/// the `live-photo` plugin. The plugin owns load and playback; this view only
/// embeds the surface.
final class NativeLivePhotoView: NativeLivePhotoBase {
    private var sessionId: UInt32?

    #if os(macOS)
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
    }
    #else
    override init(frame: CGRect) {
        super.init(frame: frame)
        clipsToBounds = true
    }
    #endif

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(session id: UInt32?) {
        if sessionId == id { return }
        clearEmbedded()
        sessionId = id
        guard let view = id.flatMap({ NativeLivePhotoBinding.view?($0) }) else { return }
        #if os(macOS)
        view.frame = bounds
        view.autoresizingMask = [.width, .height]
        addSubview(view)
        #else
        view.frame = bounds
        view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        addSubview(view)
        #endif
    }

    func close() {
        clearEmbedded()
        sessionId = nil
    }

    private func clearEmbedded() {
        subviews.forEach { $0.removeFromSuperview() }
    }

    #if os(macOS)
    override func layout() {
        super.layout()
        subviews.first?.frame = bounds
    }
    #else
    override func layoutSubviews() {
        super.layoutSubviews()
        subviews.first?.frame = bounds
    }
    #endif
}
