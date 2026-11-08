import Foundation
import LeptosKitBridge

enum VideoWire {
    static let version: UInt32 = 1
    static let timeUnknown: UInt32 = .max
    static let rateMin: UInt32 = 250
    static let rateMax: UInt32 = 2_000
    static let maxUrlBytes = 2_048

    static func source(_ payload: NativeBuffer) throws -> URL {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            guard try reader.u32() == version else { throw PluginError.invalidArgument }
            let text = try reader.string(max: maxUrlBytes)
            guard !text.isEmpty else { throw PluginError.invalidArgument }
            guard let url = URL(string: text) else { throw PluginError.invalidArgument }
            try reader.finish()
            return url
        }
    }

    static func replace(_ payload: NativeBuffer) throws -> (UInt32, URL) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            guard try reader.u32() == version else { throw PluginError.invalidArgument }
            let text = try reader.string(max: maxUrlBytes)
            guard !text.isEmpty, let url = URL(string: text) else { throw PluginError.invalidArgument }
            try reader.finish()
            return (id, url)
        }
    }

    static func player(_ payload: NativeBuffer) throws -> UInt32 {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            try reader.finish()
            return id
        }
    }

    static func seek(_ payload: NativeBuffer) throws -> (id: UInt32, positionMs: UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let position = try reader.u32()
            guard position != timeUnknown else { throw PluginError.invalidArgument }
            try reader.finish()
            return (id, position)
        }
    }

    static func flag(_ payload: NativeBuffer) throws -> (id: UInt32, enabled: Bool) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let flag = try reader.u32()
            guard flag <= 1 else { throw PluginError.invalidArgument }
            try reader.finish()
            return (id, flag == 1)
        }
    }

    static func rate(_ payload: NativeBuffer) throws -> (id: UInt32, thousandths: UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let thousandths = try reader.u32()
            guard (rateMin ... rateMax).contains(thousandths) else { throw PluginError.invalidArgument }
            try reader.finish()
            return (id, thousandths)
        }
    }

    static func playerId(_ id: UInt32) -> Data {
        words(id)
    }

    static func status(playing: Bool, currentMs: UInt32, durationMs: UInt32) -> Data {
        words(version, playing ? 1 : 0, currentMs, durationMs)
    }

    private static func words(_ values: UInt32...) -> Data {
        var data = Data(capacity: values.count * 4)
        for value in values {
            var little = value.littleEndian
            withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
        }
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

    mutating func string(max: Int) throws -> String {
        let length = Int(try u32())
        guard length <= max, offset + length <= bytes.count else {
            throw length > max ? PluginError.tooLarge : PluginError.invalidArgument
        }
        let slice = bytes[offset ..< offset + length]
        offset += length
        guard let text = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }
        return text
    }

    mutating func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}
