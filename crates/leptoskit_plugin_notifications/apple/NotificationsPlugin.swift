import Foundation
import LeptosKitBridge
import UserNotifications
#if os(macOS)
import AppKit
#endif
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
/// `UIApplication.registerForRemoteNotifications` on iOS and
/// `NSApplication.registerForRemoteNotifications` on macOS; tests substitute
/// a fake.
@MainActor
protocol RemoteRegistrar {
    func register() async
}

/// What the device says about itself for push targeting, as `DEVICE`
/// (method 9) answers it. A `nil` value is unknown.
public struct DeviceDetails: Equatable, Sendable {
    public var locale: String?
    public var timeZone: String?
    public var appVersion: String?

    public init(locale: String?, timeZone: String?, appVersion: String?) {
        self.locale = locale
        self.timeZone = timeZone
        self.appVersion = appVersion
    }
}

/// Where the device's details come from. Production reads Foundation;
/// tests substitute a fake.
@MainActor
protocol DeviceFacts {
    func current() -> DeviceDetails
}

/// The OS's own answers: the first preferred language, the current time
/// zone's identifier, and the app bundle's `CFBundleShortVersionString` —
/// read fresh on every call, so a language change or an app update shows
/// on the next read.
@MainActor
struct SystemDeviceFacts: DeviceFacts {
    func current() -> DeviceDetails {
        DeviceDetails(
            locale: Locale.preferredLanguages.first,
            timeZone: TimeZone.current.identifier,
            appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        )
    }
}

/// Which APNs environment the running build is signed for, as
/// `APS_ENVIRONMENT` (method 11) answers it. A device token is known only
/// to that environment's server.
enum ApsEnvironment: UInt32 {
    case unknown = 0
    case development = 1
    case production = 2

    /// The environment a provisioning profile's bytes name, or `nil` when
    /// they name none. A profile is a signed envelope around a property
    /// list, which is read from between its own markers.
    static func named(inProfile profile: Data) -> ApsEnvironment? {
        guard let start = profile.range(of: Data("<?xml".utf8)),
              let end = profile.range(of: Data("</plist>".utf8), in: start.lowerBound..<profile.endIndex),
              let list = try? PropertyListSerialization.propertyList(
                  from: profile.subdata(in: start.lowerBound..<end.upperBound),
                  options: [],
                  format: nil
              ),
              let entitlements = (list as? [String: Any])?["Entitlements"] as? [String: Any] else {
            return nil
        }

        // iOS spells the entitlement `aps-environment`; macOS prefixes it.
        let named = entitlements["aps-environment"] ?? entitlements["com.apple.developer.aps-environment"]

        switch named as? String {
        case "development":
            return .development
        case "production":
            return .production
        default:
            return nil
        }
    }

    /// The running build's environment. A simulator registers with the
    /// sandbox. A build with an embedded provisioning profile is what the
    /// profile says. An iOS build without one came through TestFlight or
    /// the App Store, which strip the profile and sign for production.
    static func current(bundle: Bundle = .main) -> ApsEnvironment {
        #if targetEnvironment(simulator)
        return .development
        #elseif os(macOS)
        let url = bundle.bundleURL.appendingPathComponent("Contents/embedded.provisionprofile")

        guard let profile = try? Data(contentsOf: url) else { return .unknown }

        return named(inProfile: profile) ?? .unknown
        #else
        let url = bundle.bundleURL.appendingPathComponent("embedded.mobileprovision")

        guard let profile = try? Data(contentsOf: url) else { return .production }

        return named(inProfile: profile) ?? .unknown
        #endif
    }
}

