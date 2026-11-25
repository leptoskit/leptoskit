import Foundation
import LeptosKitBridge
#if canImport(CoreSpotlight)
@preconcurrency import CoreSpotlight
import UniformTypeIdentifiers
#endif
#if canImport(UIKit)
import UIKit
#endif

/// One portable shortcut the OS may show for this app.
public struct AppShortcutItem: Equatable, Sendable {
    public let id: String
    public let title: String
    public let subtitle: String

    public init(id: String, title: String, subtitle: String) {
        self.id = id
        self.title = title
        self.subtitle = subtitle
    }
}

/// Decoded CoreSpotlight search item payload.
public struct SpotlightItemPayload: Equatable, Sendable {
    public let id: String
    public let domain: String
    public let title: String
    public let description: String
    public let deepLink: String
    public let keywords: [String]
    public let expirationEpochSecs: UInt64?

    public init(
        id: String,
        domain: String,
        title: String,
        description: String,
        deepLink: String,
        keywords: [String],
        expirationEpochSecs: UInt64?
    ) {
        self.id = id
        self.domain = domain
        self.title = title
        self.description = description
        self.deepLink = deepLink
        self.keywords = keywords
        self.expirationEpochSecs = expirationEpochSecs
    }
}

/// Decoded Apple Handoff / Continuity user activity payload.
public struct UserActivityPayload: Equatable, Sendable {
    public let activityType: String
    public let title: String
    public let webpageUrl: String?
    public let eligibleForHandoff: Bool
    public let eligibleForSearch: Bool
    public let eligibleForPrediction: Bool

    public init(
        activityType: String,
        title: String,
        webpageUrl: String?,
        eligibleForHandoff: Bool,
        eligibleForSearch: Bool,
        eligibleForPrediction: Bool
    ) {
        self.activityType = activityType
        self.title = title
        self.webpageUrl = webpageUrl
        self.eligibleForHandoff = eligibleForHandoff
        self.eligibleForSearch = eligibleForSearch
        self.eligibleForPrediction = eligibleForPrediction
    }
}

/// Incoming share payload recorded by a Share Extension or host delegate.
public struct IncomingSharePayload: Equatable, Sendable {
    public let text: String?
    public let url: String?
    public let subject: String?

    public init(text: String? = nil, url: String? = nil, subject: String? = nil) {
        self.text = text
        self.url = url
        self.subject = subject
    }
}

@MainActor
protocol ShortcutPublisher: AnyObject {
    func set(_ items: [AppShortcutItem]) throws
    func clear() throws
    func indexSpotlight(_ item: SpotlightItemPayload) async throws
    func deleteSpotlight(id: String) async throws
    func startUserActivity(_ activity: UserActivityPayload) throws
    func endUserActivity()
}

extension ShortcutPublisher {
    func indexSpotlight(_ item: SpotlightItemPayload) async throws {
        _ = item
        throw PluginError.unavailable
    }

    func deleteSpotlight(id: String) async throws {
        _ = id
        throw PluginError.unavailable
    }

    func startUserActivity(_ activity: UserActivityPayload) throws {
        _ = activity
        throw PluginError.unavailable
    }

    func endUserActivity() {}
}

/// Latest shortcuts published for the macOS dock menu. Generated hosts return
/// these from `applicationDockMenu(_:)` and call ``noteLaunch(id:)`` when the
/// user picks an item.
@MainActor
public enum DockShortcutStore {
    public static var items: [AppShortcutItem] = []
    /// The registered plugin instance, when the generated host selected
    /// `app-shortcuts`.
    public static weak var plugin: AppShortcutsPlugin?
}

@MainActor
private final class SystemShortcutPublisher: ShortcutPublisher {
    private var activeUserActivity: NSUserActivity?

    func set(_ items: [AppShortcutItem]) throws {
        #if canImport(UIKit)
        UIApplication.shared.shortcutItems = items.map { item in
            UIApplicationShortcutItem(
                type: item.id,
                localizedTitle: item.title,
                localizedSubtitle: item.subtitle.isEmpty ? nil : item.subtitle,
                icon: nil,
                userInfo: nil
            )
        }
        #elseif canImport(AppKit)
        DockShortcutStore.items = items
        #endif
    }

    func clear() throws {
        try set([])
    }

