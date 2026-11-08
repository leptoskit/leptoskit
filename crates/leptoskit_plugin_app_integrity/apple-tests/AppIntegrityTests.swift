import CLeptosKitBridge
import Foundation
import LeptosKitAppIntegrity
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

@MainActor
struct AppIntegrityTests {
    @Test func challengeBounds() throws {
        #expect(AppIntegrityWire.capability(.unavailable).count == 4)
        var challenge = Data()
        appendU32(&challenge, 5)
        challenge.append(contentsOf: Array("nonce".utf8))
        #expect(try AppIntegrityWire.challenge(buffer(challenge)).count == 5)
    }

    @Test func rejectsEmptyChallenge() async throws {
        var empty = Data()
        appendU32(&empty, 0)
        await #expect(throws: PluginError.invalidArgument) {
            _ = try AppIntegrityWire.challenge(buffer(empty))
        }
    }
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}
