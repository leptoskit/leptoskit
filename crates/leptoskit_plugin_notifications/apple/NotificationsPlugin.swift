import Foundation
import LeptosKitBridge
import UserNotifications
#if canImport(UIKit)
import UIKit
#endif

@MainActor
protocol NotificationService {
    func authorization() async -> UNAuthorizationStatus
    func requestAuthorization() async throws
    func add(_ request: UNNotificationRequest) async throws
    func identifiers() async -> [String]
    func remove(_ identifiers: [String])
}

/// The remote-registration boundary the OS owns. Production calls
/// `UIApplication.registerForRemoteNotifications`; tests substitute a fake.
@MainActor
protocol RemoteRegistrar {
    func register() async
}

@MainActor
private struct SystemRemoteRegistrar: RemoteRegistrar {
    func register() async {
        #if canImport(UIKit) && os(iOS)
        UIApplication.shared.registerForRemoteNotifications()
        #endif
    }
}

/// The latest APNs registration answer the app delegate forwarded. Static on
/// purpose: answers can arrive before the plugin is constructed and its guest
/// exists, a closed plugin must leave the OS's answer readable, and nothing
/// here retains a plugin instance. The token is held only in memory: the
/// process asks the OS again on the next launch rather than treating a stored
/// copy as authoritative.
@MainActor
public enum RemoteRegistration {
    /// Bounds shared with the Rust contract: the token is variable length and
    /// never assumed to be 32 bytes.
    static let minTokenCharacters = 32
    static let maxTokenCharacters = 200
    static let maxFailureBytes = 256

    /// The registration answer's shape.
    public enum State: Equatable, Sendable {
        case none
        /// The OS issued a device token, as lower-case hex.
        case token(String)
        /// The OS refused registration; bounded, sanitized reason.
        case failed(String)
    }

    public struct Snapshot: Equatable, Sendable {
        /// Monotonic answer counter; `0` means no answer has arrived since
        /// the process started.
        public var revision: UInt64
        public var state: State
    }

    private static var snapshot = Snapshot(revision: 0, state: .none)

    static func current() -> Snapshot { snapshot }

    /// The app delegate's `didRegisterForRemoteNotificationsWithDeviceToken`.
    /// Apple's bytes become exact lower-case hex. A token outside the
    /// contract's bounds is recorded as a failure instead of a token, and its
    /// bytes never reach a log.
    public static func noteDeviceToken(_ token: Data) {
        let hex = token.map { String(format: "%02x", $0) }.joined()

        guard hex.count >= minTokenCharacters, hex.count <= maxTokenCharacters else {
            note(failure: "the device token has an unexpected length")
            return
        }

        snapshot = Snapshot(revision: snapshot.revision + 1, state: .token(hex))
    }

    /// The app delegate's `didFailToRegisterForRemoteNotificationsWithError`.
    /// The reason is sanitized to bounded, control-free text; the token never
    /// travels through an error.
    public static func noteRegistrationFailure(_ error: any Error) {
        note(failure: sanitize(String(describing: error)))
    }

    static func note(failure: String) {
        snapshot = Snapshot(revision: snapshot.revision + 1, state: .failed(sanitize(failure)))
    }

    /// Resets the broker. Process-scoped state, so only tests call this.
    static func reset(_ snapshot: Snapshot = Snapshot(revision: 0, state: .none)) {
        self.snapshot = snapshot
    }

    private static func sanitize(_ text: String) -> String {
        var bytes = Array(text.utf8.filter { !($0 < 32 || $0 == 127) })

        if bytes.count > maxFailureBytes { bytes = Array(bytes[..<maxFailureBytes]) }

        return String(bytes: bytes, encoding: .utf8) ?? ""
    }
}

@MainActor
private final class SystemNotificationService: NotificationService {
    private let center = UNUserNotificationCenter.current()
    func authorization() async -> UNAuthorizationStatus { await center.notificationSettings().authorizationStatus }
    // Notifications are silent on Apple, so asking for sound would request
    // a permission the plugin never uses.
    func requestAuthorization() async throws { _ = try await center.requestAuthorization(options: [.alert]) }
    func add(_ request: UNNotificationRequest) async throws { try await center.add(request) }
    func identifiers() async -> [String] {
        let pending = await center.pendingNotificationRequests().map(\.identifier)
        let delivered = await center.deliveredNotifications().map { $0.request.identifier }

        return pending + delivered
    }
    func remove(_ identifiers: [String]) {
        center.removePendingNotificationRequests(withIdentifiers: identifiers)
        center.removeDeliveredNotifications(withIdentifiers: identifiers)
    }
}