    func indexSpotlight(_ item: SpotlightItemPayload) async throws {
        #if canImport(CoreSpotlight)
        let attributes = CSSearchableItemAttributeSet(contentType: UTType.content)
        attributes.title = item.title
        attributes.contentDescription = item.description.isEmpty ? nil : item.description
        attributes.keywords = item.keywords.isEmpty ? nil : item.keywords
        attributes.relatedUniqueIdentifier = item.deepLink
        if let url = URL(string: item.deepLink) {
            attributes.contentURL = url
        }
        let searchable = CSSearchableItem(
            uniqueIdentifier: item.id,
            domainIdentifier: item.domain,
            attributeSet: attributes
        )
        if let expiration = item.expirationEpochSecs {
            searchable.expirationDate = Date(timeIntervalSince1970: TimeInterval(expiration))
        }
        do {
            try await CSSearchableIndex.default().indexSearchableItems([searchable])
        } catch {
            throw PluginError.native
        }
        #else
        _ = item
        throw PluginError.unavailable
        #endif
    }

    func deleteSpotlight(id: String) async throws {
        #if canImport(CoreSpotlight)
        do {
            try await CSSearchableIndex.default().deleteSearchableItems(withIdentifiers: [id])
        } catch {
            throw PluginError.native
        }
        #else
        _ = id
        throw PluginError.unavailable
        #endif
    }

    func startUserActivity(_ request: UserActivityPayload) throws {
        activeUserActivity?.invalidate()
        let activity = NSUserActivity(activityType: request.activityType)
        activity.title = request.title
        if let webpageUrl = request.webpageUrl, let url = URL(string: webpageUrl) {
            activity.webpageURL = url
        }
        activity.isEligibleForHandoff = request.eligibleForHandoff
        activity.isEligibleForSearch = request.eligibleForSearch
        #if os(iOS)
        activity.isEligibleForPrediction = request.eligibleForPrediction
        #endif
        activity.becomeCurrent()
        activeUserActivity = activity
    }

    func endUserActivity() {
        activeUserActivity?.invalidate()
        activeUserActivity = nil
    }
}

