import AppKit

/// A static text label that draws its own string.
///
/// An `NSTextField` label is a control, a cell, and a private drawing subview,
/// and a text change runs through all three; a screen of changing numbers spends
/// most of its frame there. This view draws the same attributed string into the
/// box layout measured for it (see `AppKitTextMeasurer`): line-fragment origins,
/// word wrapping, and the horizontal inset a label cell reserves. Like `UILabel`
/// and Android's `TextView`, it is not selectable.
///
/// It inks exactly the pixels an `NSTextField` label inks. AppKit's own label
/// also darkens glyph edges with private font dilation, which no public drawing
/// API offers, so edges here are a shade lighter.
final class TextLabel: NSView {
    /// The horizontal inset a label cell draws its text with, on each side;
    /// a centered paragraph reserves it twice.
    private static let inset: CGFloat = 2

    private(set) var text = ""
    private(set) var font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
    private(set) var textColor = NSColor.labelColor
    private(set) var alignment = NSTextAlignment.natural
    /// The height of every line, or nil for the font's own.
    private(set) var lineHeight: CGFloat?
    private var string = NSAttributedString()

    override var isFlipped: Bool { true }

    /// Updates what the label shows, redrawing only when something changed.
    func update(
        text: String, font: NSFont, color: NSColor, alignment: NSTextAlignment, lineHeight: CGFloat? = nil
    ) {
        guard text != self.text || font != self.font || color != textColor || alignment != self.alignment
            || lineHeight != self.lineHeight
        else { return }

        self.text = text
        self.font = font
        textColor = color
        self.alignment = alignment
        self.lineHeight = lineHeight
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = alignment
        paragraph.lineBreakMode = .byWordWrapping
        var attributes: [NSAttributedString.Key: Any] = [
            .font: font, .foregroundColor: color, .paragraphStyle: paragraph
        ]

        // Every line is exactly the line height with its glyphs centered, as
        // `AppKitTextMeasurer` measures them: AppKit puts the extra above the glyphs.
        if let lineHeight {
            paragraph.minimumLineHeight = lineHeight
            paragraph.maximumLineHeight = lineHeight
            attributes[.baselineOffset] = (lineHeight - (font.ascender - font.descender)) / 2
        }

        string = NSAttributedString(string: text, attributes: attributes)

        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        let inset = alignment == .center ? Self.inset * 2 : Self.inset
        string.draw(with: bounds.insetBy(dx: inset, dy: 0), options: .usesLineFragmentOrigin)
    }

    /// The default label color is dynamic: it resolves again in the new appearance.
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .staticText }
    override func accessibilityValue() -> Any? { text }
}
