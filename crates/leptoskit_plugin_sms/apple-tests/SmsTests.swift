import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitSms
import Testing

#if canImport(UIKit)
import UIKit

@MainActor
private final class SmsSpy: SmsComposerService {
    var canSend = true
    var presented: [SmsCompose] = []
    var resume: CheckedContinuation<Void, Error>?

    func present(_ compose: SmsCompose, from owner: NativePresentationOwner) async throws {
        presented.append(compose)
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

private func composeBytes(recipients: [String], body: String) -> [UInt8] {
    var bytes: [UInt8] = []
    func appendU32(_ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
    }
    func appendString(_ value: String) {
        let utf8 = Array(value.utf8)
        appendU32(UInt32(utf8.count))
        bytes.append(contentsOf: utf8)
    }
    appendU32(UInt32(recipients.count))
    for recipient in recipients { appendString(recipient) }
    appendString(body)

    return bytes
}

@Test
@MainActor
func readinessAndInjectedComposerNeverClaimDelivery() async throws {
    let owner = UIViewController()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    window.rootViewController = owner
    window.makeKeyAndVisible()
    let spy = SmsSpy()
    let plugin = SmsPlugin(context: NativePluginContext(owner: owner), service: spy)

    defer { plugin.close(); window.isHidden = true }
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([2, 0, 0, 0]))
    let task = Task {
        try await plugin.invoke(method: 1, payload: payload(composeBytes(recipients: ["5550100"], body: "hi")))
    }
    try await Task.sleep(nanoseconds: 20_000_000)
    #expect(spy.presented.count == 1)
    #expect(spy.presented[0].recipients == ["5550100"])
    #expect(spy.presented[0].body == "hi")
    spy.finish()
    #expect(try await task.value == Data())
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload([]))
    }
}

@Test
@MainActor
func malformedPayloadNeverReachesComposer() async throws {
    let owner = UIViewController()
    let spy = SmsSpy()
    let plugin = SmsPlugin(context: NativePluginContext(owner: owner), service: spy)

    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload([0]))
    }
    #expect(spy.presented.isEmpty)
    plugin.close()
}
#endif
