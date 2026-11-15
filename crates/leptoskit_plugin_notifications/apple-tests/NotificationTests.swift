import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitNotifications
import Testing
import UserNotifications

@MainActor
private final class Service: NotificationService {
    var pause = false
    var continuation: CheckedContinuation<Void, any Error>?
    var posted: [String] = []
    var removed: [String] = []
    var existing: [String] = []
    func authorization() async -> UNAuthorizationStatus { .authorized }
    func requestAuthorization() async throws {}
    func add(_ request: UNNotificationRequest) async throws {
        posted.append(request.identifier)

        if pause { try await withCheckedThrowingContinuation { continuation = $0 } }
    }
    func identifiers() async -> [String] { existing }
    func remove(_ identifiers: [String]) { removed += identifiers }
    func complete() { continuation?.resume(); continuation = nil }
}

private func bytes(_ fields: [String]) -> [UInt8] {
    var result: [UInt8] = []

    for field in fields {
        let value = Array(field.utf8)
        var length = UInt32(value.count).littleEndian
        withUnsafeBytes(of: &length) { result += $0 }
        result += value
    }

    return result
}

private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let id = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(id == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }

    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@Test
@MainActor
func cancellationRemovesLatePostAndPreventsReplacementRace() async throws {
    let service = Service()
    service.pause = true
    let plugin = NotificationsPlugin(service: service)
    let content = try payload(bytes(["job", "Title", "Body", "general"]))
    let task = Task { try await plugin.invoke(method: 2, payload: content) }
    defer { service.complete(); task.cancel(); plugin.close() }

    for _ in 0..<100 where service.continuation == nil { await Task.yield() }
    try #require(service.continuation != nil)
    _ = try await plugin.invoke(method: 3, payload: payload(bytes(["job"])))
    await #expect(throws: PluginError.busy) { try await plugin.invoke(method: 2, payload: content) }
    service.complete()
    await #expect(throws: PluginError.cancelled) { try await task.value }
    #expect(service.removed.filter { $0 == "leptoskit.notifications.job" }.count >= 2)
    service.pause = false
    let removalCount = service.removed.count
    _ = try await plugin.invoke(method: 2, payload: content)
    plugin.close()
    #expect(service.removed.count == removalCount, "Closing must preserve completed notifications")
}

@Test
@MainActor
func closeCancelsPendingPostsAndCancellationIsNamespaceScoped() async throws {
    let service = Service()
    service.existing = ["other-provider", "leptoskit.notifications.old"]
    let plugin = NotificationsPlugin(service: service)
    _ = try await plugin.invoke(method: 6, payload: payload([]))
    #expect(service.removed == ["leptoskit.notifications.old"])
    service.pause = true
    let task = Task { try await plugin.invoke(method: 2, payload: payload(bytes(["new", "Title", "", "general"]))) }
    defer { service.complete(); task.cancel(); plugin.close() }

    for _ in 0..<100 where service.continuation == nil { await Task.yield() }
    try #require(service.continuation != nil)
    plugin.close()
    service.complete()
    await #expect(throws: PluginError.cancelled) { try await task.value }
    #expect(!service.removed.contains("other-provider"))
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: payload([])) }
}

@Test
@MainActor
func taskCancellationRetractsAnUnabortableLatePost() async throws {
    let service = Service()
    service.pause = true
    let plugin = NotificationsPlugin(service: service)
    let task = Task { try await plugin.invoke(method: 2, payload: payload(bytes(["job", "Title", "", "general"]))) }
    defer { service.complete(); task.cancel(); plugin.close() }

    for _ in 0..<100 where service.continuation == nil { await Task.yield() }
    try #require(service.continuation != nil)
    task.cancel()

    for _ in 0..<100 where service.removed.isEmpty { await Task.yield() }
    #expect(service.removed == ["leptoskit.notifications.job"])
    service.complete()
    await #expect(throws: PluginError.cancelled) { try await task.value }
    #expect(service.removed.count == 2)
}

@Test
@MainActor
func malformedMessagesNeverReachTheNotificationService() async throws {
    let service = Service()
    let plugin = NotificationsPlugin(service: service)
    let invalidMessages = [
        bytes(["id", "", "body", "general"]), bytes(["id", "Title", "bad\0body", "general"]), [255, 0, 0, 0]
    ]

    for invalid in invalidMessages {
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 2, payload: payload(invalid))
        }
    }

    #expect(service.posted.isEmpty)
    #expect(service.removed.isEmpty)
    plugin.close()
}

// MARK: Remote registration (APNs)

/// Decodes a snapshot reply the way the Rust contract does: revision `u64`,
/// state `u32`, then a length-prefixed string.
private func snapshot(_ data: Data) -> (revision: UInt64, state: UInt32, text: String) {
    var offset = 0
    func u32() -> UInt32 {
        let value = data.withUnsafeBytes {
            $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self).littleEndian
        }
        offset += 4
        return value
    }
    let revision = UInt64(u32()) << 32 | UInt64(u32())
    let state = u32()
    let length = Int(u32())
    let text = String(bytes: data[offset..<(offset + length)], encoding: .utf8) ?? ""
    offset += length

    #expect(offset == data.count, "the reply carries no trailing bytes")
    return (revision, state, text)
}

