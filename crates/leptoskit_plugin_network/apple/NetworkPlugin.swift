import Foundation
import LeptosKitBridge
import Network

/// The wire snapshot: nine little-endian u32 fields.
enum Wire {
    static let snapshotBytes = 36
    static let fieldBytes = 4
    static let fieldCount = 9

    /// Field offsets inside the reading.
    static let connectionField = 0
    static let firstFlagField = 1
    static let lastFlagField = 4
    static let effectiveTypeField = 7
    static let transportsField = 8

    /// The highest code each field may carry.
    static let lastConnection: UInt32 = 7
    static let lastFlag: UInt32 = 2
    static let lastEffectiveType: UInt32 = 4

    /// Tri-state encodings shared by every boolean field.
    static let flagUnknown: UInt32 = 0
    static let flagFalse: UInt32 = 1
    static let flagTrue: UInt32 = 2

    /// Numeric estimates use the reserved maximum for unknown.
    static let unknown: UInt32 = .max

    /// Method identifiers.
    static let snapshot: UInt32 = 0
    static let waitForChange: UInt32 = 1

    /// Connection codes the wire snapshot defines.
    static let none: UInt32 = 1
    static let wifi: UInt32 = 2
    static let cellular: UInt32 = 3
    static let ethernet: UInt32 = 4
    static let other: UInt32 = 7

    /// Transport bits, matching `Transport` in the Rust crate.
    static let wifiBit: UInt32 = 1 << 0
    static let cellularBit: UInt32 = 1 << 1
    static let ethernetBit: UInt32 = 1 << 2
    static let satelliteBit: UInt32 = 1 << 5
    static let otherBit: UInt32 = 1 << 6
    /// Every bit the wire defines.
    static let transportMask: UInt32 = (1 << 7) - 1
}

/// The parts of an `NWPath` the snapshot encodes, so tests can build one.
struct PathReading: Equatable {
    var satisfied: Bool
    var wifi = false
    var cellular = false
    var wiredEthernet = false
    var other = false
    var expensive = false
    var constrained = false
    /// Apple's ultra-constrained mode, which satellite data uses.
    var ultraConstrained = false
}

extension PathReading {
    init(_ path: NWPath) {
        self.init(
            satisfied: path.status == .satisfied,
            wifi: path.usesInterfaceType(.wifi),
            cellular: path.usesInterfaceType(.cellular),
            wiredEthernet: path.usesInterfaceType(.wiredEthernet),
            other: path.usesInterfaceType(.other),
            expensive: path.isExpensive,
            constrained: path.isConstrained
        )

        if #available(macOS 26.0, iOS 26.0, *) { ultraConstrained = path.isUltraConstrained }
    }
}

/// Reads NWPath state and waits for the OS to report a change.
@MainActor
public final class NetworkPlugin: NativePlugin {
    public let name = "network"
    public let methods: [UInt32] = [Wire.snapshot, Wire.waitForChange]

    /// One in-flight request with its own monitor. Snapshots never wait on
    /// each other; only one change request may be outstanding, and a second
    /// one receives `busy`.
    private struct Pending {
        let identifier: UUID
        let monitor: NWPathMonitor
        let continuation: CheckedContinuation<Data, Error>
        let previous: Data?
    }

    private var pending: [UUID: Pending] = [:]
    private var changeRequest: UUID?
    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        let previous = try previousReading(method: method, payload: payload)

        guard method != Wire.waitForChange || changeRequest == nil else { throw PluginError.busy }

