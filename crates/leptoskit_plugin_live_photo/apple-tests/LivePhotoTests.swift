import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitLivePhoto
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
struct LivePhotoTests {
    @Test func wireCreatePlaybackAndStatusRoundTrip() throws {
        var create = Data()
        appendString(&create, "ph://asset-1")
        #expect(try LivePhotoWire.create(buffer(create)) == "ph://asset-1")
        let start = try LivePhotoWire.startPlayback(buffer(words(3, 1)))
        #expect(start.id == 3)
        #expect(start.style == 1)
        #expect(try LivePhotoWire.session(buffer(words(9))) == 9)
        let status = LivePhotoWire.status(active: true, playing: false)
        #expect(status.count == 12)
    }

    @Test func rejectsBadStyleAndEmptyUri() async throws {
        await #expect(throws: PluginError.invalidArgument) {
            _ = try LivePhotoWire.startPlayback(buffer(words(1, 9)))
        }
        var empty = Data()
        appendString(&empty, "")
        await #expect(throws: PluginError.invalidArgument) {
            _ = try LivePhotoWire.create(buffer(empty))
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

private func appendString(_ data: inout Data, _ text: String) {
    let utf8 = Array(text.utf8)
    appendU32(&data, UInt32(utf8.count))
    data.append(contentsOf: utf8)
}
