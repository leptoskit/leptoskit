import CoreGraphics
import Foundation
import LeptosKitBridge
@testable import LeptosKitSharing
import XCTest

/// The request the Rust and Kotlin suites use byte for byte: text "hi", subject "S",
/// title "T", anchor (1, 2, 3, 4), Mail excluded, app file `a.txt` as `text/plain`,
/// and the bytes 1, 2, 3 named `b.bin`.
let goldenRequest: [UInt8] = [
    1, 0, 0, 0,
    2, 0, 0, 0, 104, 105,
    0, 0, 0, 0,
    1, 0, 0, 0, 83,
    1, 0, 0, 0, 84,
    1, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0, 0x40, 0, 0, 0x40, 0x40, 0, 0, 0x80, 0x40,
    1, 0, 0, 0, 4, 0, 0, 0,
    2, 0, 0, 0,
    0, 0, 0, 0, 5, 0, 0, 0, 97, 46, 116, 120, 116,
    10, 0, 0, 0, 116, 101, 120, 116, 47, 112, 108, 97, 105, 110,
    0, 0, 0, 0,
    1, 0, 0, 0, 3, 0, 0, 0, 1, 2, 3,
    0, 0, 0, 0,
    5, 0, 0, 0, 98, 46, 98, 105, 110
]

/// One file of a hand-built request.
struct WireFile {
    var source: UInt32 = 0
    var payload: [UInt8]
    var mime = ""
    var name = ""

    static func app(_ path: String, mime: String = "", name: String = "") -> WireFile {
        WireFile(payload: Array(path.utf8), mime: mime, name: name)
    }
}

/// Little-endian request builder for hostile cases.
struct Request {
    var bytes: [UInt8] = []

    mutating func u32(_ value: UInt32) { withUnsafeBytes(of: value.littleEndian) { bytes += $0 } }

    mutating func string(_ value: String) { field(Array(value.utf8)) }

    mutating func field(_ value: [UInt8]) {
        u32(UInt32(value.count))
        bytes += value
    }

    /// A request with only the given text, URL, and files.
    static func with(text: String = "", url: String = "", excluded: [UInt32] = [],
                     files: [WireFile] = []) -> Request {
        var request = Request()
        request.u32(1)
        request.string(text)
        request.string(url)
        request.string("")
        request.string("")
        request.u32(0)
        request.u32(UInt32(excluded.count))
        excluded.forEach { request.u32($0) }
        request.u32(UInt32(files.count))

        for file in files {
            request.u32(file.source)
            request.field(file.payload)
            request.string(file.mime)
            request.string(file.name)
        }

        return request
    }
}

func decode(_ bytes: [UInt8]) throws -> ShareContent {
    try bytes.withUnsafeBytes(ShareContent.decode)
}

func assertRejects(_ bytes: [UInt8], _ error: PluginError = .invalidArgument, line: UInt = #line) {
    XCTAssertThrowsError(try decode(bytes), line: line) { XCTAssertEqual($0 as? PluginError, error, line: line) }
}

final class SharingWireTests: XCTestCase {
    func testGoldenRequestDecodesLikeRust() throws {
        let content = try decode(goldenRequest)
        XCTAssertEqual(content.text, "hi")
        XCTAssertNil(content.url)
        XCTAssertEqual(content.subject, "S")
        XCTAssertEqual(content.title, "T")
        XCTAssertEqual(content.anchor, CGRect(x: 1, y: 2, width: 3, height: 4))
        XCTAssertEqual(content.excluded, [4])
        XCTAssertEqual(content.files, [
            ShareFile(source: .app("a.txt"), mime: "text/plain", name: nil),
            ShareFile(source: .bytes(Data([1, 2, 3])), mime: nil, name: "b.bin")
        ])

        for count in 0..<goldenRequest.count { assertRejects(Array(goldenRequest.prefix(count))) }

        assertRejects(goldenRequest + [0])
    }

    func testGoldenRepliesEncodeLikeRust() throws {
        XCTAssertEqual(ShareReply.success("x.y").encoded(), Data([1, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 120, 46, 121]))
        XCTAssertEqual(ShareReply.dismissed.encoded(), Data([1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]))
        XCTAssertEqual(ShareReply.unavailable.encoded(), Data([1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]))
        // An identifier the guest would refuse is reported as unavailable instead.
        for raw in ["", "a\nb", String(repeating: "a", count: 1025)] {
            XCTAssertEqual(ShareReply.success(raw).encoded(), ShareReply.unavailable.encoded())
        }
    }

