import Foundation
import LeptosKitBridge
import WidgetKit

/// Shared storage and WidgetKit reload surface used by the plugin.
@MainActor
protocol HomeWidgetService {
    var sharedStoreReady: Bool { get }
    func write(kind: String, bytes: Data)
    func read(kind: String) -> Data?
    func remove(kind: String)
    func kinds() -> [String]
    func reload(kind: String?)
    func reloadControls(kind: String?)
}

extension HomeWidgetService {
    func reloadControls(kind: String?) {
        _ = kind
    }
}

/// Decoded home/lock-screen snapshot for WidgetKit extension targets.
public struct LeptosKitWidgetSnapshot: Equatable, Sendable {
    public let kind: String
    public let family: UInt32
    public let title: String
    public let body: String
    public let url: String

    public init(kind: String, family: UInt32, title: String, body: String, url: String) {
        self.kind = kind
        self.family = family
        self.title = title
        self.body = body
        self.url = url
    }
}

/// Decoded iOS 18+ Control Center descriptor for `ControlWidget` targets.
public struct LeptosKitControlState: Equatable, Sendable {
    public let id: String
    public let title: String
    public let valueLabel: String
    public let symbolName: String
    /// `0` = Button, `1` = Toggle(off), `2` = Toggle(on).
    public let kindTag: UInt32
    public let actionId: String

    public var isOn: Bool { kindTag == 2 }
    public var isToggle: Bool { kindTag == 1 || kindTag == 2 }

    public init(
        id: String,
        title: String,
        valueLabel: String,
        symbolName: String,
        kindTag: UInt32,
        actionId: String
    ) {
        self.id = id
        self.title = title
        self.valueLabel = valueLabel
        self.symbolName = symbolName
        self.kindTag = kindTag
        self.actionId = actionId
    }
}

/// Helper for WidgetKit and ControlWidget extensions to read data published by
/// `HomeWidgetsPlugin` from the shared App Group `UserDefaults`.
public enum LeptosKitWidgetStore {
    public static let storagePrefix = "leptoskit.home-widgets."
    public static let timelineKeyPrefix = "timeline."
    public static let controlKeyPrefix = "control."

    /// Resolves the App Group identifier from an explicit value or the
    /// `LeptosKitAppGroup` key in `Bundle.main`'s `Info.plist`.
    public static func resolvedAppGroup(_ explicit: String? = nil) -> String? {
        if let explicit, !explicit.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return explicit
        }
        guard let raw = Bundle.main.object(forInfoDictionaryKey: "LeptosKitAppGroup") as? String else {
            return nil
        }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// Reads the latest snapshot published for `kind` from the App Group suite
    /// (falling back to `.standard` if no App Group is configured).
    public static func readSnapshot(kind: String, appGroup: String? = nil) -> LeptosKitWidgetSnapshot? {
        let defaults = sharedDefaults(appGroup: appGroup)
        guard let data = defaults.data(forKey: storagePrefix + kind) else { return nil }
        return decodeSnapshot(data)
    }

    /// Reads the latest iOS 18+ Control Center descriptor published for `id`.
    public static func readControl(id: String, appGroup: String? = nil) -> LeptosKitControlState? {
        let defaults = sharedDefaults(appGroup: appGroup)
        guard let data = defaults.data(forKey: storagePrefix + controlKeyPrefix + id) else {
            return nil
        }
        return decodeControl(data)
    }

    /// Decodes raw snapshot wire bytes written by `HomeWidgetsPlugin`.
    public static func decodeSnapshot(_ data: Data) -> LeptosKitWidgetSnapshot? {
        data.withUnsafeBytes { bytes in
            guard let snapshot = try? Snapshot(bytes) else { return nil }
            return LeptosKitWidgetSnapshot(
                kind: snapshot.kind,
                family: snapshot.family,
                title: snapshot.title,
                body: snapshot.body,
                url: snapshot.url
            )
        }
    }

    /// Decodes raw Control Center wire bytes written by `HomeWidgetsPlugin`.
    public static func decodeControl(_ data: Data) -> LeptosKitControlState? {
        data.withUnsafeBytes { bytes in
            guard let control = try? ControlPayload(bytes) else { return nil }
            return LeptosKitControlState(
                id: control.id,
                title: control.title,
                valueLabel: control.valueLabel,
                symbolName: control.symbolName,
                kindTag: control.kindTag,
                actionId: control.actionId
            )
        }
    }

    private static func sharedDefaults(appGroup: String?) -> UserDefaults {
        if let group = resolvedAppGroup(appGroup), let suite = UserDefaults(suiteName: group) {
            return suite
        }
        return .standard
    }
}

@MainActor
private final class SystemHomeWidgetService: HomeWidgetService {
    static let prefix = LeptosKitWidgetStore.storagePrefix
    private let defaults: UserDefaults
    let sharedStoreReady: Bool

