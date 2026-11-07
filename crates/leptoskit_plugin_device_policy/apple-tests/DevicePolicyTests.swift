import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitDevicePolicy
import Testing

private func payload(_ bytes: [UInt8] = []) throws -> NativeBuffer {
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

@Test
@MainActor
func emptyPayloadMethodsAreUnavailableWithoutInferringAdminRights() async throws {
    let plugin = DevicePolicyPlugin()

    for method: UInt32 in [0, 2, 7] {
        await #expect(throws: PluginError.unavailable) {
            try await plugin.invoke(method: method, payload: payload())
        }
    }

    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func payloadMethodsRejectEmptyBeforeUnavailable() async throws {
    let plugin = DevicePolicyPlugin()

    for method: UInt32 in [1, 3, 4, 5, 6] {
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: method, payload: payload())
        }
    }

    for method: UInt32 in [1, 3, 4, 5, 6] {
        await #expect(throws: PluginError.unavailable) {
            try await plugin.invoke(method: method, payload: payload([1, 0, 0, 0]))
        }
    }
}
