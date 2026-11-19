import AppKit

/// The window a `Modal` shows as a sheet. Escape reaches it as `cancelOperation` once nothing inside has
/// taken the key, and becomes a request to close.
final class MacSheetPanel: NSPanel {
    var onCancel: (() -> Void)?
    override var canBecomeKey: Bool { true }

    override func cancelOperation(_ sender: Any?) {
        onCancel?()
    }
}

/// A `Modal` or `Popover`'s place in the tree.
///
/// The view itself is hidden and holds nothing. What the overlay holds goes into `content`, which AppKit
/// shows above the window: a modal's as a sheet on the window this view is in, and a popover's in an
/// `NSPopover` hanging from its anchor. Keeping a view in the tree keeps every sibling at the index the
/// transaction gives it, and tells the overlay which window it belongs to.
@MainActor
final class MacOverlayView: NSView, NSPopoverDelegate {
    enum Presentation { case sheet, popover }

    let presentation: Presentation
    /// Where the overlay's children go, and what its own style is applied to.
    let content = FlippedView()
    /// Escape, or a press outside a popover: a request the app answers by closing the overlay.
    var onDismiss: (() -> Void)?
    /// The view joined or left a window, so an open overlay may now have somewhere to show.
    var onWindowChange: (() -> Void)?
    /// A sheet's content view, as large as what the modal holds.
    private let frameView = FlippedView()
    private var panel: MacSheetPanel?
    private var host: NSWindow?
    private var popover: NSPopover?

    init(_ presentation: Presentation) {
        self.presentation = presentation
        super.init(frame: .zero)
        isHidden = true

        switch presentation {
        case .sheet:
            let panel = MacSheetPanel(
                contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: true)
            // The app draws the dialog's own panel, corners and all.
            panel.isOpaque = false
            panel.backgroundColor = .clear
            panel.isReleasedWhenClosed = false
            panel.onCancel = { [weak self] in self?.onDismiss?() }
            frameView.addSubview(content)
            panel.contentView = frameView
            self.panel = panel
        case .popover:
            let controller = NSViewController()
            controller.view = content
            let popover = NSPopover()
            popover.contentViewController = controller
            // Transient: AppKit wants to close it on Escape and on a press outside, and
            // `popoverShouldClose` turns that wish into a request.
            popover.behavior = .transient
            popover.delegate = self
            self.popover = popover
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { return nil }

    override var isFlipped: Bool { true }

    /// The place holder takes no clicks: what the overlay holds is in another window.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        onWindowChange?()
    }

    /// Whether the sheet or the popover is on screen.
    var isShown: Bool {
        switch presentation {
        case .sheet: host != nil
        case .popover: popover?.isShown ?? false
        }
    }

    /// The window the overlay's content is shown in, once it is.
    var presentedWindow: NSWindow? {
        switch presentation {
        case .sheet: panel
        case .popover: content.window
        }
    }

    /// Sets how large the overlay is. A sheet shows only what the modal holds; a popover is this large.
    func resize(_ size: CGSize) {
        content.setFrameSize(size)
        popover?.contentSize = size
    }

    /// A sheet dims the window behind it, so the backdrop a modal draws on the web has nothing to cover.
    func styled() {
        if presentation == .sheet { content.layer?.backgroundColor = nil }
    }

    /// Shows the overlay if it is open and has somewhere to show, and takes it down otherwise.
    /// Returns whether this call showed it.
    func present(open: Bool, anchor: NSView?, placement: String?) -> Bool {
        guard open else {
            hide()

            return false
        }

        switch presentation {
        case .sheet: return showSheet()
        case .popover: return showPopover(anchor, placement)
        }
    }

    /// A modal covers the viewport and centres what it holds. The sheet is sized to that content alone.
    private func showSheet() -> Bool {
        let held = content.subviews.filter { !$0.isHidden }.reduce(CGRect.null) { $0.union($1.frame) }

        guard let panel, let window, !held.isNull, held.width > 0, held.height > 0 else {
            hide()

            return false
        }
        frameView.frame = CGRect(origin: .zero, size: held.size)
        content.setFrameOrigin(CGPoint(x: -held.minX, y: -held.minY))
        panel.setContentSize(held.size)

        guard host == nil else { return false }
        window.beginSheet(panel)
        host = window

        return true
    }

    /// AppKit centres a popover on its anchor's edge and points an arrow at it, so a placement picks the
    /// side and nothing more. It also moves the popover to another side when the chosen one has no room.
    private func showPopover(_ anchor: NSView?, _ placement: String?) -> Bool {
        // AppKit raises when the anchor is in no window.
        guard let popover, let anchor, anchor.window != nil else {
            hide()

            return false
        }
        guard !popover.isShown else { return false }
        let above = placement?.hasPrefix("above") ?? false
        // The edge follows the anchor's own coordinates: in a flipped view the largest y is the bottom.
        let edge: NSRectEdge = above == anchor.isFlipped ? .minY : .maxY
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: edge)

        return popover.isShown
    }

    /// Takes the overlay off screen without asking anyone.
    func hide() {
        if let panel, let host {
            host.endSheet(panel)
            self.host = nil
        }

        if let popover, popover.isShown { popover.close() }
    }

    /// Asks to close as Escape does. The overlay stays until the app closes it.
    func cancel() {
        switch presentation {
        case .sheet: panel?.cancelOperation(nil)
        case .popover: popover?.performClose(nil)
        }
    }

    func popoverShouldClose(_ popover: NSPopover) -> Bool {
        onDismiss?()

        // The app closes it by sending the next tree without it open.
        return false
    }
}
