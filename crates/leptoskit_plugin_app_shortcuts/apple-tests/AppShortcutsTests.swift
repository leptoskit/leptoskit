import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitAppShortcuts
import Testing

@MainActor
private final class PublisherSpy: ShortcutPublisher {
    var items: [[AppShortcutItem]] = []
    var indexed: [SpotlightItemPayload] = []
    var deletedSpotlightIds: [String] = []
    var activeActivity: UserActivityPayload?
    var endedActivityCount = 0

    func set(_ items: [AppShortcutItem]) throws { self.items.append(items) }
    func clear() throws { items.append([]) }
    func indexSpotlight(_ item: SpotlightItemPayload) async throws { indexed.append(item) }
    func deleteSpotlight(id: String) async throws { deletedSpotlightIds.append(id) }
    func startUserActivity(_ activity: UserActivityPayload) throws { activeActivity = activity }
    func endUserActivity() {
        activeActivity = nil
        endedActivityCount += 1
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

private func appendU32(_ value: UInt32, to bytes: inout [UInt8]) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
}

private func appendString(_ value: String, to bytes: inout [UInt8]) {
    let utf8 = Array(value.utf8)
    appendU32(UInt32(utf8.count), to: &bytes)
    bytes.append(contentsOf: utf8)
}

private func encodeList(_ items: [(String, String, String)]) -> [UInt8] {
    var bytes: [UInt8] = []
    appendU32(UInt32(items.count), to: &bytes)

    for item in items {
        appendString(item.0, to: &bytes)
        appendString(item.1, to: &bytes)
        appendString(item.2, to: &bytes)
    }

    return bytes
}

@Test
@MainActor
func setClearAndConsumeLaunchUsePublisherWithoutSystemUI() async throws {
    let publisher = PublisherSpy()
    let plugin = AppShortcutsPlugin(publisher: publisher)
    #expect(DockShortcutStore.plugin === plugin)
    let encoded = encodeList([("inbox", "Inbox", "Mail"), ("compose", "Compose", "")])
    #expect(try await plugin.invoke(method: 0, payload: payload(encoded)) == Data())
    #expect(publisher.items.last?.count == 2)
    #expect(publisher.items.last?.first?.id == "inbox")
    AppShortcutsPlugin.noteLaunchIfPresent(id: "inbox")
    let launch = try await plugin.invoke(method: 2, payload: payload())
    #expect(String(data: launch.dropFirst(4), encoding: .utf8) == "inbox")
    let empty = try await plugin.invoke(method: 2, payload: payload())
    #expect(empty == Data([0, 0, 0, 0]))
    #expect(try await plugin.invoke(method: 1, payload: payload()) == Data())
    #expect(publisher.items.last?.isEmpty == true)
    #expect(try await plugin.invoke(method: 0, payload: payload(encoded)) == Data())
    #expect(publisher.items.last?.count == 2)
    plugin.close()
    #expect(publisher.items.last?.isEmpty == true)
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload(encoded))
    }
}

@Test
@MainActor
func spotlightHandoffAndIncomingShareRoundTrip() async throws {
    let publisher = PublisherSpy()
    let plugin = AppShortcutsPlugin(publisher: publisher)

    var spotlight: [UInt8] = []
    appendU32(1, to: &spotlight)
    appendString("note_1", to: &spotlight)
    appendString("notes", to: &spotlight)
    appendString("Roadmap", to: &spotlight)
    appendString("Q4 plans", to: &spotlight)
    appendString("myapp://notes/1", to: &spotlight)
    appendU32(1, to: &spotlight)
    appendString("plans", to: &spotlight)
    appendU32(0, to: &spotlight)
    appendU32(0, to: &spotlight)
    appendU32(0, to: &spotlight)

    #expect(try await plugin.invoke(method: 3, payload: payload(spotlight)) == Data())
    #expect(publisher.indexed.first?.id == "note_1")

    var deleteId: [UInt8] = []
    appendString("note_1", to: &deleteId)
    #expect(try await plugin.invoke(method: 4, payload: payload(deleteId)) == Data())
    #expect(publisher.deletedSpotlightIds == ["note_1"])

    var activity: [UInt8] = []
    appendU32(1, to: &activity)
    appendString("com.example.notes.view", to: &activity)
    appendString("Roadmap", to: &activity)
    appendU32(1, to: &activity)
    appendString("https://example.com/notes/1", to: &activity)
    appendU32(0b111, to: &activity)

    #expect(try await plugin.invoke(method: 5, payload: payload(activity)) == Data())
    #expect(publisher.activeActivity?.activityType == "com.example.notes.view")
    #expect(try await plugin.invoke(method: 6, payload: payload()) == Data())
    #expect(publisher.activeActivity == nil)

    try plugin.noteIncomingShare(text: "Hello", url: "https://leptoskit.dev", subject: "Docs")
    let received = try await plugin.invoke(method: 7, payload: payload())
    #expect(received.prefix(4) == Data([1, 0, 0, 0]))
    let cleared = try await plugin.invoke(method: 7, payload: payload())
    #expect(cleared == Data([0, 0, 0, 0]))
}

@Test
@MainActor
func malformedPayloadNeverReachesPublisher() async throws {
    let publisher = PublisherSpy()
    let plugin = AppShortcutsPlugin(publisher: publisher)
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([1]))
    }
    #expect(publisher.items.isEmpty)
}
