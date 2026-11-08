import Darwin
import Foundation
import LeptosKitBridge

/// Path, name and MIME rules shared with the Rust and Kotlin decoders.
enum SharePaths {
    /// The filesystem plugin's in-progress writes; never a finished file.
    private static let tempPrefix = ".leptoskit-tmp-"
    private static let reserved: Set<Character> = ["\\", ":", "<", ">", "\"", "|", "?", "*"]
    private static let devices: Set<String> = ["CON", "PRN", "AUX", "NUL"]
    private static let mimePunctuation = Set("!#$&-^_.+".utf8)
    private static let mimeNameBytes = 127

    /// A relative path inside the app's files directory, with the filesystem plugin's rules.
    static func validatePath(_ path: String) throws {
        guard path.utf8.count <= ShareContent.pathBytes else { throw PluginError.tooLarge }

        guard !path.isEmpty,
              path.split(separator: "/", omittingEmptySubsequences: false).allSatisfy({ component(String($0)) })
        else { throw PluginError.invalidArgument }
    }

    /// A display name: one path segment.
    static func validateName(_ name: String) throws {
        guard name.utf8.count <= ShareContent.nameBytes else { throw PluginError.tooLarge }

        guard !name.contains("/"), component(name) else { throw PluginError.invalidArgument }
    }

    /// `type/subtype` with RFC 6838 name characters, `type/*`, or `*/*`.
    static func validateMime(_ mime: String) throws {
        guard mime.utf8.count <= ShareContent.mimeBytes else { throw PluginError.tooLarge }

        let parts = mime.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)

        guard parts.count == 2 else { throw PluginError.invalidArgument }

        let valid = switch (parts[0], parts[1]) {
        case ("*", "*"): true
        case ("*", _): false
        case (let kind, "*"): mimeName(kind)
        case (let kind, let subtype): mimeName(kind) && mimeName(subtype)
        }

        guard valid else { throw PluginError.invalidArgument }
    }

    private static func mimeName(_ name: String) -> Bool {
        let bytes = Array(name.utf8)

        guard let first = bytes.first, bytes.count <= mimeNameBytes, isAlphanumeric(first) else { return false }

        return bytes.dropFirst().allSatisfy { isAlphanumeric($0) || mimePunctuation.contains($0) }
    }

    private static func isAlphanumeric(_ byte: UInt8) -> Bool {
        (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte)
    }

    /// One segment: nonempty, no dot segment, no control or reserved character, no
    /// trailing dot or space, no Windows device name, no reserved temporary name.
    private static func component(_ part: String) -> Bool {
        let base = String(part.split(separator: ".", omittingEmptySubsequences: false).first ?? "").uppercased()
        let numbered = base.count == 4 && (base.hasPrefix("COM") || base.hasPrefix("LPT"))
            && base.last.map { ("1"..."9").contains($0) } == true
        let invalid = part.isEmpty || part == "." || part == ".." || part.hasSuffix(".") || part.hasSuffix(" ")
            || part.hasPrefix(tempPrefix) || devices.contains(base) || numbered
            || part.unicodeScalars.contains { $0.properties.generalCategory == .control }
            || part.contains { reserved.contains($0) }

        return !invalid
    }
}

/// Copies shared files into a private cache directory the share sheet can read.
///
/// App files are opened without following a final symlink, and the opened file
/// must still live inside the app's files directory, so neither a symlink nor
/// a swapped directory can expose another file. The cache is cleared at the
/// start of the next share, because a destination may read it after the sheet closes.
struct ShareStaging: Sendable {
    /// The app's files directory: the filesystem plugin's root.
    let filesRoot: URL
    /// The private cache this plugin owns.
    let cache: URL
    var maxFileBytes: Int64 = 512 * 1024 * 1024
    var maxTotalBytes: Int64 = 1024 * 1024 * 1024

