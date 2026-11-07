import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitDevice
import Testing

/// Allocates a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ bytes: [UInt8] = []) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("device".utf8)
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

private func hex(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
}

/// The same fixture and bytes as `apple_fixture` in the Rust crate's tests.
private let appleFixture = DeviceFacts(
    kind: 1,
    simulator: true,
    totalMemory: 6 << 30,
    availableMemory: 1 << 20,
    totalDisk: 128 << 30,
    freeDisk: 0,
    cpuCount: 6,
    manufacturer: "Apple",
    brand: "Apple",
    model: "iPhone",
    modelIdentifier: "iPhone15,2",
    osName: "iOS",
    osVersion: "18.1",
    architecture: "aarch64",
    kernelVersion: "24.1.0",
    iosAppOnMac: false,
    macCatalyst: false,
    iosAppOnVision: nil,
    cpuFrequency: 0,
    localizedModel: "iPhone",
    osBuild: "22B83"
)

private let appleGolden = [
    "02000000010000000200000000000080010000000100000000001000000000000000000020000000",
    "01000000000000000000000006000000050000004170706c65050000004170706c65060000006950",
    "686f6e650a0000006950686f6e6531352c3203000000694f530400000031382e3107000000616172",
    "636836340600000032342e312e300100000001000000010000000000000000000000000000000600",
    "00006950686f6e65050000003232423833"
].joined()

private let identifiersFixture = DeviceIdentifiers(
    deviceName: "Ada's iPhone",
    vendorId: "01234567-89AB-CDEF-0123-456789ABCDEF",
    machineId: nil
)

private let identifiersGolden = [
    "010000000c0000004164612773206950686f6e652400000030313233343536372d383941422d4344",
    "45462d303132332d34353637383941424344454600000000"
].joined()

@Test
func snapshotBytesMatchTheRustGolden() throws {
    #expect(hex(try DeviceWire.snapshot(appleFixture)) == appleGolden)
}

@Test
func identifierBytesMatchTheRustGolden() throws {
    #expect(hex(try DeviceWire.identifiers(identifiersFixture)) == identifiersGolden)
}

@Test
func unknownFieldsEncodeAsEmptyAndZero() throws {
    let data = try DeviceWire.snapshot(DeviceFacts())
    // Header (14 words), eight empty strings, the Apple tag and its section.
    #expect(data.count == 4 * 14 + 4 * 8 + 4 + 4 * 3 + 8 + 4 * 2)
    #expect(data.prefix(4) == Data([2, 0, 0, 0]))
    #expect(data.dropFirst(4).prefix(52).allSatisfy { $0 == 0 })
    #expect(try DeviceWire.identifiers(DeviceIdentifiers()) == Data([1, 0, 0, 0] + [UInt8](repeating: 0, count: 12)))
}

@Test
func oversizedTextIsRejected() throws {
    var facts = appleFixture
    facts.kernelVersion = String(repeating: "x", count: DeviceWire.maxText + 1)
    #expect(throws: PluginError.tooLarge) { try DeviceWire.snapshot(facts) }
    facts.kernelVersion = String(repeating: "é", count: DeviceWire.maxText / 2)
    #expect(try DeviceWire.snapshot(facts).count > DeviceWire.maxText)
    var identifiers = identifiersFixture
    identifiers.machineId = String(repeating: "x", count: DeviceWire.maxIdentifier + 1)
    #expect(throws: PluginError.tooLarge) { try DeviceWire.identifiers(identifiers) }
}

@Test
func genericIosDeviceNamesAreNotUserAssigned() {
    #expect(DeviceIdentifiers.userAssignedName("iPhone", model: "iPhone") == nil)
    #expect(DeviceIdentifiers.userAssignedName("Ada's iPhone", model: "iPhone") == "Ada's iPhone")
    #expect(DeviceIdentifiers.userAssignedName("", model: "iPhone") == nil)
    let long = String(repeating: "n", count: DeviceWire.maxIdentifier + 1)
    #expect(DeviceIdentifiers.userAssignedName(long, model: "iPhone") == nil)
}

@Test
@MainActor
func pluginDispatchesFactsAndValidatesCalls() async throws {
    var reads = 0
    let plugin = DevicePlugin(
        facts: { appleFixture },
        identifiers: {
            reads += 1
            return identifiersFixture
        }
    )
    #expect(plugin.name == "device")
    #expect(plugin.methods == [0, 1])
    #expect(hex(try await plugin.invoke(method: 0, payload: payload())) == appleGolden)
    #expect(reads == 0, "a snapshot never reads identifiers")
    #expect(hex(try await plugin.invoke(method: 1, payload: payload())) == identifiersGolden)
    #expect(reads == 1)
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 2, payload: payload()) }
    await #expect(throws: PluginError.invalidArgument) { try await plugin.invoke(method: 0, payload: payload([1])) }
    await #expect(throws: PluginError.invalidArgument) { try await plugin.invoke(method: 1, payload: payload([1])) }
}

@Test
@MainActor
func realFactsDescribeThisMac() async throws {
    let facts = DeviceFacts.current()
    #expect(facts.manufacturer == "Apple")
    #expect(facts.totalMemory == ProcessInfo.processInfo.physicalMemory)
    #expect(facts.cpuCount > 0)
    #expect(facts.totalDisk > 0)
    #expect(facts.freeDisk.map { $0 <= facts.totalDisk } == true)
    #expect(facts.availableMemory.map { $0 <= facts.totalMemory } == true)
    #expect(facts.kernelVersion?.isEmpty == false)
    #expect(facts.osBuild?.isEmpty == false)
    #if os(macOS)
    #expect(facts.osName == "macOS")
    #expect(facts.kind == 4)
    #expect(facts.simulator == nil)
    #expect(facts.modelIdentifier?.contains("Mac") == true)
    #expect(facts.iosAppOnMac == false)
    #expect(facts.macCatalyst == false)
    let identifiers = DeviceIdentifiers.current()
    #expect(identifiers.vendorId == nil)
    #expect(identifiers.machineId?.count == 36)
    #endif
    let data = try await DevicePlugin().invoke(method: 0, payload: payload())
    #expect(data.prefix(4) == Data([2, 0, 0, 0]))
}

@Test
func sysctlReadsRejectUnknownNames() {
    #expect(DeviceFacts.sysctlString("leptoskit.missing") == nil)
    #expect(DeviceFacts.sysctlNumber("leptoskit.missing") == nil)
    #expect((DeviceFacts.sysctlNumber("hw.ncpu") ?? 0) > 0)
    #expect(DeviceFacts.volume("/leptoskit/missing/volume") == nil)
}
