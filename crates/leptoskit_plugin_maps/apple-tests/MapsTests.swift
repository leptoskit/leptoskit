import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitMaps
import Testing

private func buffer(_ bytes: Data = Data()) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBytes {
        lk_bridge_submit(bridge, 0, 0, $0.baseAddress?.assumingMemoryBound(to: UInt8.self), $0.count)
    }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)
    return NativeBuffer(taking: request.payload)
}

@MainActor
struct MapsTests {
    @Test func wireCreateCameraMarkersAndStatusRoundTrip() throws {
        try MapsWire.create(buffer())
        let camera = try MapCamera.validated(latitude: 37.77, longitude: -122.42, zoom: 12)
        var setCamera = Data()
        appendU32(&setCamera, 3)
        appendF64(&setCamera, camera.latitude)
        appendF64(&setCamera, camera.longitude)
        appendF64(&setCamera, camera.zoom)
        let decoded = try MapsWire.setCamera(buffer(setCamera))
        #expect(decoded.id == 3)
        #expect(decoded.camera == camera)

        var markers = Data()
        appendU32(&markers, 1)
        appendU32(&markers, 1)
        appendF64(&markers, 1)
        appendF64(&markers, 2)
        appendString(&markers, "Pin")
        let listed = try MapsWire.setMarkers(buffer(markers))
        #expect(listed.id == 1)
        #expect(listed.markers.count == 1)
        #expect(listed.markers[0].title == "Pin")

        let status = MapsWire.status(camera: .default, markerCount: 1, active: true)
        #expect(status.count == 36)
        #expect(try MapsWire.session(buffer(words(9))) == 9)
    }

    @Test func rejectsBadCoordinatesAndOversizeMarkerLists() async throws {
        await #expect(throws: PluginError.invalidArgument) {
            _ = try MapCamera.validated(latitude: 91, longitude: 0, zoom: 1)
        }
        var over = Data()
        appendU32(&over, 1)
        appendU32(&over, MapsWire.maxMarkers &+ 1)
        await #expect(throws: PluginError.tooLarge) {
            _ = try MapsWire.setMarkers(buffer(over))
        }
    }
}

private func words(_ values: UInt32...) -> Data {
    var data = Data(capacity: values.count * 4)
    for value in values {
        appendU32(&data, value)
    }
    return data
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

private func appendF64(_ data: inout Data, _ value: Double) {
    var bits = value.bitPattern.littleEndian
    withUnsafeBytes(of: &bits) { data.append(contentsOf: $0) }
}

private func appendString(_ data: inout Data, _ text: String) {
    let utf8 = Array(text.utf8)
    appendU32(&data, UInt32(utf8.count))
    data.append(contentsOf: utf8)
}