    func testActivityCompletionMapsToResults() throws {
        let mail = "com.apple.UIKit.activity.Mail"
        XCTAssertEqual(try ShareReply.activity(type: mail, completed: true, failed: false), .success(mail))
        XCTAssertEqual(try ShareReply.activity(type: mail, completed: false, failed: false), .dismissed)
        XCTAssertEqual(try ShareReply.activity(type: nil, completed: false, failed: false), .dismissed)
        XCTAssertEqual(try ShareReply.activity(type: nil, completed: true, failed: false), .unavailable)
        XCTAssertThrowsError(try ShareReply.activity(type: mail, completed: true, failed: true)) {
            XCTAssertEqual($0 as? PluginError, .native)
        }
    }

    func testTextUrlAndLabelBounds() throws {
        XCTAssertEqual(try decode(Request.with(text: "hello\n世界").bytes).text, "hello\n世界")
        XCTAssertEqual(try decode(Request.with(url: "https://example.test/a%20b").bytes).url?.absoluteString,
                       "https://example.test/a%20b")

        for value in ["file:///private", "content://provider/item", "https://user:secret@example.test",
                      "https:///empty", "https://example.test/%xy", "https://example.test/a b"] {
            assertRejects(Request.with(url: value).bytes)
        }

        assertRejects(Request.with(text: String(repeating: "a", count: 65537)).bytes, .tooLarge)
        assertRejects(Request.with(text: "a\0b").bytes)
        // Nothing to share, a URL with text, and a URL with files.
        assertRejects(Request.with().bytes)
        assertRejects(Request.with(text: "t", url: "https://example.test").bytes)
        assertRejects(Request.with(url: "https://example.test", files: [.app("a")]).bytes)

        var subject = Request()
        subject.u32(1)
        subject.string("t")
        subject.string("")
        subject.string("line\nbreak")
        subject.string("")
        subject.u32(0)
        subject.u32(0)
        subject.u32(0)
        assertRejects(subject.bytes)
    }

    func testPreviousProtocolAndUnknownVersionsAreRefused() {
        assertRejects([0, 0, 0, 0, 1, 0, 0, 0, 120])
        var future = goldenRequest
        future[0] = 2
        assertRejects(future)
        assertRejects([UInt8](repeating: 0, count: ShareContent.requestBytes + 1), .tooLarge)
    }

    func testAnchorAndExcludedActivities() {
        var nan = goldenRequest
        withUnsafeBytes(of: Float.nan.bitPattern.littleEndian) { nan.replaceSubrange(28..<32, with: $0) }
        assertRejects(nan)
        var negative = goldenRequest
        withUnsafeBytes(of: Float(-1).bitPattern.littleEndian) { negative.replaceSubrange(36..<40, with: $0) }
        assertRejects(negative)
        var flag = goldenRequest
        flag[24] = 2
        assertRejects(flag)
        assertRejects(Request.with(text: "t", excluded: [20]).bytes)
        assertRejects(Request.with(text: "t", excluded: [4, 4]).bytes)
        assertRejects(Request.with(text: "t", excluded: Array(repeating: 0, count: 21)).bytes, .tooLarge)
        XCTAssertNoThrow(try decode(Request.with(text: "t", excluded: Array(0..<20)).bytes))
    }

    func testFilePathsNamesAndMimeTypes() {
        for path in ["", "/etc/passwd", "a/../b", "../secret", "a//b", "a/", "a\\b", "con", "a/LPT9", "name.",
                     ".leptoskit-tmp-x", "a?"] {
            assertRejects(Request.with(files: [.app(path)]).bytes)
        }

        assertRejects(Request.with(files: [WireFile(payload: [0xFF])]).bytes)
        assertRejects(Request.with(files: [WireFile(source: 1, payload: [1])]).bytes)
        assertRejects(Request.with(files: [WireFile(source: 2, payload: Array("a".utf8))]).bytes)
        assertRejects(Request.with(files: [.app("a", name: "x/y")]).bytes)

        for mime in ["image", "*/png", "image/png; q=1", "image/ png", "-a/b"] {
            assertRejects(Request.with(files: [.app("a", mime: mime)]).bytes)
        }

        for mime in ["image/png", "image/*", "*/*", "text/x-c++"] {
            XCTAssertNoThrow(try decode(Request.with(files: [.app("a", mime: mime)]).bytes))
        }

        let many = Array(repeating: WireFile.app("a"), count: 33)
        assertRejects(Request.with(files: many).bytes, .tooLarge)
        let half = [UInt8](repeating: 0, count: ShareContent.inlineBytes / 2)
        let inline = { (bytes: [UInt8], name: String) in WireFile(source: 1, payload: bytes, name: name) }
        XCTAssertNoThrow(try decode(Request.with(files: [inline(half, "a"), inline(half, "b")]).bytes))
        assertRejects(Request.with(files: [inline(half, "a"), inline(half, "b"), inline([0], "c")]).bytes,
                      .tooLarge)
    }

