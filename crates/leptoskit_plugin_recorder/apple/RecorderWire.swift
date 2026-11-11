import Foundation
import LeptosKitBridge

/// Wire codes shared with the Rust guest client; `wire.rs` documents the
/// framing. Little-endian throughout, and an optional string is either a u32
/// byte length and its bytes, or the length `absentLength`.
public enum RecorderWire {
    /// The length that marks an absent optional string.
    public static let absentLength: UInt32 = UInt32.max

    /// Permission codes, in wire order.
    public static let permissionUnknown: UInt32 = 0
    public static let permissionGranted: UInt32 = 1
    public static let permissionDenied: UInt32 = 2

    /// State codes, in wire order.
    public static let stateIdle: UInt32 = 0
    public static let stateRecording: UInt32 = 1
    public static let stateInterrupted: UInt32 = 2
    public static let stateSilenced: UInt32 = 3
    public static let stateFailed: UInt32 = 4

    /// Why a segment ended, in wire order.
    public static let endedRotation: UInt32 = 0
    public static let endedStop: UInt32 = 1
    public static let endedInterruption: UInt32 = 2
    public static let endedRecovered: UInt32 = 3

    /// The bounds the start request enforces, mirroring `types.rs`.
    public static let minSegmentSecs: UInt32 = 5
    public static let maxSegmentSecs: UInt32 = 3_600
    public static let minSampleRate: UInt32 = 8_000
    public static let maxSampleRate: UInt32 = 48_000
    public static let minBitrate: UInt32 = 8_000
    public static let maxBitrate: UInt32 = 320_000
    public static let minThresholdDbfs: Double = -100
    public static let maxThresholdDbfs: Double = 0

    /// The longest directory path, reason and segment path the wire carries.
    public static let maxDirectoryBytes = 1_024
    public static let maxReasonBytes = 512
    public static let maxPathBytes = 4_096
}

/// A typed little-endian reply under construction.
struct WireWriter {
    private var output = Data()

    mutating func u32(_ value: UInt32) {
        append(value.littleEndian)
    }

    mutating func i64(_ value: Int64) {
        append(value.littleEndian)
    }

    mutating func f64(_ value: Double) throws {
        guard value.isFinite else { throw PluginError.native }
        append(value.bitPattern.littleEndian)
    }

    mutating func string(_ value: String) throws {
        let bytes = Data(value.utf8)
        guard bytes.count <= 64 * 1024 else { throw PluginError.native }
        u32(UInt32(bytes.count))
        output.append(bytes)
    }

    mutating func optional(_ value: String?) throws {
        if let value {
            try string(value)
        } else {
            u32(RecorderWire.absentLength)
        }
    }

    func bytes() -> Data { output }

    private mutating func append<T>(_ value: T) {
        withUnsafeBytes(of: value) { output.append(contentsOf: $0) }
    }
}

/// A typed request the Rust guest client wrote.
struct WireReader {
    private let bytes: [UInt8]
    private var position = 0

    init(_ data: Data) {
        bytes = [UInt8](data)
    }

    var remaining: Int { bytes.count - position }

    mutating func u32() throws -> UInt32 {
        try load(4) { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
    }

    mutating func i64() throws -> Int64 {
        try load(8) { Int64(littleEndian: $0.loadUnaligned(as: Int64.self)) }
    }

    mutating func f64() throws -> Double {
        Double(bitPattern: try load(8) { UInt64(littleEndian: $0.loadUnaligned(as: UInt64.self)) })
    }

    mutating func string() throws -> String {
        let length = try u32()
        guard length != RecorderWire.absentLength else { throw PluginError.invalidArgument }

        let count = Int(length)
        guard count >= 0, count <= remaining else { throw PluginError.invalidArgument }
        defer { position += count }
        guard let value = String(bytes: bytes[position..<position + count], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        return value
    }

    mutating func optional() throws -> String? {
        if try u32() == RecorderWire.absentLength { return nil }
        position -= 4
        return try string()
    }

    func finished() throws {
        guard remaining == 0 else { throw PluginError.invalidArgument }
    }

    private mutating func load<T>(_ size: Int, _ decode: (UnsafeRawBufferPointer) throws -> T) throws -> T {
        guard remaining >= size else { throw PluginError.invalidArgument }
        defer { position += size }
        return try bytes.withUnsafeBytes { raw in
            try decode(UnsafeRawBufferPointer(rebasing: raw[position..<position + size]))
        }
    }
}

/// The start request: a directory and four tuned bounds.
public struct StartRequest: Equatable {
    /// The app-private directory segments are written to.
    public let directory: String
    /// The segment length in seconds.
    public let segmentSecs: UInt32
    /// The sample rate in hertz.
    public let sampleRate: UInt32
    /// The AAC bitrate in bits per second.
    public let bitrate: UInt32
    /// The activity threshold in dBFS.
    public let thresholdDbfs: Double

    public init(
        directory: String,
        segmentSecs: UInt32,
        sampleRate: UInt32,
        bitrate: UInt32,
        thresholdDbfs: Double
    ) {
        self.directory = directory
        self.segmentSecs = segmentSecs
        self.sampleRate = sampleRate
        self.bitrate = bitrate
        self.thresholdDbfs = thresholdDbfs
    }

    /// Decodes the guest's request, clamping the segment length and
    /// rejecting what the Rust codec rejects.
    public init(_ data: Data) throws {
        guard data.count <= RecorderWire.maxDirectoryBytes + 32 else { throw PluginError.invalidArgument }
        var reader = WireReader(data)
        let directory = try reader.string()
        let segmentSecs = try reader.u32()
        let sampleRate = try reader.u32()
        let bitrate = try reader.u32()
        let threshold = try reader.f64()
        try reader.finished()

        guard !directory.isEmpty,
            Data(directory.utf8).count <= RecorderWire.maxDirectoryBytes,
            directory.hasPrefix("/"),
            !directory.split(separator: "/").contains(".."),
            !directory.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }),
            (RecorderWire.minSampleRate...RecorderWire.maxSampleRate).contains(sampleRate),
            (RecorderWire.minBitrate...RecorderWire.maxBitrate).contains(bitrate),
            (RecorderWire.minThresholdDbfs...RecorderWire.maxThresholdDbfs).contains(threshold)
        else { throw PluginError.invalidArgument }

        self.init(
            directory: directory,
            segmentSecs: min(max(segmentSecs, RecorderWire.minSegmentSecs), RecorderWire.maxSegmentSecs),
            sampleRate: sampleRate,
            bitrate: bitrate,
            thresholdDbfs: threshold
        )
    }
}

