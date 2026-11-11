import LeptosKitBridge
@testable import LeptosKitRecorder
import XCTest

/// Codec, level-math, rotation and recovery tests over synthetic payloads
/// and temporary directories; these never open the microphone or prompt.
final class RecorderTests: XCTestCase {
    // MARK: Wire framing

    func testPermissionAndRecoveryRepliesMatchTheRustFraming() {
        XCTAssertEqual(encodePermission(RecorderWire.permissionDenied), Data([2, 0, 0, 0]))
        XCTAssertEqual(
            encodeRecovery(recovered: 3, discarded: 1),
            Data([3, 0, 0, 0, 1, 0, 0, 0])
        )
    }

    func testStartRequestsClampAndRejectLikeTheRustCodec() throws {
        let payload = try start("/var/recorder", segmentSecs: 2, sampleRate: 16_000, bitrate: 24_000, threshold: -45)
        let clamped = try StartRequest(payload)
        XCTAssertEqual(clamped.segmentSecs, RecorderWire.minSegmentSecs)
        XCTAssertEqual(clamped.directory, "/var/recorder")

        let tuned = try StartRequest(
            try start("/d", segmentSecs: 60, sampleRate: 48_000, bitrate: 128_000, threshold: -60)
        )
        XCTAssertEqual(tuned.segmentSecs, 60)
        XCTAssertEqual(tuned.sampleRate, 48_000)

        for invalid in [
            try start("relative", segmentSecs: 60, sampleRate: 16_000, bitrate: 24_000, threshold: -45),
            try start("/a/../b", segmentSecs: 60, sampleRate: 16_000, bitrate: 24_000, threshold: -45),
            try start("/o\0k", segmentSecs: 60, sampleRate: 16_000, bitrate: 24_000, threshold: -45),
            try start("/d", segmentSecs: 60, sampleRate: 4_000, bitrate: 24_000, threshold: -45),
            try start("/d", segmentSecs: 60, sampleRate: 16_000, bitrate: 24_000, threshold: 1),
            try start("/d", segmentSecs: 60, sampleRate: 16_000, bitrate: 24_000, threshold: -120)
        ] {
            XCTAssertThrowsError(try StartRequest(invalid))
        }

        var trailing = try start("/d", segmentSecs: 60, sampleRate: 16_000, bitrate: 24_000, threshold: -45)
        trailing.append(0)
        XCTAssertThrowsError(try StartRequest(trailing))
    }

    func testStatusRepliesPairStatesWithReasonsAndOpenSegments() throws {
        var recording = StatusReply()
        recording.state = RecorderWire.stateRecording
        recording.currentSegmentStartedAtMs = 1_700_000_000_000
        var reader = Reader(try recording.encode())
        XCTAssertEqual(try reader.u32(), RecorderWire.stateRecording)
        XCTAssertEqual(try reader.i64(), 1_700_000_000_000)
        XCTAssertEqual(try reader.optional(), nil)
        try reader.finished()

        var failed = StatusReply()
        failed.state = RecorderWire.stateFailed
        failed.reason = "engine stopped"
        var failureReader = Reader(try failed.encode())
        XCTAssertEqual(try failureReader.u32(), RecorderWire.stateFailed)
        XCTAssertEqual(try failureReader.i64(), 0)
        XCTAssertEqual(try failureReader.optional(), "engine stopped")
        try failureReader.finished()

        // Only the failed state carries a reason, and only an open segment
        // a start time.
        var mixed = StatusReply()
        mixed.state = RecorderWire.stateRecording
        mixed.reason = "x"
        XCTAssertThrowsError(try mixed.encode())

        mixed.reason = nil
        mixed.state = RecorderWire.stateInterrupted
        mixed.currentSegmentStartedAtMs = 5
        XCTAssertThrowsError(try mixed.encode())
    }

    func testSegmentRepliesCarryTheirMetrics() throws {
        let segment = SegmentReply(
            path: "/var/recorder/seg-1.m4a",
            startedAtMs: 1_700_000_000_000,
            durationMs: 300_000,
            bytes: 900_000,
            peakDbfs: -12.5,
            activeMs: 42_000,
            endedBy: RecorderWire.endedRotation
        )
        var reader = Reader(try segment.encode())
        XCTAssertEqual(try reader.u32(), 1)
        XCTAssertEqual(try reader.string(), "/var/recorder/seg-1.m4a")
        XCTAssertEqual(try reader.i64(), 1_700_000_000_000)
        XCTAssertEqual(try reader.i64(), 300_000)
        XCTAssertEqual(try reader.i64(), 900_000)
        XCTAssertEqual(try reader.f64(), -12.5, accuracy: 0)
        XCTAssertEqual(try reader.i64(), 42_000)
        XCTAssertEqual(try reader.u32(), RecorderWire.endedRotation)
        try reader.finished()

        XCTAssertEqual(encodeAbsentSegment(), Data([0, 0, 0, 0]))

        for broken in [
            SegmentReply(
                path: "", startedAtMs: 1, durationMs: 1, bytes: 1, peakDbfs: -10, activeMs: 0,
                endedBy: RecorderWire.endedStop
            ),
            SegmentReply(
                path: "/d/seg.m4a", startedAtMs: 0, durationMs: 1, bytes: 1, peakDbfs: -10, activeMs: 0,
                endedBy: RecorderWire.endedStop
            ),
            SegmentReply(
                path: "/d/seg.m4a", startedAtMs: 1, durationMs: 1, bytes: 0, peakDbfs: -10, activeMs: 0,
                endedBy: RecorderWire.endedStop
            ),
            SegmentReply(
                path: "/d/seg.m4a", startedAtMs: 1, durationMs: 10, bytes: 1, peakDbfs: -10, activeMs: 11,
                endedBy: RecorderWire.endedStop
            )
        ] {
            XCTAssertThrowsError(try broken.encode())
        }
    }

