import LeptosKitBridge
@testable import LeptosKitHealth
import XCTest

/// Codec tests over synthetic payloads; these never open HealthKit or prompt.
final class HealthTests: XCTestCase {
    func testAvailabilityAndMaskRepliesMatchTheRustFraming() {
        XCTAssertEqual(encodeAvailability(3), Data([3, 0, 0, 0]))
        XCTAssertEqual(encodeMask(HealthWire.appleTypes, flags: HealthWire.flagExact), reply(0x1FB, 4))
        XCTAssertEqual(HealthWire.appleTypes, 0x1FB)
        XCTAssertEqual((HealthWire.appleTypes >> 2) & 1, 0, "RMSSD stays Health Connect only")
    }

    func testHeartRateAndSleepEncodeWithTheirSources() throws {
        let heart = HealthSample(
            id: "hr-1",
            startMs: 1_000,
            endMs: 61_000,
            source: HealthSource(appId: "com.example.watch", appName: "Watch", device: "Apple Watch"),
            value: .heartRate([
                HeartRatePoint(timeMs: 1_000, bpm: 62),
                HeartRatePoint(timeMs: 31_000, bpm: 64.5)
            ])
        )
        var input = Reader(try encodeSample(heart))
        XCTAssertEqual(try input.string(), "hr-1")
        XCTAssertEqual(try input.u32(), 0)
        XCTAssertEqual(try input.i64(), 1_000)
        XCTAssertEqual(try input.i64(), 61_000)
        XCTAssertEqual(try input.string(), "com.example.watch")
        XCTAssertEqual(try input.string(), "Watch")
        XCTAssertEqual(try input.string(), "Apple Watch")
        XCTAssertEqual(try input.u32(), 2)
        XCTAssertEqual(try input.i64(), 1_000)
        XCTAssertEqual(try input.f64(), 62, accuracy: 0)
        XCTAssertEqual(try input.i64(), 31_000)
        XCTAssertEqual(try input.f64(), 64.5, accuracy: 0)
        try input.finished()

        let session = HealthSample(
            id: "sleep-1",
            startMs: 0,
            endMs: 3_600_000,
            source: HealthSource(),
            value: .sleep(stages: [
                SleepStage(startMs: 0, endMs: 1_800_000, stage: .light),
                SleepStage(startMs: 1_800_000, endMs: 3_600_000, stage: .rem)
            ])
        )
        var sleepReader = Reader(try encodeSample(session))
        XCTAssertEqual(try sleepReader.string(), "sleep-1")
        XCTAssertEqual(try sleepReader.u32(), 5)
        XCTAssertEqual(try sleepReader.i64(), 0)
        XCTAssertEqual(try sleepReader.i64(), 3_600_000)
        try sleepReader.skipOptionals(count: 3)
        XCTAssertEqual(try sleepReader.u32(), 2)
        for stage in [
            (start: 0, end: 1_800_000, stage: 3),
            (start: 1_800_000, end: 3_600_000, stage: 5)
        ] {
            XCTAssertEqual(try sleepReader.i64(), Int64(stage.start))
            XCTAssertEqual(try sleepReader.i64(), Int64(stage.end))
            XCTAssertEqual(try sleepReader.u32(), UInt32(stage.stage))
        }
        try sleepReader.finished()
    }

    func testScalarsAndWorkoutsKeepTheirUnitsAndCodes() throws {
        let workout = HealthSample(
            id: "run-1",
            startMs: 0,
            endMs: 60_000,
            source: HealthSource(appId: "com.example.app"),
            value: .exercise(activity: 2, platformCode: 52, title: "Morning run")
        )
        var runReader = Reader(try encodeSample(workout))
        XCTAssertEqual(try runReader.string(), "run-1")
        XCTAssertEqual(try runReader.u32(), 8)
        XCTAssertEqual(try runReader.i64(), 0)
        XCTAssertEqual(try runReader.i64(), 60_000)
        XCTAssertEqual(try runReader.string(), "com.example.app")
        try runReader.skipOptionals(count: 2)
        XCTAssertEqual(try runReader.u32(), 2)
        XCTAssertEqual(try runReader.u32(), 52)
        XCTAssertEqual(try runReader.string(), "Morning run")
        try runReader.finished()
    }

