import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitCamera
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
struct CameraTests {
    @Test func wireCreateTorchTakeAndStatusRoundTrip() throws {
        let facing = try CameraWire.create(buffer(words(1)))
        #expect(facing == 1)
        let torch = try CameraWire.torch(buffer(words(3, 1)))
        #expect(torch.id == 3)
        #expect(torch.enabled)
        let take = try CameraWire.takePicture(buffer(words(4, 1024)))
        #expect(take.id == 4)
        #expect(take.maxBytes == 1024)
        let status = CameraWire.status(facing: 0, torchOn: true, torchAvailable: false, active: true)
        #expect(status.count == 20)
        let asset = try CameraWire.asset(
            uri: "file:///tmp/a.jpg", mime: "image/jpeg", width: 10, height: 8, size: 12)
        #expect(!asset.isEmpty)
    }

    @Test func rejectsBadFacingAndOversizeBudget() async throws {
        await #expect(throws: PluginError.invalidArgument) {
            _ = try CameraWire.create(buffer(words(9)))
        }
        await #expect(throws: PluginError.tooLarge) {
            _ = try CameraWire.takePicture(buffer(words(1, CameraWire.maxBytes &+ 1)))
        }
    }

    @Test func createWithoutPermissionFailsHonestly() async throws {
        let plugin = CameraPlugin()
        // Simulator/host without a prior grant returns permissionDenied rather than hanging.
        await #expect(throws: PluginError.permissionDenied) {
            _ = try await plugin.invoke(method: 0, payload: buffer(words(0)))
        }
        plugin.close()
    }

    private func words(_ values: UInt32...) -> Data {
        var data = Data(capacity: values.count * 4)
        for value in values {
            var little = value.littleEndian
            withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
        }
        return data
    }
}