    init(appGroup: String?) {
        if let resolved = LeptosKitWidgetStore.resolvedAppGroup(appGroup),
           let suite = UserDefaults(suiteName: resolved) {
            defaults = suite
            sharedStoreReady = true
        } else {
            defaults = .standard
            sharedStoreReady = false
        }
    }

    func write(kind: String, bytes: Data) {
        defaults.set(bytes, forKey: Self.prefix + kind)
    }

    func read(kind: String) -> Data? {
        defaults.data(forKey: Self.prefix + kind)
    }

    func remove(kind: String) {
        defaults.removeObject(forKey: Self.prefix + kind)
    }

    func kinds() -> [String] {
        defaults.dictionaryRepresentation().keys.compactMap { key in

            guard key.hasPrefix(Self.prefix) else { return nil }
            return String(key.dropFirst(Self.prefix.count))
        }
    }

    func reload(kind: String?) {
        if let kind {
            WidgetCenter.shared.reloadTimelines(ofKind: kind)
        } else {
            WidgetCenter.shared.reloadAllTimelines()
        }
    }

    func reloadControls(kind: String?) {
        #if os(iOS)
        if #available(iOS 18.0, *) {
            if let kind {
                ControlCenter.shared.reloadControls(ofKind: kind)
            } else {
                ControlCenter.shared.reloadAllControls()
            }
        }
        #else
        _ = kind
        #endif
    }
}

/// Publishes snapshot, timeline, and Control Center bytes for WidgetKit
/// extensions and asks WidgetCenter / ControlCenter to reload.
/// Does not create, pin, or draw a widget; the app still owns its Widget extension.
@MainActor
public final class HomeWidgetsPlugin: NativePlugin {
    public let name = "home-widgets"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    private let service: any HomeWidgetService
    private var closed = false

    /// Reads `LeptosKitAppGroup` from `Info.plist` when `appGroup` is nil,
    /// falling back to standard `UserDefaults`.
    public init(appGroup: String? = nil) {
        service = SystemHomeWidgetService(appGroup: appGroup)
    }

    init(service: any HomeWidgetService) {
        self.service = service
    }

    public func close() {
        closed = true
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return capability()
        case 1:
            let snapshot = try payload.withUnsafeBytes { try Snapshot($0) }
            let bytes = payload.withUnsafeBytes { Data($0) }
            service.write(kind: snapshot.kind, bytes: bytes)
            return Data()
        case 2:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }

            guard let bytes = service.read(kind: kind) else { throw PluginError.unavailable }
            return bytes
        case 3:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }
            service.remove(kind: kind)
            return Data()
        case 4:
            let kind = try payload.withUnsafeBytes { try optionalKind($0) }
            service.reload(kind: kind)
            return Data()
        case 5:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }
            _ = kind
            // WidgetKit has no public pin-state query.
            return Data([0, 0, 0, 0])
        case 6:
            let timeline = try payload.withUnsafeBytes { try TimelinePayload($0) }
            let bytes = payload.withUnsafeBytes { Data($0) }
            service.write(kind: LeptosKitWidgetStore.timelineKeyPrefix + timeline.kind, bytes: bytes)
            return Data()
        case 7:
            let kind = try payload.withUnsafeBytes { try KindReader($0).kind }
            guard let bytes = service.read(kind: LeptosKitWidgetStore.timelineKeyPrefix + kind) else {
                throw PluginError.unavailable
            }
            return bytes
        case 8:
            let control = try payload.withUnsafeBytes { try ControlPayload($0) }
            let bytes = payload.withUnsafeBytes { Data($0) }
            service.write(kind: LeptosKitWidgetStore.controlKeyPrefix + control.id, bytes: bytes)
            return Data()
        case 9:
            let id = try payload.withUnsafeBytes { try KindReader($0).kind }
            guard let bytes = service.read(kind: LeptosKitWidgetStore.controlKeyPrefix + id) else {
                throw PluginError.unavailable
            }
            return bytes
        case 10:
            let kind = try payload.withUnsafeBytes { try optionalKind($0) }
            service.reloadControls(kind: kind)
            return Data()
        case 11:
            _ = try payload.withUnsafeBytes { try QuickTilePayload($0) }
            throw PluginError.unavailable
        case 12:
            _ = try payload.withUnsafeBytes { try KindReader($0).kind }
            throw PluginError.unavailable
        default:
            throw PluginError.unavailable
        }
    }

    private func capability() -> Data {
        var flags: UInt32 = 1 | (1 << 1) // supported + can_reload

        if service.sharedStoreReady {
            flags |= 1 << 3
        }

        var data = Data(count: 4)
        withUnsafeBytes(of: flags.littleEndian) { data.replaceSubrange(0..<4, with: $0) }

        return data
    }
}

private func optionalKind(_ bytes: UnsafeRawBufferPointer) throws -> String? {
    if bytes.isEmpty { return nil }
    return try KindReader(bytes).kind
}

private struct KindReader {
    let kind: String
    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 68)
        kind = try reader.identifier()
        try reader.finish()
    }
}

