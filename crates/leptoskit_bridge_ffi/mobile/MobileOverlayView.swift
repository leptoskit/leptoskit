import UIKit

/// The view an overlay's children go into. It remembers the place holder it belongs to, so the renderer
/// can count the children of the widget and not of this view.
final class MobileOverlayContent: UIView {
    weak var owner: MobileOverlayView?
}

/// The controller UIKit presents for one overlay. It keeps what the overlay holds inside the safe area,
/// and turns Escape and the VoiceOver escape gesture into a request to close.
final class MobileOverlayController: UIViewController {
    var onDismiss: (() -> Void)?
    /// Sized to what the overlay shows.
    let holder = UIView()
    /// A popover's arrow takes part of the view, which the safe area leaves out.
    var hangs = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.addSubview(holder)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let insets = view.safeAreaInsets
        let size = holder.frame.size
        // A sheet can be wider than what it holds.
        let offsetX = hangs ? insets.left : max((view.bounds.width - size.width) / 2, insets.left)
        holder.frame = CGRect(origin: CGPoint(x: offsetX, y: insets.top), size: size)
    }

    override func accessibilityPerformEscape() -> Bool {
        onDismiss?()

        return true
    }

    override var keyCommands: [UIKeyCommand]? {
        [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(dismissRequested))]
    }

    @objc private func dismissRequested() {
        onDismiss?()
    }
}

/// A `Modal` or `Popover`'s place in the tree.
///
/// The view itself is hidden and holds nothing. What the overlay holds goes into `content`, which UIKit
/// presents over the screen: a modal's as a form sheet, and a popover's hanging from its anchor. Keeping
/// a view in the tree keeps every sibling at the index the transaction gives it, and tells the overlay
/// which view controller to present over.
final class MobileOverlayView: UIView, UIPopoverPresentationControllerDelegate {
    enum Presentation { case sheet, popover }

    let presentation: Presentation
    /// Where the overlay's children go, and what its own style is applied to.
    let content = MobileOverlayContent()
    /// A swipe on a sheet, a press outside a popover, Escape: a request the app answers by closing it.
    var onDismiss: (() -> Void)?
    /// The view joined or left a window, so an open overlay may now have somewhere to show.
    var onWindowChange: (() -> Void)?
    let controller = MobileOverlayController()

    init(_ presentation: Presentation) {
        self.presentation = presentation
        super.init(frame: .zero)
        isHidden = true
        content.owner = self
        controller.hangs = presentation == .popover
        controller.modalPresentationStyle = presentation == .popover ? .popover : .formSheet
        controller.holder.addSubview(content)
        controller.onDismiss = { [weak self] in self?.onDismiss?() }

        if presentation == .sheet { controller.view.backgroundColor = .systemBackground }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { return nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        onWindowChange?()
    }

    /// Whether UIKit is presenting the overlay.
    var isShown: Bool { controller.presentingViewController != nil && !controller.isBeingDismissed }

    /// Sets how large the overlay is. A sheet shows only what the modal holds; a popover is this large.
    func resize(_ size: CGSize) {
        content.frame.size = size

        if presentation == .popover { fit(size) }
    }

    /// A sheet dims the screen behind it, so the backdrop a modal draws on the web has nothing to cover.
    func styled() {
        if presentation == .sheet { content.backgroundColor = .clear }
    }

    /// Shows the overlay if it is open and has somewhere to show, and takes it down otherwise.
    /// Returns whether this call showed it.
    func present(open: Bool, anchor: UIView?, placement: String?) -> Bool {
        guard open else {
            hide()

            return false
        }

        switch presentation {
        case .sheet: return showSheet()
        case .popover: return showPopover(anchor, placement)
        }
    }

    private func fit(_ size: CGSize) {
        if controller.holder.frame.size != size {
            controller.holder.frame.size = size
            controller.view.setNeedsLayout()
        }

        if controller.preferredContentSize != size { controller.preferredContentSize = size }
    }

    /// A modal covers the viewport and centres what it holds. The sheet is sized to that content alone.
    private func showSheet() -> Bool {
        let held = content.subviews.filter { !$0.isHidden }.reduce(CGRect.null) { $0.union($1.frame) }

        guard let below = presenter(from: self), !held.isNull, held.width > 0, held.height > 0 else {
            hide()

            return false
        }
        content.frame.origin = CGPoint(x: -held.minX, y: -held.minY)
        fit(held.size)

        guard !isShown, !controller.isBeingDismissed else { return false }
        // UIKit makes a new presentation controller for each presentation.
        controller.presentationController?.delegate = self
        below.present(controller, animated: UIView.areAnimationsEnabled)

        return isShown
    }

    /// UIKit centres a popover on its anchor's edge and points an arrow at it, so a placement picks the
    /// side and nothing more.
    private func showPopover(_ anchor: UIView?, _ placement: String?) -> Bool {
        guard let anchor, let below = presenter(from: anchor) else {
            hide()

            return false
        }
        guard !isShown, !controller.isBeingDismissed else { return false }
        let above = placement?.hasPrefix("above") ?? false

        if let popover = controller.popoverPresentationController {
            popover.sourceView = anchor
            popover.sourceRect = anchor.bounds
            // The arrow points at the anchor, from the side the popover is on.
            popover.permittedArrowDirections = above ? .down : .up
            popover.delegate = self
        }
        below.present(controller, animated: UIView.areAnimationsEnabled)

        return isShown
    }

    /// The controller to present over: the one holding `view`, once it is in a window, or whatever that
    /// one has already presented.
    private func presenter(from view: UIView) -> UIViewController? {
        guard view.window != nil else { return nil }
        var responder = view.next

        while let current = responder, !(current is UIViewController) { responder = current.next }

        guard var found = responder as? UIViewController else { return nil }

        while let above = found.presentedViewController, above !== controller { found = above }

        return found
    }

    /// Takes the overlay off screen without asking anyone, along with anything presented over it.
    func hide() {
        guard !controller.isBeingDismissed else { return }
        // Asking the presenter, not the controller itself, takes this one down even when it has
        // presented something of its own.
        controller.presentingViewController?.dismiss(animated: UIView.areAnimationsEnabled)
    }

    /// Asks to close as Escape does. The overlay stays until the app closes it.
    func cancel() {
        onDismiss?()
    }

    func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool {
        // A press outside a popover ends here. A sheet's swipe goes on to
        // `presentationControllerDidAttemptToDismiss`.
        if presentation == .popover { onDismiss?() }

        // The app closes it by sending the next tree without it open.
        return false
    }

    func presentationControllerDidAttemptToDismiss(_ presentationController: UIPresentationController) {
        if presentation == .sheet { onDismiss?() }
    }

    /// A popover stays a popover on a phone, and does not turn into a sheet that covers the control it
    /// hangs from.
    func adaptivePresentationStyle(
        for controller: UIPresentationController, traitCollection: UITraitCollection
    ) -> UIModalPresentationStyle { .none }
}
