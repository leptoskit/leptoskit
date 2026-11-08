import Foundation
import LeptosKitBridge
#if os(iOS)
import BackgroundTasks

private let refreshIdentifier = "dev.leptoskit.background.refresh"
private let processingIdentifier = "dev.leptoskit.background.processing"
#endif

/// BGTaskScheduler background tasks that wake the app's `AppService`. iOS gives
/// no guarantee of continuous execution: the system schedules the tasks at its
/// own discretion, so an iOS app cannot promise a fixed wake interval.
@MainActor
public final class BackgroundPlugin: NativePlugin {
    public let name = "background"
    public let methods: [UInt32] = [0, 1, 2, 3]

    private nonisolated static let state = BackgroundState()
    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        switch method {
        case 0:
            let options = try payload.withUnsafeBytes { try Options($0) }
            try start(options)
        case 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            stop()
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return status()
        case 3:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return batteryReply()
        default:
            throw PluginError.unavailable
        }

        return Data()
    }

    public func close() {
        closed = true
        #if os(iOS)
        Self.cancelTasks()
        #endif
    }

    private func start(_ options: Options) throws {
        #if os(iOS)
        Self.registerTasks()
        Self.state.setScheduled(true)
        #else
        throw PluginError.unavailable
        #endif
    }

    private func stop() {
        #if os(iOS)
        Self.cancelTasks()
        Self.state.setScheduled(false)
        #endif
    }

    private func status() -> Data {
        Data()
            + Self.u32(Self.state.isScheduled() ? 1 : 0)
            + Self.u32(Self.state.lastWake())
            + Self.u32(0) // Battery optimisation is an Android concept.
    }

    private func batteryReply() -> Data {
        Data() + Self.u32(0)
    }

    #if os(iOS)
    private nonisolated static func registerTasks() {
        BGTaskScheduler.shared.register(
            forTaskWithIdentifier: refreshIdentifier, using: nil
        ) { task in
            NativeServiceWakeBroker.wakeBackground()
            state.recordWake()
            scheduleRefresh()
            task.setTaskCompleted(success: true)
        }

        BGTaskScheduler.shared.register(
            forTaskWithIdentifier: processingIdentifier, using: nil
        ) { task in
            NativeServiceWakeBroker.wakeBackground()
            state.recordWake()
            task.setTaskCompleted(success: true)
        }

        scheduleRefresh()
    }

    private nonisolated static func scheduleRefresh() {
        let request = BGAppRefreshTaskRequest(identifier: refreshIdentifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    private nonisolated static func cancelTasks() {
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: refreshIdentifier)
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: processingIdentifier)
    }
    #endif

    private static func u32(_ value: UInt32) -> Data {
        var little = value.littleEndian

        return withUnsafeBytes(of: &little) { Data($0) }
    }
}

/// Thread-safe process-wide state, updated from background-task handlers.
private final class BackgroundState: @unchecked Sendable {
    private let lock = NSLock()
    private var scheduled = false
    private var lastWakeSecs: UInt32 = 0

    func setScheduled(_ value: Bool) {
        lock.lock()
        scheduled = value
        lock.unlock()
    }

    func isScheduled() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return scheduled
    }

    func recordWake() {
        lock.lock()
        lastWakeSecs = UInt32(Date().timeIntervalSince1970)
        lock.unlock()
    }

    func lastWake() -> UInt32 {
        lock.lock()
        defer { lock.unlock() }
        return lastWakeSecs
    }
}

/// The start request: two strings, a service-type mask, and an interval.
private struct Options {
    let title: String
    let text: String
    let types: UInt32
    let interval: UInt32

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 128 + 1024 + 16)
        title = try reader.text(maximum: 128, required: true)
        text = try reader.text(maximum: 1024, required: false)
        types = try reader.number()
        interval = try reader.number()
        try reader.finish()

        guard (1...86_400).contains(interval) else { throw PluginError.invalidArgument }
    }
}

/// Strict little-endian reader matching the Rust codec.
private struct Reader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0

    init(_ bytes: UnsafeRawBufferPointer, maximum: Int) throws {
        guard bytes.count <= maximum else { throw PluginError.invalidArgument }
        self.bytes = bytes
    }

    mutating func number() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        offset += 4

        return value
    }

    mutating func text(maximum: Int, required: Bool) throws -> String {
        let length = Int(try number())

        guard length <= maximum, length <= bytes.count - offset,
            let value = String(bytes: bytes[offset..<(offset + length)], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        offset += length

        guard !required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
            !value.unicodeScalars.contains(where: { $0.value <= 31 || (127...159).contains($0.value) }) else {
            throw PluginError.invalidArgument
        }

        return value
    }

    func finish() throws { guard offset == bytes.count else { throw PluginError.invalidArgument } }
}
