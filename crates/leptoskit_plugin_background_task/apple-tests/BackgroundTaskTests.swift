import CLeptosKitBridge
import Foundation
@testable import LeptosKitBackgroundTask
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
struct BackgroundTaskTests {
    @Test func wireRegisterAndNameRoundTrip() throws {
        var register = Data()
        appendU32(&register, 900)
        appendString(&register, "sync")
        let decoded = try BackgroundTaskWire.register(buffer(register))
        #expect(decoded.name == "sync")
        #expect(decoded.interval == 900)
        var name = Data()
        appendString(&name, "sync")
        #expect(try BackgroundTaskWire.name(buffer(name)) == "sync")
        #expect(BackgroundTaskWire.status(.available).count == 4)
    }

    @Test func rejectsShortIntervalAndEmptyName() async throws {
        var short = Data()
        appendU32(&short, 60)
        appendString(&short, "sync")
        await #expect(throws: PluginError.invalidArgument) {
            _ = try BackgroundTaskWire.register(buffer(short))
        }
        var empty = Data()
        appendString(&empty, "")
        await #expect(throws: PluginError.invalidArgument) {
            _ = try BackgroundTaskWire.name(buffer(empty))
        }
    }
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
