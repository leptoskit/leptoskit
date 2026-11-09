import Foundation
import LeptosKitBridge

enum PrintAvailability: UInt32 {
    case unsupported = 0
    case ready = 1
    case inactive = 2
    case busy = 3
}

enum PrintJob: Equatable {
    case html(String)
    case pdfUri(String)

    private static let maxHtmlBytes = 256 * 1024
    private static let maxUriBytes = 8192

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> PrintJob {
        var cursor = 0
        let kind = try readU32(bytes, cursor: &cursor)
        let body = try readString(bytes, cursor: &cursor)

        guard cursor == bytes.count else { throw PluginError.invalidArgument }

        switch kind {
        case 0:
            return try makeHtml(body)
        case 1:
            return try makePdfUri(body)
        default:
            throw PluginError.invalidArgument
        }
    }

    private static func makeHtml(_ value: String) throws -> PrintJob {
        guard value.utf8.count <= maxHtmlBytes else { throw PluginError.tooLarge }

        guard !value.contains("\0") else { throw PluginError.invalidArgument }

        return .html(value)
    }

    private static func makePdfUri(_ value: String) throws -> PrintJob {
        guard value.utf8.count <= maxUriBytes else { throw PluginError.tooLarge }

        try requireGraphic(value)
        try requireEscapes(value)

        guard let schemeEnd = value.range(of: "://") else { throw PluginError.invalidArgument }

        let scheme = value[..<schemeEnd.lowerBound]
        let rest = value[schemeEnd.upperBound...]

        switch scheme {
        case "https", "http":
            let authority = rest.split(whereSeparator: { "/?#".contains($0) }).first ?? ""
            guard !authority.isEmpty, !authority.contains("@") else { throw PluginError.invalidArgument }
        case "file":
            guard rest.hasPrefix("/"), !rest.contains("@") else { throw PluginError.invalidArgument }
        default:
            throw PluginError.invalidArgument
        }

        return .pdfUri(value)
    }
}

private func requireGraphic(_ value: String) throws {
    if value.utf8.contains(where: { byte in
        !(0x21...0x7E).contains(byte) || byte == UInt8(ascii: "\\")
    }) {
        throw PluginError.invalidArgument
    }
}

private func requireEscapes(_ value: String) throws {
    let utf8 = Array(value.utf8)
    var index = 0

    while index < utf8.count {
        if utf8[index] != UInt8(ascii: "%") {
            index += 1
            continue
        }

        guard index + 2 < utf8.count,
              isHex(utf8[index + 1]),
              isHex(utf8[index + 2]) else {
            throw PluginError.invalidArgument
        }

        index += 3
    }
}

private func isHex(_ byte: UInt8) -> Bool {
    (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte)
        || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains(byte)
        || (UInt8(ascii: "A")...UInt8(ascii: "F")).contains(byte)
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
