import Foundation
import LeptosKitBridge

enum CameraWire {
    static let version: UInt32 = 1
    static let maxBytes: UInt32 = 16 * 1024 * 1024
    static let maxUriBytes = 4_096
    static let maxMimeBytes = 256

    static func create(_ payload: NativeBuffer) throws -> UInt32 {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let facing = try reader.u32()
            guard facing <= 1 else { throw PluginError.invalidArgument }
            try reader.finish()
            return facing
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

    static func torch(_ payload: NativeBuffer) throws -> (id: UInt32, enabled: Bool) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let flag = try reader.u32()
            guard flag <= 1 else { throw PluginError.invalidArgument }
            try reader.finish()
            return (id, flag == 1)
        }
    }

    static func takePicture(_ payload: NativeBuffer) throws -> (id: UInt32, maxBytes: UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let max = try reader.u32()
            guard max <= maxBytes else { throw PluginError.tooLarge }
            try reader.finish()
            return (id, max)
        }
    }

    static func sessionId(_ id: UInt32) -> Data {
        words(id)
    }

    static func status(
        facing: UInt32,
        torchOn: Bool,
        torchAvailable: Bool,
        active: Bool
    ) -> Data {
        words(version, facing, torchOn ? 1 : 0, torchAvailable ? 1 : 0, active ? 1 : 0)
    }

    static func asset(
        uri: String,
        mime: String?,
        width: UInt32?,
        height: UInt32?,
        size: UInt64?
    ) throws -> Data {
        guard !uri.isEmpty, uri.utf8.count <= maxUriBytes else { throw PluginError.invalidArgument }
        let mimeText = mime ?? ""
        guard mimeText.utf8.count <= maxMimeBytes, mime.map({ !$0.isEmpty }) ?? true else {
            throw PluginError.invalidArgument
        }
        var data = Data()
        appendString(&data, uri)
        appendString(&data, mimeText)
        append(&data, width ?? .max)
        append(&data, height ?? .max)
        let sizeValue = size ?? .max
        append(&data, UInt32(truncatingIfNeeded: sizeValue))
        append(&data, UInt32(truncatingIfNeeded: sizeValue >> 32))
        return data
    }

    private static func words(_ values: UInt32...) -> Data {
        var data = Data(capacity: values.count * 4)
        for value in values {
            append(&data, value)
        }
        return data
    }

    private static func append(_ data: inout Data, _ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    private static func appendString(_ data: inout Data, _ text: String) {
        let utf8 = Array(text.utf8)
        append(&data, UInt32(utf8.count))
        data.append(contentsOf: utf8)
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