/// Encodes the permission reply.
public func encodePermission(_ code: UInt32) -> Data {
    var writer = WireWriter()
    writer.u32(code)
    return writer.bytes()
}

/// Encodes the recovery report: finalised files re-queued and truncated
/// files deleted.
public func encodeRecovery(recovered: UInt32, discarded: UInt32) -> Data {
    var writer = WireWriter()
    writer.u32(recovered)
    writer.u32(discarded)
    return writer.bytes()
}

/// The status reply under construction.
public struct StatusReply {
    public var state: UInt32 = RecorderWire.stateIdle
    public var currentSegmentStartedAtMs: Int64 = 0
    public var reason: String?

    public init() {}

    /// Encodes the reply exactly as the Rust codec decodes it: only the
    /// failed state carries a reason, and only an open segment a start time.
    public func encode() throws -> Data {
        if let reason {
            guard !reason.isEmpty, reason.utf8.count <= RecorderWire.maxReasonBytes else {
                throw PluginError.native
            }
            guard state == RecorderWire.stateFailed else { throw PluginError.native }
        }
        if currentSegmentStartedAtMs > 0 {
            guard state == RecorderWire.stateRecording || state == RecorderWire.stateSilenced else {
                throw PluginError.native
            }
        }

        var writer = WireWriter()
        writer.u32(state)
        writer.i64(currentSegmentStartedAtMs)
        try writer.optional(reason)
        return writer.bytes()
    }
}

/// One finished segment, before encoding.
public struct SegmentReply: Equatable, Sendable {
    /// The segment file's absolute path.
    public let path: String
    /// When the segment started, in Unix milliseconds.
    public let startedAtMs: Int64
    /// How long the segment recorded, in milliseconds.
    public let durationMs: Int64
    /// The segment file's size in bytes.
    public let bytes: Int64
    /// The loudest frame's peak level in dBFS.
    public let peakDbfs: Double
    /// How many milliseconds were above the activity threshold.
    public let activeMs: Int64
    /// Why the segment ended, as a `RecorderWire.ended…` code.
    public let endedBy: UInt32

    public init(
        path: String,
        startedAtMs: Int64,
        durationMs: Int64,
        bytes: Int64,
        peakDbfs: Double,
        activeMs: Int64,
        endedBy: UInt32
    ) {
        self.path = path
        self.startedAtMs = startedAtMs
        self.durationMs = durationMs
        self.bytes = bytes
        self.peakDbfs = peakDbfs
        self.activeMs = activeMs
        self.endedBy = endedBy
    }

    /// Encodes the next-segment reply carrying this segment.
    public func encode() throws -> Data {
        guard !path.isEmpty, path.utf8.count <= RecorderWire.maxPathBytes,
            startedAtMs > 0, durationMs >= 0, bytes > 0, activeMs >= 0, activeMs <= durationMs
        else { throw PluginError.native }

        var writer = WireWriter()
        writer.u32(1)
        try writer.string(path)
        writer.i64(startedAtMs)
        writer.i64(durationMs)
        writer.i64(bytes)
        try writer.f64(peakDbfs)
        writer.i64(activeMs)
        writer.u32(endedBy)
        return writer.bytes()
    }
}

/// Encodes the empty next-segment reply: nothing is waiting and nothing is
/// recording.
public func encodeAbsentSegment() -> Data {
    var writer = WireWriter()
    writer.u32(0)
    return writer.bytes()
}
