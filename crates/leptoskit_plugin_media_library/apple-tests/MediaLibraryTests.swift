import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitMediaLibrary
import Testing
#if canImport(AppKit)
import AppKit
#elseif canImport(UIKit)
import UIKit
#endif

@MainActor
private final class MediaLibrarySpy: MediaLibraryService {
    var statusValue: MediaLibraryStatus = .granted
    var listed: [MediaLibraryListOptions] = []
    var saved: [MediaLibrarySaveRequest] = []

    func status() -> MediaLibraryStatus { statusValue }

    func requestPermission() async -> MediaLibraryStatus { statusValue }

    func list(_ options: MediaLibraryListOptions) async throws -> [MediaLibraryAsset] {
        listed.append(options)
        return [
            MediaLibraryAsset(
                id: "a1",
                kind: .images,
                uri: "ph://a1",
                width: 10,
                height: 20,
                creationTimeMs: 1_700
            )
        ]
    }

    func save(_ request: MediaLibrarySaveRequest) async throws -> MediaLibraryAsset {
        saved.append(request)
        return MediaLibraryAsset(
            id: "saved",
            kind: request.kind,
            uri: "ph://saved",
            width: nil,
            height: nil,
            creationTimeMs: nil
        )
    }
}

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

private func listBytes(kind: UInt32, limit: UInt32) -> [UInt8] {
    var bytes: [UInt8] = []
    func appendU32(_ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
    }
    appendU32(kind)
    appendU32(limit)
    return bytes
}

@Test
@MainActor
func injectedLibraryNeverInventsDeniedAssets() async throws {
    let spy = MediaLibrarySpy()
    #if canImport(AppKit)
    let owner = NSViewController()
    #elseif canImport(UIKit)
    let owner = UIViewController()
    #endif
    let plugin = MediaLibraryPlugin(context: NativePluginContext(owner: owner), service: spy)
    defer { plugin.close() }
    #expect(try await plugin.invoke(method: 0, payload: payload([])) == Data([3, 0, 0, 0]))
    let listed = try await plugin.invoke(method: 2, payload: payload(listBytes(kind: 0, limit: 4)))
    #expect(!listed.isEmpty)
    #expect(spy.listed.count == 1)
}

@Test
func wireRejectsZeroLimitAndAllSaveKind() throws {
    #expect(throws: PluginError.invalidArgument) {
        try listBytes(kind: 0, limit: 0).withUnsafeBytes(MediaLibraryListOptions.decode)
    }
}
