import LeptosKitBridge
import MapKit
#if os(macOS)
import AppKit
typealias NativeMapBase = NSView
#else
import UIKit
typealias NativeMapBase = UIView
#endif

/// A `Map` element: hosts an `MKMapView` bound to a session id from the `maps`
/// plugin. The plugin owns camera and markers; this view only embeds the map.
final class NativeMapView: NativeMapBase {
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
        clearEmbeddedMap()
        sessionId = id
        guard let map = id.flatMap({ NativeMapBinding.mapView?($0) }) else { return }
        #if os(macOS)
        map.frame = bounds
        map.autoresizingMask = [.width, .height]
        addSubview(map)
        #else
        map.frame = bounds
        map.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        addSubview(map)
        #endif
    }

    func close() {
        clearEmbeddedMap()
        sessionId = nil
    }

    private func clearEmbeddedMap() {
        #if os(macOS)
        subviews.forEach { $0.removeFromSuperview() }
        #else
        subviews.forEach { $0.removeFromSuperview() }
        #endif
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
