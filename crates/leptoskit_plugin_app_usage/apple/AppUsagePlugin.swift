import Foundation
import LeptosKitBridge

/// Wire bounds the request decoders enforce.
private enum Wire {
    /// A query is a range plus one u32 cap: four little-endian fields.
    static let eventBytes = 20
    static let rangeBytes = 16
    static let maxEvents: UInt32 = 1024

    /// Lock and unlock events buffered while the app is alive.
    static let bufferCapacity = 64

    /// Usage access states, in wire order.
    static let statusUnknown: UInt32 = 0
    static let statusGranted: UInt32 = 1

    /// Event kinds the wire carries; iOS reports only the last two.
    static let locked: UInt32 = 8
    static let unlocked: UInt32 = 9
}

/// iOS has no per-app usage for third-party apps: sessions and summaries are
/// unavailable, and events are the lock and unlock transitions buffered while
/// this app is alive.
@MainActor
public final class AppUsagePlugin: NativePlugin {
    public let name = "app-usage"
    public let methods: [UInt32] = [0, 1, 2, 3, 4]
    private var closed = false
    private let buffer = EventBuffer(capacity: Wire.bufferCapacity)
    private var observers: [NSObjectProtocol] = []
    private var waiter: CheckedContinuation<Data, any Error>?
    private var waiterID: UUID?

    public init() {
        observe()
    }

    deinit {
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
    }

    /// Answers one call. The status call never opens a screen; the events
    /// call waits for the next buffered transition when the buffer is empty.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            // Nothing gates the lock and unlock notifications on Apple
            // platforms; there is no usage-access screen to check.
            return field(Wire.statusGranted)
        case 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            // iOS has no Usage Access screen to open.
            throw PluginError.unavailable
        case 2:
            return try await eventsCall(payload)
        case 3, 4:
            // Per-app usage is withheld from third-party apps.
            throw PluginError.unavailable
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true

        if let waiter {
            self.waiter = nil
            waiterID = nil
            waiter.resume(throwing: PluginError.cancelled)
        }
    }

    /// Returns the buffered lock and unlock events, or waits for the next
    /// one. The range is validated for wire consistency and then ignored:
    /// iOS has no history to query.
    private func eventsCall(_ payload: NativeBuffer) async throws -> Data {
        let cap = try payload.withUnsafeBytes { try decodeEventQuery($0) }

        if let batch = buffer.drain(upTo: cap) { return batch }

        guard waiter == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                waiter = continuation
                waiterID = identifier
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.cancelWaiter(identifier) }
        }
    }

    /// Abandons the waiter a cancelled caller left behind, if it still owns
    /// the slot.
    private func cancelWaiter(_ identifier: UUID) {
        guard waiterID == identifier else { return }

        waiter = nil
        waiterID = nil
    }

    /// Buffers both protected-data transitions for as long as the app lives.
    private func observe() {
        observers = [
            (Notification.Name.protectedDataWillBecomeUnavailable, Wire.locked),
            (Notification.Name.protectedDataDidBecomeAvailable, Wire.unlocked),
        ].map { (name, kind) in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) {
                [weak self] _ in
                MainActor.assumeIsolated {
                    self?.buffer.append(kind: kind, timestampMs: Self.now())
                    self?.answerWaiter()
                }
            }
        }
    }

    /// Hands a waiting events caller whatever the buffer holds now.
    private func answerWaiter() {
        guard let waiter else { return }

        guard let batch = buffer.drain() else { return }

        self.waiter = nil
        waiterID = nil
        waiter.resume(returning: batch)
    }

    /// The current Unix time in milliseconds.
    private static func now() -> Int64 {
        Int64((Date().timeIntervalSince1970 * 1000).rounded())
    }

    /// One little-endian u32 reply field.
    private func field(_ value: UInt32) -> Data {
        var little = value.littleEndian

        return withUnsafeBytes(of: &little) { Data($0) }
    }
}

/// Validates a query the way the Android handler does, returning its cap;
/// Apple platforms have no history, so only the wire bounds matter.
private func decodeEventQuery(_ bytes: UnsafeRawBufferPointer) throws -> Int {
    guard bytes.count == Wire.eventBytes else { throw PluginError.invalidArgument }
    let since = Int64(littleEndian: bytes.loadUnaligned(as: Int64.self))
    let until = Int64(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: Int64.self))
    let max = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 16, as: UInt32.self))

    guard since >= 0, until >= since, (1...Wire.maxEvents).contains(max) else {
        throw PluginError.invalidArgument
    }

    return Int(max)
}

/// Bounded lock and unlock buffer; the oldest event is dropped first.
struct EventBuffer {
    private var events: [(kind: UInt32, timestampMs: Int64)] = []
    private let capacity: Int

    init(capacity: Int) {
        self.capacity = capacity
    }

    /// Adds one transition, dropping the oldest when the buffer is full.
    mutating func append(kind: UInt32, timestampMs: Int64) {
        events.append((kind, timestampMs))

        if events.count > capacity { events.removeFirst() }
    }

    /// The buffered events oldest first, up to `cap`, when any exist.
    mutating func drain(upTo cap: Int) -> Data? {
        guard !events.isEmpty else { return nil }

        return batch(Array(events.prefix(cap)))
    }

    /// Every buffered event, oldest first, when any exist.
    mutating func drain() -> Data? {
        guard !events.isEmpty else { return nil }

        return batch(events)
    }

    /// Frames one batch and clears what it returned.
    private mutating func batch(_ entries: [(kind: UInt32, timestampMs: Int64)]) -> Data {
        let returned = Array(entries)
        events.removeFirst(min(returned.count, events.count))

        return encodeEventBatch(returned)
    }
}

/// Frames an event batch: one little-endian u32 count, then the events.
/// Apple events name no app, so each carries the absent-package length.
func encodeEventBatch(_ events: [(kind: UInt32, timestampMs: Int64)]) -> Data {
    var result = Data(capacity: 4 + events.count * 16)

    for var field in [UInt32(events.count)].map(\.littleEndian) {
        withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
    }

    for event in events {
        for var field in [event.kind].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }

        for var field in [event.timestampMs].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }

        for var field in [UInt32.max].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }
    }

    return result
}
