import Foundation
import LeptosKitBridge

enum VideoThumbnailsWire {
    static let maxUriBytes = 4096
    static let minQuality: UInt32 = 1
    static let maxQuality: UInt32 = 100

    static func getThumbnail(_ payload: NativeBuffer) throws -> (uri: String, timeMs: UInt64, quality: UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let timeMs = try reader.u64()
            let quality = try reader.u32()
            let uri = try reader.string()
            try reader.finish()
            guard !uri.isEmpty, !uri.contains("\0"), uri.utf8.count <= maxUriBytes else {
                throw PluginError.invalidArgument
            }
            guard timeMs != .max else { throw PluginError.invalidArgument }
            guard (minQuality...maxQuality).contains(quality) else { throw PluginError.invalidArgument }
            return (uri, timeMs, quality)
        }
    }

    static func thumbnail(uri: String, width: UInt32, height: UInt32) throws -> Data {
        guard !uri.isEmpty, !uri.contains("\0"), uri.utf8.count <= maxUriBytes else {
            throw PluginError.invalidArgument
        }
        guard width > 0, height > 0, width != .max, height != .max else {
            throw PluginError.invalidArgument
        }
        var data = Data(capacity: 12 + uri.utf8.count)
        appendU32(&data, width)
        appendU32(&data, height)
        appendString(&data, uri)
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

    mutating func u64() throws -> UInt64 {
        let low = UInt64(try u32())
        let high = UInt64(try u32())
        return low | (high << 32)
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

private func appendString(_ data: inout Data, _ text: String) {
    let utf8 = Array(text.utf8)
    appendU32(&data, UInt32(utf8.count))
    data.append(contentsOf: utf8)
}
