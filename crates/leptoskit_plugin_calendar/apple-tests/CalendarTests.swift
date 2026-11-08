import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitCalendar
import Testing

private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
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

@Test
func calendarWireRoundTripsAndRejectsNul() throws {
    let calendar = try CalendarRecord(id: "1", title: "Work", allowsModifications: true)
    let encoded = try calendar.encode()
    #expect(encoded.count > 8)

    #expect(throws: PluginError.invalidArgument) {
        _ = try CalendarRecord(id: "", title: "Work", allowsModifications: false)
    }
    #expect(throws: PluginError.invalidArgument) {
        _ = try CalendarRecord(id: "1\0", title: "Work", allowsModifications: false)
    }

    let event = try CalendarEventRecord(
        id: "e1",
        calendarID: "1",
        title: "Standup",
        startMs: 1_700_000_000_000,
        endMs: 1_700_000_360_000,
        notes: "daily",
        location: "Room A"
    )
    let eventBytes = try event.encode()
    #expect(eventBytes.count > 16)

    #expect(throws: PluginError.invalidArgument) {
        _ = try CalendarEventRecord(
            id: "e1",
            calendarID: "1",
            title: "Standup",
            startMs: 2,
            endMs: 1,
            notes: nil,
            location: nil
        )
    }

    var create = Data()
    try write(&create, "1")
    try write(&create, "Standup")
    write(&create, Int64(10))
    write(&create, Int64(20))
    try write(&create, "notes")
    try write(&create, "")
    let decoded = try create.withUnsafeBytes { try CalendarEventRecord.decodeCreate($0) }
    #expect(decoded.calendarID == "1")
    #expect(decoded.title == "Standup")
    #expect(decoded.notes == "notes")
    #expect(decoded.location == nil)

    var query = Data()
    write(&query, Int64(0))
    write(&query, Int64(1_000))
    write(&query, UInt32(8))
    let list = try query.withUnsafeBytes { try CalendarEventRecord.decodeListQuery($0) }
    #expect(list.limit == 8)

    var zero = Data()
    write(&zero, Int64(0))
    write(&zero, Int64(1))
    write(&zero, UInt32(0))
    #expect(throws: PluginError.invalidArgument) {
        _ = try zero.withUnsafeBytes { try CalendarEventRecord.decodeListQuery($0) }
    }

    let listBytes = try CalendarRecord.encodeList([calendar])
    #expect(listBytes.count >= 4)
    let eventList = try CalendarEventRecord.encodeList([event])
    #expect(eventList.count >= 4)
    let id = try CalendarEventRecord.encodeEventID("evt-9")
    #expect(try id.withUnsafeBytes { try CalendarEventRecord.decodeEventID($0) } == "evt-9")
}

#if canImport(UIKit)
import UIKit

@Test
@MainActor
func closedPluginRejectsFurtherCalls() async throws {
    let owner = UIViewController()
    let plugin = CalendarPlugin(context: NativePluginContext(owner: owner))
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        _ = try await plugin.invoke(method: 0, payload: payload([]))
    }
}

@Test
@MainActor
func malformedCreateNeverOpensEventStore() async throws {
    let owner = UIViewController()
    let plugin = CalendarPlugin(context: NativePluginContext(owner: owner))
    defer { plugin.close() }
    await #expect(throws: PluginError.invalidArgument) {
        _ = try await plugin.invoke(method: 3, payload: payload([0, 0, 0]))
    }
}
#endif

private func write(_ result: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
}

private func write(_ result: inout Data, _ value: Int64) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
}

private func write(_ result: inout Data, _ text: String) throws {
    let bytes = Data(text.utf8)
    write(&result, UInt32(bytes.count))
    result.append(bytes)
}
