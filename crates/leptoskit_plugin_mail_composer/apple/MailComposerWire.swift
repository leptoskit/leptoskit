import Foundation
import LeptosKitBridge

enum MailAvailability: UInt32 {
    case unsupported = 0
    case absent = 1
    case ready = 2
    case inactive = 3
    case busy = 4
}

struct MailCompose: Equatable {
    let to: [String]
    let cc: [String]
    let bcc: [String]
    let subject: String
    let body: String

    private static let maxAddresses = 32
    private static let maxAddressBytes = 320
    private static let maxSubjectBytes = 998
    private static let maxBodyBytes = 64 * 1024

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> MailCompose {
        var cursor = 0
        let to = try readList(bytes, cursor: &cursor)
        let cc = try readList(bytes, cursor: &cursor)
        let bcc = try readList(bytes, cursor: &cursor)
        let subject = try bounded(try readString(bytes, cursor: &cursor), max: maxSubjectBytes)
        let body = try bounded(try readString(bytes, cursor: &cursor), max: maxBodyBytes)

        guard cursor == bytes.count else { throw PluginError.invalidArgument }

        return MailCompose(to: to, cc: cc, bcc: bcc, subject: subject, body: body)
    }

    private static func readList(
        _ bytes: UnsafeRawBufferPointer,
        cursor: inout Int
    ) throws -> [String] {
        let count = Int(try readU32(bytes, cursor: &cursor))

        guard count <= maxAddresses else { throw PluginError.tooLarge }

        var values: [String] = []
        values.reserveCapacity(count)

        for _ in 0..<count {
            values.append(try validateAddress(try readString(bytes, cursor: &cursor)))
        }

        return values
    }

    private static func validateAddress(_ value: String) throws -> String {
        guard !value.isEmpty, value.utf8.count <= maxAddressBytes, !value.contains("\0") else {
            throw PluginError.invalidArgument
        }

        guard value.allSatisfy(\.isASCII), !value.contains(where: \.isWhitespace) else {
            throw PluginError.invalidArgument
        }

        let parts = value.split(separator: "@", omittingEmptySubsequences: false)

        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else {
            throw PluginError.invalidArgument
        }

        return value
    }

    private static func bounded(_ value: String, max: Int) throws -> String {
        guard value.utf8.count <= max else { throw PluginError.tooLarge }

        guard !value.contains("\0") else { throw PluginError.invalidArgument }

        return value
    }
}

private func readU32(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> UInt32 {
    guard cursor + 4 <= bytes.count else { throw PluginError.invalidArgument }

    let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: cursor, as: UInt32.self))
    cursor += 4

    return value
}

private func readString(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> String {
    let count = Int(try readU32(bytes, cursor: &cursor))

    guard cursor + count <= bytes.count else { throw PluginError.invalidArgument }

    let slice = UnsafeRawBufferPointer(rebasing: bytes[cursor..<(cursor + count)])
    cursor += count

    guard let value = String(bytes: slice, encoding: .utf8) else {
        throw PluginError.invalidArgument
    }

    return value
}