private struct Snapshot {
    let kind: String
    let family: UInt32
    let title: String
    let body: String
    let url: String

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 16 + 64 + 256 + 1024 + 2048)
        kind = try reader.identifier()
        family = try reader.u32()

        guard (0...4).contains(family) else { throw PluginError.invalidArgument }
        title = try reader.text(maximum: 256, multiline: false, required: true)
        body = try reader.text(maximum: 1024, multiline: true, required: false)
        url = try reader.text(maximum: 2048, multiline: false, required: false)

        if !url.isEmpty {
            guard isAllowedURL(url) else { throw PluginError.invalidArgument }
        }

        try reader.finish()
    }
}

private struct TimelinePayload {
    let kind: String

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 32 * 4096)
        guard try reader.u32() == 1 else { throw PluginError.invalidArgument }
        let surface = try reader.u32()
        guard (0...6).contains(surface) else { throw PluginError.invalidArgument }
        let policyTag = try reader.u32()
        let policySeconds = try reader.u32()
        switch policyTag {
        case 0, 2:
            guard policySeconds == 0 else { throw PluginError.invalidArgument }
        case 1:
            guard policySeconds > 0 else { throw PluginError.invalidArgument }
        default:
            throw PluginError.invalidArgument
        }
        let count = Int(try reader.u32())
        guard count > 0 else { throw PluginError.invalidArgument }
        guard count <= 32 else { throw PluginError.tooLarge }
        var expectedKind: String?
        var previousEpoch: UInt64 = 0
        for index in 0..<count {
            let lo = UInt64(try reader.u32())
            let hi = UInt64(try reader.u32())
            let epoch = lo | (hi << 32)
            if index > 0 && epoch < previousEpoch {
                throw PluginError.invalidArgument
            }
            previousEpoch = epoch
            let snapshotData = try reader.rawBytes(maximum: 16 + 64 + 256 + 1024 + 2048)
            let snapshot = try snapshotData.withUnsafeBytes { try Snapshot($0) }
            if let expectedKind {
                guard snapshot.kind == expectedKind else { throw PluginError.invalidArgument }
            } else {
                expectedKind = snapshot.kind
            }
        }
        try reader.finish()
        kind = expectedKind!
    }
}

private struct ControlPayload {
    let id: String
    let title: String
    let valueLabel: String
    let symbolName: String
    let kindTag: UInt32
    let actionId: String

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 2048)
        guard try reader.u32() == 1 else { throw PluginError.invalidArgument }
        id = try reader.identifier()
        title = try reader.text(maximum: 128, multiline: false, required: true)
        valueLabel = try reader.text(maximum: 128, multiline: false, required: false)
        symbolName = try reader.identifier()
        kindTag = try reader.u32()
        guard (0...2).contains(kindTag) else { throw PluginError.invalidArgument }
        actionId = try reader.identifier()
        try reader.finish()
    }
}

private struct QuickTilePayload {
    let id: String

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 2048)
        guard try reader.u32() == 1 else { throw PluginError.invalidArgument }
        id = try reader.identifier()
        _ = try reader.text(maximum: 128, multiline: false, required: true)
        _ = try reader.text(maximum: 128, multiline: false, required: false)
        _ = try reader.identifier()
        let state = try reader.u32()
        guard (0...2).contains(state) else { throw PluginError.invalidArgument }
        _ = try reader.identifier()
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

    mutating func u32() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        offset += 4

        return value
    }

    mutating func rawBytes(maximum: Int) throws -> Data {
        let length = Int(try u32())
        guard length <= maximum, length <= bytes.count - offset else {
            throw PluginError.invalidArgument
        }
        let slice = Data(bytes[offset..<(offset + length)])
        offset += length
        return slice
    }

    mutating func identifier() throws -> String {
        let value = try text(maximum: 64, multiline: false, required: true)
        guard value.utf8.allSatisfy({
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
                || [45, 46, 95].contains($0)
        }) else { throw PluginError.invalidArgument }

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

              !value.unicodeScalars.contains(where: {
                  ($0.value <= 31 || (127...159).contains($0.value))
                      && !(multiline && ($0.value == 10 || $0.value == 9))
              }) else { throw PluginError.invalidArgument }

        return value
    }

    func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}

/// HTTP(S) or an app deep-link scheme; rejects javascript/data/file/blob/vbscript.
private func isAllowedURL(_ url: String) -> Bool {
    if url.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }) {
        return false
    }

    guard let separator = url.firstIndex(of: ":"),
          separator > url.startIndex,
          separator < url.index(before: url.endIndex)
    else { return false }

    let scheme = String(url[..<separator]).lowercased()

    guard let first = scheme.unicodeScalars.first,
          (97...122).contains(first.value)
    else { return false }

    guard scheme.unicodeScalars.dropFirst().allSatisfy({
        let value = $0.value
        return (97...122).contains(value)
            || (48...57).contains(value)
            || value == 43 || value == 45 || value == 46
    }) else { return false }

    switch scheme {
    case "javascript", "data", "file", "blob", "vbscript":
        return false
    default:
        return true
    }
}