/// Immediate local notifications. Does not replace the app's notification center delegate.
@MainActor
public final class NotificationsPlugin: NativePlugin {
    public let name = "notifications"
    #if os(iOS)
    public let methods: [UInt32] = [0, 1, 2, 3, 6, 7, 8]
    #else
    public let methods: [UInt32] = [0, 1, 2, 3, 6]
    #endif
    /// How long `REMOTE_REGISTER` waits for the OS answer before replying
    /// with the unchanged snapshot, in nanoseconds. Apple answers
    /// registration quickly; the bound keeps a silent OS from hanging the
    /// call.
    static let remoteWaitLimitNanos: UInt64 = 10_000_000_000
    /// How often the bounded wait re-reads the broker, in nanoseconds. A
    /// plain nanoseconds sleep keeps the wait off the Duration/Clock path,
    /// whose runtime support varies across SDKs.
    static let remoteWaitSliceNanos: UInt64 = 50_000_000

    private let service: any NotificationService
    private let registrar: any RemoteRegistrar
    private let remoteWaitLimitNanos: UInt64
    private var closed = false
    private var requesting = false
    private struct Pending { let ticket: UUID; var cancelled = false }
    private var pending: [String: Pending] = [:]
    static let prefix = "leptoskit.notifications."

    public init() {
        service = SystemNotificationService()
        registrar = SystemRemoteRegistrar()
        remoteWaitLimitNanos = Self.remoteWaitLimitNanos
    }

    init(service: any NotificationService) {
        self.service = service
        registrar = SystemRemoteRegistrar()
        remoteWaitLimitNanos = Self.remoteWaitLimitNanos
    }

    init(
        service: any NotificationService,
        registrar: any RemoteRegistrar,
        remoteWaitLimitNanos: UInt64
    ) {
        self.service = service
        self.registrar = registrar
        self.remoteWaitLimitNanos = remoteWaitLimitNanos
    }

    /// Forwards the app delegate's device-token callback into the broker.
    /// Safe whether or not a plugin instance exists.
    public static func noteDeviceToken(_ token: Data) {
        RemoteRegistration.noteDeviceToken(token)
    }

    /// Forwards the app delegate's registration failure into the broker.
    /// Safe whether or not a plugin instance exists.
    public static func noteRegistrationFailure(_ error: any Error) {
        RemoteRegistration.noteRegistrationFailure(error)
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()
        switch method {
        case 0, 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try await authorization(request: method == 1)
        case 2:
            let content = try payload.withUnsafeBytes { try Content($0) }
            try await show(content)
        case 3:
            let id = try payload.withUnsafeBytes { bytes in
                var reader = try Reader(bytes, maximum: 68)
                let id = try reader.identifier()
                try reader.finish()
                return Self.prefix + id
            }
            pending[id]?.cancelled = true
            service.remove([id])
        case 6:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            for id in pending.keys { pending[id]?.cancelled = true }
            let identifiers = await service.identifiers().filter { $0.hasPrefix(Self.prefix) }

            guard !closed else { throw PluginError.cancelled }
            service.remove(identifiers)
        // Methods 7 and 8 are dispatched only where `methods` advertises
        // them: iOS hosts. The macOS switch keeps them for tests, which
        // inject the registration boundary.
        case 7:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try Self.encodeSnapshot(RemoteRegistration.current())
        case 8:
            return try await registerRemote(payload: payload)
        default:
            throw PluginError.unavailable
        }

        return Data()
    }

    /// Asks the OS to register for remote notifications on the main actor,
    /// then waits a bounded time for the answer the app delegate forwards.
    /// A silent OS answers the unchanged snapshot, so the caller can tell
    /// silence from a token or a failure by the revision. Only iOS hosts
    /// dispatch this: `methods` advertises 8 there alone.
    private func registerRemote(payload: NativeBuffer) async throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        let since = RemoteRegistration.current().revision
        await registrar.register()

