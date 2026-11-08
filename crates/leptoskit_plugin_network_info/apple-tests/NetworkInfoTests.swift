import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitNetworkInfo
import Testing

/// The fully populated snapshot the Rust and Kotlin suites assert too.
private let goldenHex = "7f000000" + "04000000" + "686f6d65" + String(repeating: "00", count: 28)
    + "123456789abc" + "0000" + "c0a80114" + "20010db8000000000000000000000001"
    + "ffffff00" + "c0a801ff" + "c0a80101"

/// The Wi-Fi interface's index in the fake system.
private let wifiIndex: UInt32 = 4

private func hex(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
}

private func v4(_ interface: String, _ address: [UInt8], mask: [UInt8], broadcast: [UInt8]? = nil) -> InterfaceAddress {
    InterfaceAddress(interface: interface, family: .ipv4, address: address, netmask: mask, broadcast: broadcast)
}

private func v6(_ interface: String, _ address: [UInt8]) -> InterfaceAddress {
    InterfaceAddress(interface: interface, family: .ipv6, address: address, netmask: nil, broadcast: nil)
}

/// Fixed system answers; counts calls so tests can see what was read.
@MainActor
private final class FakeSystem: NetworkInfoSystem {
    var interface: String? = "en0"
    var current = WifiAssociation(ssid: Data("home".utf8), bssid: "12:34:56:78:9a:bc")
    var entries: [InterfaceAddress] = [
        v4("lo0", [127, 0, 0, 1], mask: [255, 0, 0, 0]),
        v6("en0", [0xFE, 0x80] + Array(repeating: 0, count: 13) + [1]),
        v4("en0", [169, 254, 1, 1], mask: [255, 255, 0, 0], broadcast: [169, 254, 255, 255]),
        v4("en0", [192, 168, 1, 20], mask: [255, 255, 255, 0], broadcast: [192, 168, 1, 255]),
        v6("en0", [0x20, 0x01, 0x0D, 0xB8] + Array(repeating: 0, count: 11) + [1]),
        v4("en7", [10, 0, 0, 5], mask: [255, 0, 0, 0])
    ]
    var dump: Data? = joinedDump([
        route(index: 9, gateway: [10, 0, 0, 1]),
        route(index: wifiIndex, destination: [10, 0, 0, 0], gateway: [10, 9, 9, 9], mask: [8, 2, 0, 0, 255, 0, 0, 0]),
        route(index: wifiIndex, flags: 0x1, gateway: [172, 16, 0, 1]),
        route(index: wifiIndex, flags: 0x0100_0003, gateway: [192, 168, 1, 1], mask: [0])
    ])
    var associationReads = 0

    func wifiInterface() async -> String? { interface }

    func association() async -> WifiAssociation {
        associationReads += 1
        return current
    }

    func addresses() -> [InterfaceAddress] { entries }

    func routeDump() -> Data? { dump }

    func interfaceIndex(_ name: String) -> UInt32 { name == "en0" ? wifiIndex : 0 }
}

/// Builds one Darwin routing message: an IPv4 destination and gateway, then an optional mask.
private func route(
    index: UInt32, flags: UInt32 = 0x3, destination: [UInt8] = [0, 0, 0, 0], gateway: [UInt8], mask: [UInt8]? = nil
) -> [UInt8] {
    func sockaddr(_ address: [UInt8]) -> [UInt8] {
        [16, 2, 0, 0] + address + Array(repeating: 0, count: 8)
    }

    func little(_ value: UInt32) -> [UInt8] {
        withUnsafeBytes(of: value.littleEndian) { Array($0) }
    }

    var body = sockaddr(destination) + sockaddr(gateway)
    var addresses: UInt32 = 0b011

    if let mask {
        addresses |= 0b100
        // An empty address still takes four bytes.
        let padded = mask[0] == 0 ? 4 : (mask.count + 3) & ~3
        body += mask + Array(repeating: 0, count: padded - mask.count)
    }

    var header = [UInt8](repeating: 0, count: 92)
    let length = UInt16(header.count + body.count)
    header[0] = UInt8(length & 0xFF)
    header[1] = UInt8(length >> 8)
    header[2] = 5
    header[4] = UInt8(index & 0xFF)
    header[5] = UInt8(index >> 8)
    header.replaceSubrange(8..<12, with: little(flags))
    header.replaceSubrange(12..<16, with: little(addresses))
    return header + body
}

private func joinedDump(_ messages: [[UInt8]]) -> Data {
    Data(messages.joined())
}

/// Allocate a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ data: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("network-info".utf8)
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
func aFullReadingMatchesTheSharedGoldenBytes() async throws {
    let plugin = NetworkInfoPlugin(system: FakeSystem())
    let reading = try await plugin.invoke(method: 0, payload: payload())
    #expect(reading.count == 80)
    #expect(hex(reading) == goldenHex)
}

