import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitVideoThumbnails
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
struct VideoThumbnailsTests {
    @Test func wireGetThumbnailAndReplyRoundTrip() throws {
        var request = Data()
        appendU64(&request, 1500)
        appendU32(&request, 80)
        appendString(&request, "file:///tmp/a.mp4")
        let decoded = try VideoThumbnailsWire.getThumbnail(buffer(request))
        #expect(decoded.uri == "file:///tmp/a.mp4")
        #expect(decoded.timeMs == 1500)
        #expect(decoded.quality == 80)
        let reply = try VideoThumbnailsWire.thumbnail(uri: "file:///tmp/t.jpg", width: 320, height: 180)
        #expect(!reply.isEmpty)
    }

    @Test func rejectsBadQualityAndEmptyUri() async throws {
        var badQuality = Data()
        appendU64(&badQuality, 0)
        appendU32(&badQuality, 0)
        appendString(&badQuality, "file:///tmp/a.mp4")
        await #expect(throws: PluginError.invalidArgument) {
            _ = try VideoThumbnailsWire.getThumbnail(buffer(badQuality))
        }
        var empty = Data()
        appendU64(&empty, 0)
        appendU32(&empty, 50)
        appendString(&empty, "")
        await #expect(throws: PluginError.invalidArgument) {
            _ = try VideoThumbnailsWire.getThumbnail(buffer(empty))
        }
    }
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

private func appendU64(_ data: inout Data, _ value: UInt64) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

private func appendString(_ data: inout Data, _ text: String) {
    let utf8 = Array(text.utf8)
    appendU32(&data, UInt32(utf8.count))
    data.append(contentsOf: utf8)
}