        let snapshot: RemoteRegistration.Snapshot
        do {
            snapshot = try await Self.waitRemoteChange(from: since, limitNanos: remoteWaitLimitNanos)
        } catch {
            throw mapped(error)
        }
        try Task.checkCancellation()
        return try Self.encodeSnapshot(snapshot)
    }

    /// Polls the broker until its revision moves past `since` or the wait
    /// limit passes. Every waiter sees the same answer, the wait always
    /// terminates, and cancellation ends it without touching the broker.
    static func waitRemoteChange(
        from since: UInt64,
        limitNanos: UInt64
    ) async throws -> RemoteRegistration.Snapshot {
        var waited: UInt64 = 0

        while true {
            let snapshot = RemoteRegistration.current()

            if snapshot.revision != since { return snapshot }
            if waited >= limitNanos { return snapshot }

            try await Task.sleep(nanoseconds: Self.remoteWaitSliceNanos)
            waited += Self.remoteWaitSliceNanos
        }
    }

    /// Encodes a snapshot reply: revision `u64`, state `u32`, then a
    /// length-prefixed string (empty for no answer).
    static func encodeSnapshot(_ snapshot: RemoteRegistration.Snapshot) throws -> Data {
        var data = Data()
        func append(_ value: UInt32) {
            var little = value.littleEndian
            withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
        }
        func append(_ text: String) throws {
            let bytes = Array(text.utf8)
            guard bytes.count <= RemoteRegistration.maxFailureBytes else {
                throw PluginError.tooLarge
            }
            append(UInt32(bytes.count))
            data.append(contentsOf: bytes)
        }

        append(UInt32(truncatingIfNeeded: snapshot.revision >> 32))
        append(UInt32(truncatingIfNeeded: snapshot.revision))

        switch snapshot.state {
        case .none:
            append(0)
            try append("")
        case .token(let hex):
            append(1)
            try append(hex)
        case .failed(let reason):
            append(2)
            try append(reason)
        }

        return data
    }

    private func authorization(request: Bool) async throws -> Data {
        if request {
            guard !requesting else { throw PluginError.busy }
            requesting = true
            defer { requesting = false }

            do { try await service.requestAuthorization() } catch { throw mapped(error) }
        }

        let status = await service.authorization()

        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        return encode(status)
    }

    /// Delivered notifications remain visible. In-flight posts are cancelled and
    /// removed again after completion if an OS operation could not be aborted.
    public func close() {
        closed = true

        for id in pending.keys { pending[id]?.cancelled = true }
        service.remove(Array(pending.keys))
    }

    private func show(_ content: Content) async throws {
        let id = Self.prefix + content.id

        guard pending[id] == nil, pending.count < 64 else { throw PluginError.busy }
        let ticket = UUID()
        pending[id] = Pending(ticket: ticket)
        defer { pending.removeValue(forKey: id) }

        try await withTaskCancellationHandler {
            try await deliver(id, content)
        } onCancel: { [weak self] in
            Task { @MainActor in

                guard let self, self.pending[id]?.ticket == ticket else { return }
                self.pending[id]?.cancelled = true
                self.service.remove([id])
            }
        }
    }

    /// Posts one notification, once the system allows it.
    private func deliver(_ id: String, _ content: Content) async throws {
        let status = await service.authorization()
        try checkPending(id)

        guard (3...5).contains(authorizationValue(status)) else {
            throw PluginError.permissionDenied
        }

        let native = UNMutableNotificationContent()
        native.title = content.title
        native.body = content.body
        // Sound is deliberately absent. Android sound is an explicit channel setting.
        do {
            try await service.add(UNNotificationRequest(identifier: id, content: native, trigger: nil))
        } catch {
            service.remove([id])
            throw mapped(error)
        }

        do {
            try checkPending(id)
        } catch {
            service.remove([id])
            throw error
        }
    }

    private func checkPending(_ id: String) throws {
        guard !closed, pending[id]?.cancelled == false else { throw PluginError.cancelled }
        try Task.checkCancellation()
    }

    private func mapped(_ error: any Error) -> any Error {
        if error is CancellationError { return PluginError.cancelled }
        let native = error as NSError

        if native.domain == UNErrorDomain && native.code == UNError.Code.notificationsNotAllowed.rawValue {
            return PluginError.permissionDenied
        }

        return PluginError.native
    }

    private func encode(_ status: UNAuthorizationStatus) -> Data {
        Data([authorizationValue(status), 0, 0, 0])
    }

    private func authorizationValue(_ status: UNAuthorizationStatus) -> UInt8 {
        switch status {
        case .notDetermined: 1
        case .denied: 2
        case .authorized: 3
        case .provisional: 4

        #if os(iOS)
        case .ephemeral: 5
        #endif
        @unknown default: 0
        }
    }
}

private struct Content {
    let id: String
    let title: String
    let body: String
    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4496)
        id = try reader.identifier()
        title = try reader.text(maximum: 256, multiline: false, required: true)
        body = try reader.text(maximum: 4096, multiline: true, required: false)
        _ = try reader.identifier() // Validated Android channel; unsupported channel mutation is separate.
        try reader.finish()
    }
}

private struct Reader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0
    init(_ bytes: UnsafeRawBufferPointer, maximum: Int) throws {
        guard bytes.count <= maximum else { throw PluginError.invalidArgument }
        self.bytes = bytes
    }
    mutating func identifier() throws -> String {
        let value = try text(maximum: 64, multiline: false, required: true)
        guard value.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) ||
            (97...122).contains($0) || [45, 46, 95].contains($0) }) else { throw PluginError.invalidArgument }

        return value
    }

    mutating func text(maximum: Int, multiline: Bool, required: Bool) throws -> String {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let length = Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
        offset += 4

        guard length <= maximum, length <= bytes.count - offset,
              let value = String(bytes: bytes[offset..<(offset + length)], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        offset += length

        guard !required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,

              !value.unicodeScalars.contains(where: { ($0.value <= 31 || (127...159).contains($0.value)) &&
                  !(multiline && ($0.value == 10 || $0.value == 9)) }) else { throw PluginError.invalidArgument }

        return value
    }

    func finish() throws { guard offset == bytes.count else { throw PluginError.invalidArgument } }
}
