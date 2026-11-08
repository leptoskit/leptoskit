import Foundation
import LeptosKitBridge

enum BackgroundTaskWire {
    static let maxNameBytes = 128
    static let minInterval: UInt32 = 900
    static let maxInterval: UInt32 = 86_400

    enum Status: UInt32 {
        case restricted = 0
        case available = 1
        case unavailable = 2
    }

    static func status(_ value: Status) -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, value.rawValue)
        return data
    }

    static func register(_ payload: NativeBuffer) throws -> (name: String, interval: UInt32) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let interval = try reader.u32()
            let name = try reader.string()
            try reader.finish()
            guard (minInterval...maxInterval).contains(interval) else { throw PluginError.invalidArgument }
            try validateName(name)
            return (name, interval)
        }
    }

    static func name(_ payload: NativeBuffer) throws -> String {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let name = try reader.string()
            try reader.finish()
            try validateName(name)
            return name
        }
    }

    private static func validateName(_ name: String) throws {
        guard !name.isEmpty, !name.contains("\0"), name.utf8.count <= maxNameBytes else {
            throw PluginError.invalidArgument
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
