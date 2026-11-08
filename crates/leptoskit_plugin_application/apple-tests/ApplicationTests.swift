import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitApplication
import Testing

/// Allocates a payload through the real C ABI; its bytes outlive the bridge.
@MainActor
private func payload(_ bytes: [UInt8] = []) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("application".utf8)
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

private func hex(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
}

/// The same fixture and bytes as `apple_fixture` in the Rust crate's tests.
private let appleFixture = ApplicationFacts(
    id: "dev.example.app",
    name: "Example",
    version: "1.2.3",
    build: "42",
    installerStore: "com.apple.testflight",
    installTime: Date(timeIntervalSince1970: 1_700_000_000),
    updateTime: nil
)

private let appleGolden = [
    "020000000f0000006465762e6578616d706c652e617070070000004578616d706c6505000000312e",
    "322e330200000034320000000014000000636f6d2e6170706c652e74657374666c69676874010000",
    "000068e5cf8b010000000000000000000000000000"
].joined()

@Test
func snapshotBytesMatchTheRustGolden() throws {
    #expect(hex(try ApplicationWire.snapshot(appleFixture)) == appleGolden)
}

@Test
func unknownFactsEncodeAsEmptyStringsAndAbsentTimes() throws {
    let data = try ApplicationWire.snapshot(ApplicationFacts())
    #expect(data == Data([2, 0, 0, 0] + [UInt8](repeating: 0, count: 6 * 4 + 2 * 12)))
}

@Test
func timesTruncateToMillisecondsAndRejectEarlierDates() throws {
    #expect(ApplicationWire.millis(Date(timeIntervalSince1970: 1.9999)) == 1_999)
    #expect(ApplicationWire.millis(Date(timeIntervalSince1970: 0)) == 0)
    #expect(ApplicationWire.millis(Date(timeIntervalSince1970: -1)) == nil)
    var facts = appleFixture
    facts.installTime = Date(timeIntervalSince1970: -1)
    #expect(try ApplicationWire.snapshot(facts).suffix(24) == Data(repeating: 0, count: 24))
}

@Test
func oversizedTextIsRejected() {
    var facts = appleFixture
    facts.name = String(repeating: "x", count: ApplicationWire.maxText + 1)
    #expect(throws: PluginError.tooLarge) { try ApplicationWire.snapshot(facts) }
    facts = appleFixture
    facts.installerStore = String(repeating: "x", count: ApplicationWire.maxInstaller + 1)
    #expect(throws: PluginError.tooLarge) { try ApplicationWire.snapshot(facts) }
}

@Test
func installerHeuristicDistinguishesStoreTestFlightDevelopmentAndSimulator() {
    let store = URL(fileURLWithPath: "/private/var/containers/Bundle/Application/X/App.app/StoreKit/receipt")
    let sandbox = store.deletingLastPathComponent().appendingPathComponent("sandboxReceipt")
    let installer = ApplicationFacts.installerStore

    #expect(installer(store, false, false) == "com.apple")
    #expect(installer(sandbox, false, false) == "com.apple.testflight")
    #expect(installer(sandbox, true, false) == "com.apple.development")
    #expect(installer(nil, true, false) == "com.apple.development")
    #expect(installer(nil, false, false) == nil)
    #expect(installer(store, false, true) == "com.apple.simulator")
}

@Test
@MainActor
func pluginReportsFactsAndValidatesCalls() async throws {
    let plugin = ApplicationPlugin(facts: { appleFixture })
    #expect(plugin.name == "application")
    #expect(plugin.methods == [0])
    #expect(hex(try await plugin.invoke(method: 0, payload: payload())) == appleGolden)
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 1, payload: payload()) }
    await #expect(throws: PluginError.invalidArgument) { try await plugin.invoke(method: 0, payload: payload([1])) }
}

@Test
@MainActor
func realBundleFactsDescribeTheTestRunner() async throws {
    let bundle = Bundle.main
    let facts = ApplicationFacts.read(bundle: bundle)
    #expect(facts.id == bundle.bundleIdentifier)
    #expect(facts.build == bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String)
    let updated = try #require(facts.updateTime)
    #expect(updated <= Date())
    #if os(macOS)
    #expect(facts.installerStore == nil)
    if ProcessInfo.processInfo.environment["APP_SANDBOX_CONTAINER_ID"] == nil {
        #expect(facts.installTime == nil, "an unsandboxed Mac app has no container to date")
    }
    #endif
    let data = try await ApplicationPlugin().invoke(method: 0, payload: payload())
    #expect(data.prefix(4) == Data([2, 0, 0, 0]))
}

@Test
func containerDatesComeFromTheFileSystem() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let modified = Date(timeIntervalSince1970: 1_600_000_000)
    try FileManager.default.setAttributes([.modificationDate: modified], ofItemAtPath: directory.path)
    #expect(ApplicationFacts.modificationDate(directory, .default) == modified)
    #expect(ApplicationFacts.modificationDate(directory.appendingPathComponent("missing"), .default) == nil)
}
