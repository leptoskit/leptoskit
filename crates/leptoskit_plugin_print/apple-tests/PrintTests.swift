import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitPrint
import Testing

#if canImport(UIKit)
import UIKit

@MainActor
private final class PrintSpy: PrintService {
    var presented: [PrintJob] = []
    var resume: CheckedContinuation<Void, Error>?

    func present(_ job: PrintJob, from owner: NativePresentationOwner) async throws {
        presented.append(job)
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

private func jobBytes(kind: UInt32, body: String) -> [UInt8] {
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
    appendU32(kind)
    appendString(body)

    return bytes
}

@Test
@MainActor
func readinessAndInjectedPrinterNeverClaimPaper() async throws {
    let owner = UIViewController()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    window.rootViewController = owner
    window.makeKeyAndVisible()
    let spy = PrintSpy()
    let plugin = PrintPlugin(context: NativePluginContext(owner: owner), service: spy)

    defer { plugin.close(); window.isHidden = true }
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([1, 0, 0, 0]))
    let task = Task {
        try await plugin.invoke(method: 1, payload: payload(jobBytes(kind: 0, body: "<p>hi</p>")))
    }
    try await Task.yield()
    #expect(spy.presented == [.html("<p>hi</p>")])
    spy.finish()
    #expect(try await task.value == Data())
}

@Test
func wireRejectsBadPdfUrisAndUnknownKinds() throws {
    #expect(throws: PluginError.invalidArgument) {
        try jobBytes(kind: 1, body: "mailto:a@example.test").withUnsafeBytes(PrintJob.decode)
    }
    #expect(throws: PluginError.invalidArgument) {
        try jobBytes(kind: 9, body: "x").withUnsafeBytes(PrintJob.decode)
    }
    let good = try jobBytes(kind: 1, body: "https://example.test/a.pdf").withUnsafeBytes(PrintJob.decode)
    #expect(good == .pdfUri("https://example.test/a.pdf"))
}
#endif
