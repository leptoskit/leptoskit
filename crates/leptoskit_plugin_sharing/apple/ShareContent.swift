import CoreGraphics
import Foundation
import LeptosKitBridge

/// One file to share: a path inside the app's files directory, or guest bytes.
struct ShareFile: Sendable, Equatable {
    enum Source: Sendable, Equatable {
        case app(String)
        case bytes(Data)
    }

    let source: Source
    let mime: String?
    let name: String?
}

/// A decoded, validated share request. Every rule matches the Rust and Kotlin decoders.
struct ShareContent: Sendable, Equatable {
    var text: String?
    var url: URL?
    var subject: String?
    var title: String?
    var anchor: CGRect?
    var excluded: [UInt32] = []
    var files: [ShareFile] = []

    /// The version the request and reply start with.
    static let version: UInt32 = 1

    /// Bounds shared with the Rust crate.
    static let textBytes = 65536
    static let urlBytes = 8192
    static let labelBytes = 1024
    static let maxFiles = 32
    static let pathBytes = 4096
    static let nameBytes = 255
    static let mimeBytes = 255
    static let inlineBytes = 4 * 1024 * 1024
    static let maxAnchor: Float = 1_000_000
    static let activityCount: UInt32 = 20
    static let requestBytes = 4_418_112

    /// Decodes an untrusted guest payload, refusing unknown versions, codes and trailing data.
    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> ShareContent {
        guard bytes.count <= requestBytes else { throw PluginError.tooLarge }

        var reader = WireReader(bytes)

        guard try reader.u32() == version else { throw PluginError.invalidArgument }

        var content = ShareContent()
        let text = try reader.optionalString()
        let url = try reader.optionalString()
        content.subject = try label(reader.optionalString())
        content.title = try label(reader.optionalString())
        content.anchor = try anchor(&reader)
        content.excluded = try excluded(&reader)
        content.files = try files(&reader)
        try reader.finish()

        if let text { content.text = try validText(text) }

        if let url { content.url = try ShareURL.web(url) }

        guard content.url == nil || (content.text == nil && content.files.isEmpty),
              content.text != nil || content.url != nil || !content.files.isEmpty else {
            throw PluginError.invalidArgument
        }

        return content
    }

    private static func validText(_ value: String) throws -> String {
        guard value.utf8.count <= textBytes else { throw PluginError.tooLarge }

        guard !value.contains("\0") else { throw PluginError.invalidArgument }

        return value
    }

    /// A subject or title: bounded UTF-8 without control characters.
    private static func label(_ value: String?) throws -> String? {
        guard let value else { return nil }

        guard value.utf8.count <= labelBytes else { throw PluginError.tooLarge }

        guard !value.unicodeScalars.contains(where: { $0.properties.generalCategory == .control }) else {
            throw PluginError.invalidArgument
        }

        return value
    }

    private static func anchor(_ reader: inout WireReader) throws -> CGRect? {
        switch try reader.u32() {
        case 0:
            return nil
        case 1:
            let values = try (0..<4).map { _ in Float(bitPattern: try reader.u32()) }

            guard values.allSatisfy({ $0.isFinite && $0 >= 0 && $0 <= maxAnchor }) else {
                throw PluginError.invalidArgument
            }

            return CGRect(x: CGFloat(values[0]), y: CGFloat(values[1]),
                          width: CGFloat(values[2]), height: CGFloat(values[3]))
        default:
            throw PluginError.invalidArgument
        }
    }

    private static func excluded(_ reader: inout WireReader) throws -> [UInt32] {
        let count = try reader.u32()

        guard count <= activityCount else { throw PluginError.tooLarge }

        var codes: [UInt32] = []

        for _ in 0..<count {
            let code = try reader.u32()

            guard code < activityCount, !codes.contains(code) else { throw PluginError.invalidArgument }

            codes.append(code)
        }

        return codes
    }

    private static func files(_ reader: inout WireReader) throws -> [ShareFile] {
        let count = try reader.u32()

        guard count <= maxFiles else { throw PluginError.tooLarge }

        var files: [ShareFile] = []
        var inline = 0

        for _ in 0..<count {
            let file = try file(&reader)

            if case .bytes(let data) = file.source { inline += data.count }

            guard inline <= inlineBytes else { throw PluginError.tooLarge }

            files.append(file)
        }

        return files
    }

    private static func file(_ reader: inout WireReader) throws -> ShareFile {
        let source = try reader.u32()
        let payload = try reader.field()
        let mime = try reader.optionalString()
        let name = try reader.optionalString()

        if let mime { try SharePaths.validateMime(mime) }

        if let name { try SharePaths.validateName(name) }

        switch source {
        case 0:
            guard let path = String(bytes: payload, encoding: .utf8) else { throw PluginError.invalidArgument }

            try SharePaths.validatePath(path)

            return ShareFile(source: .app(path), mime: mime, name: name)
        case 1:
            guard payload.count <= inlineBytes else { throw PluginError.tooLarge }

            guard name != nil else { throw PluginError.invalidArgument }

            return ShareFile(source: .bytes(Data(payload)), mime: mime, name: name)
        default:
            throw PluginError.invalidArgument
        }
    }
}

/// Little-endian u32 fields and u32-length-prefixed byte strings.
struct WireReader {
    private let buffer: UnsafeRawBufferPointer
    private var offset = 0

    init(_ buffer: UnsafeRawBufferPointer) { self.buffer = buffer }

    mutating func u32() throws -> UInt32 {
        guard buffer.count - offset >= 4 else { throw PluginError.invalidArgument }

        let value = UInt32(littleEndian: buffer.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        offset += 4

        return value
    }

    mutating func field() throws -> UnsafeRawBufferPointer {
        let count = Int(try u32())

        guard buffer.count - offset >= count else { throw PluginError.invalidArgument }

        let slice = UnsafeRawBufferPointer(rebasing: buffer[offset..<(offset + count)])
        offset += count

        return slice
    }

    /// Strict UTF-8; an empty value means absent.
    mutating func optionalString() throws -> String? {
        let slice = try field()

        guard !slice.isEmpty else { return nil }

        guard let value = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }

        return value
    }

    func finish() throws {
        guard offset == buffer.count else { throw PluginError.invalidArgument }
    }
}

/// The reply: version, status (0 success, 1 dismissed, 2 unavailable), raw identifier.
enum ShareReply: Equatable {
    case success(String)
    case dismissed
    case unavailable

    /// The largest raw identifier the guest accepts.
    static let rawBytes = 1024

    /// Encodes the reply. A success without a usable identifier becomes unavailable.
    func encoded() -> Data {
        var status: UInt32
        var raw = ""

        switch self {
        case .success(let value):
            let usable = !value.isEmpty && value.utf8.count <= Self.rawBytes
                && !value.unicodeScalars.contains { $0.properties.generalCategory == .control }
            status = usable ? 0 : 2
            raw = usable ? value : ""
        case .dismissed:
            status = 1
        case .unavailable:
            status = 2
        }

        var data = Data()

        for var number in [ShareContent.version.littleEndian, status.littleEndian,
                           UInt32(raw.utf8.count).littleEndian] {
            withUnsafeBytes(of: &number) { data.append(contentsOf: $0) }
        }

        data.append(contentsOf: raw.utf8)

        return data
    }

    /// Maps an iOS activity completion to the reply.
    static func activity(type: String?, completed: Bool, failed: Bool) throws -> ShareReply {
        guard !failed else { throw PluginError.native }

        guard completed else { return .dismissed }

        guard let type, !type.isEmpty else { return .unavailable }

        return .success(type)
    }
}
