import AppKit
import LeptosKitBridge

/// A status item owns the application. The Wasm UI is created on demand.
@main
@MainActor
final class MacApp: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var status: NSStatusItem?
    private var window: NSWindow?
    private var controller: LeptosKitMacViewController?

    static func main() {
        let app = NSApplication.shared
        let delegate = MacApp()
        app.setActivationPolicy(.accessory)
        app.delegate = delegate
        app.run()
        withExtendedLifetime(delegate) {}
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String ?? "App"
        let status = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        status.button?.title = name
        let menu = NSMenu()
        let open = menu.addItem(withTitle: "Open \(name)", action: #selector(openWindow), keyEquivalent: "")
        open.target = self
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        status.menu = menu
        self.status = status
    }

    @objc private func openWindow() {
        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate()
            return
        }
        do {
            guard let url = Bundle.main.url(forResource: "app", withExtension: "wasm") else {
                throw CocoaError(.fileNoSuchFile)
            }
            let controller = try LeptosKitMacViewController(
                wasm: Data(contentsOf: url), register: GeneratedPlugins.register
            )
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 560, height: 760),
                styleMask: [.titled, .closable, .miniaturizable, .resizable],
                backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.delegate = self
            window.contentViewController = controller
            self.controller = controller
            self.window = window
            window.center()
            window.makeKeyAndOrderFront(nil)
            NSApp.activate()
        } catch {
            NativeLog.error(NativeLog.bridge, "cannot open the app", error)
        }
    }

    func windowWillClose(_ notification: Notification) {
        controller?.close()
        controller = nil
        window = nil
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationWillTerminate(_ notification: Notification) { controller?.close() }
}
