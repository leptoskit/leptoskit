import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitHomeWidgets
import Testing

@MainActor
private final class HomeWidgetSpy: HomeWidgetService {
    var sharedStoreReady = true
    var writes: [(String, Data)] = []
    var removed: [String] = []
    var reloads: [String?] = []
    var controlReloads: [String?] = []
    private var store: [String: Data] = [:]

    func write(kind: String, bytes: Data) {
        writes.append((kind, bytes))
        store[kind] = bytes
    }

    func read(kind: String) -> Data? { store[kind] }

    func remove(kind: String) {
        removed.append(kind)
        store.removeValue(forKey: kind)
    }

    func kinds() -> [String] { Array(store.keys) }

    func reload(kind: String?) {
        reloads.append(kind)
    }

    func reloadControls(kind: String?) {
        controlReloads.append(kind)
    }
}

private func payload(_ bytes: [UInt8] = []) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }

    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

private func lengthPrefixed(_ value: String) -> [UInt8] {
    var bytes: [UInt8] = []
    let utf8 = Array(value.utf8)
    let length = UInt32(utf8.count).littleEndian
    withUnsafeBytes(of: length) { bytes.append(contentsOf: $0) }
    bytes.append(contentsOf: utf8)

    return bytes
}

private func appendU32(_ value: UInt32, to bytes: inout [UInt8]) {
    withUnsafeBytes(of: value.littleEndian) { bytes.append(contentsOf: $0) }
}

private func encodeSnapshot(
    kind: String = "status",
    family: UInt32 = 2,
    title: String = "Inbox",
    body: String = "3 unread",
    url: String = "myapp://inbox"
) -> [UInt8] {
    var bytes = lengthPrefixed(kind)
    appendU32(family, to: &bytes)
    bytes += lengthPrefixed(title)
    bytes += lengthPrefixed(body)
    bytes += lengthPrefixed(url)

    return bytes
}

private func encodeControl(
    id: String = "focus_mode",
    title: String = "Focus",
    valueLabel: String = "25m",
    symbolName: String = "timer",
    kindTag: UInt32 = 2,
    actionId: String = "toggle_focus"
) -> [UInt8] {
    var bytes: [UInt8] = []
    appendU32(1, to: &bytes)
    bytes += lengthPrefixed(id)
    bytes += lengthPrefixed(title)
    bytes += lengthPrefixed(valueLabel)
    bytes += lengthPrefixed(symbolName)
    appendU32(kindTag, to: &bytes)
    bytes += lengthPrefixed(actionId)
    return bytes
}

@Test
@MainActor
func capabilitySetGetClearReloadAndClose() async throws {
    let service = HomeWidgetSpy()
    let plugin = HomeWidgetsPlugin(service: service)
    let kindOnly = lengthPrefixed("status")
    let encoded = encodeSnapshot()

    #expect(try await plugin.invoke(method: 0, payload: payload()) == Data([1 | (1 << 1) | (1 << 3), 0, 0, 0]))
    #expect(try await plugin.invoke(method: 1, payload: payload(encoded)) == Data())
    #expect(service.writes.count == 1)
    #expect(service.writes[0].0 == "status")
    #expect(try await plugin.invoke(method: 2, payload: payload(kindOnly)) == Data(encoded))
    #expect(LeptosKitWidgetStore.decodeSnapshot(Data(encoded))?.title == "Inbox")
    #expect(try await plugin.invoke(method: 5, payload: payload(kindOnly)) == Data([0, 0, 0, 0]))
    #expect(try await plugin.invoke(method: 4, payload: payload(kindOnly)) == Data())
    #expect(service.reloads == ["status"])
    #expect(try await plugin.invoke(method: 4, payload: payload()) == Data())
    #expect(service.reloads == ["status", nil])
    #expect(try await plugin.invoke(method: 3, payload: payload(kindOnly)) == Data())
    #expect(service.removed == ["status"])

    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func timelineControlCenterAndQuickTileBoundary() async throws {
    let service = HomeWidgetSpy()
    let plugin = HomeWidgetsPlugin(service: service)

    let snapshot = encodeSnapshot(kind: "steps", family: 1, title: "Steps", body: "5,000", url: "")
    var timeline: [UInt8] = []
    appendU32(1, to: &timeline) // version
    appendU32(2, to: &timeline) // LockScreenRectangular
    appendU32(1, to: &timeline) // AfterSeconds
    appendU32(900, to: &timeline)
    appendU32(1, to: &timeline) // 1 entry
    appendU32(1_700_000_000, to: &timeline) // lo
    appendU32(0, to: &timeline) // hi
    appendU32(UInt32(snapshot.count), to: &timeline)
    timeline += snapshot

    #expect(try await plugin.invoke(method: 6, payload: payload(timeline)) == Data())
    #expect(try await plugin.invoke(method: 7, payload: payload(lengthPrefixed("steps"))) == Data(timeline))

    let control = encodeControl()
    #expect(try await plugin.invoke(method: 8, payload: payload(control)) == Data())
    let readControl = try await plugin.invoke(method: 9, payload: payload(lengthPrefixed("focus_mode")))
    #expect(readControl == Data(control))
    #expect(LeptosKitWidgetStore.decodeControl(readControl)?.isOn == true)

    #expect(try await plugin.invoke(method: 10, payload: payload(lengthPrefixed("focus_mode"))) == Data())
    #expect(try await plugin.invoke(method: 10, payload: payload()) == Data())
    #expect(service.controlReloads == ["focus_mode", nil])

    // Android Quick Settings tiles validate then answer Unavailable on Apple.
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 12, payload: payload(lengthPrefixed("quick_capture")))
    }
    plugin.close()
}

@Test
@MainActor
func invalidPayloadsNeverTouchStorage() async throws {
    let service = HomeWidgetSpy()
    let plugin = HomeWidgetsPlugin(service: service)

    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([0]))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: payload([0, 0, 0, 1, 0]))
    }
    #expect(service.writes.isEmpty)
    #expect(service.reloads.isEmpty)
    plugin.close()
}
