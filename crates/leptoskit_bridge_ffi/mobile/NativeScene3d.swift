import CLeptosKitBridge
import Foundation
import LeptosKitBridge
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
    private var pointerOrigin: CGPoint?
    private var pointerLast = CGPoint.zero
    private var pointerDragged = false
    #if !os(macOS)
    private var pointerTouch: UITouch?
    #endif

    /// Read once before the guest starts: the app bundle's `assets` folder.
    private static let assets: Void = {
        guard let folder = Bundle.main.resourceURL?.appendingPathComponent("assets").path else { return }
        let bytes = Array(folder.utf8)
        _ = bytes.withUnsafeBufferPointer { lk_scene3d_set_asset_directory($0.baseAddress, $0.count) }
    }()

    /// Install the bundle reader before the guest queues its first model.
    static func configure() {
        if lk_scene3d_available() == 1 { _ = assets }
    }

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
        cancelPointer()
        self.scene = scene
        reattach()
    }

    func close() {
        guard open else { return }
        cancelPointer()
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

    /// Clockwise quarter turns of the interface from portrait, which a scene drawn over the camera needs to
    /// turn the picture upright. The Mac has no camera feed, so it stays 0.
    private var quarterTurns: UInt32 {
        #if os(macOS)
        0
        #else
        switch window?.windowScene?.interfaceOrientation {
        case .landscapeRight: 1
        case .portraitUpsideDown: 2
        case .landscapeLeft: 3
        default: 0
        }
        #endif
    }

    #if os(macOS)
    override func layout() {
        super.layout()
        fit()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { cancelPointer() }
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
        if window == nil { cancelPointer() }
        reattach()
    }
    #endif

    private func emitPointer(_ phase: UInt32, at point: CGPoint, dx: CGFloat = 0, dy: CGFloat = 0) {
        guard let scene else { return }
        let across = min(1, max(0, (point.x - bounds.minX) / max(1, bounds.width)))
        let down = min(1, max(0, (point.y - bounds.minY) / max(1, bounds.height)))
        _ = lk_scene3d_pointer(scene, phase, Float(across), Float(down), Float(dx), Float(dy))
    }

    private func beginPointer(at point: CGPoint) {
        guard open else { return }
        cancelPointer()
        pointerOrigin = point
        pointerLast = point
        pointerDragged = false
        emitPointer(0, at: point)
    }

    private func movePointer(to point: CGPoint) {
        guard let origin = pointerOrigin else { return }
        let dx = point.x - origin.x
        let dy = point.y - origin.y
        guard pointerDragged || dx * dx + dy * dy > 64 else { return }
        pointerDragged = true
        emitPointer(1, at: point, dx: point.x - pointerLast.x, dy: point.y - pointerLast.y)
        pointerLast = point
    }

    private func endPointer(at point: CGPoint, cancelled: Bool = false) {
        guard pointerOrigin != nil else { return }
        emitPointer(cancelled ? 3 : 2, at: point)
        pointerOrigin = nil
    }

    private func cancelPointer() {
        endPointer(at: pointerLast, cancelled: true)
        #if !os(macOS)
            pointerTouch = nil
        #endif
    }

    #if os(macOS)
        private func pointerLocation(_ event: NSEvent) -> CGPoint {
            let point = convert(event.locationInWindow, from: nil)
            return isFlipped ? point : CGPoint(x: point.x, y: bounds.maxY - point.y + bounds.minY)
        }

        override func mouseDown(with event: NSEvent) { beginPointer(at: pointerLocation(event)) }
        override func mouseDragged(with event: NSEvent) { movePointer(to: pointerLocation(event)) }
        override func mouseUp(with event: NSEvent) { endPointer(at: pointerLocation(event)) }
    #else
        override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
            guard pointerTouch == nil, let touch = touches.first else { return }
            beginPointer(at: touch.location(in: self))
            pointerTouch = touch
        }

        override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
            guard let touch = pointerTouch, touches.contains(touch) else { return }
            movePointer(to: touch.location(in: self))
        }

        override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
            guard let touch = pointerTouch, touches.contains(touch) else { return }
            endPointer(at: touch.location(in: self))
            pointerTouch = nil
        }

        override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
            cancelPointer()
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
            let bytes = UnsafeBufferPointer(start: error.data, count: error.len)
            let message = String(bytes: bytes, encoding: .utf8) ?? ""
            NativeLog.warn(NativeLog.gpu, "a 3D scene could not attach: \(message)")

            return
        }
        handle = view
        attachedScene = wanted
        lk_scene3d_view_set_rotation(view, quarterTurns)

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
        // A half turn between the two landscapes changes no size, so the rotation is read every frame.
        lk_scene3d_view_set_rotation(handle, quarterTurns)

        if lk_scene3d_view_frame(handle, display.timestamp) != 1 { stop() }
    }

    private nonisolated static let wakeCallback: lk_scene3d_wake_fn = { context in

        guard let context else { return }
        let box = Unmanaged<NativeScene3dWake>.fromOpaque(context).takeUnretainedValue()
        DispatchQueue.main.async { MainActor.assumeIsolated { box.view?.requestFrame() } }
    }
}
