import Foundation
import LeptosKitBridge

enum CalendarStatus: UInt32 {
    case unknown = 0
    case prompt = 1
    case denied = 2
    case granted = 3
    case restricted = 4
}

enum CalendarMethod: UInt32 {
    case status = 0
    case requestPermission = 1
    case listCalendars = 2
    case createEvent = 3
    case getEvent = 4
    case listEvents = 5
}

struct CalendarRecord: Equatable {
    let id: String
    let title: String
    let allowsModifications: Bool

    private static let maxIDBytes = 256
    private static let maxTitleBytes = 256
    static let maxCalendars = 64

    init(id: String, title: String, allowsModifications: Bool) throws {
        self.id = try Self.required(id, max: Self.maxIDBytes)
        self.title = try Self.required(title, max: Self.maxTitleBytes)
        self.allowsModifications = allowsModifications
    }

    func encode() throws -> Data {
        var result = Data(capacity: 32)
        try write(&result, id)
        try write(&result, title)
        write(&result, allowsModifications ? UInt32(1) : UInt32(0))

        return result
    }

    static func encodeList(_ calendars: [CalendarRecord]) throws -> Data {
        guard calendars.count <= maxCalendars else { throw PluginError.tooLarge }

        var result = Data(capacity: 4 + calendars.count * 32)
        write(&result, UInt32(calendars.count))
        for calendar in calendars {
            result.append(try calendar.encode())
        }

        return result
    }
}

struct CalendarEventRecord: Equatable {
    let id: String
    let calendarID: String
    let title: String
    let startMs: Int64
    let endMs: Int64
    let notes: String?
    let location: String?

    private static let maxIDBytes = 256
    private static let maxTitleBytes = 256
    private static let maxNotesBytes = 4096
    private static let maxLocationBytes = 512
    static let maxEvents = 64
    static let maxWindowMs: Int64 = 31_622_400_000

    init(
        id: String,
        calendarID: String,
        title: String,
        startMs: Int64,
        endMs: Int64,
        notes: String?,
        location: String?
    ) throws {
        guard endMs >= startMs else { throw PluginError.invalidArgument }

        self.id = try Self.required(id, max: Self.maxIDBytes)
        self.calendarID = try Self.required(calendarID, max: Self.maxIDBytes)
        self.title = try Self.required(title, max: Self.maxTitleBytes)
        self.startMs = startMs
        self.endMs = endMs
        self.notes = try Self.optional(notes, max: Self.maxNotesBytes)
        self.location = try Self.optional(location, max: Self.maxLocationBytes)
    }

    func encode() throws -> Data {
        var result = Data(capacity: 64)
        try write(&result, id)
        try write(&result, calendarID)
        try write(&result, title)
        write(&result, startMs)
        write(&result, endMs)
        try write(&result, notes ?? "")
        try write(&result, location ?? "")

        return result
    }

    static func encodeList(_ events: [CalendarEventRecord]) throws -> Data {
        guard events.count <= maxEvents else { throw PluginError.tooLarge }

        var result = Data(capacity: 4 + events.count * 48)
        write(&result, UInt32(events.count))
        for event in events {
            result.append(try event.encode())
        }

        return result
    }

    static func decodeCreate(_ bytes: UnsafeRawBufferPointer) throws -> (
        calendarID: String,
        title: String,
        startMs: Int64,
        endMs: Int64,
        notes: String?,
        location: String?
    ) {
        var cursor = 0
        let calendarID = try required(readString(bytes, cursor: &cursor), max: maxIDBytes)
        let title = try required(readString(bytes, cursor: &cursor), max: maxTitleBytes)
        let startMs = try readI64(bytes, cursor: &cursor)
        let endMs = try readI64(bytes, cursor: &cursor)
        guard endMs >= startMs else { throw PluginError.invalidArgument }

        let notes = try optional(readString(bytes, cursor: &cursor), max: maxNotesBytes)
        let location = try optional(readString(bytes, cursor: &cursor), max: maxLocationBytes)
        guard cursor == bytes.count else { throw PluginError.invalidArgument }

        return (calendarID, title, startMs, endMs, notes, location)
    }

    static func decodeEventID(_ bytes: UnsafeRawBufferPointer) throws -> String {
        var cursor = 0
        let id = try required(readString(bytes, cursor: &cursor), max: maxIDBytes)
        guard cursor == bytes.count else { throw PluginError.invalidArgument }

        return id
    }

    static func encodeEventID(_ id: String) throws -> Data {
        var result = Data(capacity: 8 + id.utf8.count)
        try write(&result, required(id, max: maxIDBytes))

        return result
    }

    static func decodeListQuery(_ bytes: UnsafeRawBufferPointer) throws -> (
        startMs: Int64,
        endMs: Int64,
        limit: Int
    ) {
        var cursor = 0
        let startMs = try readI64(bytes, cursor: &cursor)
        let endMs = try readI64(bytes, cursor: &cursor)
        let limit = Int(try readU32(bytes, cursor: &cursor))
        guard cursor == bytes.count else { throw PluginError.invalidArgument }
        guard endMs >= startMs else { throw PluginError.invalidArgument }

        let span = endMs &- startMs
        guard span >= 0, span <= maxWindowMs else { throw PluginError.invalidArgument }
        guard limit > 0, limit <= maxEvents else { throw PluginError.invalidArgument }

        return (startMs, endMs, limit)
    }

    private static func required(_ value: String, max: Int) throws -> String {
        guard !value.isEmpty, value.utf8.count <= max, !value.contains("\0") else {
            throw PluginError.invalidArgument
        }

        return value
    }

    private static func optional(_ value: String, max: Int) throws -> String? {
        if value.isEmpty { return nil }
        guard value.utf8.count <= max, !value.contains("\0") else {
            throw PluginError.invalidArgument
        }

        return value
    }
}

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

private func readU32(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> UInt32 {
    guard cursor + 4 <= bytes.count else { throw PluginError.invalidArgument }

    let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: cursor, as: UInt32.self))
    cursor += 4

    return value
}

private func readI64(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> Int64 {
    guard cursor + 8 <= bytes.count else { throw PluginError.invalidArgument }

    let value = Int64(littleEndian: bytes.loadUnaligned(fromByteOffset: cursor, as: Int64.self))
    cursor += 8

    return value
}

private func readString(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> String {
    let size = Int(try readU32(bytes, cursor: &cursor))
    guard size >= 0, cursor + size <= bytes.count else { throw PluginError.invalidArgument }

    let slice = Data(bytes[cursor ..< (cursor + size)])
    cursor += size
    guard let text = String(data: slice, encoding: .utf8) else { throw PluginError.invalidArgument }

    return text
}