    func testAnchorsStayInsideThePresenter() throws {
        let bounds = CGRect(x: 0, y: 0, width: 100, height: 50)
        XCTAssertEqual(try ShareAnchor.rect(nil, in: bounds), CGRect(x: 50, y: 25, width: 1, height: 1))
        XCTAssertEqual(try ShareAnchor.rect(CGRect(x: 90, y: 40, width: 30, height: 30), in: bounds),
                       CGRect(x: 90, y: 40, width: 10, height: 10))
        XCTAssertThrowsError(try ShareAnchor.rect(CGRect(x: 101, y: 0, width: 1, height: 1), in: bounds))
    }
}

final class SharingStagingTests: XCTestCase {
    private var root: URL!
    private var staging: ShareStaging!

    override func setUpWithError() throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let files = root.appendingPathComponent("data/files", isDirectory: true)
        try FileManager.default.createDirectory(at: files.appendingPathComponent("docs"),
                                                withIntermediateDirectories: true)
        try Data("report".utf8).write(to: files.appendingPathComponent("docs/report.txt"))
        try Data("secret".utf8).write(to: root.appendingPathComponent("secret.txt"))
        staging = ShareStaging(filesRoot: files, cache: root.appendingPathComponent("cache/leptoskit-sharing"))
    }

    override func tearDownWithError() throws {
        try FileManager.default.removeItem(at: root)
    }

    private func app(_ path: String, name: String? = nil) -> ShareFile {
        ShareFile(source: .app(path), mime: nil, name: name)
    }

    func testCopiesAppFilesAndBytesUnderTheirNames() throws {
        let urls = try staging.stage([
            app("docs/report.txt"),
            app("docs/report.txt", name: "renamed.txt"),
            ShareFile(source: .bytes(Data([1, 2, 3])), mime: "application/octet-stream", name: "b.bin")
        ])
        XCTAssertEqual(urls.map(\.lastPathComponent), ["report.txt", "renamed.txt", "b.bin"])
        XCTAssertEqual(try Data(contentsOf: urls[0]), Data("report".utf8))
        XCTAssertEqual(try Data(contentsOf: urls[1]), Data("report".utf8))
        XCTAssertEqual(try Data(contentsOf: urls[2]), Data([1, 2, 3]))

        for url in urls { XCTAssertTrue(url.path.hasPrefix(staging.cache.path)) }

        // The next share replaces the previous copies.
        let next = try staging.stage([app("docs/report.txt")])
        XCTAssertFalse(FileManager.default.fileExists(atPath: urls[2].path))
        XCTAssertEqual(next.count, 1)
        try staging.clear()
        XCTAssertFalse(FileManager.default.fileExists(atPath: staging.cache.path))
    }

    func testSymlinksCannotEscapeTheFilesDirectory() throws {
        let files = staging.filesRoot
        let secret = root.appendingPathComponent("secret.txt")
        try FileManager.default.createSymbolicLink(at: files.appendingPathComponent("link.txt"),
                                                   withDestinationURL: secret)
        try FileManager.default.createSymbolicLink(at: files.appendingPathComponent("outside"),
                                                   withDestinationURL: root)
        try FileManager.default.createSymbolicLink(at: files.appendingPathComponent("inside"),
                                                   withDestinationURL: files.appendingPathComponent("docs"))

        for path in ["link.txt", "outside/secret.txt"] {
            XCTAssertThrowsError(try staging.stage([app(path)]), path) {
                XCTAssertEqual($0 as? PluginError, .invalidArgument)
            }
        }

        // A directory symlink that stays inside the files directory still resolves.
        XCTAssertEqual(try staging.stage([app("inside/report.txt")]).count, 1)
    }

    func testMissingDirectoriesAndOversizedFilesFail() throws {
        XCTAssertThrowsError(try staging.stage([app("missing.txt")])) { XCTAssertEqual($0 as? PluginError, .native) }
        XCTAssertThrowsError(try staging.stage([app("docs")])) {
            XCTAssertEqual($0 as? PluginError, .invalidArgument)
        }
        var small = staging!
        small.maxFileBytes = 3
        XCTAssertThrowsError(try small.stage([app("docs/report.txt")])) {
            XCTAssertEqual($0 as? PluginError, .tooLarge)
        }
        var budget = staging!
        budget.maxTotalBytes = 8
        XCTAssertThrowsError(try budget.stage([app("docs/report.txt"), app("docs/report.txt")])) {
            XCTAssertEqual($0 as? PluginError, .tooLarge)
        }
    }
}

