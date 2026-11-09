import CLeptosKitBridge
import Foundation
@testable import LeptosKitDiagnostics
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
struct DiagnosticsTests {
    @Test func nameBoundsAndNul() throws {
        var payload = Data()
        appendU32(&payload, 4)
        payload.append(contentsOf: Array("boot".utf8))
        #expect(try DiagnosticsWire.name(buffer(payload)) == "boot")

        var nul = Data()
        appendU32(&nul, 3)
        nul.append(contentsOf: [0x61, 0x00, 0x62])
        #expect(throws: PluginError.invalidArgument) {
            _ = try DiagnosticsWire.name(buffer(nul))
        }

        var trailing = payload
        trailing.append(0)
        #expect(throws: PluginError.invalidArgument) {
            _ = try DiagnosticsWire.name(buffer(trailing))
        }
        #expect(DiagnosticsWire.capability(.available) == DiagnosticsWire.encodeId(1))
    }

    @Test func idRejectsTrailing() throws {
        var id = Data()
        appendU32(&id, 9)
        #expect(try DiagnosticsWire.id(buffer(id)) == 9)
        id.append(0)
        #expect(throws: PluginError.invalidArgument) {
            _ = try DiagnosticsWire.id(buffer(id))
        }
    }
}
