import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitCellular
import Testing

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("cellular".utf8)
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

@Test
@MainActor
func cellularSnapshotRejectsTrailingPayload() async throws {
    let plugin = CellularPlugin()
    await #expect(throws: PluginError.invalidArgument) {
        _ = try await plugin.invoke(method: 0, payload: payload(Data([0])))
    }
}

@Test
@MainActor
func cellularUnknownMethodIsUnavailable() async throws {
    let plugin = CellularPlugin()
    await #expect(throws: PluginError.unavailable) {
        _ = try await plugin.invoke(method: 9, payload: payload())
    }
}

@Test
@MainActor
func cellularSnapshotReturnsValidatedWire() async throws {
    let plugin = CellularPlugin()
    let data = try await plugin.invoke(method: 0, payload: payload())
    #expect(data.count >= 28)
    let generation = data.withUnsafeBytes { $0.loadUnaligned(as: UInt32.self).littleEndian }
    #expect(generation <= 6)
}