@MainActor
private struct SystemRemoteRegistrar: RemoteRegistrar {
    func register() async {
        #if canImport(UIKit) && os(iOS)
        UIApplication.shared.registerForRemoteNotifications()
        #endif
        #if os(macOS)
        NSApplication.shared.registerForRemoteNotifications()
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

    /// The registration answer's shape. Descriptions never print a token's
    /// bytes.
    public enum State: Equatable, Sendable, CustomStringConvertible {
        case none
        /// The OS issued a device token, as lower-case hex.
        case token(String)
        /// The OS refused registration; one of the contract's stable codes.
        case failed(String)
        /// A registration wait ended without a new answer. The broker's last
        /// answer, if any, is unchanged and still readable.
        case unconfirmed

        public var description: String {
            switch self {
            case .none: "none"
            case .token(let hex): "token(<\(hex.count) hex chars>)"
            case .failed(let reason): "failed(\(reason))"
            case .unconfirmed: "unconfirmed"
            }
        }
    }

    public struct Snapshot: Equatable, Sendable, CustomStringConvertible {
        /// Monotonic answer counter; `0` means no answer has arrived since
        /// the process started.
        public var revision: UInt64
        public var state: State

        public var description: String { "Snapshot(revision: \(revision), state: \(state))" }
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
            note(failure: FailureCode.tokenShape)
            return
        }

        snapshot = Snapshot(revision: nextRevision(), state: .token(hex))
    }

    /// The app delegate's `didFailToRegisterForRemoteNotificationsWithError`.
    /// The reason becomes one of the contract's stable codes: the OS's own
    /// error text can carry device or provisioning detail an app has no
    /// business logging, so none of it crosses the bridge, and the token
    /// never travels through an error.
    public static func noteRegistrationFailure(_ error: any Error) {
        note(failure: failureCode(for: error))
    }

    /// Maps the delegate's error to its stable code. The one distinction
    /// worth keeping is the entitlement refusal — the app's own messaging
    /// wants to name it — and it is matched on domain and code, never on the
    /// error's text.
    private static func failureCode(for error: any Error) -> String {
        let native = error as NSError
        if native.domain == NSCocoaErrorDomain, native.code == 3000 {
            return FailureCode.apsEnvironment
        }
        return FailureCode.osRefused
    }

    /// The contract's stable failure codes, mirrored from the Rust side.
    private enum FailureCode {
        static let osRefused = "os-refused"
        static let apsEnvironment = "aps-environment"
        static let tokenShape = "token-shape"
    }

    static func note(failure: String) {
        snapshot = Snapshot(revision: nextRevision(), state: .failed(failure))
    }

    /// The next revision, saturating instead of trapping should the counter
    /// ever reach the top.
    private static func nextRevision() -> UInt64 {
        let next = snapshot.revision &+ 1
        return next == 0 ? .max : next
    }

    /// Resets the broker. Process-scoped state, so only tests call this.
    static func reset(_ snapshot: Snapshot = Snapshot(revision: 0, state: .none)) {
        self.snapshot = snapshot
    }
}

/// What the app has received about notifications and not yet handed to its
/// guest, as `RECEIVED` (method 10) answers it: a tap on a notification, a
/// notification that arrived while the app was open, and a message handed
/// to the app without a tap. Static on purpose, like `RemoteRegistration`:
/// a tap that launches the app arrives before the plugin and its guest
/// exist. Guarded by a lock rather than an actor, because the notification
/// centre calls its delegate without promising one.
///
/// The host interprets nothing. Each item carries the provider's payload
/// as JSON text, and the Rust side — which knows the envelope its service
/// sends — reads the notification's id, the link, and the send's data out
/// of it. Only the latest `maxItems` are kept: the inbox is the durable
/// record.
public enum ReceivedNotifications {
    /// Which callback an item came from, as the Rust contract numbers
    /// them.
    public enum Kind: UInt32, Sendable {
        /// The person tapped a notification or one of its action buttons.
        case opened = 0
        /// A notification arrived while the app was open, and was shown.
        case presented = 1
        /// A message was handed to the app without a tap.
        case message = 2
    }