    // MARK: Level math

    func testFrameLevelsMeasureRmsAndPeakAgainstFullScale() {
        // A full-scale sine: RMS −3.01 dBFS, peak 0 dBFS.
        let loud = RecorderLevels.measure(sine(amplitude: 1, count: 480)[...])
        XCTAssertEqual(loud.rmsDbfs, -3.0103, accuracy: 0.001)
        XCTAssertEqual(loud.peakDbfs, 0, accuracy: 0.001)
        XCTAssertTrue(loud.isActive(thresholdDbfs: -45))

        // A half-scale sine: RMS −9.03 dBFS, peak −6.02 dBFS.
        let half = RecorderLevels.measure(sine(amplitude: 0.5, count: 480)[...])
        XCTAssertEqual(half.rmsDbfs, -9.0309, accuracy: 0.001)
        XCTAssertEqual(half.peakDbfs, -6.0206, accuracy: 0.001)

        // Digital silence floors at −120 dBFS.
        let silence = RecorderLevels.measure([Float](repeating: 0, count: 480)[...])
        XCTAssertEqual(silence.rmsDbfs, RecorderLevels.floorDbfs)
        XCTAssertEqual(silence.peakDbfs, RecorderLevels.floorDbfs)
        XCTAssertFalse(silence.isActive(thresholdDbfs: -45))

        // A quiet sine below the threshold: its RMS is amplitude over √2.
        let quiet = RecorderLevels.measure(sine(amplitude: 0.0055, count: 480)[...])
        XCTAssertEqual(quiet.rmsDbfs, 20 * log10(0.0055 / 2.squareRoot()), accuracy: 0.01)
        XCTAssertFalse(quiet.isActive(thresholdDbfs: -45))
        XCTAssertTrue(quiet.isActive(thresholdDbfs: -50))
    }

    func testSegmentTotalsAccumulateActiveAndQuietFrames() {
        var levels = SegmentLevels(thresholdDbfs: -45)
        for _ in 0..<10 {
            levels.add(RecorderLevels.measure(sine(amplitude: 1, count: 480)[...]))
        }
        for _ in 0..<5 {
            levels.add(RecorderLevels.measure([Float](repeating: 0, count: 480)[...]))
        }
        XCTAssertEqual(levels.durationMs, 450)
        XCTAssertEqual(levels.activeMs, 300)
        XCTAssertEqual(levels.peakDbfs, 0, accuracy: 0.001)
    }

    // MARK: Rotation timing

    func testSegmentsRotateExactlyAtTheBoundary() {
        // 5 seconds at 16 kHz: 80_000 samples per segment.
        XCTAssertEqual(RecorderLevels.frameSamples(sampleRate: 16_000), 480)

        // A frame that fits stays whole.
        XCTAssertNil(RecorderLevels.split(written: 0, count: 480, segmentFrames: 80_000))
        XCTAssertNil(RecorderLevels.split(written: 79_500, count: 480, segmentFrames: 80_000))

        // A frame that crosses the boundary splits at exactly the samples
        // remaining: 79_900 + 100 reaches 80_000, the last 380 open the
        // next segment.
        XCTAssertEqual(RecorderLevels.split(written: 79_900, count: 480, segmentFrames: 80_000), 100)
        XCTAssertNil(RecorderLevels.split(written: 80_000, count: 480, segmentFrames: 80_000))

        // Rotation lands on the boundary after a whole number of frames.
        let frames = 80_000 / 480
        var written = 0
        for _ in 0..<frames {
            XCTAssertNil(RecorderLevels.split(written: written, count: 480, segmentFrames: 80_000))
            written += 480
        }
        XCTAssertEqual(written, 79_680)
        // The last partial frame fills the segment and splits.
        XCTAssertEqual(RecorderLevels.split(written: written, count: 480, segmentFrames: 80_000), 320)
    }

    // MARK: Index journal and recovery

    func testJournalRoundTripsAndSkipsTornLines() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("recorder-tests-\(UUID().uuidString)")
        let index = try RecorderIndex(url: url)

