import Foundation
import LeptosKitBridge

enum WebBrowserAvailability: UInt32 {
    case unsupported = 0
    case ready = 1
    case inactive = 2
    case busy = 3
}

struct WebBrowserOpen: Equatable {
    let url: URL

    private static let maxUrlBytes = 8192
    private static let firstPrintable: UInt8 = 33
    private static let lastPrintable: UInt8 = 126
    private static let backslash: UInt8 = 92
    private static let percent: UInt8 = 37

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> WebBrowserOpen {
        var cursor = 0
        let value = try readString(bytes, cursor: &cursor)

        guard cursor == bytes.count else { throw PluginError.invalidArgument }

        return try open(value)
    }

    static func open(_ value: String) throws -> WebBrowserOpen {
        guard value.utf8.count <= maxUrlBytes else { throw PluginError.tooLarge }

        guard value.hasPrefix("https://") || value.hasPrefix("http://") else {
            throw PluginError.invalidArgument
        }

        guard isVisible(value) else { throw PluginError.invalidArgument }

        try requireEscapes(Array(value.utf8))

        guard let url = URL(string: value), let host = url.host, !host.isEmpty else {
            throw PluginError.invalidArgument
        }

        guard url.user == nil, url.password == nil else { throw PluginError.invalidArgument }

        let authority = value.components(separatedBy: "://").dropFirst().first?
            .split(whereSeparator: { "/?#".contains($0) }).first

        guard let authority, !authority.isEmpty, !authority.contains("@") else {
            throw PluginError.invalidArgument
        }

        return WebBrowserOpen(url: url)
    }

    private static func isVisible(_ value: String) -> Bool {
        value.utf8.allSatisfy { (firstPrintable...lastPrintable).contains($0) && $0 != backslash }
    }

    private static func requireEscapes(_ bytes: [UInt8]) throws {
        for index in bytes.indices where bytes[index] == percent {
            guard index + 2 < bytes.count, isHex(bytes[index + 1]), isHex(bytes[index + 2]) else {
                throw PluginError.invalidArgument
            }
        }
    }

    private static func isHex(_ value: UInt8) -> Bool {
        (48...57).contains(value) || (65...70).contains(value) || (97...102).contains(value)
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
