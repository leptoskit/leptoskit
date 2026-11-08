import CLeptosKitBridge
import Foundation
import QuartzCore
#if os(macOS)
import AppKit
typealias NativeScene3dBase = NSView
#else
import UIKit
typealias NativeScene3dBase = UIView
#endif

/// Lets a Rust waker reach its view from any thread without keeping it alive.
private final class NativeScene3dWake: @unchecked Sendable {
    weak var view: NativeScene3dView?
}

/// A `Scene3d` element: a `CAMetalLayer` that Rust draws the scene into with Metal.
///
/// The guest's ops reach the scene through the `leptoskit.scene3d` plugin, never through this view. It
/// attaches while it is in a window with a size, draws on a display link only while the scene changes or
/// animates, and starts the display link again when Rust wakes it.
@MainActor
final class NativeScene3dView: NativeScene3dBase {
    private let metal = CAMetalLayer()
    private var handle: OpaquePointer?
    private var scene: UInt32?
    private var attachedScene: UInt32?
    private var link: CADisplayLink?
    private var wake: Unmanaged<NativeScene3dWake>?
    private var open = true

    /// Read once, the first time any view attaches: the app bundle's `assets` folder.
    private static let assets: Void = {
        guard let folder = Bundle.main.resourceURL?.appendingPathComponent("assets").path else { return }
        let bytes = Array(folder.utf8)
        _ = bytes.withUnsafeBufferPointer { lk_scene3d_set_asset_directory($0.baseAddress, $0.count) }
    }()

    init() {
        super.init(frame: .zero)
        metal.isOpaque = false
        metal.framebufferOnly = true
        metal.contentsGravity = .resize
        #if os(macOS)
        wantsLayer = true
        layer?.addSublayer(metal)
        setAccessibilityElement(true)
        setAccessibilityRole(.image)
        #else
        layer.addSublayer(metal)
        isAccessibilityElement = true
        accessibilityTraits = .image
        #endif
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    isolated deinit { close() }

    /// Shows `scene`, or nothing when it is nil.
    func show(scene: UInt32?) {
        guard open, scene != self.scene else { return }
        self.scene = scene
        reattach()
    }

    func close() {
        guard open else { return }
        open = false
        detach()
    }

    private var scale: CGFloat {
        #if os(macOS)
        window?.backingScaleFactor ?? 1
        #else
        window?.screen.scale ?? UIScreen.main.scale
        #endif
    }

    private var pixels: (UInt32, UInt32) {
        let size = bounds.size
        return (UInt32(max(0, (size.width * scale).rounded())), UInt32(max(0, (size.height * scale).rounded())))
    }

    #if os(macOS)
    override func layout() {
        super.layout()
        fit()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        reattach()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        fit()
    }
    #else
    override func layoutSubviews() {
        super.layoutSubviews()
        fit()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        reattach()
    }
    #endif

    private func fit() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        metal.frame = bounds
        metal.contentsScale = scale
        CATransaction.commit()

        guard let handle else { reattach(); return }
        let (width, height) = pixels
        lk_scene3d_view_resize(handle, width, height)
    }

    /// Attaches to the current scene when the view is on screen with a size, and detaches otherwise.
    private func reattach() {
        let (width, height) = pixels
        let wanted = open && window != nil && width > 0 && height > 0 ? scene : nil

        guard wanted != attachedScene || (wanted != nil && handle == nil) else { return }
        detach()

        guard let wanted, lk_scene3d_available() == 1 else { return }
        _ = Self.assets
        guard let view = lk_scene3d_view_attach_layer(
            wanted, Unmanaged.passUnretained(metal).toOpaque(), width, height)
        else {
            let error = lk_scene3d_error()
            defer { lk_buffer_release(error) }
            let message = String(decoding: UnsafeBufferPointer(start: error.data, count: error.len), as: UTF8.self)
            NSLog("leptoskit: a 3D scene could not attach: %@", message)

            return
        }
        handle = view
        attachedScene = wanted

        let box = NativeScene3dWake()
        box.view = self
        let retained = Unmanaged.passRetained(box)
        wake = retained
        lk_scene3d_view_set_waker(view, retained.toOpaque(), Self.wakeCallback)
        requestFrame()
    }

    private func detach() {
        stop()
        if let handle { lk_scene3d_view_detach(handle) }
        handle = nil
        attachedScene = nil
        // The waker is gone with the view, so its context can go too.
        wake?.release()
        wake = nil
    }

    /// Starts the display link if the scene has something to draw.
    func requestFrame() {
        guard open, handle != nil, link == nil, window != nil else { return }

        #if os(macOS)
        let display = displayLink(target: self, selector: #selector(step(_:)))
        #else
        let display = CADisplayLink(target: self, selector: #selector(step(_:)))
        #endif
        display.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 120, preferred: 60)
        link = display
        display.add(to: .main, forMode: .common)
        // A scene with nothing new still draws once, so a resized view never shows stale pixels.
    }

    private func stop() {
        link?.invalidate()
        link = nil
    }

    @objc private func step(_ display: CADisplayLink) {
        guard let handle else { stop(); return }

        if lk_scene3d_view_frame(handle, display.timestamp) != 1 { stop() }
    }

    private nonisolated static let wakeCallback: lk_scene3d_wake_fn = { context in
        guard let context else { return }
        let box = Unmanaged<NativeScene3dWake>.fromOpaque(context).takeUnretainedValue()
        DispatchQueue.main.async { MainActor.assumeIsolated { box.view?.requestFrame() } }
    }
}
