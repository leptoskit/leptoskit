import UIKit

/// Applies each text-capable view's existing UIKit defaults.
@MainActor
enum WidgetTextStyle {
    private static let defaultSize: CGFloat = 14

    static func apply(_ props: WidgetProps, to widget: UIView) {
        let font = font(props)
        let alignment = alignment(props)

        if let label = widget as? UILabel {
            // Each setter re-lays out the label's text, so only changes are set.
            if label.text != props.text { label.text = props.text }
            let accessibilityLabel = props.accessibilityLabel ?? props.text

            if label.accessibilityLabel != accessibilityLabel { label.accessibilityLabel = accessibilityLabel }
            let accessibilityValue = props.accessibilityLabel == nil ? nil : props.text

            if label.accessibilityValue != accessibilityValue { label.accessibilityValue = accessibilityValue }

            if label.font !== font { label.font = font }
            let color = textColor(props)

            if label.textColor != color { label.textColor = color }

            if label.textAlignment != alignment { label.textAlignment = alignment }

            if let lineHeight = props.lineHeight?.points(fontSize: font.pointSize) {
                let lined = lined(props.text ?? "", font: font, color: color, alignment: alignment,
                                  lineHeight: lineHeight)

                if label.attributedText != lined { label.attributedText = lined }
            } else if let shown = label.attributedText, shown.length > 0,
                      shown.attribute(.baselineOffset, at: 0, effectiveRange: nil) != nil {
                // Plain text drops the line height's paragraph style.
                label.text = props.text
            }
        }

        if let button = widget as? UIButton {
            button.setTitle(props.text, for: .normal)
            button.titleLabel?.font = font
            button.setTitleColor(
                props.color.map(WidgetStyle.color) ?? .systemBlue,
                for: .normal
            )
            button.isEnabled = props.enabled ?? true
        }

        if let field = widget as? UITextField {
            applyField(props, to: field, font: font, alignment: alignment)
        }
    }

    /// Text whose every line is `lineHeight` tall with its glyphs centered, as
    /// CSS's `line-height` sets them and `UIKitTextMeasurer` measures them.
    static func lined(
        _ text: String, font: UIFont, color: UIColor, alignment: NSTextAlignment, lineHeight: CGFloat
    ) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = alignment
        paragraph.lineBreakMode = .byWordWrapping
        paragraph.minimumLineHeight = lineHeight
        paragraph.maximumLineHeight = lineHeight

        return NSAttributedString(string: text, attributes: [
            .font: font, .foregroundColor: color, .paragraphStyle: paragraph,
            // UIKit puts a line's extra height above its glyphs; this centers them.
            .baselineOffset: (lineHeight - (font.ascender - font.descender)) / 2
        ])
    }

    private static func applyField(
        _ props: WidgetProps, to field: UITextField, font: UIFont, alignment: NSTextAlignment
    ) {
        let value = props.value ?? props.text ?? ""

        if field.text != value { field.text = value }

        field.placeholder = props.placeholder
        field.textContentType = contentType(props.textContent)
        field.font = font
        field.textColor = textColor(props)
        field.textAlignment = alignment
        field.isEnabled = props.enabled ?? true
    }

    private static func contentType(_ content: TextContent?) -> UITextContentType? {
        switch content {
        case .username: .username
        case .email: .emailAddress
        case .currentPassword: .password
        case .newPassword: .newPassword
        case nil: nil
        }
    }

    private static func textColor(_ props: WidgetProps) -> UIColor {
        props.color.map(WidgetStyle.color) ?? .label
    }

    private static func alignment(_ props: WidgetProps) -> NSTextAlignment {
        switch props.textAlign {
        case "center": .center
        case "end": .right
        default: .natural
        }
    }

    /// System fonts by size and weight: every label asks for one, and a
    /// screen uses only a few.
    private static var fonts: [FontKey: UIFont] = [:]

    /// The most fonts kept: an animated arbitrary size makes a new one every
    /// frame, and the cache outlives every app the process runs.
    private static let fontLimit = 64

    private struct FontKey: Hashable {
        let size: CGFloat
        let weight: CGFloat
    }

    private static func font(_ props: WidgetProps) -> UIFont {
        let weight = WidgetFontWeight.uiKit(props.fontWeight)
        let size = props.fontSize ?? defaultSize
        let key = FontKey(size: size, weight: weight.rawValue)

        if let cached = fonts[key] { return cached }

        let font = UIFont.systemFont(ofSize: size, weight: weight)

        if fonts.count >= fontLimit { fonts.removeAll(keepingCapacity: true) }

        fonts[key] = font

        return font
    }
}

/// UIKit's weight for each weight a render transaction names: ultralight is
/// CSS's 100 and thin its 200, as Apple's fonts number them, and heavy is 800.
enum WidgetFontWeight {
    static func uiKit(_ name: String?) -> UIFont.Weight {
        switch name {
        case "thin": .ultraLight
        case "extra_light": .thin
        case "light": .light
        case "medium": .medium
        case "semibold": .semibold
        case "bold": .bold
        case "extra_bold": .heavy
        case "black": .black
        default: .regular
        }
    }
}

extension UITextField {
    /// Masks the field's text, and keeps the keyboard from correcting or learning it.
    func maskEntry() {
        isSecureTextEntry = true
        autocorrectionType = .no
        spellCheckingType = .no
        autocapitalizationType = .none
    }
}