        return try await awaitChange(previous: previous, change: method == Wire.waitForChange)
    }

    /// Reads the reading each method compares against; a snapshot compares against nothing.
    private func previousReading(method: UInt32, payload: NativeBuffer) throws -> Data? {
        switch method {
        case Wire.snapshot:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return nil
        case Wire.waitForChange:
            return try Self.previous(from: payload)
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true

        for identifier in pending.keys { finish(identifier: identifier, result: .failure(PluginError.cancelled)) }
    }

    /// Reads the caller's last reading, refusing fields the wire cannot carry.
    nonisolated private static func previous(from payload: NativeBuffer) throws -> Data {
        try payload.withUnsafeBytes { try validated(Data($0)) }
    }

    /// Returns `bytes` when they form a reading the wire can carry.
    nonisolated static func validated(_ bytes: Data) throws -> Data {
        guard bytes.count == Wire.snapshotBytes else { throw PluginError.invalidArgument }

        let fields = bytes.withUnsafeBytes { raw in
            (0..<Wire.fieldCount).map {
                UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: $0 * Wire.fieldBytes, as: UInt32.self))
            }
        }

        guard fields[Wire.connectionField] <= Wire.lastConnection,
              fields[Wire.firstFlagField...Wire.lastFlagField].allSatisfy({ $0 <= Wire.lastFlag }),
              fields[Wire.effectiveTypeField] <= Wire.lastEffectiveType,
              fields[Wire.transportsField] & ~Wire.transportMask == 0
        else {
            throw PluginError.invalidArgument
        }

        return bytes
    }

    /// Starts a monitor and answers the first reading that differs from `previous`.
    private func awaitChange(previous: Data?, change: Bool) async throws -> Data {
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try Task.checkCancellation()

            return try await withCheckedThrowingContinuation { continuation in
                let monitor = NWPathMonitor()
                pending[identifier] = Pending(
                    identifier: identifier, monitor: monitor, continuation: continuation, previous: previous
                )

                if change { changeRequest = identifier }
                monitor.pathUpdateHandler = { [weak self] path in
                    let data = Self.encode(PathReading(path))
                    Task { @MainActor [weak self] in
                        self?.updated(identifier: identifier, data: data)
                    }
                }
                monitor.start(queue: DispatchQueue(label: "dev.leptoskit.network"))
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.finish(identifier: identifier, result: .failure(PluginError.cancelled))
            }
        }
    }

    private func updated(identifier: UUID, data: Data) {
        guard let request = pending[identifier], request.previous != data else { return }
        finish(identifier: identifier, result: .success(data))
    }

    /// Answers the waiting caller once, then releases its monitor.
    private func finish(identifier: UUID, result: Result<Data, Error>) {
        guard let request = pending.removeValue(forKey: identifier) else { return }

        if changeRequest == identifier { changeRequest = nil }

        request.monitor.pathUpdateHandler = nil
        request.monitor.cancel()
        request.continuation.resume(with: result)
    }

    /// Encodes a path; no probe or request is made, so reachability stays a hint.
    nonisolated static func encode(_ path: PathReading) -> Data {
        let primary = connection(path)
        // A satisfied route does not prove internet access or server reachability.
        let fields: [UInt32] = [
            primary,
            path.satisfied ? Wire.flagTrue : Wire.flagFalse,
            Wire.flagUnknown,
            path.expensive ? Wire.flagTrue : Wire.flagFalse,
            path.constrained ? Wire.flagTrue : Wire.flagFalse,
            Wire.unknown,
            Wire.unknown,
            Wire.flagUnknown,
            transports(path, primary: primary)
        ]

        return fields.reduce(into: Data(capacity: Wire.snapshotBytes)) { result, field in
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }
    }

    /// Maps the path's interface to the wire connection code.
    nonisolated private static func connection(_ path: PathReading) -> UInt32 {
        guard path.satisfied else { return Wire.none }

        if path.wifi { return Wire.wifi }

        if path.cellular { return Wire.cellular }

        if path.wiredEthernet { return Wire.ethernet }

        return Wire.other
    }

    /// Every interface type the path uses. `NWPath` names no VPN or Bluetooth
    /// type, so a tunnel reports as other; the primary's bit is always present.
    nonisolated private static func transports(_ path: PathReading, primary: UInt32) -> UInt32 {
        guard path.satisfied else { return 0 }

        let present: [(Bool, UInt32)] = [
            (path.wifi, Wire.wifiBit),
            (path.cellular, Wire.cellularBit),
            (path.wiredEthernet, Wire.ethernetBit),
            (path.other || primary == Wire.other, Wire.otherBit),
            (path.ultraConstrained, Wire.satelliteBit)
        ]

        return present.reduce(0) { bits, entry in entry.0 ? bits | entry.1 : bits }
    }
}
