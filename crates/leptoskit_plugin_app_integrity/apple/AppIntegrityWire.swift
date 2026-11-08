import Foundation
import LeptosKitBridge

enum AppIntegrityWire {
    static let maxChallenge = 256

    enum Capability: UInt32 {
        case restricted = 0
        case available = 1
        case unavailable = 2
    }

    static func capability(_ value: Capability) -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, value.rawValue)
        return data
    }

    static func attestationUnavailable() -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, 0)
        return data
    }

    static func challenge(_ payload: NativeBuffer) throws -> Data {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let challenge = try reader.bytes()
            try reader.finish()
            guard !challenge.isEmpty, challenge.count <= maxChallenge else {
                throw PluginError.invalidArgument
            }
            return Data(challenge)
        }
    }
}

private struct WireReader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0

    init(_ bytes: UnsafeRawBufferPointer) { self.bytes = bytes }

    mutating func u32() throws -> UInt32 {
        guard offset + 4 <= bytes.count else { throw PluginError.invalidArgument }
        defer { offset += 4 }
        var value: UInt32 = 0
        withUnsafeMutableBytes(of: &value) { destination in
            destination.copyBytes(from: UnsafeRawBufferPointer(rebasing: bytes[offset ..< offset + 4]))
        }
        return UInt32(littleEndian: value)
    }

    mutating func bytes() throws -> [UInt8] {
        let count = Int(try u32())
        guard count >= 0, offset + count <= bytes.count else { throw PluginError.invalidArgument }
        let slice = Array(bytes[offset ..< offset + count])
        offset += count
        return slice
    }

    mutating func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}
