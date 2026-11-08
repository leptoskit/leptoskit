import Foundation
import LeptosKitBridge
import PhotosUI

enum LivePhotoWire {
    static let maxUriBytes = 4096
    static let wireVersion: UInt32 = 1

    static func create(_ payload: NativeBuffer) throws -> String {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let uri = try reader.string()
            try reader.finish()
            guard !uri.isEmpty, !uri.contains("\0"), uri.utf8.count <= maxUriBytes else {
                throw PluginError.invalidArgument
            }
            return uri
        }
    }

    static func session(_ payload: NativeBuffer) throws -> UInt32 {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            try reader.finish()
            return id
        }
    }

    static func startPlayback(_ payload: NativeBuffer) throws -> (id: UInt32, style: UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let style = try reader.u32()
            guard style <= 1 else { throw PluginError.invalidArgument }
            try reader.finish()
            return (id, style)
        }
    }

    static func sessionId(_ id: UInt32) -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, id)
        return data
    }

    static func status(active: Bool, playing: Bool) -> Data {
        var data = Data(capacity: 12)
        appendU32(&data, wireVersion)
        appendU32(&data, active ? 1 : 0)
        appendU32(&data, playing ? 1 : 0)
        return data
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
