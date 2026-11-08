import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitBackground
import Testing

private func u32(_ value: UInt32) -> [UInt8] {
    var little = value.littleEndian

    return withUnsafeBytes(of: &little) { Array($0) }
}

private func startBytes(
    title: String,
    text: String,
    types: UInt32,
    interval: UInt32
) -> [UInt8] {
    var result: [UInt8] = []

    for field in [Array(title.utf8), Array(text.utf8)] {
        result += u32(UInt32(field.count)) + field
    }

    result += u32(types)
    result += u32(interval)

    return result
}

private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let id = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(id == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@Test
@MainActor
func startIsUnavailableOnMacOSAndRejectsMalformedMessages() async throws {
    let plugin = BackgroundPlugin()
    defer { plugin.close() }
    let empty = NativeBuffer(taking: lk_buffer())
    let valid = try payload(startBytes(title: "Running", text: "", types: 0, interval: 60))

    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 0, payload: valid)
    }

    for invalid in [[1, 2, 3], startBytes(title: "x".repeated(129), text: "", types: 0, interval: 60)] {
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 0, payload: payload(invalid))
        }
    }

    _ = try await plugin.invoke(method: 1, payload: empty)
}

@Test
@MainActor
func statusAndBatteryReplyHaveExactWireSizes() async throws {
    let plugin = BackgroundPlugin()
    defer { plugin.close() }
    let empty = NativeBuffer(taking: lk_buffer())

    let status = try await plugin.invoke(method: 2, payload: empty)
    #expect(status.count == 12)
    #expect(status[0] == 0, "Not running before start")

    let battery = try await plugin.invoke(method: 3, payload: empty)
    #expect(battery.count == 4)
    #expect(battery[0] == 0, "Battery state is Unknown on Apple")

    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 2, payload: payload([1]))
    }
}

private extension String {
    func repeated(_ count: Int) -> String {
        String(repeating: "x", count: count)
    }
}
