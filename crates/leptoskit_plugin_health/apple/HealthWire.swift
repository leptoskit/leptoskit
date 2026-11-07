import Foundation
import LeptosKitBridge

/// Wire codes shared with the Rust guest client; `records.rs` documents the
/// framing. Little-endian throughout, and an optional string is either a u32
/// byte length and its bytes, or the length `absentLength`.
public enum HealthWire {
    /// The length that marks an absent optional string.
    public static let absentLength: UInt32 = UInt32.max

    /// Data type codes, in wire order; RMSSD (2) is Health Connect only.
    public static let heartRate = 0
    public static let restingHeartRate = 1
    public static let hrvSdnn = 3
    public static let steps = 4
    public static let sleep = 5
    public static let oxygenSaturation = 6
    public static let respiratoryRate = 7
    public static let exercise = 8

    /// Every type HealthKit records, as a bit mask; RMSSD (bit 2) stays
    /// Health Connect only and SDNN (bit 3) HealthKit only.
    public static let appleTypes: UInt32 = 0x1FB

    /// Reply flag bits, in the order `types.rs` decodes them.
    public static let flagBackground: UInt32 = 1
    public static let flagHistory: UInt32 = 2
    public static let flagExact: UInt32 = 4

    /// Changes-reply flag bits, in the order `records.rs` decodes them.
    public static let flagExpired: UInt32 = 1
    public static let flagHasMore: UInt32 = 2
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
            u32(HealthWire.absentLength)
        }
    }

    mutating func raw(_ bytes: Data) {
        output.append(bytes)
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

    mutating func string() throws -> String {
        let length = try u32()
        guard length != HealthWire.absentLength else { throw PluginError.invalidArgument }

        let count = Int(length)
        guard count >= 0, count <= remaining else { throw PluginError.invalidArgument }
        defer { position += count }
        guard let value = String(bytes: bytes[position..<position + count], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        return value
    }

    mutating func optional() throws -> String? {
        if try u32() == HealthWire.absentLength { return nil }
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

/// One sleep stage of a session, before encoding.
public struct SleepStage: Equatable {
    /// Stage codes, in wire order.
    public enum Stage: Int {
        case unknown = 0
        case awake = 1
        case asleep = 2
        case light = 3
        case deep = 4
        case rem = 5
        case inBed = 6
    }

    public let startMs: Int64
    public let endMs: Int64
    public let stage: Stage
}

/// One heart-rate measurement of a series, before encoding.
public struct HeartRatePoint: Equatable {
    public let timeMs: Int64
    public let bpm: Double
}

/// A sample's measurement, which also fixes its data type.
public enum HealthValue: Equatable {
    case heartRate([HeartRatePoint])
    case restingHeartRate(bpm: Double)
    case hrvSdnn(milliseconds: Double)
    case steps(count: Int64)
    case sleep(stages: [SleepStage])
    case oxygenSaturation(percent: Double)
    case respiratoryRate(breathsPerMinute: Double)
    case exercise(activity: Int, platformCode: Int32, title: String?)

    var typeCode: Int {
        switch self {
        case .heartRate: HealthWire.heartRate
        case .restingHeartRate: HealthWire.restingHeartRate
        case .hrvSdnn: HealthWire.hrvSdnn
        case .steps: HealthWire.steps
        case .sleep: HealthWire.sleep
        case .oxygenSaturation: HealthWire.oxygenSaturation
        case .respiratoryRate: HealthWire.respiratoryRate
        case .exercise: HealthWire.exercise
        }
    }
}

/// Where a sample came from, as far as the platform says.
public struct HealthSource: Equatable {
    public let appId: String?
    public let appName: String?
    public let device: String?

    public init(appId: String? = nil, appName: String? = nil, device: String? = nil) {
        self.appId = appId
        self.appName = appName
        self.device = device
    }
}

/// One health record, before encoding.
public struct HealthSample: Equatable {
    public let id: String
    public let startMs: Int64
    public let endMs: Int64
    public let source: HealthSource
    public let value: HealthValue

    public init(id: String, startMs: Int64, endMs: Int64, source: HealthSource, value: HealthValue) {
        self.id = id
        self.startMs = startMs
        self.endMs = endMs
        self.source = source
        self.value = value
    }
}

/// Encodes one sample exactly as the Rust codec decodes it.
public func encodeSample(_ sample: HealthSample) throws -> Data {
    guard !sample.id.isEmpty, sample.endMs >= sample.startMs else { throw PluginError.native }
    var writer = WireWriter()
    try writer.string(sample.id)
    writer.u32(UInt32(sample.value.typeCode))
    writer.i64(sample.startMs)
    writer.i64(sample.endMs)
    try writer.optional(sample.source.appId)
    try writer.optional(sample.source.appName)
    try writer.optional(sample.source.device)
    try writer.encode(sample.value)
    return writer.bytes()
}

extension WireWriter {
    /// Writes one sample's value payload, which also fixes its data type.
    mutating func encode(_ value: HealthValue) throws {
        switch value {
        case .heartRate(let points):
            u32(UInt32(points.count))
            for point in points {
                i64(point.timeMs)
                try f64(point.bpm)
            }
        case .restingHeartRate(let bpm):
            try f64(bpm)
        case .hrvSdnn(let milliseconds):
            try f64(milliseconds)
        case .steps(let count):
            i64(count)
        case .sleep(let stages):
            u32(UInt32(stages.count))
            for stage in stages {
                i64(stage.startMs)
                i64(stage.endMs)
                u32(UInt32(stage.stage.rawValue))
            }
        case .oxygenSaturation(let percent):
            try f64(percent)
        case .respiratoryRate(let breaths):
            try f64(breaths)
        case .exercise(let activity, let platformCode, let title):
            u32(UInt32(activity))
            u32(UInt32(bitPattern: platformCode))
            try optional(title)
        }
    }
}

/// Encodes a page of samples and the token the next page needs.
public func encodePage(_ samples: [HealthSample], token: String?) throws -> Data {
    var writer = WireWriter()
    writer.u32(UInt32(samples.count))
    for sample in samples {
        try writer.raw(encodeSample(sample))
    }
    try writer.optional(token)
    return writer.bytes()
}

/// Encodes grants or features: a type mask and flag bits.
public func encodeMask(_ mask: UInt32, flags: UInt32) -> Data {
    var writer = WireWriter()
    writer.u32(mask)
    writer.u32(flags)
    return writer.bytes()
}

/// Encodes the availability code the Rust codec decodes.
public func encodeAvailability(_ code: Int) -> Data {
    var writer = WireWriter()
    writer.u32(UInt32(code))
    return writer.bytes()
}

/// A read request: one type, a window, a page size and a page token.
public struct ReadRequest: Equatable {
    public let type: Int
    public let startMs: Int64
    public let endMs: Int64
    public let pageSize: Int
    public let token: String?

    /// The largest page HealthKit serves in one call, bound by the wire's
    /// page-size limit.
    public static let maxPageSize = 1000

    public init(type: Int, startMs: Int64, endMs: Int64, pageSize: Int, token: String?) {
        self.type = type
        self.startMs = startMs
        self.endMs = endMs
        self.pageSize = pageSize
        self.token = token
    }

    /// Decodes the guest's request, rejecting what the Rust codec rejects.
    public init(_ data: Data) throws {
        var reader = WireReader(data)
        let type = try reader.u32()
        let start = try reader.i64()
        let end = try reader.i64()
        let size = try reader.u32()
        let token = try reader.optional()
        try reader.finished()
        let supported = [
            HealthWire.heartRate, HealthWire.restingHeartRate, HealthWire.hrvSdnn, HealthWire.steps,
            HealthWire.sleep, HealthWire.oxygenSaturation, HealthWire.respiratoryRate, HealthWire.exercise
        ]
        guard supported.contains(Int(type)), start < end,
            size >= 1 && size <= UInt32(ReadRequest.maxPageSize), token != ""
        else { throw PluginError.invalidArgument }
        self.init(type: Int(type), startMs: start, endMs: end, pageSize: Int(size), token: token)
    }
}

/// A permission or changes-token request: a type mask and extra flags.
public struct TypesRequest: Equatable {
    public let mask: UInt32
    public let flags: UInt32

    public init(mask: UInt32, flags: UInt32) {
        self.mask = mask
        self.flags = flags
    }

    /// Decodes the guest's request. Types HealthKit never records are
    /// dropped rather than refused, so one request can name both platforms'
    /// heart-rate-variability types; an empty remainder is caller error.
    public init(_ data: Data) throws {
        var reader = WireReader(data)
        let requested = try reader.u32()
        let flags = try reader.u32()
        try reader.finished()
        let mask = requested & HealthWire.appleTypes
        guard requested != 0, mask != 0, flags & ~3 == 0 else {
            throw PluginError.invalidArgument
        }
        self.init(mask: mask, flags: flags)
    }

    /// The type codes the mask selects, in wire order.
    public var types: [Int] {
        (0...8).filter { mask & (1 << UInt32($0)) != 0 }
    }
}

/// The changes reply under construction: upserts, deletions and a next token.
public struct ChangesReply {
    public var upserted: [HealthSample] = []
    public var deleted: [String] = []
    public var nextToken: String?
    public var hasMore = false
    public var expired = false

    public init() {}

    /// Encodes the reply exactly as the Rust codec decodes it; an expired
    /// token carries no continuation, and a live one always does.
    public func encode() throws -> Data {
        var writer = WireWriter()
        if expired {
            writer.u32(0)
            writer.u32(0)
            try writer.optional(nil)
            writer.u32(HealthWire.flagExpired)
            return writer.bytes()
        }
        writer.u32(UInt32(upserted.count))
        for sample in upserted {
            writer.raw(try encodeSample(sample))
        }
        writer.u32(UInt32(deleted.count))
        for id in deleted {
            try writer.string(id)
        }
        try writer.optional(nextToken)
        writer.u32(hasMore ? HealthWire.flagHasMore : 0)
        return writer.bytes()
    }
}
