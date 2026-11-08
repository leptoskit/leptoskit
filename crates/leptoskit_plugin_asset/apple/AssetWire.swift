import Foundation
import LeptosKitBridge

enum AssetWire {
    static let maxName = 256
    enum Capability: UInt32 { case restricted = 0, available = 1, unavailable = 2 }
    static func capability(_ value: Capability) -> Data {
        var data = Data(capacity: 4); appendU32(&data, value.rawValue); return data
    }
    static func resolvedUnavailable() -> Data {
        var data = Data(capacity: 4); appendU32(&data, 0); return data
    }
    static func name(_ payload: NativeBuffer) throws -> String {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let name = try reader.string()
            try reader.finish()
            guard !name.isEmpty, !name.contains("\0"), name.utf8.count <= maxName else {
                throw PluginError.invalidArgument
            }
            return name
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
    mutating func string() throws -> String {
        let count = Int(try u32())
        guard count >= 0, offset + count <= bytes.count else { throw PluginError.invalidArgument }
        let slice = UnsafeRawBufferPointer(rebasing: bytes[offset ..< offset + count])
        offset += count
        guard let value = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }
        return value
    }
    mutating func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}
private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}
