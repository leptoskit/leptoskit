import AppKit
import LeptosKitBridge

final class MacChoiceButton: NSButton {
    var radio = false
}

/// Keyboard and accessibility increments use the guest's step grid.
final class MacValueSlider: NSSlider {
    var state = WidgetSlider.initial

    private func step(_ forward: Bool) -> Bool {
        guard isEnabled else { return false }
        let delta = state.step ?? (state.max - state.min) / 100
        doubleValue = state.snapped(state.value + (forward ? delta : -delta))

        return sendAction(action, to: target)
    }

    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 123, 125: _ = step(false)
        case 124, 126: _ = step(true)
        default: super.keyDown(with: event)
        }
    }

    override func accessibilityPerformIncrement() -> Bool { step(true) }
    override func accessibilityPerformDecrement() -> Bool { step(false) }
}

/// Several lines of editable text: a text view inside a scroll view. Return
/// adds a line, the text wraps at the view's width, and the view scrolls its
/// own content. AppKit's text view has no placeholder, so a label over the
/// empty text shows it.
final class MacTextArea: NSScrollView, NSTextViewDelegate {
    /// Lets clicks through to the text underneath.
    private final class Hint: NSTextField {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }

    let text = NSTextView()
    private let hint = Hint(labelWithString: "")
    private var maxLength: Int?
    var onChange: ((String) -> Void)?
    var onFocus: ((Bool) -> Void)?

    override init(frame: NSRect) {
        super.init(frame: frame)
        hasVerticalScroller = true
        autohidesScrollers = true
        drawsBackground = false
        text.isRichText = false
        text.allowsUndo = true
        text.drawsBackground = false
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        text.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        text.delegate = self
        documentView = text
        hint.textColor = .placeholderTextColor
        hint.isHidden = true
        text.addSubview(hint)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override func layout() {
        super.layout()
        let inset = text.textContainerInset
        let padding = text.textContainer?.lineFragmentPadding ?? 0
        hint.frame = NSRect(
            x: inset.width + padding, y: inset.height,
            width: max(0, text.bounds.width - 2 * (inset.width + padding)),
            height: hint.intrinsicContentSize.height)
    }

    func apply(_ props: WidgetProps, font: NSFont, color: NSColor, alignment: NSTextAlignment) {
        let value = props.value ?? props.text ?? ""

        if text.string != value { text.string = value }
        maxLength = props.maxLength
        hint.stringValue = props.placeholder ?? ""
        hint.font = font
        hint.alignment = alignment
        hint.isHidden = !value.isEmpty
        text.font = font
        text.textColor = color
        text.alignment = alignment
        // A read-only area refuses edits but stays selectable and copyable.
        text.isEditable = props.enabled != false && props.readOnly != true
        text.isSelectable = props.enabled != false
        // Unset or `true` hands correction back to the person's own settings.
        if props.autocorrect == false {
            text.isAutomaticSpellingCorrectionEnabled = false
            text.isContinuousSpellCheckingEnabled = false
        }
        borderType = props.background == nil && props.borderWidth == nil && props.cornerRadius == nil
            ? .bezelBorder : .noBorder
        needsLayout = true
    }

    func textDidChange(_ notification: Notification) {
        var value = text.string
        // The view accepts more characters than the app allows, so truncate
        // to `maxLength` before the proposal leaves.
        if let max = maxLength, value.count > max {
            value = String(value.prefix(max))
            text.string = value
        }
        hint.isHidden = !value.isEmpty
        onChange?(value)
    }

    func textDidBeginEditing(_ notification: Notification) { onFocus?(true) }
    func textDidEndEditing(_ notification: Notification) { onFocus?(false) }
}

extension MacWidgetRenderer {
    func makeControl(_ item: Change.Widget) throws -> NSView {
        switch item.kind {
        case "switch", "Switch":
            let control = NSSwitch()
            control.target = self
            control.action = #selector(toggled(_:))
            return control
        case "checkbox", "Checkbox", "radio", "Radio":
            let control = MacChoiceButton()
            control.radio = ["radio", "Radio"].contains(item.kind)
            control.setButtonType(control.radio ? .radio : .switch)
            control.target = self
            control.action = #selector(toggled(_:))
            return control
        case "slider", "Slider":
            let control = MacValueSlider(value: 0, minValue: 0, maxValue: 1, target: self, action: #selector(slid(_:)))
            control.isContinuous = true
            return control
        case "linear_progress_indicator", "LinearProgressIndicator", "activity_indicator", "ActivityIndicator":
            let control = NSProgressIndicator()
            control.style = ["activity_indicator", "ActivityIndicator"].contains(item.kind) ? .spinning : .bar
            return control
        default:
            throw NativeAppError(message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
    }

    @objc private func toggled(_ control: NSControl) {
        guard let id = node(control) else { return }
        let checked: Bool

        if let button = control as? MacChoiceButton {
            checked = button.radio || button.state == .on
        } else {
            checked = (control as? NSSwitch)?.state == .on
        }

        event(id, 6, "{\"checked\":\(checked)}")
        // AppKit can deselect sibling radios before invoking the target. Restore
        // every control from the guest's synchronous response, even on rejection.
        for (id, widget) in widgets {

            if let props = properties[id] { MacControlStyle.apply(props, to: widget) }
        }
    }

    @objc private func slid(_ control: NSSlider) {
        guard let id = node(control) else { return }
        let state = properties[id]?.slider ?? .initial
        event(id, 7, "{\"value\":\(state.snapped(control.doubleValue))}")

        if let props = properties[id] { MacControlStyle.apply(props, to: control) }
    }
}

@MainActor
enum MacControlStyle {
    static func apply(_ props: WidgetProps, to widget: NSView) {
        if let control = widget as? NSControl { control.isEnabled = props.enabled ?? true }

        if let control = widget as? NSSwitch { control.state = props.checked == true ? .on : .off }

        if let control = widget as? MacChoiceButton {
            control.state = props.checked == true ? .on : .off
            control.isBordered = false
        }

        if let control = widget as? NSSlider {
            let state = props.slider ?? .initial
            (control as? MacValueSlider)?.state = state

            control.minValue = state.min
            control.maxValue = state.max
            control.doubleValue = state.value
            control.setAccessibilityValue(NSNumber(value: (props.slider ?? .initial).value))
        }

        if let control = widget as? NSProgressIndicator {
            if control.style == .spinning {
                control.isIndeterminate = true

                if props.running ?? true { control.startAnimation(nil) } else { control.stopAnimation(nil) }
            } else {
                switch props.progress ?? .indeterminate {
                case .indeterminate:
                    control.isIndeterminate = true
                    control.startAnimation(nil)
                case .determinate(let value):
                    control.stopAnimation(nil)
                    control.isIndeterminate = false
                    control.minValue = 0
                    control.maxValue = 1
                    control.doubleValue = Double(value)
                }
            }
        }
    }
}