        try index.recordOpen(filename: "seg-1.m4a", startedMs: 100, plannedMs: 300_000)
        try index.recordFinalised(
            filename: "seg-1.m4a", durationMs: 300_000, bytes: 900_000, peakDbfs: -12.5,
            activeMs: 42_000, endedBy: RecorderWire.endedRotation
        )
        try index.recordDelivered(filename: "seg-1.m4a")

        let lines = try index.read()
        XCTAssertEqual(lines.count, 3)
        XCTAssertEqual(
            lines.first,
            .open(file: "seg-1.m4a", startedMs: 100, plannedMs: 300_000)
        )
        XCTAssertEqual(
            lines.last,
            .delivered(file: "seg-1.m4a")
        )

        // Torn and unknown lines are skipped, never fatal.
        let torn = "F\tseg-2.m4a\t100\nC\tseg-2.m4a\t1\t2\t3\nD\nX\tnonsense\n"
        XCTAssertEqual(RecorderIndex.parse(torn), [])

        try? FileManager.default.removeItem(at: url)
    }

    func testRecoveryDeliversUndeliveredFilesAndDeletesTruncatedOnes() throws {
        let journal = """
        F\tseg-1.m4a\t100\t300000
        C\tseg-1.m4a\t300000\t900000\t-12.5\t42000\t0
        D\tseg-1.m4a
        F\tseg-2.m4a\t300100\t300000
        C\tseg-2.m4a\t300000\t900000\t-30\t1000\t0
        F\tseg-3.m4a\t600200\t300000
        """
        let lines = RecorderIndex.parse(journal)

        // seg-1 was delivered, seg-2 finished but was never delivered,
        // seg-3 was still being written when the process died.
        let plan = RecorderIndex.plan(
            lines: lines,
            existingFiles: ["seg-1.m4a", "seg-2.m4a", "seg-3.m4a", "seg-4.m4a", "notes.txt"]
        )
        XCTAssertEqual(plan.deliverable.map(\.filename), ["seg-2.m4a"])
        XCTAssertEqual(plan.deliverable.first?.startedAtMs, 300_100)
        XCTAssertEqual(plan.deliverable.first?.peakDbfs ?? 0, -30, accuracy: 0)
        XCTAssertEqual(
            plan.discardable,
            ["seg-3.m4a", "seg-4.m4a"],
            "the truncated file and the unindexed orphan are deleted, the app's other files are not"
        )

        // Everything delivered means nothing to do.
        let settled = RecorderIndex.plan(
            lines: RecorderIndex.parse(
                "F\ts.m4a\t1\t1\nC\ts.m4a\t1\t1\t-1\t0\t1\nD\ts.m4a\n"
            ),
            existingFiles: ["s.m4a"]
        )
        XCTAssertEqual(settled.deliverable, [])
        XCTAssertEqual(settled.discardable, [])
    }

    func testCompactionKeepsOnlyUndeliveredEntries() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("recorder-tests-\(UUID().uuidString)")
        let index = try RecorderIndex(url: url)
        for number in 1...3 {
            try index.recordOpen(filename: "seg-\(number).m4a", startedMs: Int64(100 * number), plannedMs: 1)
            try index.recordFinalised(
                filename: "seg-\(number).m4a", durationMs: 1, bytes: 10, peakDbfs: -1,
                activeMs: 0, endedBy: RecorderWire.endedRotation
            )
            try index.recordDelivered(filename: "seg-\(number).m4a")
        }
        XCTAssertEqual(try index.read().count, 9)

        let keeping = [
            RecorderIndex.Finalised(
                filename: "seg-4.m4a", startedAtMs: 400, durationMs: 1, bytes: 10, peakDbfs: -1,
                activeMs: 0, endedBy: RecorderWire.endedRotation
            )
        ]
        try index.compact(keeping: keeping)

        let lines = try index.read()
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(
            lines,
            [
                .open(file: "seg-4.m4a", startedMs: 400, plannedMs: 1),
                .finalised(
                    file: "seg-4.m4a", durationMs: 1, bytes: 10, peakDbfs: -1, activeMs: 0,
                    endedBy: RecorderWire.endedRotation
                )
            ]
        )

        try? FileManager.default.removeItem(at: url)
    }
}

/// Builds a start request payload.
private func start(
    _ directory: String,
    segmentSecs: UInt32,
    sampleRate: UInt32,
    bitrate: UInt32,
    threshold: Double
) throws -> Data {
    var writer = WireWriter()
    try writer.string(directory)
    writer.u32(segmentSecs)
    writer.u32(sampleRate)
    writer.u32(bitrate)
    try writer.f64(threshold)
    return writer.bytes()
}

/// A sine wave of `count` samples at `amplitude`.
private func sine(amplitude: Float, count: Int) -> [Float] {
    (0..<count).map { index in
        amplitude * Float(sin(Double(index) * 2 * .pi / 12))
    }
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
        guard length != RecorderWire.absentLength else {
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

    mutating func optional() throws -> String? {
        if try u32() == RecorderWire.absentLength { return nil }
        position -= 4
        return try string()
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