    public struct Item: Equatable, Sendable {
        public var kind: Kind
        /// The pressed action button's identifier; empty for a tap on the
        /// notification itself.
        public var action: String
        /// The identifier of the local notification the plugin posted,
        /// without the plugin's prefix; empty for a push.
        public var local: String
        /// The provider's payload as JSON text; empty when it has none
        /// that fits.
        public var payload: String
    }

    /// Bounds shared with the Rust contract.
    static let maxItems = 16
    static let maxPayloadBytes = 8192
    static let maxActionBytes = 256
    static let maxLocalBytes = 64
    /// The prefix the plugin gives every local notification it posts.
    static let localPrefix = "leptoskit.notifications."

    /// What a notification that arrives while the app is open is allowed
    /// to do: show its banner, stay in Notification Centre, and play the
    /// sound and set the badge it carries. Without a delegate answering
    /// this, the system shows nothing while the app is in front.
    static let foregroundPresentation: UNNotificationPresentationOptions = [.banner, .list, .sound, .badge]

    private static let lock = NSLock()
    nonisolated(unsafe) private static var items: [Item] = []

    /// The delegate's `didReceive response`: a tap. Dismissing a
    /// notification is not a tap and is not recorded.
    public static func noteResponse(_ response: UNNotificationResponse) {
        guard response.actionIdentifier != UNNotificationDismissActionIdentifier else { return }

        let request = response.notification.request
        note(
            .opened,
            action: response.actionIdentifier == UNNotificationDefaultActionIdentifier
                ? "" : response.actionIdentifier,
            identifier: request.identifier,
            userInfo: request.content.userInfo
        )
    }

    /// The delegate's `willPresent notification`: a notification that
    /// arrived while the app was open.
    public static func notePresented(_ notification: UNNotification) {
        let request = notification.request
        note(.presented, action: "", identifier: request.identifier, userInfo: request.content.userInfo)
    }

    /// The app delegate's `didReceiveRemoteNotification`: a push handed to
    /// the app without a tap — a silent data message.
    public static func noteRemoteNotification(_ userInfo: [AnyHashable: Any]) {
        note(.message, action: "", identifier: "", userInfo: userInfo)
    }

    /// Records one item, bounded: an action or a payload outside the
    /// contract's bounds is recorded as empty rather than cut, and the
    /// oldest item makes room for the newest.
    static func note(_ kind: Kind, action: String, identifier: String, userInfo: [AnyHashable: Any]) {
        let item = Item(
            kind: kind,
            action: shippable(action, maximum: maxActionBytes) ? action : "",
            local: local(identifier),
            payload: json(userInfo)
        )

        lock.withLock {
            items.append(item)

            if items.count > maxItems { items.removeFirst(items.count - maxItems) }
        }
    }

    /// Takes everything recorded, oldest first, and clears it.
    static func take() -> [Item] {
        lock.withLock { () -> [Item] in
            let taken = items
            items = []
            return taken
        }
    }

    /// Clears the broker. Process-scoped state, so only tests call this.
    static func reset() {
        lock.withLock { items = [] }
    }

    /// The plugin's own identifier out of a request's: what follows the
    /// plugin's prefix when it is an identifier the contract carries, and
    /// empty for a push, whose identifier is the system's.
    static func local(_ identifier: String) -> String {
        guard identifier.hasPrefix(localPrefix) else { return "" }
        let id = String(identifier.dropFirst(localPrefix.count))

        guard !id.isEmpty, id.utf8.count <= maxLocalBytes,
              id.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) ||
                  (97...122).contains($0) || [45, 46, 95].contains($0) }) else { return "" }

        return id
    }

    /// A payload as JSON text. A push's payload arrived as JSON, so it
    /// encodes as it came; anything that doesn't, or doesn't fit, is sent
    /// as empty.
    static func json(_ userInfo: [AnyHashable: Any]) -> String {
        var object: [String: Any] = [:]

        for (key, value) in userInfo {
            if let key = key as? String { object[key] = value }
        }

        guard !object.isEmpty, JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
              data.count <= maxPayloadBytes,
              let text = String(data: data, encoding: .utf8) else { return "" }

        return text
    }

    /// Whether the contract can carry a short text: within its bound and
    /// free of control characters.
    private static func shippable(_ value: String, maximum: Int) -> Bool {
        value.utf8.count <= maximum &&
            !value.unicodeScalars.contains(where: { $0.value <= 31 || (127...159).contains($0.value) })
    }

    /// Encodes a `RECEIVED` reply: a `u32` count, then for each item a
    /// `u32` kind and three length-prefixed UTF-8 strings — action, local
    /// identifier, payload — all little-endian.
    static func encode(_ items: [Item]) -> Data {
        var data = Data()
        func append(_ value: UInt32) {
            var little = value.littleEndian
            withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
        }

        append(UInt32(items.count))

        for item in items {
            append(item.kind.rawValue)

            for text in [item.action, item.local, item.payload] {
                let bytes = Array(text.utf8)
                append(UInt32(bytes.count))
                data.append(contentsOf: bytes)
            }
        }

        return data
    }
}

