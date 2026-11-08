import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitWebBrowser
import Testing

#if canImport(UIKit)
import UIKit

@MainActor
private final class BrowserSpy: WebBrowserService {
    var opened: [URL] = []
    var resume: CheckedContinuation<Void, Error>?

    func open(_ url: URL, from owner: NativePresentationOwner) async throws {
        opened.append(url)
        try await withCheckedThrowingContinuation { continuation in
            resume = continuation
        }
    }

    func finish() {
        resume?.resume()
        resume = nil
    }
}

private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

private func openBytes(_ url: String) -> [UInt8] {
    var bytes: [UInt8] = []
    let utf8 = Array(url.utf8)
    var length = UInt32(utf8.count).littleEndian
    withUnsafeBytes(of: &length) { bytes.append(contentsOf: $0) }
    bytes.append(contentsOf: utf8)

    return bytes
}

@Test
@MainActor
func readinessAndInjectedBrowserWaitsForDismiss() async throws {
    let owner = UIViewController()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    window.rootViewController = owner
    window.makeKeyAndVisible()
    let spy = BrowserSpy()
    let plugin = WebBrowserPlugin(context: NativePluginContext(owner: owner), service: spy)

    defer { plugin.close(); window.isHidden = true }
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([1, 0, 0, 0]))
    let task = Task {
        try await plugin.invoke(
            method: 1,
            payload: payload(openBytes("https://example.test/path"))
        )
    }
    try await Task.sleep(nanoseconds: 20_000_000)
    #expect(spy.opened.count == 1)
    #expect(spy.opened[0].absoluteString == "https://example.test/path")
    spy.finish()
    #expect(try await task.value == Data())
    plugin.close()
}

@Test
@MainActor
func rejectsNonHttpUrlsAndDoesNotOpen() async throws {
    let owner = UIViewController()
    let spy = BrowserSpy()
    let plugin = WebBrowserPlugin(context: NativePluginContext(owner: owner), service: spy)

    for bad in ["mailto:a@b.test", "file:///etc/passwd", "javascript:alert(1)"] {
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 1, payload: payload(openBytes(bad)))
        }
    }
    #expect(spy.opened.isEmpty)
    plugin.close()
}
#endif
