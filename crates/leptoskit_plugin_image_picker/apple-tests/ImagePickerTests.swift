import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitImagePicker
import Testing
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("image-picker".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = data.withUnsafeBytes {
        lk_bridge_submit(bridge, 0, 0, $0.baseAddress?.assumingMemoryBound(to: UInt8.self), $0.count)
    }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@MainActor
private func request(media: UInt32 = 0, maxBytes: UInt32 = 1024) -> Data {
    var data = Data(count: 8)
    data.replaceSubrange(0..<4, with: withUnsafeBytes(of: media.littleEndian) { Data($0) })
    data.replaceSubrange(4..<8, with: withUnsafeBytes(of: maxBytes.littleEndian) { Data($0) })
    return data
}

@Test
@MainActor
func imagePickerRejectsTruncatedRequest() async throws {
    #if canImport(AppKit)
    let owner = NSViewController()
    #else
    let owner = UIViewController()
    #endif
    let plugin = ImagePickerPlugin(context: NativePluginContext(owner: owner))
    await #expect(throws: PluginError.invalidArgument) {
        _ = try await plugin.invoke(method: 0, payload: payload(Data([0, 0, 0, 0])))
    }
}

@Test
@MainActor
func imagePickerRejectsOversizeLimit() async throws {
    #if canImport(AppKit)
    let owner = NSViewController()
    #else
    let owner = UIViewController()
    #endif
    let plugin = ImagePickerPlugin(context: NativePluginContext(owner: owner))
    await #expect(throws: PluginError.tooLarge) {
        _ = try await plugin.invoke(
            method: 0,
            payload: payload(request(maxBytes: 16 * 1024 * 1024 + 1))
        )
    }
}

@Test
@MainActor
func imagePickerUnknownMethodIsUnavailable() async throws {
    #if canImport(AppKit)
    let owner = NSViewController()
    #else
    let owner = UIViewController()
    #endif
    let plugin = ImagePickerPlugin(context: NativePluginContext(owner: owner))
    await #expect(throws: PluginError.unavailable) {
        _ = try await plugin.invoke(method: 9, payload: payload(request()))
    }
}

@Test
@MainActor
func imagePickerDetachedOwnerCannotPresent() async throws {
    #if canImport(AppKit)
    let owner = NSViewController()
    #else
    let owner = UIViewController()
    #endif
    let plugin = ImagePickerPlugin(context: NativePluginContext(owner: owner))
    await #expect(throws: PluginError.unavailable) {
        _ = try await plugin.invoke(method: 0, payload: payload(request()))
    }
}