private let oddToken = Data((0..<33).map { UInt8(($0 * 7 + 3) % 256) })

/// The remote tests share the process-wide broker, so this suite runs its
/// tests one at a time even while the rest of the file runs in parallel.
/// Every test resets the broker first, so none starts from another's answer.
@Suite(.serialized)
@MainActor
struct RemoteNotifications {
    /// The app delegate can forward a token before any plugin or guest exists,
    /// and a plugin created afterwards reads it. The hex is exact and lower-case,
    /// and a 33-byte token proves no 32-byte assumption.
    @Test
    func aDelegateTokenReachesAPluginCreatedAfterIt() async throws {
        RemoteRegistration.reset()
        NotificationsPlugin.noteDeviceToken(oddToken)

        let hex = oddToken.map { String(format: "%02x", $0) }.joined()
        #expect(hex.count == 66)
        #expect(!hex.contains { $0.isUppercase })

        let plugin = NotificationsPlugin(service: Service())
        let reply = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(reply.revision == 1)
        #expect(reply.state == 1)
        #expect(reply.text == hex)
        plugin.close()
        RemoteRegistration.reset()
    }

    /// A token outside the contract's bounds is an honest failure, never a
    /// shortened or padded token, and its bytes never reach the reason text.
    @Test
    func anUnrepresentableTokenBecomesAFailureWithoutLeakingBytes() async throws {
        RemoteRegistration.reset()
        NotificationsPlugin.noteDeviceToken(Data(repeating: 0xab, count: 8))

        let reply = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(reply.revision == 1)
        #expect(reply.state == 2)
        #expect(reply.text == "the device token has an unexpected length")
        RemoteRegistration.reset()
    }

    /// Failure reasons are sanitized to bounded, control-free text.
    @Test
    func registrationFailuresAreSanitizedAndBounded() async throws {
        RemoteRegistration.reset()
        struct Loud: Error {}
        let long = String(repeating: "x", count: 600)
        NotificationsPlugin.noteRegistrationFailure(Loud())
        #expect(RemoteRegistration.current().revision == 1)

        let first = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(first.state == 2)

        RemoteRegistration.note(failure: "bad\nreason\u{7}")

        let second = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(second.revision == 2)
        #expect(second.state == 2)
        #expect(second.text == "badreason")
        #expect(!second.text.unicodeScalars.contains { $0.value < 32 })

        RemoteRegistration.note(failure: long)
        let third = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(third.text.utf8.count <= 256)
        #expect(third.text.hasPrefix("xxx"))
        RemoteRegistration.reset()
    }

    /// Every waiter of one registration sees the same answer.
    @Test
    func everyWaiterSeesTheSameAnswer() async throws {
        RemoteRegistration.reset()
        let waiters = (0..<3).map { _ in
            Task { try await NotificationsPlugin.waitRemoteChange(from: 0, limitNanos: 5_000_000_000) }
        }

        // Let the waiters reach their sleep before the delegate answers.
        for _ in 0..<100 { await Task.yield() }
        NotificationsPlugin.noteDeviceToken(oddToken)

        for waiter in waiters {
            let snapshot = try await waiter.value
            #expect(snapshot.revision == 1)
            #expect(snapshot.state == .token(oddToken.map { String(format: "%02x", $0) }.joined()))
        }
        RemoteRegistration.reset()
    }

    /// The full registration call: the OS boundary is asked once, the delegate's
    /// answer lands in the reply as exact lower-case hex, a second caller while
    /// one is waiting converges on the same answer, and a refresh sees the new
    /// revision. The fake stands in for `registerForRemoteNotifications` and the
    /// tests call the same delegate-shaped entry points the generated app
    /// delegate does.
    @Test
    func registrationForwardsTheDelegateAnswerToEveryCaller() async throws {
        RemoteRegistration.reset()
        let registrar = FakeRegistrar()
        let plugin = NotificationsPlugin(service: Service(), registrar: registrar, remoteWaitLimitNanos: 5_000_000_000)

        let first = Task { try await plugin.invoke(method: 8, payload: payload([])) }
        let second = Task { try await plugin.invoke(method: 8, payload: payload([])) }

        for _ in 0..<100 { await Task.yield() }
        #expect(registrar.calls >= 1, "the OS boundary was asked")
        NotificationsPlugin.noteDeviceToken(oddToken)

        let hex = oddToken.map { String(format: "%02x", $0) }.joined()
        for answered in [try await first.value, try await second.value] {
            let reply = try snapshot(answered)
            #expect(reply.revision == 1)
            #expect(reply.state == 1)
            #expect(reply.text == hex)
        }

        // A refresh answers again through the delegate with a new revision.
        let refresh = Task { try await plugin.invoke(method: 8, payload: payload([])) }
        for _ in 0..<100 { await Task.yield() }
        #expect(registrar.calls >= 2, "the second call asked the OS boundary again")
        NotificationsPlugin.noteDeviceToken(Data(repeating: 0x5a, count: 40))
        let refreshed = try snapshot(try await refresh.value)
        #expect(refreshed.revision == 2)
        #expect(refreshed.state == 1)

        plugin.close()
        RemoteRegistration.reset()
    }

