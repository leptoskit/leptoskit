import Foundation
import LeptosKitBridge

/// The wire snapshot: two little-endian u32 fields.
private enum Wire {
    static let snapshotBytes = 8
    static let fieldBytes = 4

    /// Shared state codes, in Rust's `State` order.
    static let unknown: UInt32 = 0
    static let nominal: UInt32 = 1
    static let fair: UInt32 = 2
    static let serious: UInt32 = 3
    static let critical: UInt32 = 4

    /// Apple has no Android throttling status.
    static let noAndroidStatus: UInt32 = .max

    /// Method identifiers.
    static let snapshot: UInt32 = 0
    static let waitForChange: UInt32 = 1
}

/// Thermal state from `ProcessInfo`, with one cancellable change waiter rather than an event queue.
@MainActor
public final class ThermalPlugin: NativePlugin {
    public let name = "thermal"
    public let methods: [UInt32] = [Wire.snapshot, Wire.waitForChange]
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    private var pendingID: UUID?
    private var previous = Data()
    private var observer: NSObjectProtocol?

    public init() {}

    /// Runs one snapshot request, or waits for the next different reading.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case Wire.snapshot:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return snapshot()
        case Wire.waitForChange:
            return try await changeCall(payload)
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
    }

    /**
     Answers the first reading that differs from the caller's last one. Nothing
     suspends between reserving the waiter and installing the observer, so a
     change cannot slip into that gap.
     */
    private func changeCall(_ payload: NativeBuffer) async throws -> Data {
        try payload.withUnsafeBytes { try validateSnapshot($0) }

        guard pending == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        let identifier = UUID()
        let expected = payload.withUnsafeBytes { Data($0) }

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                reserve(continuation, identifier: identifier, expected: expected)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in
                self?.cancel(identifier: identifier)
            }
        }
    }

    /// Reserves the waiter, installs the observer, then samples once.
    private func reserve(_ continuation: CheckedContinuation<Data, any Error>, identifier: UUID, expected: Data) {
        pending = continuation
        pendingID = identifier
        previous = expected
        observer = NotificationCenter.default.addObserver(
            forName: ProcessInfo.thermalStateDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.changed() }
        }
        changed()
    }

    /// Finishes only this call; a newer waiter may already own the slot.
    private func cancel(identifier: UUID) {
        guard pendingID == identifier else { return }

        finish(.failure(CancellationError()))
    }

    /// Resumes the waiter once, and only when the reading actually differs.
    private func changed() {
        guard pending != nil else { return }

        let current = snapshot()

        if current != previous { finish(.success(current)) }
    }

    /// Answers the waiting caller once, then removes the observer it installed.
    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil

        if let observer { NotificationCenter.default.removeObserver(observer) }
        observer = nil
        previous.removeAll(keepingCapacity: false)
        continuation.resume(with: result)
    }

    /// Encodes the process thermal state; Apple never reports an Android status.
    private func snapshot() -> Data {
        let fields: [UInt32] = [stateCode(ProcessInfo.processInfo.thermalState), Wire.noAndroidStatus]

        return fields.reduce(into: Data(capacity: Wire.snapshotBytes)) { result, field in
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }
    }
}

/// Maps `ProcessInfo.ThermalState` to the wire state; a future case stays unknown.
func stateCode(_ state: ProcessInfo.ThermalState) -> UInt32 {
    switch state {
    case .nominal: Wire.nominal
    case .fair: Wire.fair
    case .serious: Wire.serious
    case .critical: Wire.critical
    @unknown default: Wire.unknown
    }
}

/// Echoes the caller's snapshot back, refusing fields an Apple host cannot have produced.
private func validateSnapshot(_ bytes: UnsafeRawBufferPointer) throws {
    guard bytes.count == Wire.snapshotBytes else { throw PluginError.invalidArgument }
    let state = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
    let status = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: Wire.fieldBytes, as: UInt32.self))

    guard state <= Wire.critical, status == Wire.noAndroidStatus else { throw PluginError.invalidArgument }
}