/// The notification-centre delegate the generated apps install before
/// launch finishes. Without one, the system shows nothing while the app is
/// in front — neither a push nor the plugin's own local notification — and
/// a tap opens the app and tells it nothing. This one lets a notification
/// show as it would with the app closed, and records what arrived and what
/// was tapped for the guest to take.
///
/// It keeps no state and decides nothing else, so an app that needs its
/// own delegate can forward the same two callbacks to
/// `ReceivedNotifications`.
public final class NotificationCenterDelegate: NSObject, UNUserNotificationCenterDelegate, Sendable {
    public static let shared = NotificationCenterDelegate()

    /// Makes this the notification centre's delegate. Call it from
    /// `application(_:willFinishLaunchingWithOptions:)` on iOS and
    /// `applicationWillFinishLaunching(_:)` on macOS: a tap that launches
    /// the app is delivered as launch finishes, and only to a delegate
    /// that is already there. The centre holds its delegate weakly;
    /// `shared` lives as long as the process.
    @MainActor
    public static func install() {
        UNUserNotificationCenter.current().delegate = shared
    }

    public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        ReceivedNotifications.notePresented(notification)
        completionHandler(ReceivedNotifications.foregroundPresentation)
    }

    public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        ReceivedNotifications.noteResponse(response)
        completionHandler()
    }
}

@MainActor
private final class SystemNotificationService: NotificationService {
    private let center = UNUserNotificationCenter.current()
    func authorization() async -> UNAuthorizationStatus { await center.notificationSettings().authorizationStatus }
    // Alerts, badges, and sounds together: the one question covers what a
    // push from the notifications service can carry (`aps.badge`,
    // `aps.sound`), and the system applies only what the person allowed.
    // Asking for alerts alone left a push's badge and sound without effect.
    // The plugin's own local posts stay silent and set no badge.
    func requestAuthorization() async throws {
        _ = try await center.requestAuthorization(options: NotificationsPlugin.authorizationOptions)
    }
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

/// Immediate local notifications, remote registration, and what the app
/// received. The plugin never sets the notification centre's delegate
/// itself: the generated app installs `NotificationCenterDelegate`.
@MainActor
public final class NotificationsPlugin: NativePlugin {
    public let name = "notifications"
    // The iOS and macOS hosts advertise the remote methods, the device
    // details enrollment sends, what the app received, and the build's
    // APNs environment; the other hosts keep the local-only set.
    // The dispatcher answers Unavailable for anything `methods` omits.
    #if os(iOS) || os(macOS)
    public let methods: [UInt32] = [0, 1, 2, 3, 6, 7, 8, 9, 10, 11]
    #else
    public let methods: [UInt32] = [0, 1, 2, 3, 6]
    #endif
    /// What the permission request asks the person to allow. Alerts alone
    /// would leave a push's badge (`aps.badge`) and sound (`aps.sound`)
    /// without effect: the system applies only what was asked for and
    /// granted.
    static let authorizationOptions: UNAuthorizationOptions = [.alert, .badge, .sound]
    /// The longest device detail a `DEVICE` reply carries, in bytes,
    /// matching the Rust contract's `MAX_DEVICE_DETAIL_BYTES`.
    static let maxDeviceDetailBytes = 256
    /// How long `REMOTE_REGISTER` waits for the OS answer before replying
    /// with the unchanged snapshot, in nanoseconds. Apple answers
    /// registration quickly; the bound keeps a silent OS from hanging the
    /// call.
    static let remoteWaitLimitNanos: UInt64 = 10_000_000_000
    /// How often the bounded wait re-reads the broker, in nanoseconds. A
    /// plain nanoseconds sleep keeps the wait off the Duration/Clock path,
    /// whose runtime support varies across SDKs.
    static let remoteWaitSliceNanos: UInt64 = 50_000_000
    /// The longest `RECEIVED` waits for something to arrive, in
    /// milliseconds, matching the Rust contract's
    /// `MAX_RECEIVED_WAIT_MILLIS`. A caller that keeps listening asks
    /// again.
    static let maxReceivedWaitMillis: UInt32 = 10_000
    /// How often a waiting `RECEIVED` re-reads the broker, in nanoseconds:
    /// the longest a tap waits for a listening guest.
    static let receivedWaitSliceNanos: UInt64 = 100_000_000