@MainActor
final class SharingSessionTests: XCTestCase {
    func testCancellationRetiresCallerAndLateCompletionIsHarmless() async {
        var session: ShareSession?
        do {
            _ = try await withCheckedThrowingContinuation { continuation in
                let pending = ShareSession(continuation)
                session = pending
                pending.complete(.failure(PluginError.cancelled))
                pending.complete(.success(Data()))
            }
            XCTFail("Expected cancellation")
        } catch { XCTAssertEqual(error as? PluginError, .cancelled) }
        // A retained external service may finish after caller cancellation.
        session?.complete(.success(Data()))
        let first = session?.id
        let result = try? await withCheckedThrowingContinuation { continuation in
            let replacement = ShareSession(continuation)
            XCTAssertNotEqual(first, replacement.id)
            session?.complete(.failure(PluginError.native))
            replacement.complete(.success(ShareReply.dismissed.encoded()))
        }
        XCTAssertEqual(result, ShareReply.dismissed.encoded())
    }
}

#if canImport(AppKit)
import AppKit
import CLeptosKitBridge

@MainActor
final class SharingHostTests: XCTestCase {
    func testDetachedOwnerAndClosedPluginCannotPresent() async throws {
        let owner = NSViewController()
        let plugin = SharingPlugin(context: NativePluginContext(owner: owner))
        do {
            _ = try await plugin.invoke(method: 0, payload: try ownedPayload(Request.with(text: "x").bytes))
            XCTFail("Detached presenter must fail")
        } catch { XCTAssertEqual(error as? PluginError, .unavailable) }
        XCTAssertFalse(owner.isViewLoaded)
        plugin.close()
        plugin.close()
        do {
            _ = try await plugin.invoke(method: 0, payload: try ownedPayload(Request.with(text: "x").bytes))
            XCTFail("Closed plugin must fail")
        } catch { XCTAssertEqual(error as? PluginError, .cancelled) }
    }

    func testInvalidPayloadFailsBeforeAnyPresentation() async throws {
        let plugin = SharingPlugin(context: NativePluginContext(owner: NSViewController()))
        do {
            _ = try await plugin.invoke(method: 0, payload: try ownedPayload([0, 0, 0, 0, 1, 0, 0, 0, 120]))
            XCTFail("The previous protocol must be refused")
        } catch { XCTAssertEqual(error as? PluginError, .invalidArgument) }
        do {
            _ = try await plugin.invoke(method: 1, payload: try ownedPayload(goldenRequest))
            XCTFail("Unknown methods must be refused")
        } catch { XCTAssertEqual(error as? PluginError, .unavailable) }
    }

    private func ownedPayload(_ bytes: [UInt8]) throws -> NativeBuffer {
        let bridge = try XCTUnwrap(lk_bridge_create())
        defer { lk_bridge_destroy(bridge) }
        let name = Array("sharing".utf8)
        let methods: [UInt32] = [0, 1]
        let plugin = name.withUnsafeBufferPointer { name in
            methods.withUnsafeBufferPointer { methods in
                lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
            }
        }
        XCTAssertEqual(plugin, 0)
        XCTAssertEqual(lk_bridge_grant(bridge, 0, 0, true), 0)
        XCTAssertEqual(lk_bridge_grant(bridge, 0, 1, true), 0)

        let method: UInt32 = 0
        let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, method, $0.baseAddress, $0.count) }
        XCTAssertGreaterThan(call, 0)
        var request = lk_request()
        XCTAssertEqual(lk_bridge_next_request(bridge, &request), 1)

        return NativeBuffer(taking: request.payload)
    }
}
#endif
