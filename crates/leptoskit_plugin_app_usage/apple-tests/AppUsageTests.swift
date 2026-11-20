import Foundation
import LeptosKitBridge
@testable import LeptosKitAppUsage
import Testing

/// Wire bounds and buffer behaviour the tests hold the plugin to.
private enum Bound {
    /// One event's framing: kind, timestamp and an absent package length.
    static let eventBytes = 16
    /// The buffer's documented capacity.
    static let capacity = 64
}

private let locked: UInt32 = 8
private let unlocked: UInt32 = 9

@Test
func eventBatchesFrameCountKindAndAbsentPackage() {
    let batch = encodeEventBatch([(kind: locked, timestampMs: 1), (kind: unlocked, timestampMs: 2)])
    #expect(batch.count == 4 + 2 * Bound.eventBytes)

    batch.withUnsafeBytes { bytes in
        #expect(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) == 2)
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self)) == locked)
        #expect(Int64(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: Int64.self)) == 1)
        // The timestamp takes bytes 8 to 15, so the package length follows at 16.
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 16, as: UInt32.self)) == .max)
        #expect(
            UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 20, as: UInt32.self)) == unlocked
        )
    }

    #expect(encodeEventBatch([]) == Data([0, 0, 0, 0]))
}

@Test
func eventBuffersDropTheOldestTransitionAndDrainOldestFirst() {
    var buffer = EventBuffer(capacity: 3)
    #expect(buffer.drain() == nil)

    buffer.append(kind: locked, timestampMs: 1)
    buffer.append(kind: unlocked, timestampMs: 2)
    buffer.append(kind: locked, timestampMs: 3)
    buffer.append(kind: unlocked, timestampMs: 4)

    let batch = buffer.drain()
    #expect(batch != nil)
    batch?.withUnsafeBytes { bytes in
        #expect(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) == 3)
        // The first transition was dropped when the fourth arrived.
        #expect(Int64(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: Int64.self)) == 2)
    }
    #expect(buffer.drain() == nil)

    for index in 0..<6 {
        buffer.append(kind: locked, timestampMs: Int64(index))
    }

    // The cap keeps the rest buffered, dropping only the returned ones.
    let capped = buffer.drain(upTo: 2)
    capped?.withUnsafeBytes { bytes in
        #expect(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) == 2)
        #expect(Int64(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: Int64.self)) == 3)
    }
    #expect(buffer.drain() != nil)
}

@Test
func eventQueriesValidateTheWireBoundsAppleHandlersShare() throws {
    func query(
        since: Int64 = 0,
        until: Int64 = 1_000,
        max: UInt32 = 128
    ) -> Data {
        var bytes = Data()

        for var field in [since, until].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { bytes.append(contentsOf: $0) }
        }

        for var field in [max].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { bytes.append(contentsOf: $0) }
        }

        return bytes
    }

    #expect(try query().withUnsafeBytes { try decodeEventQuery($0) } == 128)
    #expect(try query(max: 1).withUnsafeBytes { try decodeEventQuery($0) } == 1)

    for invalid in [query(since: -1), query(until: -1), query(max: 0), query(max: 1_025)] {
        #expect(throws: PluginError.invalidArgument) {
            try invalid.withUnsafeBytes { _ = try decodeEventQuery($0) }
        }
    }

    #expect(throws: PluginError.invalidArgument) {
        try query().dropFirst().withUnsafeBytes { _ = try decodeEventQuery($0) }
    }
}
