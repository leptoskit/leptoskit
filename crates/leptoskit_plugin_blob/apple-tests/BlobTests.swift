import CLeptosKitBridge
import Foundation
@testable import LeptosKitBlob
import LeptosKitBridge
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

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

@MainActor
struct BlobTests {
    @Test func createBoundsAndTrailing() throws {
        var payload = Data()
        appendU32(&payload, 2)
        payload.append(contentsOf: Array("hi".utf8))
        #expect(try BlobWire.create(buffer(payload)).count == 2)

        var trailing = payload
        trailing.append(0)
        #expect(throws: PluginError.invalidArgument) {
            _ = try BlobWire.create(buffer(trailing))
        }
        #expect(throws: PluginError.invalidArgument) {
            _ = try BlobWire.create(buffer(Data()))
        }
    }

    @Test func readAndIdRejectTrailing() throws {
        var payload = Data()
        appendU32(&payload, 1)
        appendU32(&payload, 0)
        appendU32(&payload, 1)
        #expect(try BlobWire.read(buffer(payload)) == (1, 0, 1))
        payload.append(0)
        #expect(throws: PluginError.invalidArgument) {
            _ = try BlobWire.read(buffer(payload))
        }

        var id = Data()
        appendU32(&id, 9)
        #expect(try BlobWire.id(buffer(id)) == 9)
        id.append(0)
        #expect(throws: PluginError.invalidArgument) {
            _ = try BlobWire.id(buffer(id))
        }
        #expect(BlobWire.capability(.available) == BlobWire.encodeId(1))
        #expect(try BlobWire.encodeSlice(Data("ab".utf8)).count == 6)
    }
}
