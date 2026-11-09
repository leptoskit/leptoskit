import Foundation
import LeptosKitBridge

enum BlobCapability: UInt32 {
    case restricted = 0
    case available = 1
    case unavailable = 2
}

enum BlobWire {
    static let maxCreate = 65536
    static let maxRead: UInt32 = 8192

    static func capability(_ value: BlobCapability) -> Data {
        var little = value.rawValue.littleEndian
        return withUnsafeBytes(of: &little) { Data($0) }
    }

    static func emptyPayload(_ payload: NativeBuffer) throws {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            try reader.finish()
        }
    }

    static func create(_ payload: NativeBuffer) throws -> Data {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let data = try reader.bytes()
            try reader.finish()
            guard !data.isEmpty, data.count <= maxCreate else { throw PluginError.invalidArgument }
            return data
        }
    }

    static func read(_ payload: NativeBuffer) throws -> (UInt32, UInt32, UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let offset = try reader.u32()
            let length = try reader.u32()
            try reader.finish()
            guard length > 0, length <= maxRead else { throw PluginError.invalidArgument }
            return (id, offset, length)
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

    static func encodeId(_ id: UInt32) -> Data {
        var little = id.littleEndian
        return withUnsafeBytes(of: &little) { Data($0) }
    }

    static func encodeSlice(_ data: Data) throws -> Data {
        guard data.count <= Int(maxRead) else { throw PluginError.invalidArgument }
        var output = Data()
        var count = UInt32(data.count).littleEndian
        withUnsafeBytes(of: &count) { output.append(contentsOf: $0) }
        output.append(data)
        return output
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
    mutating func bytes() throws -> Data {
        let count = Int(try u32())
        guard count >= 0, offset + count <= bytes.count else { throw PluginError.invalidArgument }
        let data = Data(bytes[offset ..< offset + count])
        offset += count
        return data
    }
    mutating func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}