    /// The directories the generated hosts give the runtime: Application Support and
    /// Caches, namespaced by bundle identifier on macOS.
    static func standard() throws -> ShareStaging {
        let manager = FileManager.default
        var support = try manager.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                      appropriateFor: nil, create: false)
        var caches = try manager.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: false)
        #if os(macOS)
        guard let identifier = Bundle.main.bundleIdentifier else { throw PluginError.unavailable }

        support.appendPathComponent(identifier, isDirectory: true)
        caches.appendPathComponent(identifier, isDirectory: true)
        #endif

        return ShareStaging(filesRoot: support.appendingPathComponent("files", isDirectory: true),
                            cache: caches.appendingPathComponent("leptoskit-sharing", isDirectory: true))
    }

    /// Replaces the previous share's copies with this share's files, in order.
    func stage(_ files: [ShareFile]) throws -> [URL] {
        try clear()
        try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        var total: Int64 = 0

        return try files.enumerated().map { index, file in
            try checkCancellation()
            let directory = cache.appendingPathComponent(String(index), isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)

            switch file.source {
            case .app(let path):
                let name = file.name ?? String(path.split(separator: "/").last ?? "")
                let destination = directory.appendingPathComponent(name, isDirectory: false)
                total += try copyAppFile(path, to: destination, budget: maxTotalBytes - total)

                return destination
            case .bytes(let data):
                let destination = directory.appendingPathComponent(file.name ?? "file", isDirectory: false)
                total += Int64(data.count)

                guard total <= maxTotalBytes else { throw PluginError.tooLarge }

                try write(data, to: destination)

                return destination
            }
        }
    }

    /// Removes every earlier copy; the cache holds nothing else.
    func clear() throws {
        guard FileManager.default.fileExists(atPath: cache.path) else { return }

        try FileManager.default.removeItem(at: cache)
    }

    private func checkCancellation() throws {
        if Task.isCancelled { throw PluginError.cancelled }
    }

    /// Copies one regular file from the files directory, never through a final symlink.
    private func copyAppFile(_ path: String, to destination: URL, budget: Int64) throws -> Int64 {
        let root = try realPath(filesRoot.path)
        let source = open(filesRoot.appendingPathComponent(path).path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)

        guard source >= 0 else { throw errno == ELOOP ? PluginError.invalidArgument : PluginError.native }

        defer { close(source) }

        var info = stat()

        guard fstat(source, &info) == 0 else { throw PluginError.native }

        guard info.st_mode & S_IFMT == S_IFREG else { throw PluginError.invalidArgument }

        guard try openedPath(source).hasPrefix(root + "/") else { throw PluginError.invalidArgument }

        guard info.st_size <= min(maxFileBytes, budget) else { throw PluginError.tooLarge }

        return try copy(source, to: destination, limit: min(maxFileBytes, budget))
    }

    private func copy(_ source: Int32, to destination: URL, limit: Int64) throws -> Int64 {
        let output = open(destination.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)

        guard output >= 0 else { throw PluginError.native }

        defer { close(output) }

        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        var copied: Int64 = 0

        while true {
            try checkCancellation()
            let count = buffer.withUnsafeMutableBytes { read(source, $0.baseAddress, $0.count) }

            guard count >= 0 else { throw PluginError.native }

            if count == 0 { return copied }

            copied += Int64(count)

            // The file may grow after the size check.
            guard copied <= limit else { throw PluginError.tooLarge }

            try buffer.withUnsafeBytes { try writeAll(output, UnsafeRawBufferPointer(rebasing: $0[0..<count])) }
        }
    }

    private func write(_ data: Data, to destination: URL) throws {
        let output = open(destination.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)

        guard output >= 0 else { throw PluginError.native }

        defer { close(output) }

        try data.withUnsafeBytes { try writeAll(output, $0) }
    }

    private func writeAll(_ output: Int32, _ bytes: UnsafeRawBufferPointer) throws {
        var offset = 0

        while offset < bytes.count {
            let written = Darwin.write(output, bytes.baseAddress?.advanced(by: offset), bytes.count - offset)

            guard written > 0 else { throw PluginError.native }

            offset += written
        }
    }

    private func realPath(_ path: String) throws -> String {
        guard let resolved = Darwin.realpath(path, nil) else { throw PluginError.native }

        defer { free(resolved) }

        return String(cString: resolved)
    }

    /// The path the kernel reports for an open file, after every symlink.
    private func openedPath(_ descriptor: Int32) throws -> String {
        var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN))

        guard fcntl(descriptor, F_GETPATH, &buffer) == 0 else { throw PluginError.native }

        guard let path = String(bytes: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, encoding: .utf8)
        else { throw PluginError.native }

        return path
    }
}