    private let service: any NotificationService
    private let registrar: any RemoteRegistrar
    private let device: any DeviceFacts
    private let remoteWaitLimitNanos: UInt64
    private var closed = false
    private var requesting = false
    private var registering = false
    private struct Pending { let ticket: UUID; var cancelled = false }
    private var pending: [String: Pending] = [:]
    static let prefix = "leptoskit.notifications."

    public init() {
        service = SystemNotificationService()
        registrar = SystemRemoteRegistrar()
        device = SystemDeviceFacts()
        remoteWaitLimitNanos = Self.remoteWaitLimitNanos
    }

    init(service: any NotificationService) {
        self.service = service
        registrar = SystemRemoteRegistrar()
        device = SystemDeviceFacts()
        remoteWaitLimitNanos = Self.remoteWaitLimitNanos
    }

    init(service: any NotificationService, device: any DeviceFacts) {
        self.service = service
        registrar = SystemRemoteRegistrar()
        self.device = device
        remoteWaitLimitNanos = Self.remoteWaitLimitNanos
    }

    init(
        service: any NotificationService,
        registrar: any RemoteRegistrar,
        remoteWaitLimitNanos: UInt64
    ) {
        self.service = service
        self.registrar = registrar
        device = SystemDeviceFacts()
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

    /// Installs the notification-centre delegate, so a notification shows
    /// while the app is open and a tap reaches the guest. The app delegate
    /// calls it before launch finishes.
    public static func installDelegate() {
        NotificationCenterDelegate.install()
    }

    /// Forwards the app delegate's `didReceiveRemoteNotification` into the
    /// received broker: a silent data message. Safe whether or not a
    /// plugin instance exists.
    public static func noteRemoteNotification(_ userInfo: [AnyHashable: Any]) {
        ReceivedNotifications.noteRemoteNotification(userInfo)
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
        // them: the iOS and macOS hosts. Other platforms keep the cases for
        // tests, which inject the registration boundary.
        case 7:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try Self.encodeSnapshot(RemoteRegistration.current())
        case 8:
            return try await registerRemote(payload: payload)
        // Method 9 is dispatched where `methods` advertises it: the iOS and
        // macOS hosts, whose enrollment sends what it answers.
        case 9:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return Self.encodeDevice(device.current())
        // Method 10 is dispatched where `methods` advertises it: the iOS
        // and macOS hosts, whose delegates feed the received broker.
        case 10:
            return try await received(payload: payload)
        // Method 11 is dispatched where `methods` advertises it: the iOS
        // and macOS hosts, whose builds are signed for one APNs
        // environment.
        case 11:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return Self.encodeApsEnvironment(ApsEnvironment.current())
        default:
            throw PluginError.unavailable
        }

        return Data()
    }

