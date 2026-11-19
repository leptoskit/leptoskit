@testable import LeptosKitMacControllers
import AppKit
import Foundation
import LeptosKitBridge
import Testing

@Suite(.serialized)
@MainActor
struct MacOverlayTests {
    private struct Event: Equatable { let node: Int; let kind: UInt32 }

    private func window() -> (NSWindow, NSView) {
        _ = NSApplication.shared
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.titled],
            backing: .buffered, defer: true)
        let view = NSView()
        window.contentView = view
        window.isReleasedWhenClosed = false

        return (window, view)
    }

    /// A modal's content shows as a sheet sized to what it holds, with the named field focused.
    /// Escape asks the app, and the transaction that closes the modal takes the sheet down.
    @Test func modalsShowAsSheets() throws {
        let (window, view) = window()
        let renderer = MacWidgetRenderer(view: view)
        var events: [Event] = []
        renderer.onEvent = { node, kind, _ in events.append(Event(node: node, kind: kind)) }
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{}}},
          {"Create":{"id":2,"kind":"text","props":{"text":"Behind"}}},
          {"Create":{"id":3,"kind":"modal","props":{
            "background":255,"overlay":{"open":true,"initial_focus":"reason"}}}},
          {"Create":{"id":4,"kind":"view","props":{}}},
          {"Create":{"id":5,"kind":"text_area","props":{"id":"reason"}}},
          {"Create":{"id":6,"kind":"text","props":{"text":"After"}}},
          {"InsertChild":{"parent":1,"child":2,"index":0}},
          {"InsertChild":{"parent":1,"child":3,"index":1}},
          {"InsertChild":{"parent":1,"child":6,"index":2}},
          {"InsertChild":{"parent":3,"child":4,"index":0}},
          {"InsertChild":{"parent":4,"child":5,"index":0}},
          {"UpdateLayout":{"id":3,"x":0,"y":0,"width":400,"height":300}},
          {"UpdateLayout":{"id":4,"x":72,"y":86,"width":256,"height":128}},
          {"UpdateLayout":{"id":5,"x":8,"y":8,"width":200,"height":60}}
        ]}
        """))
        let overlay = try #require(renderer.widgets[3] as? MacOverlayView)
        let sheet = try #require(overlay.presentedWindow)

        #expect(overlay.isShown)
        // The modal covers the viewport and centres its panel. The sheet is the panel alone, and the
        // modal's backdrop is not drawn.
        #expect(sheet.contentView?.frame.size == CGSize(width: 256, height: 128))
        #expect(overlay.content.frame == CGRect(x: -72, y: -86, width: 400, height: 300))
        #expect(overlay.content.layer?.backgroundColor == nil)
        // The modal's place in the tree is hidden and keeps its siblings where the transaction put them.
        let root = try #require(view.subviews.first)
        #expect(root.subviews.count == 3)
        #expect(root.subviews[1] === overlay)
        #expect(overlay.isHidden)
        #expect(overlay.subviews.isEmpty)
        let notes = try #require(renderer.widgets[5] as? MacTextArea)
        #expect(notes.window === sheet)
        #expect(sheet.firstResponder === notes.text)

        overlay.cancel()
        #expect(events == [Event(node: 3, kind: 10)])
        #expect(overlay.isShown)

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateProps":{"id":3,"props":{"overlay":{"open":false}}}}]}
        """))
        #expect(!overlay.isShown)

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateProps":{"id":3,"props":{"overlay":{"open":true}}}}]}
        """))
        #expect(overlay.isShown)

        // Removing an open modal takes its sheet down with it.
        try renderer.render(decode("""
        {"version":1,"mutations":[{"RemoveChild":{"parent":1,"child":3}},{"Delete":{"id":3}}]}
        """))
        #expect(!overlay.isShown)
        renderer.close()
        window.close()
    }

    /// A modal with no window to show on stays down, and shows once its place in the tree joins one.
    @Test func modalsWaitForAWindow() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{}}},
          {"Create":{"id":2,"kind":"modal","props":{"overlay":{"open":true}}}},
          {"Create":{"id":3,"kind":"text","props":{"text":"Sure?"}}},
          {"InsertChild":{"parent":1,"child":2,"index":0}},
          {"InsertChild":{"parent":2,"child":3,"index":0}},
          {"UpdateLayout":{"id":2,"x":0,"y":0,"width":400,"height":300}},
          {"UpdateLayout":{"id":3,"x":150,"y":140,"width":100,"height":20}}
        ]}
        """))
        let overlay = try #require(renderer.widgets[2] as? MacOverlayView)
        #expect(!overlay.isShown)

        let (window, content) = window()
        content.addSubview(view)
        #expect(overlay.isShown)

        view.removeFromSuperview()
        #expect(!overlay.isShown)
        renderer.close()
        window.close()
    }

    /// A popover hangs from the widget its anchor names, is as large as its layout, and turns AppKit's
    /// wish to close into a request.
    @Test func popoversHangFromTheirAnchor() throws {
        let (window, view) = window()
        window.orderFront(nil)
        let renderer = MacWidgetRenderer(view: view)
        var events: [Event] = []
        renderer.onEvent = { node, kind, _ in events.append(Event(node: node, kind: kind)) }
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{}}},
          {"Create":{"id":2,"kind":"button","props":{"id":"me","text":"Account"}}},
          {"Create":{"id":3,"kind":"popover","props":{"overlay":{"open":false,"anchor":"me"}}}},
          {"Create":{"id":4,"kind":"button","props":{"text":"Sign out"}}},
          {"InsertChild":{"parent":1,"child":2,"index":0}},
          {"InsertChild":{"parent":1,"child":3,"index":1}},
          {"InsertChild":{"parent":3,"child":4,"index":0}},
          {"UpdateLayout":{"id":1,"x":0,"y":0,"width":400,"height":300}},
          {"UpdateLayout":{"id":2,"x":150,"y":100,"width":100,"height":30}},
          {"UpdateLayout":{"id":3,"x":150,"y":130,"width":160,"height":80}},
          {"UpdateLayout":{"id":4,"x":8,"y":8,"width":100,"height":30}}
        ]}
        """))
        let overlay = try #require(renderer.widgets[3] as? MacOverlayView)
        let item = try #require(renderer.widgets[4])

        #expect(!overlay.isShown)
        #expect(item.superview === overlay.content)
        #expect(overlay.content.frame.size == CGSize(width: 160, height: 80))

        try renderer.render(decode("""
        {"version":1,"mutations":[
          {"UpdateProps":{"id":3,"props":{"overlay":{"open":true,"anchor":"me"}}}}
        ]}
        """))
        #expect(overlay.isShown)
        #expect(item.window !== window)

        overlay.cancel()
        #expect(events == [Event(node: 3, kind: 10)])
        #expect(overlay.isShown)

        try renderer.render(decode("""
        {"version":1,"mutations":[
          {"UpdateProps":{"id":3,"props":{"overlay":{"open":false,"anchor":"me"}}}}
        ]}
        """))
        #expect(!overlay.isShown)
        renderer.close()
        window.close()
    }

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
