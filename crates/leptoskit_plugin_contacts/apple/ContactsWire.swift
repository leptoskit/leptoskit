import Foundation
import LeptosKitBridge

enum ContactsStatus: UInt32 {
    case unknown = 0
    case prompt = 1
    case denied = 2
    case granted = 3
    case restricted = 4
}

enum ContactsMethod: UInt32 {
    case status = 0
    case requestPermission = 1
    case pick = 2
    case query = 3
}

struct ContactsRecord: Equatable {
    let id: String
    let displayName: String
    let phone: String?
    let email: String?

    private static let maxIDBytes = 256
    private static let maxNameBytes = 256
    private static let maxPhoneBytes = 64
    private static let maxEmailBytes = 320
    static let maxQuery = 32

    init(id: String, displayName: String, phone: String?, email: String?) throws {
        self.id = try Self.required(id, max: Self.maxIDBytes)
        self.displayName = try Self.required(displayName, max: Self.maxNameBytes)
        self.phone = try Self.optional(phone, max: Self.maxPhoneBytes)
        self.email = try Self.optional(email, max: Self.maxEmailBytes)
    }

    func encode() throws -> Data {
        var result = Data(capacity: 64)
        try write(&result, id)
        try write(&result, displayName)
        try write(&result, phone ?? "")
        try write(&result, email ?? "")

        return result
    }

    static func decodeQueryLimit(_ bytes: UnsafeRawBufferPointer) throws -> Int {
        var cursor = 0
        let limit = Int(try readU32(bytes, cursor: &cursor))
        guard cursor == bytes.count else { throw PluginError.invalidArgument }
        guard limit > 0, limit <= maxQuery else { throw PluginError.invalidArgument }

        return limit
    }

    static func encodeList(_ contacts: [ContactsRecord]) throws -> Data {
        guard contacts.count <= maxQuery else { throw PluginError.tooLarge }

        var result = Data(capacity: 4 + contacts.count * 32)
        write(&result, UInt32(contacts.count))
        for contact in contacts {
            result.append(try contact.encode())
        }

        return result
    }

    private static func required(_ value: String, max: Int) throws -> String {
        guard !value.isEmpty, value.utf8.count <= max, !value.contains("\0") else {
            throw PluginError.invalidArgument
        }

        return value
    }

    private static func optional(_ value: String?, max: Int) throws -> String? {
        guard let value else { return nil }
        guard !value.isEmpty, value.utf8.count <= max, !value.contains("\0") else {
            throw PluginError.invalidArgument
        }

        return value
    }
}

private func write(_ result: inout Data, _ value: UInt32) {
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
