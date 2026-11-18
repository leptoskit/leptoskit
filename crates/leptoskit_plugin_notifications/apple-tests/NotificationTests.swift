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
    /// shortened or padded token, and its bytes never reach the reason code.
    @Test
    func anUnrepresentableTokenBecomesAFailureWithoutLeakingBytes() async throws {
        RemoteRegistration.reset()
        NotificationsPlugin.noteDeviceToken(Data(repeating: 0xab, count: 8))

        let reply = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(reply.revision == 1)
        #expect(reply.state == 2)
        #expect(reply.text == "token-shape")
        RemoteRegistration.reset()
    }

    /// Failure reasons are the contract's stable codes: the OS's own error
    /// text never crosses, whatever it carries, and the one diagnosis worth
    /// keeping — the entitlement refusal — is matched on domain and code.
    @Test
    func registrationFailuresBecomeStableReasonCodes() async throws {
        RemoteRegistration.reset()

        // Arbitrary error text, whatever it carries, becomes one code.
        struct Loud: Error, CustomStringConvertible {
            var description = "device 0xdeadbeef token=a1b2c3 no valid aps-environment \u{7}"
        }
        NotificationsPlugin.noteRegistrationFailure(Loud())
        var reply = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(reply.revision == 1)
        #expect(reply.state == 2)
        #expect(reply.text == "os-refused")

        // The entitlement refusal keeps its own stable code.
        NotificationsPlugin.noteRegistrationFailure(
            NSError(domain: NSCocoaErrorDomain, code: 3000)
        )
        reply = try snapshot(NotificationsPlugin.encodeSnapshot(RemoteRegistration.current()))
        #expect(reply.revision == 2)
        #expect(reply.text == "aps-environment")

        // No description of the broker's state carries token bytes.
        RemoteRegistration.noteDeviceToken(oddToken)
        let described = String(describing: RemoteRegistration.current())
        #expect(!described.contains(oddToken.map { String(format: "%02x", $0) }.joined()))
        #expect(described.contains("hex chars"))
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

    /// The full registration call: the OS boundary is asked exactly once
    /// while concurrent callers share its wait, the delegate's answer lands
    /// in every reply as exact lower-case hex, and a refresh sees the new
    /// revision. The fake stands in for `registerForRemoteNotifications`
    /// and the tests call the same delegate-shaped entry points the
    /// generated app delegate does.
    @Test
    func registrationForwardsTheDelegateAnswerToEveryCaller() async throws {
        RemoteRegistration.reset()
        let registrar = FakeRegistrar()
        let plugin = NotificationsPlugin(service: Service(), registrar: registrar, remoteWaitLimitNanos: 5_000_000_000)

        let first = Task { try await plugin.invoke(method: 8, payload: payload([])) }
        let second = Task { try await plugin.invoke(method: 8, payload: payload([])) }

        for _ in 0..<100 { await Task.yield() }
        #expect(registrar.calls == 1, "one OS request serves every concurrent caller")
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
        #expect(registrar.calls == 2, "a later call asks the OS boundary again")
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
        struct Refused: Error {}
        let registrar = FakeRegistrar()
        let plugin = NotificationsPlugin(service: Service(), registrar: registrar, remoteWaitLimitNanos: 5_000_000_000)

        let task = Task { try await plugin.invoke(method: 8, payload: payload([])) }
        for _ in 0..<100 { await Task.yield() }
        NotificationsPlugin.noteRegistrationFailure(Refused())

        let reply = try snapshot(try await task.value)
        #expect(reply.revision == 1)
        #expect(reply.state == 2)
        #expect(reply.text == "os-refused")

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

    /// A silent OS answers `unconfirmed` at the bound instead of handing
    /// back the previous token as though it were fresh — even when the
    /// broker already holds one — and a callback that arrives after the
    /// bound is still readable.
    @Test
    func theWaitIsBoundedAndNeverAnswersAPriorTokenAsFresh() async throws {
        RemoteRegistration.reset()
        // A previous registration's token, already in the broker.
        NotificationsPlugin.noteDeviceToken(oddToken)
        let plugin = NotificationsPlugin(
            service: Service(),
            registrar: FakeRegistrar(),
            remoteWaitLimitNanos: 120_000_000
        )
        let start = Date()
        let reply = try snapshot(try await plugin.invoke(method: 8, payload: payload([])))
        #expect(Date().timeIntervalSince(start) < 2, "the bound is honored, not a hang")
        #expect(reply.revision == 0, "an unconfirmed answer carries no revision")
        #expect(reply.state == 3, "silence is unconfirmed, not the prior token")
        #expect(reply.text.isEmpty)

        // The prior token is still readable, untouched, and a late callback
        // still lands for the next reader.
        let before = try snapshot(try await plugin.invoke(method: 7, payload: payload([])))
        #expect(before.revision == 1)
        #expect(before.state == 1)
        NotificationsPlugin.noteDeviceToken(Data(repeating: 0x5a, count: 40))
        let after = try snapshot(try await plugin.invoke(method: 7, payload: payload([])))
        #expect(after.revision == 2)
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

    /// A plugin closed while a registration waits never succeeds with a
    /// token: the call answers cancelled, while the broker keeps the late
    /// callback readable for the next plugin instance.
    @Test
    func closingWhileWaitingNeverSucceedsWithAToken() async throws {
        RemoteRegistration.reset()
        let plugin = NotificationsPlugin(
            service: Service(),
            registrar: FakeRegistrar(),
            remoteWaitLimitNanos: 30_000_000_000
        )
        let task = Task { try await plugin.invoke(method: 8, payload: payload([])) }

        for _ in 0..<100 { await Task.yield() }
        plugin.close()
        NotificationsPlugin.noteDeviceToken(oddToken)

        do {
            let answer = try await task.value
            Issue.record("a closed plugin must not answer a token: \(answer)")
        } catch {
            #expect(error is PluginError)
        }

        let later = try snapshot(
            try await NotificationsPlugin(service: Service()).invoke(method: 7, payload: payload([]))
        )
        #expect(later.revision == 1)
        #expect(later.state == 1)
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

    /// Apple's hosts advertise the remote methods; the generated dispatcher
    /// answers Unavailable for anything `methods` doesn't list.
    @Test
    func appleHostsAdvertiseTheRemoteMethods() {
        let plugin = NotificationsPlugin(service: Service())

        #if os(iOS) || os(macOS)
        #expect(plugin.methods.contains(7))
        #expect(plugin.methods.contains(8))
        #else
        #expect(!plugin.methods.contains(7))
        #expect(!plugin.methods.contains(8))
        #endif
        plugin.close()
    }

    /// The macOS path of the whole registration call, exercised exactly as a
    /// generated macOS app runs it: the plugin advertises `REMOTE_TOKEN` and
    /// `REMOTE_REGISTER`, the delegate-shaped forward lands in the broker
    /// before any plugin exists, and the registration call answers the
    /// forwarded token through the same codec as iOS. The OS boundaries stay
    /// injected — the production `NSApplication` call is the one line the
    /// typecheck of the generated app proves.
    @Test
    func theMacOSPathAnswersTheDelegateForwardThroughTheAdvertisedMethods() async throws {
        #if os(macOS)
        RemoteRegistration.reset()

        // The generated MacApp delegate forwards before any plugin exists;
        // a plugin created afterwards reads the same answer.
        NotificationsPlugin.noteDeviceToken(oddToken)
        let plugin = NotificationsPlugin(service: Service())
        #expect(plugin.methods.contains(7) && plugin.methods.contains(8))

        let reply = try snapshot(try await plugin.invoke(method: 7, payload: payload([])))
        #expect(reply.revision == 1)
        #expect(reply.state == 1)
        #expect(reply.text == oddToken.map { String(format: "%02x", $0) }.joined())

        // A registration through the same dispatch shape answers the new
        // answer, never a stale one.
        let registrar = FakeRegistrar()
        let asking = NotificationsPlugin(
            service: Service(),
            registrar: registrar,
            remoteWaitLimitNanos: 5_000_000_000
        )
        let task = Task { try await asking.invoke(method: 8, payload: payload([])) }
        for _ in 0..<100 { await Task.yield() }
        #expect(registrar.calls == 1)
        NotificationsPlugin.noteDeviceToken(Data(repeating: 0x5a, count: 40))
        let registered = try snapshot(try await task.value)
        #expect(registered.revision == 2)
        #expect(registered.state == 1)
        #expect(registered.text == String(repeating: "5a", count: 40))

        plugin.close()
        asking.close()
        RemoteRegistration.reset()
        #endif
    }
}

@MainActor
private final class FakeRegistrar: RemoteRegistrar {
    var calls = 0
    func register() async { calls += 1 }
}

/// A fake for what the OS reports about the device.
@MainActor
private struct FakeDevice: DeviceFacts {
    var details: DeviceDetails
    func current() -> DeviceDetails { details }
}

/// A `DEVICE` reply's three strings, decoded.
private func deviceFields(_ data: Data) -> [String] {
    var fields: [String] = []
    var offset = 0
    let bytes = [UInt8](data)

    while offset + 4 <= bytes.count {
        let length = Int(UInt32(bytes[offset]) | UInt32(bytes[offset + 1]) << 8 |
            UInt32(bytes[offset + 2]) << 16 | UInt32(bytes[offset + 3]) << 24)
        offset += 4
        fields.append(String(decoding: bytes[offset..<(offset + length)], as: UTF8.self))
        offset += length
    }

    #expect(offset == bytes.count, "no trailing bytes")
    return fields
}

/// The device method answers what the OS reports — the preferred locale,
/// the time zone, the app's version — as the contract's three strings, an
/// unknown value as empty, and an unshippable one as unknown rather than a
/// refusal of the rest.
@Test
@MainActor
func theDeviceMethodAnswersWhatTheOSReports() async throws {
    let reported = NotificationsPlugin(
        service: Service(),
        device: FakeDevice(details: DeviceDetails(locale: "en-GB", timeZone: "Europe/London", appVersion: "2.1.0"))
    )
    #expect(
        deviceFields(try await reported.invoke(method: 9, payload: payload([])))
            == ["en-GB", "Europe/London", "2.1.0"]
    )
    reported.close()

    let silent = NotificationsPlugin(
        service: Service(),
        device: FakeDevice(details: DeviceDetails(locale: nil, timeZone: nil, appVersion: nil))
    )
    #expect(deviceFields(try await silent.invoke(method: 9, payload: payload([]))) == ["", "", ""])

    // A payload is refused: the method takes none.
    do {
        _ = try await silent.invoke(method: 9, payload: payload([1]))
        Issue.record("a payload must be refused")
    } catch {
        #expect(error as? PluginError == .invalidArgument)
    }
    silent.close()

    let odd = NotificationsPlugin(
        service: Service(),
        device: FakeDevice(details: DeviceDetails(
            locale: "en\u{7}GB",
            timeZone: "Europe/London",
            appVersion: String(repeating: "9", count: NotificationsPlugin.maxDeviceDetailBytes + 1)
        ))
    )
    #expect(deviceFields(try await odd.invoke(method: 9, payload: payload([]))) == ["", "Europe/London", ""])
    odd.close()

    // The system facts are Foundation's own answers.
    let system = SystemDeviceFacts().current()
    #expect(system.timeZone == TimeZone.current.identifier)
    #expect(system.locale == Locale.preferredLanguages.first)

    #if os(iOS) || os(macOS)
    #expect(reported.methods.contains(9))
    #endif
}
