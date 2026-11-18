import CLeptosKitBridge
import Foundation
@testable import LeptosKitThermal
import LeptosKitBridge
import Testing

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("thermal".utf8)
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

/// The first little-endian u32 of a snapshot.
private func field(_ data: Data, _ index: Int) -> UInt32 {
    data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: index * 4, as: UInt32.self)) }
}

@Test
func thermalStatesMapToTheRustWireCodes() {
    #expect(stateCode(.nominal) == 1)
    #expect(stateCode(.fair) == 2)
    #expect(stateCode(.serious) == 3)
    #expect(stateCode(.critical) == 4)
}

@Test
@MainActor
func thermalSnapshotMatchesProcessInfoAndCloseIsTerminal() async throws {
    let plugin = ThermalPlugin()
    let snapshot = try await plugin.invoke(method: 0, payload: payload())
    #expect(snapshot.count == 8)
    #expect(field(snapshot, 0) == stateCode(ProcessInfo.processInfo.thermalState))
    #expect(field(snapshot, 1) == UInt32.max)

    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload(Data([0])))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(Data(repeating: 0, count: 7)))
    }
    var invalid = snapshot
    invalid[0] = 5
    let invalidState = invalid
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(invalidState))
    }
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 2, payload: payload())
    }

    plugin.close()
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func thermalWaitAnswersADifferentStateImmediately() async throws {
    let plugin = ThermalPlugin()
    defer { plugin.close() }
    let snapshot = try await plugin.invoke(method: 0, payload: payload())
    // A caller whose last reading was unknown hears the current state at once.
    let unknown = Data([0, 0, 0, 0, 255, 255, 255, 255])
    let changed = try await plugin.invoke(method: 1, payload: payload(unknown))
    #expect(changed == snapshot)
}

@Test
@MainActor
func thermalCancellationAndCloseReleaseWaiters() async throws {
    let plugin = ThermalPlugin()
    defer { plugin.close() }

    for _ in 0..<3 {
        let snapshot = try await plugin.invoke(method: 0, payload: payload())
        let pending = Task { @MainActor in
            try await plugin.invoke(method: 1, payload: payload(snapshot))
        }
        // Let the waiter install its observer before cancelling. The device
        // may heat or cool in this interval, so either completion is valid.
        try await Task.sleep(for: .milliseconds(10))
        let busy = try await plugin.invoke(method: 0, payload: payload())
        if busy == snapshot {
            await #expect(throws: PluginError.busy) {
                try await plugin.invoke(method: 1, payload: payload(snapshot))
            }
        }
        pending.cancel()
        switch await pending.result {
        case .success(let value): #expect(value != snapshot)
        case .failure(let error): #expect(error is CancellationError)
        }
    }

    let snapshot = try await plugin.invoke(method: 0, payload: payload())
    let pending = Task { @MainActor in try await plugin.invoke(method: 1, payload: payload(snapshot)) }
    try await Task.sleep(for: .milliseconds(10))
    plugin.close()
    switch await pending.result {
    case .success(let value): #expect(value != snapshot)
    case .failure(let error): #expect(error as? PluginError == .cancelled)
    }
}
