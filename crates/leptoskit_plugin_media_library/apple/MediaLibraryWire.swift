import Foundation
import LeptosKitBridge
import Photos

enum MediaLibraryStatus: UInt32 {
    case unknown = 0
    case prompt = 1
    case denied = 2
    case granted = 3
    case restricted = 4

    static func from(_ status: PHAuthorizationStatus) -> MediaLibraryStatus {
        switch status {
        case .notDetermined: .prompt
        case .denied: .denied
        case .authorized, .limited: .granted
        case .restricted: .restricted
        @unknown default: .unknown
        }
    }
}

enum MediaLibraryKind: UInt32 {
    case images = 0
    case videos = 1
    case all = 2
}

struct MediaLibraryListOptions: Equatable {
    let kind: MediaLibraryKind
    let limit: UInt32

    static let maxList: UInt32 = 64

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> MediaLibraryListOptions {
        var cursor = 0
        let kindRaw = try readU32(bytes, cursor: &cursor)
        let limit = try readU32(bytes, cursor: &cursor)
        guard cursor == bytes.count else { throw PluginError.invalidArgument }
        guard let kind = MediaLibraryKind(rawValue: kindRaw) else { throw PluginError.invalidArgument }
        guard limit > 0, limit <= maxList else { throw PluginError.invalidArgument }
        return MediaLibraryListOptions(kind: kind, limit: limit)
    }
}

struct MediaLibrarySaveRequest: Equatable {
    let uri: String
    let kind: MediaLibraryKind

    static let maxUriBytes = 4096

    static func decode(_ bytes: UnsafeRawBufferPointer) throws -> MediaLibrarySaveRequest {
        var cursor = 0
        let kindRaw = try readU32(bytes, cursor: &cursor)
        let uri = try readString(bytes, cursor: &cursor)
        guard cursor == bytes.count else { throw PluginError.invalidArgument }
        guard let kind = MediaLibraryKind(rawValue: kindRaw), kind != .all else {
            throw PluginError.invalidArgument
        }
        guard !uri.isEmpty, uri.utf8.count <= maxUriBytes, !uri.contains("\0") else {
            throw PluginError.invalidArgument
        }
        return MediaLibrarySaveRequest(uri: uri, kind: kind)
    }
}

struct MediaLibraryAsset: Equatable {
    let id: String
    let kind: MediaLibraryKind
    let uri: String
    let width: UInt32?
    let height: UInt32?
    let creationTimeMs: UInt64?

    func encode() throws -> Data {
        var data = Data()
        try appendString(&data, id)
        appendU32(&data, kind.rawValue)
        try appendString(&data, uri)
        appendU32(&data, width ?? UInt32.max)
        appendU32(&data, height ?? UInt32.max)
        appendU64(&data, creationTimeMs ?? UInt64.max)
        return data
    }
}

func encodeAssets(_ assets: [MediaLibraryAsset]) throws -> Data {
    var data = Data()
    appendU32(&data, UInt32(assets.count))
    for asset in assets {
        let frame = try asset.encode()
        try appendBytes(&data, frame)
    }
    return data
}

private func appendU32(_ data: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

private func appendU64(_ data: inout Data, _ value: UInt64) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
}

private func appendString(_ data: inout Data, _ value: String) throws {
    try appendBytes(&data, Data(value.utf8))
}

private func appendBytes(_ data: inout Data, _ value: Data) throws {
    guard value.count <= UInt32.max else { throw PluginError.tooLarge }
    appendU32(&data, UInt32(value.count))
    data.append(value)
}

private func readU32(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> UInt32 {
    guard cursor + 4 <= bytes.count else { throw PluginError.invalidArgument }
    let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: cursor, as: UInt32.self))
    cursor += 4
    return value
}

private func readString(_ bytes: UnsafeRawBufferPointer, cursor: inout Int) throws -> String {
    let count = Int(try readU32(bytes, cursor: &cursor))
    guard cursor + count <= bytes.count else { throw PluginError.invalidArgument }
    let slice = UnsafeRawBufferPointer(rebasing: bytes[cursor..<(cursor + count)])
    cursor += count
    guard let value = String(bytes: slice, encoding: .utf8) else { throw PluginError.invalidArgument }
    return value
}
