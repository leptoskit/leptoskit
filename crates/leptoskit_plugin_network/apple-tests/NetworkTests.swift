import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitNetwork
import Testing

/// A satisfied path over Wi-Fi plus a tunnel, as `APPLE_WIFI_AND_OTHER` in the
/// Rust crate's tests decodes it.
private let wifiAndOther: [UInt8] = [
    2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
    255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0x41, 0, 0, 0
]

/// Where the transport bitset starts in a reading.
private let transportsOffset = 32

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("network".utf8)
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

/// Lets queued main-actor work, such as a newly started request, run.
@MainActor
private func settle() async {
    for _ in 0..<20 { await Task.yield() }
}

@Test
@MainActor
func snapshotsAnswerWhileAChangeIsAwaited() async throws {
    let plugin = NetworkPlugin()
    let current = try await plugin.invoke(method: 0, payload: payload())
    #expect(current.count == 36)

    // Waiting for a change from the current reading keeps the request open.
    let change = Task { try await plugin.invoke(method: 1, payload: payload(current)) }
    await settle()

    // A snapshot has its own monitor, so it is not refused while the wait runs.
    #expect(try await plugin.invoke(method: 0, payload: payload()).count == 36)
    await #expect(throws: PluginError.busy) {
        try await plugin.invoke(method: 1, payload: payload(current))
    }

    change.cancel()
    await #expect(throws: (any Error).self) { try await change.value }
    await settle()

    // The cancelled wait released its slot.
    let replacement = Task { try await plugin.invoke(method: 1, payload: payload(current)) }
    await settle()
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await replacement.value }
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: payload()) }
}

/// Reads one little-endian field of an encoded reading.
private func field(_ data: Data, _ index: Int) -> UInt32 {
    data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: index * 4, as: UInt32.self)) }
}

@Test
func wifiWithATunnelEncodesTheGoldenBytes() {
    let reading = PathReading(satisfied: true, wifi: true, other: true)
    #expect(Array(NetworkPlugin.encode(reading)) == wifiAndOther)
}

@Test
func offlinePathsCarryNoTransports() {
    let encoded = NetworkPlugin.encode(PathReading(satisfied: false, wifi: true, cellular: true))
    #expect(encoded.count == 36)
    #expect(field(encoded, 0) == 1)
    #expect(field(encoded, 1) == 1)
    #expect(field(encoded, 8) == 0)
}

@Test
func everyInterfaceTypeTheProviderReportsIsListed() {
    let all = PathReading(
        satisfied: true, wifi: true, cellular: true, wiredEthernet: true, other: true,
        expensive: true, constrained: true, ultraConstrained: true
    )
    let encoded = NetworkPlugin.encode(all)
    #expect(field(encoded, 0) == 2, "Wi-Fi stays the primary connection")
    #expect(field(encoded, 3) == 2)
    #expect(field(encoded, 4) == 2)
    #expect(field(encoded, 8) == 0b110_0111, "Wi-Fi, cellular, ethernet, satellite and other")
}

@Test
func satelliteDataKeepsItsCellularPrimary() {
    let encoded = NetworkPlugin.encode(PathReading(satisfied: true, cellular: true, ultraConstrained: true))
    #expect(field(encoded, 0) == 3)
    #expect(field(encoded, 8) == 0b10_0010)
}

@Test
func anUntypedSatisfiedPathIsOther() {
    let encoded = NetworkPlugin.encode(PathReading(satisfied: true))
    #expect(field(encoded, 0) == 7)
    #expect(field(encoded, 8) == 0b100_0000, "the primary's own bit is always present")
}

@Test
func previousReadingsAreValidatedFieldByField() throws {
    let golden = Data(wifiAndOther)
    #expect(try NetworkPlugin.validated(golden) == golden)
    #expect(throws: PluginError.invalidArgument) { try NetworkPlugin.validated(golden.prefix(32)) }
    #expect(throws: PluginError.invalidArgument) { try NetworkPlugin.validated(golden + [0]) }

    for (offset, value) in [(0, UInt8(8)), (4, 3), (16, 3), (28, 5), (transportsOffset, 0x80)] {
        var invalid = golden
        invalid[offset] = value
        let candidate = invalid
        #expect(throws: PluginError.invalidArgument) { try NetworkPlugin.validated(candidate) }
    }

    var everyTransport = golden
    everyTransport[transportsOffset] = 0x7F
    #expect(try NetworkPlugin.validated(everyTransport) == everyTransport)
}

@Test
@MainActor
func malformedRequestsAreRefusedBeforeAnyMonitorStarts() async throws {
    let plugin = NetworkPlugin()
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload(Data([0])))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload(Data(repeating: 0, count: 35)))
    }
    // A connection code past the last one the wire defines.
    var invalid = Data(repeating: 0, count: 36)
    invalid[0] = 8
    await #expect(throws: PluginError.invalidArgument) { try await plugin.invoke(method: 1, payload: payload(invalid)) }
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 7, payload: payload()) }
}

@Test
@MainActor
func theRealPathMonitorAnswersAValidReadingAndCloseIsTerminal() async throws {
    let plugin = NetworkPlugin()
    let snapshot = try await plugin.invoke(method: 0, payload: payload())
    #expect(try NetworkPlugin.validated(snapshot) == snapshot)

    if field(snapshot, 0) == 1 { #expect(field(snapshot, 8) == 0) }

    let pending = Task { @MainActor in try await plugin.invoke(method: 1, payload: payload(snapshot)) }
    try await Task.sleep(for: .milliseconds(10))
    plugin.close()
    switch await pending.result {
    case .success(let value): #expect(value != snapshot)
    case .failure(let error): #expect(error as? PluginError == .cancelled)
    }
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}
