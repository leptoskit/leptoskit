import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitMailComposer
import Testing

#if canImport(UIKit)
import UIKit

@MainActor
private final class MailSpy: MailComposerService {
    var canSend = true
    var presented: [MailCompose] = []
    var resume: CheckedContinuation<Void, Error>?

    func present(_ compose: MailCompose, from owner: NativePresentationOwner) async throws {
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

private func composeBytes(
    to: [String],
    cc: [String],
    bcc: [String],
    subject: String,
    body: String
) -> [UInt8] {
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
    func appendList(_ values: [String]) {
        appendU32(UInt32(values.count))
        for value in values { appendString(value) }
    }
    appendList(to)
    appendList(cc)
    appendList(bcc)
    appendString(subject)
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
    let spy = MailSpy()
    let plugin = MailComposerPlugin(context: NativePluginContext(owner: owner), service: spy)

    defer { plugin.close(); window.isHidden = true }
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([2, 0, 0, 0]))
    let task = Task {
        try await plugin.invoke(
            method: 1,
            payload: payload(composeBytes(
                to: ["a@example.test"],
                cc: [],
                bcc: [],
                subject: "Hi",
                body: "Body"
            ))
        )
    }
    try await Task.sleep(nanoseconds: 20_000_000)
    #expect(spy.presented.count == 1)
    #expect(spy.presented[0].to == ["a@example.test"])
    #expect(spy.presented[0].subject == "Hi")
    spy.finish()
    #expect(try await task.value == Data())
    plugin.close()
}

@Test
@MainActor
func malformedPayloadNeverReachesComposer() async throws {
    let owner = UIViewController()
    let spy = MailSpy()
    let plugin = MailComposerPlugin(context: NativePluginContext(owner: owner), service: spy)

    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload([0]))
    }
    #expect(spy.presented.isEmpty)
    plugin.close()
}
#endif
