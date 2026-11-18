import CoreLocation
import Foundation
import LeptosKitBridge
@testable import LeptosKitLocation
import Testing

/// A record is five u32 fields followed by eight doubles.
private enum Record {
    /// Where the latitude double starts.
    static let latitude = 28
    /// The full record size.
    static let bytes = 84
}

/// A sampled coordinate the conversion tests measure.
private enum Sample {
    static let latitude = 37.0
    static let longitude = -122.0
    /// Horizontal accuracy in metres.
    static let accuracy = 5.0
    /// Unix seconds the sample was measured at.
    static let timestamp = 1_800_000_000.0
}

/// Request bounds the decoder enforces.
private enum Bound {
    /// Only the first timeout is valid, so the next millisecond is refused.
    static let maxTimeout: UInt32 = 120_000
    /// One degree past the northern limit.
    static let beyondLatitude = maxLatitude + 1
    private static let maxLatitude = 90.0
}

@Test
func queuedLocationsCannotCrossPermissionOrPrecisionRevocation() {
    #expect(permitsLocationDelivery(startedPrecise: true, authorized: true, precise: true))
    #expect(permitsLocationDelivery(startedPrecise: false, authorized: true, precise: false))
    #expect(permitsLocationDelivery(startedPrecise: false, authorized: true, precise: true))
    #expect(!permitsLocationDelivery(startedPrecise: true, authorized: true, precise: false))
    #expect(!permitsLocationDelivery(startedPrecise: false, authorized: false, precise: false))
    #expect(!permitsLocationDelivery(startedPrecise: true, authorized: false, precise: true))
}

@Test
func syntheticCoordinatesPreserveUnknownFieldsWithoutAccessingLocation() throws {
    let location = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: Sample.latitude, longitude: Sample.longitude),
        altitude: 0,
        horizontalAccuracy: Sample.accuracy,
        verticalAccuracy: -1,
        course: -1,
        speed: -1,
        timestamp: Date(timeIntervalSince1970: Sample.timestamp)
    )
    let result = try encodeLocation(location)
    #expect(result.count == Record.bytes)
    result.withUnsafeBytes { bytes in
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self)) == 0)
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: UInt32.self)) == 0)
        let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: Record.latitude, as: UInt64.self))
        let latitude = Double(bitPattern: bits)
        #expect(latitude == Sample.latitude)
    }
    let invalid = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: Bound.beyondLatitude, longitude: Sample.longitude),
        altitude: 0, horizontalAccuracy: Sample.accuracy, verticalAccuracy: -1, timestamp: Date()
    )
    #expect(throws: PluginError.native) { try encodeLocation(invalid) }
}

@Test
func locationRequestBoundsRejectUnboundedAndNonfiniteRequests() throws {
    func request(timeout: UInt32, after: Double? = nil) -> Data {
        var bytes = Data()

        for var field in [UInt32(1), timeout, 0].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { bytes.append(contentsOf: $0) }
        }

        if let after {
            var bits = after.bitPattern.littleEndian
            withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
        }

        return bytes
    }
    let valid = try request(timeout: 1000).withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: false) }
    #expect(valid.highAccuracy && valid.timeout == 1000 && valid.maximumAge == 0)

    for timeout in [UInt32(0), Bound.maxTimeout + 1] {
        #expect(throws: PluginError.invalidArgument) {
            try request(timeout: timeout).withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: false) }
        }
    }

    #expect(throws: PluginError.invalidArgument) {
        try request(timeout: 1000, after: .nan).withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: true) }
    }
}

@Test
func updateRequestsDecodeEveryFieldAndRejectBadWire() throws {
    func request(
        accuracy: UInt32 = 1,
        distance: Double = 12.5,
        interval: UInt32 = 5_000,
        background: UInt32 = 1,
        pauses: UInt32 = 0,
        indicator: UInt32 = 1
    ) -> Data {
        var bytes = Data()

        for var field in [accuracy].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { bytes.append(contentsOf: $0) }
        }

        var bits = distance.bitPattern.littleEndian
        withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }

        for var field in [interval, background, pauses, indicator].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { bytes.append(contentsOf: $0) }
        }

        return bytes
    }

    let valid = try request().withUnsafeBytes { try UpdateOptions(bytes: $0) }
    #expect(valid.accuracy == .navigation && valid.distanceFilterM == 12.5)
    #expect(valid.intervalMs == 5_000 && valid.background && !valid.pausesAutomatically)
    #expect(valid.showBackgroundIndicator)

    for accuracy in [UInt32(3), UInt32(4_294_967_295)] {
        #expect(throws: PluginError.invalidArgument) {
            try request(accuracy: accuracy).withUnsafeBytes { try UpdateOptions(bytes: $0) }
        }
    }

    for distance in [-1.0, 10_000.5, .nan, .infinity] {
        #expect(throws: PluginError.invalidArgument) {
            try request(distance: distance).withUnsafeBytes { try UpdateOptions(bytes: $0) }
        }
    }

    for interval in [UInt32(0), 3_600_001] {
        #expect(throws: PluginError.invalidArgument) {
            try request(interval: interval).withUnsafeBytes { try UpdateOptions(bytes: $0) }
        }
    }

    for background in [UInt32(2), 99] {
        #expect(throws: PluginError.invalidArgument) {
            try request(background: background).withUnsafeBytes { try UpdateOptions(bytes: $0) }
        }
    }

    #expect(throws: PluginError.invalidArgument) {
        try request().dropFirst(4).withUnsafeBytes { try UpdateOptions(bytes: $0) }
    }
}

@Test
func updateRecordsShareTheAcquisitionLayoutWithoutItsTag() throws {
    let location = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: Sample.latitude, longitude: Sample.longitude),
        altitude: 0,
        horizontalAccuracy: Sample.accuracy,
        verticalAccuracy: -1,
        timestamp: Date(timeIntervalSince1970: Sample.timestamp)
    )
    let record = try fixRecord(location)
    #expect(record.count == Record.bytes - 4)

    let tagged = try encodeLocation(location)
    #expect(tagged.count == Record.bytes)
    #expect(Data(tagged.dropFirst(4)) == record)

    record.withUnsafeBytes { bytes in
        // The optional mask moved to the front, so unknown stays unknown.
        #expect(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) == 0)
        let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: Record.latitude - 4, as: UInt64.self))
        #expect(Double(bitPattern: bits) == Sample.latitude)
    }
}

@Test
func updateBatchesCarryDropCountsAndRecordsInOrder() throws {
    let location = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: Sample.latitude, longitude: Sample.longitude),
        altitude: 0,
        horizontalAccuracy: Sample.accuracy,
        verticalAccuracy: -1,
        timestamp: Date(timeIntervalSince1970: Sample.timestamp)
    )
    let record = try fixRecord(location)
    let reply = batch(dropped: 9, records: [record, record])
    #expect(reply.count == 8 + 2 * (Record.bytes - 4))

    reply.withUnsafeBytes { bytes in
        #expect(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) == 9)
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self)) == 2)
        #expect(reply.dropFirst(8) == record + record)
    }

    let ended = batch(dropped: 0, records: [])
    #expect(ended == Data([0, 0, 0, 0, 0, 0, 0, 0]))
}
