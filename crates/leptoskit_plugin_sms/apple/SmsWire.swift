import Foundation
import LeptosKitBridge

enum SmsAvailability: UInt32 {
    case unsupported = 0
    case absent = 1
    case ready = 2
    case inactive = 3
    case busy = 4
}

struct SmsCompose: Equatable {
    let recipients: [String]
    let body: String

    private static let maxRecipients = 32
    private static let maxRecipientBytes = 64
    private static let maxBodyBytes = 8 * 1024

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> SmsCompose {
        var cursor = 0
        let count = Int(try readU32(bytes, cursor: &cursor))

        guard count <= maxRecipients else { throw PluginError.tooLarge }

        var recipients: [String] = []
        recipients.reserveCapacity(count)

        for _ in 0..<count {
            recipients.append(try validateRecipient(try readString(bytes, cursor: &cursor)))
        }

        let body = try validateBody(try readString(bytes, cursor: &cursor))

        guard cursor == bytes.count else { throw PluginError.invalidArgument }

        return SmsCompose(recipients: recipients, body: body)
    }

    private static func validateRecipient(_ value: String) throws -> String {
        guard !value.isEmpty, value.utf8.count <= maxRecipientBytes else {
            throw PluginError.invalidArgument
        }

        guard value.unicodeScalars.allSatisfy({
            CharacterSet.decimalDigits.contains($0)
                || "+- ()".unicodeScalars.contains($0)
        }), value.contains(where: \.isNumber) else {
            throw PluginError.invalidArgument
        }

        return value
    }

    private static func validateBody(_ value: String) throws -> String {
        guard value.utf8.count <= maxBodyBytes else { throw PluginError.tooLarge }

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
