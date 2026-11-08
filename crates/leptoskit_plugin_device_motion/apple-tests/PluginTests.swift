import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitDeviceMotion
import Testing

@MainActor
private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("device-motion-test".utf8)
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

private func intervalBytes(_ milliseconds: UInt32) -> [UInt8] {
    var little = milliseconds.littleEndian
    return withUnsafeBytes(of: &little) { Array($0) }
}

@Test @MainActor
func unavailableClosedAndInvalidPayloadsAreExplicit() async throws {
    let plugin = DeviceMotionPlugin()

    #if os(macOS)
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([0, 0, 0, 0]))
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 1, payload: payload(intervalBytes(100)))
    }
    #endif

    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([1]))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload([1, 0]))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(intervalBytes(1)))
    }

    let encoded = try DeviceMotionWire.encodeReading(
        timestamp: 1,
        accelX: 0, accelY: 0, accelZ: 0,
        gravityX: 0, gravityY: 0, gravityZ: 9.8,
        rotationX: 0, rotationY: 0, rotationZ: 0,
        attitudeAlpha: 0, attitudeBeta: 0, attitudeGamma: 0
    )
    #expect(encoded.count == DeviceMotionWire.readingBytes)

    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload([]))
    }
}