    /// The delegate's failure shape reaches waiters as a failure snapshot, and a
    /// payload that isn't empty is refused without touching the OS boundary.
    @Test
    func registrationReportsTheDelegateFailureAndGuardsItsPayload() async throws {
        RemoteRegistration.reset()
        struct Refused: Error, CustomStringConvertible {
            var description = "no valid aps-environment entitlement"
        }
        let registrar = FakeRegistrar()
        let plugin = NotificationsPlugin(service: Service(), registrar: registrar, remoteWaitLimitNanos: 5_000_000_000)

        let task = Task { try await plugin.invoke(method: 8, payload: payload([])) }
        for _ in 0..<100 { await Task.yield() }
        NotificationsPlugin.noteRegistrationFailure(Refused())

        let reply = try snapshot(try await task.value)
        #expect(reply.revision == 1)
        #expect(reply.state == 2)
        #expect(reply.text == "no valid aps-environment entitlement")

        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 8, payload: payload([120]))
        }
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 7, payload: payload([120]))
        }
        #expect(registrar.calls == 1, "a bad payload never reaches the OS boundary")

        plugin.close()
        await #expect(throws: PluginError.cancelled) {
            try await plugin.invoke(method: 8, payload: payload([]))
        }
        RemoteRegistration.reset()
    }

    /// A silent OS answers the unchanged snapshot at the bound instead of
    /// hanging, and a callback that arrives after the bound is still readable.
    @Test
    func theWaitIsBoundedAndLateCallbacksStayReadable() async throws {
        RemoteRegistration.reset()
        let plugin = NotificationsPlugin(
            service: Service(),
            registrar: FakeRegistrar(),
            remoteWaitLimitNanos: 120_000_000
        )
        let start = Date()
        let reply = try snapshot(try await plugin.invoke(method: 8, payload: payload([])))
        #expect(Date().timeIntervalSince(start) < 2, "the bound is honored, not a hang")
        #expect(reply.revision == 0)
        #expect(reply.state == 0)

        NotificationsPlugin.noteDeviceToken(oddToken)
        let after = try snapshot(try await plugin.invoke(method: 7, payload: payload([])))
        #expect(after.revision == 1)
        #expect(after.state == 1)
        plugin.close()
        RemoteRegistration.reset()
    }

    /// Cancelling a registration call ends it with the plugin's cancelled error,
    /// and the delegate's late answer lands where a later reader sees it.
    @Test
    func cancellationEndsTheWaitWithoutLosingTheCallback() async throws {
        RemoteRegistration.reset()
        let plugin = NotificationsPlugin(
            service: Service(),
            registrar: FakeRegistrar(),
            remoteWaitLimitNanos: 30_000_000_000
        )
        let task = Task { try await plugin.invoke(method: 8, payload: payload([])) }

        for _ in 0..<100 { await Task.yield() }
        task.cancel()
        await #expect(throws: PluginError.cancelled) { try await task.value }

        NotificationsPlugin.noteDeviceToken(oddToken)
        let snapshot = try snapshot(try await plugin.invoke(method: 7, payload: payload([])))
        #expect(snapshot.revision == 1)
        #expect(snapshot.state == 1)
        plugin.close()
        RemoteRegistration.reset()
    }

    /// Closing a plugin neither resets nor retains the broker: a fresh instance
    /// reads the answer, and no closed guest's state is carried into it.
    @Test
    func closingThePluginKeepsTheAnswerReadable() async throws {
        RemoteRegistration.reset()
        let first = NotificationsPlugin(service: Service())
        NotificationsPlugin.noteDeviceToken(oddToken)
        first.close()

        let second = NotificationsPlugin(service: Service())
        let snapshot = try await NotificationsPlugin.waitRemoteChange(from: 0, limitNanos: 5_000_000_000)
        #expect(snapshot.revision == 1)
        #expect(snapshot.state != .none)
        second.close()
        RemoteRegistration.reset()
    }

    /// Only iOS hosts advertise the remote methods; the generated dispatcher
    /// answers Unavailable for anything `methods` doesn't list.
    @Test
    func onlyIosAdvertisesTheRemoteMethods() {
        #if os(iOS)
        #expect(NotificationsPlugin(service: Service()).methods.contains(7))
        #expect(NotificationsPlugin(service: Service()).methods.contains(8))
        #else
        let plugin = NotificationsPlugin(service: Service())
        #expect(!plugin.methods.contains(7))
        #expect(!plugin.methods.contains(8))
        plugin.close()
        #endif
    }
}

@MainActor
private final class FakeRegistrar: RemoteRegistrar {
    var calls = 0
    func register() async { calls += 1 }
}
