import Foundation
import LeptosKitBridge

/// Device information without persistent identifiers (method 0) and, only on
/// request, the identifiers themselves (method 1). The byte layout is
/// documented in the Rust crate and mirrored by `DeviceWire`.
@MainActor
public final class DevicePlugin: NativePlugin {
    public let name = "device"
    public let methods: [UInt32] = [0, 1]

    private let facts: @MainActor () -> DeviceFacts
    private let identifiers: @MainActor () -> DeviceIdentifiers

    public convenience init() {
        self.init(facts: DeviceFacts.current, identifiers: DeviceIdentifiers.current)
    }

    /// Tests supply fixed facts instead of reading the system.
    init(
        facts: @escaping @MainActor () -> DeviceFacts,
        identifiers: @escaping @MainActor () -> DeviceIdentifiers
    ) {
        self.facts = facts
        self.identifiers = identifiers
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == 0 || method == 1 else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return method == 0 ? try DeviceWire.snapshot(facts()) : try DeviceWire.identifiers(identifiers())
    }
}

/// Everything method 0 reports. `nil` means unknown.
struct DeviceFacts: Equatable {
    /// Form factor code: unknown, phone, tablet, TV, desktop, watch, car, headset.
    var kind: UInt32 = 0
    var simulator: Bool?
    var totalMemory: UInt64 = 0
    var availableMemory: UInt64?
    var totalDisk: UInt64 = 0
    var freeDisk: UInt64?
    var cpuCount: UInt32 = 0
    var manufacturer: String?
    var brand: String?
    var model: String?
    var modelIdentifier: String?
    var osName: String?
    var osVersion: String?
    var architecture: String?
    var kernelVersion: String?
    var iosAppOnMac: Bool?
    var macCatalyst: Bool?
    var iosAppOnVision: Bool?
    var cpuFrequency: UInt64 = 0
    var localizedModel: String?
    var osBuild: String?
}

/// Everything method 1 reports. `nil` where the platform withholds it.
struct DeviceIdentifiers: Equatable {
    var deviceName: String?
    var vendorId: String?
    var machineId: String?
}

/// The little-endian wire shared with Rust and Kotlin.
enum DeviceWire {
    static let snapshotVersion: UInt32 = 2
    static let identifiersVersion: UInt32 = 1
    static let maxText = 4096
    static let maxIdentifier = 256
    /// The Apple platform section's tag.
    static let appleTag: UInt32 = 1

    static func snapshot(_ facts: DeviceFacts) throws -> Data {
        var output = WireWriter()
        output.u32(snapshotVersion)
        output.u32(facts.kind)
        output.tri(facts.simulator)
        output.u64(facts.totalMemory)
        output.optional(facts.availableMemory)
        output.u64(facts.totalDisk)
        output.optional(facts.freeDisk)
        output.u32(facts.cpuCount)

        for text in [
            facts.manufacturer, facts.brand, facts.model, facts.modelIdentifier,
            facts.osName, facts.osVersion, facts.architecture, facts.kernelVersion
        ] {
            try output.text(text, max: maxText)
        }

        output.u32(appleTag)
        output.tri(facts.iosAppOnMac)
        output.tri(facts.macCatalyst)
        output.tri(facts.iosAppOnVision)
        output.u64(facts.cpuFrequency)
        try output.text(facts.localizedModel, max: maxText)
        try output.text(facts.osBuild, max: maxText)

        return output.data
    }

    static func identifiers(_ identifiers: DeviceIdentifiers) throws -> Data {
        var output = WireWriter()
        output.u32(identifiersVersion)

        for text in [identifiers.deviceName, identifiers.vendorId, identifiers.machineId] {
            try output.text(text, max: maxIdentifier)
        }

        return output.data
    }
}

/// Appends little-endian fields.
struct WireWriter {
    private(set) var data = Data(capacity: 256)

    mutating func u32(_ value: UInt32) {
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }

    mutating func u64(_ value: UInt64) {
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }

    /// Unknown, no, yes as 0, 1, 2.
    mutating func tri(_ value: Bool?) {
        u32(value.map { $0 ? 2 : 1 } ?? 0)
    }

    /// A presence word, then the value or zero.
    mutating func optional(_ value: UInt64?) {
        u32(value == nil ? 0 : 1)
        u64(value ?? 0)
    }

    /// A length-prefixed UTF-8 string; `nil` and empty are both unknown.
    mutating func text(_ value: String?, max: Int) throws {
        let bytes = Data((value ?? "").utf8)

        guard bytes.count <= max else { throw PluginError.tooLarge }
        u32(UInt32(bytes.count))
        data.append(bytes)
    }
}