/// Publishes static home-screen / dock shortcuts, indexes CoreSpotlight items,
/// manages Apple Handoff `NSUserActivity`, and delivers incoming shares.
@MainActor
public final class AppShortcutsPlugin: NativePlugin {
    public let name = "app-shortcuts"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7]

    private let publisher: any ShortcutPublisher
    private var closed = false
    private var launchId: String?
    private var pendingShare: IncomingSharePayload?

    public init() {
        publisher = SystemShortcutPublisher()
        DockShortcutStore.plugin = self
    }

    init(publisher: any ShortcutPublisher) {
        self.publisher = publisher
        DockShortcutStore.plugin = self
    }

    /// Records that the OS launched or activated the app through this id.
    public func noteLaunch(id: String) throws {
        guard !closed else { throw PluginError.cancelled }
        try Self.validateId(id)
        launchId = id
    }

    /// Notes a home-screen quick action or dock-menu selection when a plugin
    /// instance is registered.
    public static func noteLaunchIfPresent(id: String) {
        try? DockShortcutStore.plugin?.noteLaunch(id: id)
    }

    /// Records an inbound share delivered from an iOS Share Extension or host
    /// delegate so the guest can consume it once via `consume_incoming_share`.
    public func noteIncomingShare(text: String? = nil, url: String? = nil, subject: String? = nil) throws {
        guard !closed else { throw PluginError.cancelled }
        if let text {
            guard text.utf8.count <= 4096,
                  text.unicodeScalars.allSatisfy({
                      !CharacterSet.controlCharacters.contains($0) || $0.value == 9 || $0.value == 10 || $0.value == 13
                  })
            else { throw PluginError.invalidArgument }
        }
        if let url {
            try Self.validateSafeUrl(url)
        }
        if let subject {
            try Self.validateText(subject, max: 256, allowEmpty: true)
        }
        pendingShare = IncomingSharePayload(text: text, url: url, subject: subject)
    }

    /// Notes an incoming share when a plugin instance is registered.
    public static func noteIncomingShareIfPresent(
        text: String? = nil,
        url: String? = nil,
        subject: String? = nil
    ) {
        try? DockShortcutStore.plugin?.noteIncomingShare(text: text, url: url, subject: subject)
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        switch method {
        case 0:
            try publisher.set(try Self.decodeList(payload))
            return Data()
        case 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            try publisher.clear()
            return Data()
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            let id = launchId
            launchId = nil
            return try Self.encodeLaunch(id)
        case 3:
            let item = try Self.decodeSpotlightItem(payload)
            try await publisher.indexSpotlight(item)
            return Data()
        case 4:
            let id = try Self.decodeSingleId(payload)
            try await publisher.deleteSpotlight(id: id)
            return Data()
        case 5:
            let activity = try Self.decodeUserActivity(payload)
            try publisher.startUserActivity(activity)
            return Data()
        case 6:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            publisher.endUserActivity()
            return Data()
        case 7:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            let share = pendingShare
            pendingShare = nil
            return Self.encodeOptionalShare(share)
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        guard !closed else { return }
        closed = true
        // Teardown must not leave launcher/dock entries or Handoff activities
        // the guest can no longer clear through the bridge.
        try? publisher.clear()
        publisher.endUserActivity()
    }

    private static func decodeSingleId(_ payload: NativeBuffer) throws -> String {
        try payload.withUnsafeBytes { bytes in
            var offset = 0
            let id = try readString(bytes, &offset, max: 64)
            try validateId(id)
            guard offset == bytes.count else { throw PluginError.invalidArgument }
            return id
        }
    }

    private static func decodeList(_ payload: NativeBuffer) throws -> [AppShortcutItem] {
        try payload.withUnsafeBytes { bytes in
            var offset = 0
            let count = try readU32(bytes, &offset)

            guard count <= 8 else { throw PluginError.tooLarge }
            var items: [AppShortcutItem] = []
            items.reserveCapacity(Int(count))
            var seen = Set<String>()

            for _ in 0..<count {
                let id = try readString(bytes, &offset, max: 64)
                let title = try readString(bytes, &offset, max: 128)
                let subtitle = try readString(bytes, &offset, max: 128)
                try validateId(id)
                try validateText(title, max: 128, allowEmpty: false)
                try validateText(subtitle, max: 128, allowEmpty: true)

                guard seen.insert(id).inserted else { throw PluginError.invalidArgument }

                items.append(AppShortcutItem(id: id, title: title, subtitle: subtitle))
            }

            guard offset == bytes.count else { throw PluginError.invalidArgument }

            return items
        }
    }

    private static func decodeSpotlightItem(_ payload: NativeBuffer) throws -> SpotlightItemPayload {
        try payload.withUnsafeBytes { bytes in
            var offset = 0
            guard try readU32(bytes, &offset) == 1 else { throw PluginError.invalidArgument }
            let id = try readString(bytes, &offset, max: 64)
            let domain = try readString(bytes, &offset, max: 64)
            let title = try readString(bytes, &offset, max: 128)
            let description = try readString(bytes, &offset, max: 1024)
            let deepLink = try readString(bytes, &offset, max: 2048)
            try validateId(id)
            try validateId(domain)
            try validateText(title, max: 128, allowEmpty: false)
            try validateText(description, max: 1024, allowEmpty: true)
            try validateSafeUrl(deepLink)
            let keywordCount = try readU32(bytes, &offset)
            guard keywordCount <= 16 else { throw PluginError.tooLarge }
            var keywords: [String] = []
            keywords.reserveCapacity(Int(keywordCount))
            for _ in 0..<keywordCount {
                let kw = try readString(bytes, &offset, max: 64)
                try validateText(kw, max: 64, allowEmpty: false)
                keywords.append(kw)
            }
            let hasExp = try readU32(bytes, &offset)
            let expLo = try readU32(bytes, &offset)
            let expHi = try readU32(bytes, &offset)
            let expiration: UInt64?
            switch hasExp {
            case 0:
                guard expLo == 0, expHi == 0 else { throw PluginError.invalidArgument }
                expiration = nil
            case 1:
                expiration = UInt64(expLo) | (UInt64(expHi) << 32)
            default:
                throw PluginError.invalidArgument
            }
            guard offset == bytes.count else { throw PluginError.invalidArgument }
            return SpotlightItemPayload(
                id: id,
                domain: domain,
                title: title,
                description: description,
                deepLink: deepLink,
                keywords: keywords,
                expirationEpochSecs: expiration
            )
        }
    }

    private static func decodeUserActivity(_ payload: NativeBuffer) throws -> UserActivityPayload {
        try payload.withUnsafeBytes { bytes in
            var offset = 0
            guard try readU32(bytes, &offset) == 1 else { throw PluginError.invalidArgument }
            let activityType = try readString(bytes, &offset, max: 64)
            let title = try readString(bytes, &offset, max: 128)
            try validateId(activityType)
            try validateText(title, max: 128, allowEmpty: false)
            let webpageUrl: String?
            switch try readU32(bytes, &offset) {
            case 0:
                webpageUrl = nil
            case 1:
                let url = try readString(bytes, &offset, max: 2048)
                guard url.hasPrefix("https://"),
                      url.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) })
                else { throw PluginError.invalidArgument }
                webpageUrl = url
            default:
                throw PluginError.invalidArgument
            }
            let flags = try readU32(bytes, &offset)
            guard (flags & ~0b111) == 0 else { throw PluginError.invalidArgument }
            guard offset == bytes.count else { throw PluginError.invalidArgument }
            return UserActivityPayload(
                activityType: activityType,
                title: title,
                webpageUrl: webpageUrl,
                eligibleForHandoff: (flags & 1) != 0,
                eligibleForSearch: (flags & (1 << 1)) != 0,
                eligibleForPrediction: (flags & (1 << 2)) != 0
            )
        }
    }

    private static func encodeLaunch(_ id: String?) throws -> Data {
        let value = id ?? ""
        if !value.isEmpty { try validateId(value) }
        var data = Data()
        appendString(value, to: &data)
        return data
    }

    private static func encodeOptionalShare(_ share: IncomingSharePayload?) -> Data {
        var data = Data()
        guard let share else {
            appendU32(0, to: &data)
            return data
        }
        appendU32(1, to: &data) // Some
        appendU32(1, to: &data) // version
        appendOptString(share.text, to: &data)
        appendOptString(share.url, to: &data)
        appendOptString(share.subject, to: &data)
        appendU32(0, to: &data) // attachments
        return data
    }

    private static func appendU32(_ value: UInt32, to data: inout Data) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    private static func appendString(_ value: String, to data: inout Data) {
        let utf8 = Array(value.utf8)
        appendU32(UInt32(utf8.count), to: &data)
        data.append(contentsOf: utf8)
    }

    private static func appendOptString(_ value: String?, to data: inout Data) {
        if let value {
            appendU32(1, to: &data)
            appendString(value, to: &data)
        } else {
            appendU32(0, to: &data)
        }
    }

    private static func validateId(_ id: String) throws {
        guard !id.isEmpty, id.utf8.count <= 64 else { throw PluginError.invalidArgument }
        guard id.unicodeScalars.allSatisfy({
            CharacterSet.alphanumerics.contains($0) || "-_.".unicodeScalars.contains($0)
        }) else {
            throw PluginError.invalidArgument
        }
    }

    private static func validateText(_ value: String, max: Int, allowEmpty: Bool) throws {
        guard allowEmpty || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              value.utf8.count <= max
        else {
            throw PluginError.invalidArgument
        }

        guard value.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) else {
            throw PluginError.invalidArgument
        }
    }

    private static func validateSafeUrl(_ url: String) throws {
        guard !url.isEmpty, url.utf8.count <= 2048,
              url.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }),
              url.contains("://")
        else { throw PluginError.invalidArgument }
        let lower = url.lowercased()
        for blocked in ["javascript:", "data:", "file:", "blob:", "vbscript:"] where lower.hasPrefix(blocked) {
            throw PluginError.invalidArgument
        }
    }

    private static func readU32(_ bytes: UnsafeRawBufferPointer, _ offset: inout Int) throws -> UInt32 {
        guard offset + 4 <= bytes.count else { throw PluginError.invalidArgument }
        let value = bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self).littleEndian
        offset += 4

        return value
    }

    private static func readString(
        _ bytes: UnsafeRawBufferPointer,
        _ offset: inout Int,
        max: Int
    ) throws -> String {
        let length = Int(try readU32(bytes, &offset))

        guard length <= max, offset + length <= bytes.count else { throw PluginError.invalidArgument }
        let slice = bytes[offset..<(offset + length)]
        offset += length

        guard let value = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }

        return value
    }
}
