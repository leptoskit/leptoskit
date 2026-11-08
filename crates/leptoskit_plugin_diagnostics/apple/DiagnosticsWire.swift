import Foundation
import LeptosKitBridge

enum DiagnosticsWire {
    static let maxName = 128

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

    static func id(_ payload: NativeBuffer) throws -> UInt32 {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            try reader.finish()
            return id
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
