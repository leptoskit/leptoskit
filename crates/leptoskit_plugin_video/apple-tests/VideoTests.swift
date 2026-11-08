import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitVideo
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
struct VideoTests {
    @Test func createPlayPauseStatusAndRelease() async throws {
        let plugin = VideoPlugin()
        let created = try await plugin.invoke(method: 0, payload: buffer(source("https://example.com/a.mp4")))
        let id = try readU32(created)
        _ = try await plugin.invoke(method: 2, payload: buffer(player(id)))
        _ = try await plugin.invoke(method: 3, payload: buffer(player(id)))
        let status = try await plugin.invoke(method: 8, payload: buffer(player(id)))
        #expect(status.count == 16)
        _ = try await plugin.invoke(method: 9, payload: buffer(player(id)))
        await #expect(throws: PluginError.invalidArgument) {
            _ = try await plugin.invoke(method: 8, payload: buffer(player(id)))
        }
        plugin.close()
    }

    @Test func rejectsEmptyUrlAndBadRate() async throws {
        let plugin = VideoPlugin()
        await #expect(throws: PluginError.invalidArgument) {
            _ = try await plugin.invoke(method: 0, payload: buffer(source("")))
        }
        let created = try await plugin.invoke(method: 0, payload: buffer(source("https://example.com/a.mp4")))
        let id = try readU32(created)
        await #expect(throws: PluginError.invalidArgument) {
            _ = try await plugin.invoke(method: 7, payload: buffer(rate(id, 100)))
        }
        plugin.close()
    }

    private func source(_ url: String) -> Data {
        var data = Data()
        append(&data, 1)
        append(&data, UInt32(url.utf8.count))
        data.append(contentsOf: url.utf8)
        return data
    }

    private func player(_ id: UInt32) -> Data {
        var data = Data()
        append(&data, id)
        return data
    }

    private func rate(_ id: UInt32, _ thousandths: UInt32) -> Data {
        var data = Data()
        append(&data, id)
        append(&data, thousandths)
        return data
    }

    private func append(_ data: inout Data, _ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    private func readU32(_ data: Data) throws -> UInt32 {
        guard data.count == 4 else { throw PluginError.invalidArgument }
        return data.withUnsafeBytes { $0.load(as: UInt32.self).littleEndian }
    }
}
