import Foundation
import LeptosKitBridge

enum AgeRangeWire {
    static let minThreshold: UInt32 = 1
    static let maxThreshold: UInt32 = 120

    enum Capability: UInt32 {
        case restricted = 0
        case available = 1
        case unavailable = 2
    }

    enum ResponseKind: UInt32 {
        case declined = 0
        case unavailable = 1
        case shared = 2
    }

    static func capability(_ value: Capability) -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, value.rawValue)
        return data
    }

    static func responseUnavailable() -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, ResponseKind.unavailable.rawValue)
        return data
    }

    static func threshold(_ payload: NativeBuffer) throws -> UInt32 {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let threshold = try reader.u32()
            try reader.finish()
            guard (minThreshold...maxThreshold).contains(threshold) else {
                throw PluginError.invalidArgument
            }
            return threshold
        }
    }
}

private struct WireReader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0

    init(_ bytes: UnsafeRawBufferPointer) {
        self.bytes = bytes
    }

    mutating func u32() throws -> UInt32 {
        guard offset + 4 <= bytes.count else { throw PluginError.invalidArgument }
        defer { offset += 4 }
        var value: UInt32 = 0
        withUnsafeMutableBytes(of: &value) { destination in
            destination.copyBytes(from: UnsafeRawBufferPointer(rebasing: bytes[offset ..< offset + 4]))
        }
        return UInt32(littleEndian: value)
    }

    mutating func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}