    func testReadRequestsRejectWhatTheRustCodecRejects() throws {
        let valid = try ReadRequest(read(4, 1, 2, 10, nil))
        XCTAssertEqual(valid.type, 4)
        XCTAssertEqual(valid.token, nil)

        let invalid = try [
            read(2, 1, 2, 10, nil), // RMSSD stays Health Connect only.
            read(5, 2, 2, 10, nil),
            read(4, 1, 2, 0, nil),
            read(4, 1, 2, 1001, nil),
            read(4, 1, 2, 10, "")
        ]
        for payload in invalid {
            XCTAssertThrowsError(try ReadRequest(payload))
        }
    }

    func testTypesRequestsCarryMasksAndFlags() throws {
        let request = try TypesRequest(try types(HealthWire.appleTypes, 0))
        XCTAssertEqual(request.types.count, 8)
        XCTAssertFalse(request.types.contains(2))
        XCTAssertThrowsError(try TypesRequest(try types(0, 0)))
        XCTAssertThrowsError(try TypesRequest(try types(1 << 2, 0)))
        XCTAssertThrowsError(try TypesRequest(try types(1 << 4, 4)))
    }

    func testPagesAndChangesMatchTheRustInvariants() throws {
        let page = try encodePage([], token: "next")
        var reader = Reader(page)
        XCTAssertEqual(try reader.u32(), 0)
        XCTAssertEqual(try reader.string(), "next")
        try reader.finished()

        var expired = ChangesReply()
        expired.expired = true
        var expiredReader = Reader(try expired.encode())
        XCTAssertEqual(try expiredReader.u32(), 0)
        XCTAssertEqual(try expiredReader.u32(), 0)
        XCTAssertEqual(try expiredReader.u32(), HealthWire.absentLength)
        XCTAssertEqual(try expiredReader.u32(), HealthWire.flagExpired)
        try expiredReader.finished()

        var live = ChangesReply()
        live.deleted = ["abc"]
        live.nextToken = "1;4:anchor"
        var liveReader = Reader(try live.encode())
        XCTAssertEqual(try liveReader.u32(), 0)
        XCTAssertEqual(try liveReader.u32(), 1)
        XCTAssertEqual(try liveReader.string(), "abc")
        XCTAssertEqual(try liveReader.string(), "1;4:anchor")
        XCTAssertEqual(try liveReader.u32(), 0)
        try liveReader.finished()
    }
}

/// Asserts the exact bytes of a mask reply.
private func reply(_ bytes: UInt32...) -> Data {
    Data(bytes.flatMap { value in
        withUnsafeBytes(of: value.littleEndian) { Array($0) }
    })
}

/// A types request payload: a mask and flags.
private func types(
    _ mask: UInt32,
    _ flags: UInt32
) throws -> Data {
    var writer = WireWriter()
    writer.u32(mask)
    writer.u32(flags)
    return writer.bytes()
}

/// A read-style request payload.
private func read(
    _ type: UInt32,
    _ start: Int64,
    _ end: Int64,
    _ pageSize: UInt32,
    _ token: String?
) throws -> Data {
    var writer = WireWriter()
    writer.u32(type)
    writer.i64(start)
    writer.i64(end)
    writer.u32(pageSize)
    try writer.optional(token)
    return writer.bytes()
}

/// A raw reply reader, mirroring the framing under test.
private struct Reader {
    private let bytes: [UInt8]
    private var position = 0

    init(_ data: Data) {
        bytes = [UInt8](data)
    }

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
        guard length != HealthWire.absentLength else {
            struct Absent: Error {}
            throw Absent()
        }
        let count = Int(length)
        position += count
        guard let value = String(bytes: bytes[position - count..<position], encoding: .utf8) else {
            struct Invalid: Error {}
            throw Invalid()
        }
        return value
    }

    mutating func skipOptionals(count: Int) throws {
        for _ in 0..<count {
            if try u32() == HealthWire.absentLength { continue }
            struct Present: Error {}
            throw Present()
        }
    }

    func finished() throws {
        guard position == bytes.count else {
            struct Trailing: Error {}
            throw Trailing()
        }
    }

    private mutating func load<T>(_ size: Int, _ decode: (UnsafeRawBufferPointer) throws -> T) throws -> T {
        defer { position += size }
        return try bytes.withUnsafeBytes { raw in
            try decode(UnsafeRawBufferPointer(rebasing: raw[position..<position + size]))
        }
    }
}
