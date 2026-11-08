import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitContacts
import Testing

private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
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
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@Test
func contactWireRoundTripsAndRejectsNul() throws {
    let contact = try ContactsRecord(id: "1", displayName: "Ada", phone: "+1", email: "a@b.c")
    let encoded = try contact.encode()
    #expect(encoded.count > 8)

    #expect(throws: PluginError.invalidArgument) {
        _ = try ContactsRecord(id: "", displayName: "Ada", phone: nil, email: nil)
    }
    #expect(throws: PluginError.invalidArgument) {
        _ = try ContactsRecord(id: "1\0", displayName: "Ada", phone: nil, email: nil)
    }
    #expect(throws: PluginError.invalidArgument) {
        _ = try ContactsRecord(id: "1", displayName: "Ada", phone: "", email: nil)
    }

    var limit = UInt32(8).littleEndian
    let limitBytes = withUnsafeBytes(of: &limit) { Array($0) }
    #expect(try limitBytes.withUnsafeBytes { try ContactsRecord.decodeQueryLimit($0) } == 8)

    var zero = UInt32(0).littleEndian
    let zeroBytes = withUnsafeBytes(of: &zero) { Array($0) }
    #expect(throws: PluginError.invalidArgument) {
        _ = try zeroBytes.withUnsafeBytes { try ContactsRecord.decodeQueryLimit($0) }
    }

    let list = try ContactsRecord.encodeList([contact])
    #expect(list.count >= 4)
}

#if canImport(UIKit)
import UIKit

@Test
@MainActor
func closedPluginRejectsFurtherCalls() async throws {
    let owner = UIViewController()
    let plugin = ContactsPlugin(context: NativePluginContext(owner: owner))
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        _ = try await plugin.invoke(method: 0, payload: payload([]))
    }
}

@Test
@MainActor
func malformedQueryNeverOpensAddressBook() async throws {
    let owner = UIViewController()
    let plugin = ContactsPlugin(context: NativePluginContext(owner: owner))
    defer { plugin.close() }
    await #expect(throws: PluginError.invalidArgument) {
        _ = try await plugin.invoke(method: 3, payload: payload([0, 0, 0]))
    }
}
#endif
