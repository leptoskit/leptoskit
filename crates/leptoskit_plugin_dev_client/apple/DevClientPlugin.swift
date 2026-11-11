import Darwin
import Foundation
import LeptosKitBridge

/// Follows a development guest. Scanning a code is the Android host's reader;
/// on Apple the person pastes the address, and `scan` stays unavailable.
@MainActor
public final class DevClientPlugin: NativePlugin {
    public let name = "dev-client"
    public let methods: [UInt32] = [0, 1, 2]
    private var closed = false

    public init(context: NativePluginContext) {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return status(1)
        case 1:
            let url = try DevClientWire.canonical(payload)
            let followed = DevelopmentSession.follow?(url) ?? false
            guard followed else { throw PluginError.unavailable }
            return Data()
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            throw PluginError.unavailable
        default:
            throw PluginError.unavailable
        }
    }

    public func close() { closed = true }

    private func status(_ value: UInt32) -> Data {
        var little = value.littleEndian
        return Data(bytes: &little, count: 4)
    }
}

enum DevClientWire {
    static func canonical(_ payload: NativeBuffer) throws -> String {
        let text = try text(payload)
        if let android = one(text, platform: "android") { return android }
        if let ios = one(text, platform: "ios") { return ios }
        throw PluginError.invalidArgument
    }

    static func text(_ payload: NativeBuffer) throws -> String {
        try payload.withUnsafeBytes { raw in
            guard raw.count >= 4 else { throw PluginError.invalidArgument }
            let length = Int(UInt32(littleEndian: raw.loadUnaligned(as: UInt32.self)))
            guard raw.count == 4 + length else { throw PluginError.invalidArgument }
            let body = UnsafeRawBufferPointer(rebasing: raw.dropFirst(4))
            guard let text = String(bytes: body, encoding: .utf8) else { throw PluginError.invalidArgument }
            return text
        }
    }

    private static func one(_ scanned: String, platform: String) -> String? {
        let text = scanned.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !text.contains("?"), !text.contains("#") else { return nil }
        guard text.unicodeScalars.allSatisfy({ !CharacterSet.whitespacesAndNewlines.contains($0) }) else {
            return nil
        }
        let scheme: String
        let rest: Substring
        if text.hasPrefix("http://") {
            scheme = "http"
            rest = text.dropFirst(7)
        } else if text.hasPrefix("https://") {
            scheme = "https"
            rest = text.dropFirst(8)
        } else {
            return nil
        }
        guard !rest.contains("@") else { return nil }
        let authority: Substring
        var path: String
        if let slash = rest.firstIndex(of: "/") {
            authority = rest[..<slash]
            path = String(rest[slash...])
        } else {
            authority = rest
            path = ""
        }
        while path.count > 1, path.hasSuffix("/") { path.removeLast() }
        if path == "/" { path = "" }
        guard let (host, port) = split(String(authority), scheme: scheme) else { return nil }
        let guest = "/_leptoskit/dev/\(platform)/guest"
        guard path.isEmpty || path == "/_leptoskit/dev" || path == guest else { return nil }
        return "\(scheme)://\(host):\(port)\(guest)"
    }

    private static func split(_ authority: String, scheme: String) -> (String, Int)? {
        guard !authority.isEmpty else { return nil }
        let host: String
        let explicit: Int?
        if authority.hasPrefix("[") {
            guard let end = authority.firstIndex(of: "]") else { return nil }
            host = String(authority[authority.index(after: authority.startIndex)..<end])
            let after = authority[authority.index(after: end)...]
            if after.isEmpty {
                explicit = nil
            } else if after.hasPrefix(":") {
                guard let value = port(String(after.dropFirst())) else { return nil }
                explicit = value
            } else {
                return nil
            }
        } else if let index = authority.lastIndex(of: ":") {
            let name = authority[..<index]
            guard !name.isEmpty, !name.contains(":") else { return nil }
            host = String(name)
            guard let value = port(String(authority[authority.index(after: index)...])) else { return nil }
            explicit = value
        } else {
            host = authority
            explicit = nil
        }
        guard !host.isEmpty, devHost(host) else { return nil }
        let rendered = host.contains(":") ? "[\(host)]" : host
        return (rendered, explicit ?? (scheme == "https" ? 443 : 80))
    }

    private static func port(_ text: String) -> Int? {
        guard let value = Int(text), (1...65535).contains(value) else { return nil }
        return value
    }

    private static func devHost(_ host: String) -> Bool {
        if host.lowercased() == "localhost" { return true }
        if host.lowercased().hasSuffix(".local") {
            let name = String(host.dropLast(6))
            guard !name.isEmpty else { return false }
            return name.split(separator: ".").allSatisfy { label in
                (1...63).contains(label.count) && !label.hasPrefix("-") && !label.hasSuffix("-")
                    && label.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" }
            }
        }
        var v4 = in_addr()
        if host.withCString({ inet_pton(AF_INET, $0, &v4) }) == 1 {
            let bytes = withUnsafeBytes(of: v4.s_addr) { Array($0) }
            let firstOctet = Int(bytes[0])
            let secondOctet = Int(bytes[1])
            if firstOctet == 0 || firstOctet == 255 || (224...239).contains(firstOctet) { return false }
            return firstOctet == 127 || firstOctet == 10 || (firstOctet == 169 && secondOctet == 254)
                || (firstOctet == 172 && (16...31).contains(secondOctet))
                || (firstOctet == 192 && secondOctet == 168)
        }
        var v6 = in6_addr()
        guard host.withCString({ inet_pton(AF_INET6, $0, &v6) }) == 1 else { return false }
        let bytes = withUnsafeBytes(of: v6) { Array($0) }
        let first = Int(bytes[0])
        let second = Int(bytes[1])
        let unspecified = bytes.allSatisfy { $0 == 0 }
        let loopback = bytes.dropLast().allSatisfy { $0 == 0 } && bytes.last == 1
        return !unspecified && (loopback || first & 0xfe == 0xfc || (first == 0xfe && second & 0xc0 == 0x80))
    }
}
