import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitDeviceKeys
import Testing

/// Builds a Rust-owned payload by driving a bridge submit/next-request round trip.
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

/// A valid generate request: version, biometry policy, alias.
private func generateRequest() -> Data {
    var bytes = deviceKeysFields([1, 0])
    let alias = Data("payout.v2".utf8)
    bytes.append(deviceKeysFields([UInt32(alias.count)]))
    bytes.append(alias)
    return bytes
}

/// A valid sign request: version, alias, message, reason.
private func signRequest() -> Data {
    var bytes = deviceKeysFields([1])
    let alias = Data("payout.v2".utf8)
    bytes.append(deviceKeysFields([UInt32(alias.count)]))
    bytes.append(alias)
    let message = Data([1, 2, 3])
    bytes.append(deviceKeysFields([UInt32(message.count)]))
    bytes.append(message)
    let reason = Data("Approve transfer".utf8)
    bytes.append(deviceKeysFields([UInt32(reason.count)]))
    bytes.append(reason)
    return bytes
}

@Test
func deviceKeysWireRejectsMalformedRequestsWithoutNativeEvaluation() throws {
    let generate = generateRequest()
    #expect(try DeviceKeysGenerateRequest(payload: buffer(generate)).alias == "payout.v2")

    for count in 0..<generate.count {
        #expect(throws: PluginError.self) { try DeviceKeysGenerateRequest(payload: buffer(generate.prefix(count))) }
    }

    var badPolicy = generate
    badPolicy[4] = 9
    #expect(throws: PluginError.invalidArgument) { try DeviceKeysGenerateRequest(payload: buffer(badPolicy)) }

    let sign = signRequest()
    #expect(try DeviceKeysSignRequest(payload: buffer(sign)).reason == "Approve transfer")

    for count in 0..<sign.count {
        #expect(throws: PluginError.self) { try DeviceKeysSignRequest(payload: buffer(sign.prefix(count))) }
    }

    #expect(throws: PluginError.invalidArgument) { try decodeAlias(buffer(Data([255]))) }
}

@Test
@MainActor
func availabilityAndClosedPluginsNeverPrompt() async throws {
    let plugin = DeviceKeysPlugin()
    defer { plugin.close() }
    let availability = try await plugin.invoke(method: 4, payload: buffer())
    #expect(availability.count == 16)
    #expect(availability.prefix(4) == deviceKeysFields([1]))

    do {
        _ = try await plugin.invoke(method: 4, payload: buffer(Data([0])))
        Issue.record("A non-empty availability request was accepted")
    } catch {
        #expect(error as? PluginError == .invalidArgument)
    }

    plugin.close()
    do {
        _ = try await plugin.invoke(method: 4, payload: buffer())
        Issue.record("A closed device-keys plugin accepted work")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
}
