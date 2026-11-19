import UIKit

/// Exercises `Modal` and `Popover` through wire transactions and real UIKit presentations.
/// A swipe on a sheet and a press outside a popover still need simulator touch journeys; these call
/// the delegate methods UIKit calls for them.
extension UIKitControls {
    func overlayControls() throws {
        guard let page = window?.rootViewController else {
            record(false, "overlay controls need the window's controller")

            return
        }
        // Without animation a presentation is in place when the call returns.
        let animated = UIView.areAnimationsEnabled
        UIView.setAnimationsEnabled(false)
        defer { UIView.setAnimationsEnabled(animated) }
        let host = UIView(frame: CGRect(x: 0, y: 0, width: 320, height: 640))
        page.view.addSubview(host)
        let renderer = MobileWidgetRenderer(view: host, devtools: nil)
        defer { renderer.close(); host.removeFromSuperview() }
        var dismissed: [Int] = []
        renderer.onEvent = { node, kind, _ in
            if kind == MobileWidgetRenderer.dismiss { dismissed.append(node) }
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        func render(_ mutations: String) throws {
            let json = "{\"version\":1,\"root\":1,\"mutations\":[\(mutations)]}"
            try renderer.render(decoder.decode(Transaction.self, from: Data(json.utf8)))
        }
        try render("""
        {"Create":{"id":1,"kind":"view","props":{}}},
        {"Create":{"id":2,"kind":"button","props":{"id":"menu"}}},
        {"Create":{"id":3,"kind":"modal","props":{"background":255,"overlay":{"open":false}}}},
        {"Create":{"id":4,"kind":"view","props":{}}},
        {"Create":{"id":5,"kind":"popover","props":{
          "overlay":{"open":false,"anchor":"menu","placement":"above"}}}},
        {"Create":{"id":6,"kind":"text","props":{"text":"After"}}},
        {"InsertChild":{"parent":1,"child":2,"index":0}},
        {"InsertChild":{"parent":1,"child":3,"index":1}},
        {"InsertChild":{"parent":1,"child":5,"index":2}},
        {"InsertChild":{"parent":1,"child":6,"index":3}},
        {"InsertChild":{"parent":3,"child":4,"index":0}},
        {"UpdateLayout":{"id":1,"x":0,"y":0,"width":320,"height":640}},
        {"UpdateLayout":{"id":2,"x":0,"y":300,"width":80,"height":40}},
        {"UpdateLayout":{"id":3,"x":0,"y":0,"width":320,"height":640}},
        {"UpdateLayout":{"id":4,"x":32,"y":256,"width":256,"height":128}},
        {"UpdateLayout":{"id":5,"x":0,"y":200,"width":160,"height":96}}
        """)
        let root = host.subviews[0]

        guard root.subviews.count == 4, let sheet = root.subviews[1] as? MobileOverlayView,
              let hanging = root.subviews[2] as? MobileOverlayView, root.subviews[3] is UILabel else {
            record(false, "overlays keep a place in the tree between their siblings")

            return
        }
        record(sheet.isHidden && sheet.subviews.isEmpty, "an overlay's place in the tree shows nothing")
        record(!sheet.isShown && !hanging.isShown, "closed overlays are not presented")
        record(page.presentedViewController == nil, "nothing is presented while every overlay is closed")
        try sheetControls(page, sheet, render: render) { dismissed }
        dismissed.removeAll()
        try popoverControls(page, hanging, anchor: root.subviews[0], render: render) { dismissed }
    }

    private func sheetControls(
        _ page: UIViewController, _ sheet: MobileOverlayView, render: (String) throws -> Void,
        dismissed: () -> [Int]
    ) throws {
        try render(#"{"UpdateProps":{"id":3,"props":{"background":255,"overlay":{"open":true}}}}"#)
        record(sheet.isShown, "an open modal is presented")
        record(page.presentedViewController === sheet.controller, "the modal's sheet is over the page")
        record(sheet.controller.modalPresentationStyle == .formSheet, "a modal is a form sheet")
        // The modal covers the viewport and centres its panel. The sheet asks for the panel alone.
        record(
            sheet.controller.preferredContentSize == CGSize(width: 256, height: 128),
            "a sheet is sized to what the modal holds")
        record(
            sheet.content.frame == CGRect(x: -32, y: -256, width: 320, height: 640),
            "the modal's content is shifted so its panel fills the sheet")
        record(sheet.content.backgroundColor == .clear, "a modal's backdrop is not drawn in a sheet")

        if let presentation = sheet.controller.presentationController {
            record(
                !sheet.presentationControllerShouldDismiss(presentation),
                "a swipe does not take the sheet down by itself")
            sheet.presentationControllerDidAttemptToDismiss(presentation)
        }
        record(dismissed() == [3], "a swipe on the sheet asks the app to close the modal")
        record(sheet.controller.accessibilityPerformEscape(), "the VoiceOver escape gesture is handled")
        record(dismissed() == [3, 3] && sheet.isShown, "escape asks too, and the app decides")

        try render(#"{"UpdateProps":{"id":3,"props":{"background":255,"overlay":{"open":false}}}}"#)
        record(!sheet.isShown, "closing the modal takes the sheet down")
        record(page.presentedViewController == nil, "the page presents nothing after the modal closes")
    }

    private func popoverControls(
        _ page: UIViewController, _ hanging: MobileOverlayView, anchor: UIView,
        render: (String) throws -> Void, dismissed: () -> [Int]
    ) throws {
        try render("""
        {"UpdateProps":{"id":5,"props":{"overlay":{"open":true,"anchor":"menu","placement":"above"}}}}
        """)
        record(hanging.isShown, "an open popover is presented")
        record(
            hanging.controller.preferredContentSize == CGSize(width: 160, height: 96),
            "a popover is as large as its layout")
        let presentation = hanging.controller.popoverPresentationController
        record(presentation?.sourceView === anchor, "a popover hangs from its anchor")
        record(presentation?.permittedArrowDirections == .down, "a popover above its anchor points down at it")

        if let presentation {
            record(
                !hanging.presentationControllerShouldDismiss(presentation),
                "a press outside does not take the popover down by itself")
            record(
                hanging.adaptivePresentationStyle(for: presentation, traitCollection: page.traitCollection)
                    == .none,
                "a popover stays a popover on a phone")
        }
        record(dismissed() == [5], "a press outside asks the app to close the popover")

        try render(#"{"RemoveChild":{"parent":1,"child":5}},{"Delete":{"id":5}}"#)
        record(page.presentedViewController == nil, "removing an open popover takes it down")
    }
}
