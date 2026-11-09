import CLeptosKitBridge
import Foundation
import LeptosKitAsset
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
struct AssetTests {
    @Test func nameBoundsAndNul() throws {
        #expect(AssetWire.capability(.unavailable).count == 4)
        var name = Data()
        appendU32(&name, 4)
        name.append(contentsOf: Array("icon".utf8))
        #expect(try AssetWire.name(buffer(name)) == "icon")

        var nul = Data()
        appendU32(&nul, 3)
        nul.append(contentsOf: [0x61, 0x00, 0x62])
        #expect(throws: PluginError.invalidArgument) {
            _ = try AssetWire.name(buffer(nul))
        }

        var trailing = name
        trailing.append(0)
        #expect(throws: PluginError.invalidArgument) {
            _ = try AssetWire.name(buffer(trailing))
        }
    }
}