    /// Takes what the app received since the last call. With nothing
    /// recorded it waits, up to the bound the caller named and never past
    /// the contract's own, for something to arrive, and then answers an
    /// empty batch. Cancelling the wait loses nothing: what arrives stays
    /// in the broker for the next call. A plugin closed while waiting
    /// never answers.
    private func received(payload: NativeBuffer) async throws -> Data {
        let limitNanos = try payload.withUnsafeBytes { bytes -> UInt64 in
            guard bytes.count == 4 else { throw PluginError.invalidArgument }
            let millis = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 0, as: UInt32.self))

            return UInt64(min(millis, Self.maxReceivedWaitMillis)) * 1_000_000
        }
        var waited: UInt64 = 0

        while true {
            guard !closed else { throw PluginError.cancelled }
            try Task.checkCancellation()

            let items = ReceivedNotifications.take()

            if !items.isEmpty || waited >= limitNanos { return ReceivedNotifications.encode(items) }

            do {
                try await Task.sleep(nanoseconds: Self.receivedWaitSliceNanos)
            } catch {
                throw mapped(error)
            }
            waited += Self.receivedWaitSliceNanos
        }
    }

    /// Encodes an `APS_ENVIRONMENT` reply: the answer's number as four
    /// little-endian bytes.
    static func encodeApsEnvironment(_ environment: ApsEnvironment) -> Data {
        var little = environment.rawValue.littleEndian

        return withUnsafeBytes(of: &little) { Data($0) }
    }

    /// Encodes a `DEVICE` reply: three length-prefixed UTF-8 strings —
    /// locale, time zone, app version — each empty when unknown. A value
    /// past the contract's bound, or carrying a control character, is sent
    /// as unknown rather than refused, so one odd detail never costs the
    /// others.
    static func encodeDevice(_ details: DeviceDetails) -> Data {
        var data = Data()

        for value in [details.locale, details.timeZone, details.appVersion] {
            let bytes = Array((value.flatMap(shippable) ?? "").utf8)
            var length = UInt32(bytes.count).littleEndian
            withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
            data.append(contentsOf: bytes)
        }

        return data
    }

    /// A detail the contract can carry, or `nil`.
    private static func shippable(_ value: String) -> String? {
        guard value.utf8.count <= maxDeviceDetailBytes,
              !value.unicodeScalars.contains(where: { $0.value <= 31 || (127...159).contains($0.value) })
        else { return nil }

        return value
    }

    /// Asks the OS to register for remote notifications on the main actor,
    /// then waits a bounded time for the answer the app delegate forwards.
    /// One registration is in flight at a time: callers that arrive while
    /// one runs join its wait rather than asking the OS again. A silent OS
    /// answers `unconfirmed`, never the previous token as though it were
    /// fresh — a late callback still lands in the broker for the next read,
    /// and a plugin closed while waiting never succeeds. Only iOS and macOS
    /// hosts dispatch this: `methods` advertises 8 there alone.
    private func registerRemote(payload: NativeBuffer) async throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        let since = RemoteRegistration.current().revision

        // Bound the OS requests at one: the first caller asks, the rest wait
        // for whatever answer its registration produces.
        let asking = !registering
        if asking { registering = true }
        defer { if asking { registering = false } }

        if asking { await registrar.register() }

        let snapshot: RemoteRegistration.Snapshot
        do {
            snapshot = try await Self.waitRemoteChange(from: since, limitNanos: remoteWaitLimitNanos)
        } catch {
            throw mapped(error)
        }

        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        // Silence at the bound is an explicit non-answer, not a stale token.
        if snapshot.revision == since {
            return try Self.encodeSnapshot(
                RemoteRegistration.Snapshot(revision: 0, state: .unconfirmed)
            )
        }
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
        case .unconfirmed:
            append(3)
            try append("")
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