@Test
@MainActor
func withheldValuesStayEmptyInsteadOfPlaceholders() async throws {
    let system = FakeSystem()
    system.interface = nil
    system.current = WifiAssociation()
    let plugin = NetworkInfoPlugin(system: system)
    #expect(try await plugin.snapshot() == Data(count: 80))

    system.current = WifiAssociation(ssid: Data(), bssid: "02:00:00:00:00:00")
    #expect(try await plugin.snapshot() == Data(count: 80), "Android's redaction values never pass through")
}

@Test
@MainActor
func theAssociationIsReportedWithoutAWifiInterface() async throws {
    let system = FakeSystem()
    system.interface = nil
    let reading = try await NetworkInfoPlugin(system: system).snapshot()
    #expect(hex(reading.prefix(4)) == "03000000", "only the SSID and BSSID")
    #expect(hex(reading[4..<46]) == String(goldenHex.dropFirst(8).prefix(84)))
    #expect(reading[46...] == Data(count: 34), "no addresses or gateway")
}

@Test
@MainActor
func aMissingGatewayRouteLeavesOnlyTheGatewayOut() async throws {
    let system = FakeSystem()
    system.dump = joinedDump([route(index: 9, gateway: [10, 0, 0, 1])])
    let reading = try await NetworkInfoPlugin(system: system).snapshot()
    #expect(hex(reading.prefix(4)) == "3f000000")
    #expect(reading.suffix(4) == Data(count: 4))
}

@Test
func ssidsAreRawBoundedBytes() {
    #expect(NetworkInfoWire.ssid(Data([0xFF, 0x00, 0x41])) == Data([0xFF, 0x00, 0x41]))
    #expect(NetworkInfoWire.ssid(Data(repeating: 0x78, count: 32))?.count == 32)
    #expect(NetworkInfoWire.ssid(Data(repeating: 0x78, count: 33)) == nil)
    #expect(NetworkInfoWire.ssid(Data()) == nil)
    #expect(NetworkInfoWire.ssid(nil) == nil)

    let raw = NetworkInfoWire.encode(NetworkInfoWire.Fields(ssid: Data([0xFF, 0x00, 0x41])))
    #expect(hex(raw.prefix(11)) == "0100000003000000ff0041")
}

@Test
func bssidsParseAppleSpellingsAndRefusePlaceholders() {
    #expect(NetworkInfoWire.bssid("12:34:56:78:9a:bc") == [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC])
    #expect(NetworkInfoWire.bssid("A4:2B:b0:3:4:5") == [0xA4, 0x2B, 0xB0, 3, 4, 5])

    for withheld in [
        nil, "", "02:00:00:00:00:00", "0:0:0:0:0:0", "ff:ff:ff:ff:ff:ff", "01:00:5e:00:00:01",
        "12:34:56:78:9a", "12:34:56:78:9a:bc:de", "123:4:5:6:7:8", "g1:00:00:00:00:01", "12::56:78:9a:bc"
    ] {
        #expect(NetworkInfoWire.bssid(withheld) == nil, "\(String(describing: withheld))")
    }
}

@Test
func onlyContiguousMasksAreKept() {
    #expect(NetworkInfoWire.mask([255, 255, 255, 0]) == [255, 255, 255, 0])
    #expect(NetworkInfoWire.mask([255, 255, 255, 255]) == [255, 255, 255, 255])
    #expect(NetworkInfoWire.mask([128, 0, 0, 0]) == [128, 0, 0, 0])
    #expect(NetworkInfoWire.mask([0, 0, 0, 0]) == nil)
    #expect(NetworkInfoWire.mask([255, 0, 255, 0]) == nil)
    #expect(NetworkInfoWire.mask([0, 255, 255, 255]) == nil)
    #expect(NetworkInfoWire.mask([255, 255]) == nil)
}

@Test
@MainActor
func routeDumpsYieldTheInterfacesDefaultGatewayOnly() throws {
    let dump = try #require(FakeSystem().dump)
    #expect(NetworkInfoWire.defaultGateway(in: dump, interfaceIndex: wifiIndex) == [192, 168, 1, 1])
    #expect(NetworkInfoWire.defaultGateway(in: dump, interfaceIndex: 9) == [10, 0, 0, 1])
    #expect(NetworkInfoWire.defaultGateway(in: dump, interfaceIndex: 2) == nil)
    #expect(NetworkInfoWire.defaultGateway(in: dump.prefix(100), interfaceIndex: 9) == nil, "a truncated message")
    #expect(NetworkInfoWire.defaultGateway(in: Data(), interfaceIndex: wifiIndex) == nil)
}

@Test
@MainActor
func badCallsAreRefusedAndCloseIsTerminal() async throws {
    let system = FakeSystem()
    let plugin = NetworkInfoPlugin(system: system)
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload(Data([0])))
    }
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 1, payload: payload())
    }
    #expect(system.associationReads == 0, "refused calls read nothing")
    plugin.close()
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func theLiveSystemAnswersAWellFormedReading() async throws {
    let reading = try await NetworkInfoPlugin().invoke(method: 0, payload: payload())
    #expect(reading.count == 80)
    let presence = reading.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
    #expect(presence & ~0x7F == 0)

    if presence & 1 == 0 { #expect(reading[4..<40] == Data(count: 36), "no SSID means no SSID bytes") }
}
