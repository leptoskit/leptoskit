import Foundation
import LeptosKitBridge
import MapKit

enum MapsWire {
    static let maxMarkers: UInt32 = 32
    static let maxTitleBytes = 128
    static let wireVersion: UInt32 = 1

    static func create(_ payload: NativeBuffer) throws {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            try reader.finish()
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

    static func setCamera(_ payload: NativeBuffer) throws -> (id: UInt32, camera: MapCamera) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let camera = try reader.camera()
            try reader.finish()
            return (id, camera)
        }
    }

    static func setMarkers(_ payload: NativeBuffer) throws -> (id: UInt32, markers: [MapMarker]) {
        try payload.withUnsafeBytes { bytes in
            var reader = WireReader(bytes)
            let id = try reader.u32()
            let count = try reader.u32()
            guard count <= maxMarkers else { throw PluginError.tooLarge }
            var markers: [MapMarker] = []
            markers.reserveCapacity(Int(count))
            for _ in 0..<count {
                markers.append(try reader.marker())
            }
            try reader.finish()
            return (id, markers)
        }
    }

    static func status(camera: MapCamera, markerCount: UInt32, active: Bool) -> Data {
        var data = Data(capacity: 36)
        appendU32(&data, wireVersion)
        appendF64(&data, camera.latitude)
        appendF64(&data, camera.longitude)
        appendF64(&data, camera.zoom)
        appendU32(&data, markerCount)
        appendU32(&data, active ? 1 : 0)
        return data
    }

    static func sessionId(_ id: UInt32) -> Data {
        var data = Data(capacity: 4)
        appendU32(&data, id)
        return data
    }
}

struct MapCamera: Equatable {
    let latitude: Double
    let longitude: Double
    let zoom: Double

    static let `default` = MapCamera(latitude: 0, longitude: 0, zoom: 2)

    init(latitude: Double, longitude: Double, zoom: Double) {
        self.latitude = latitude
        self.longitude = longitude
        self.zoom = zoom
    }

    static func validated(latitude: Double, longitude: Double, zoom: Double) throws -> MapCamera {
        guard latitude.isFinite, (-90...90).contains(latitude) else { throw PluginError.invalidArgument }
        guard longitude.isFinite, (-180...180).contains(longitude) else { throw PluginError.invalidArgument }
        guard zoom.isFinite, (0...22).contains(zoom) else { throw PluginError.invalidArgument }
        return MapCamera(latitude: latitude, longitude: longitude, zoom: zoom)
    }

    var region: MKCoordinateRegion {
        let delta = max(0.001, 360 / pow(2, zoom))
        return MKCoordinateRegion(
            center: CLLocationCoordinate2D(latitude: latitude, longitude: longitude),
            span: MKCoordinateSpan(latitudeDelta: delta, longitudeDelta: delta)
        )
    }
}

struct MapMarker: Equatable {
    let latitude: Double
    let longitude: Double
    let title: String
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

    mutating func f64() throws -> Double {
        let low = try u32()
        let high = try u32()
        let bits = UInt64(low) | (UInt64(high) << 32)
        return Double(bitPattern: bits)
    }

    mutating func string() throws -> String {
        let count = Int(try u32())
        guard count >= 0, offset + count <= bytes.count else { throw PluginError.invalidArgument }
        let slice = UnsafeRawBufferPointer(rebasing: bytes[offset ..< offset + count])
        offset += count
        guard let value = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }
        return value
    }

    mutating func camera() throws -> MapCamera {
        try MapCamera.validated(latitude: try f64(), longitude: try f64(), zoom: try f64())
    }

    mutating func marker() throws -> MapMarker {
        let latitude = try f64()
        let longitude = try f64()
        let title = try string()
        guard latitude.isFinite, (-90...90).contains(latitude) else { throw PluginError.invalidArgument }
        guard longitude.isFinite, (-180...180).contains(longitude) else { throw PluginError.invalidArgument }
        guard !title.contains("\0"), title.utf8.count <= MapsWire.maxTitleBytes else {
            throw PluginError.invalidArgument
        }
        return MapMarker(latitude: latitude, longitude: longitude, title: title)
    }

    mutating func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

private func appendF64(_ data: inout Data, _ value: Double) {
    var bits = value.bitPattern.littleEndian
    withUnsafeBytes(of: &bits) { data.append(contentsOf: $0) }
}
