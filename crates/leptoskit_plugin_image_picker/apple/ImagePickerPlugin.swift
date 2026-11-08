import Foundation
import ImageIO
import LeptosKitBridge
import PhotosUI
import UniformTypeIdentifiers
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Request and reply framing shared with the Rust codec.
private enum Wire {
    static let pick: UInt32 = 0
    static let requestBytes = 8
    static let maxBytes = 16 * 1024 * 1024
    static let maxURIBytes = 4096
    static let maxMIMEBytes = 256
    static let images: UInt32 = 0
    static let videos: UInt32 = 1
    static let all: UInt32 = 2
}

private struct Options {
    let media: UInt32
    let maxBytes: Int
}

/// Picks one library image or video into an app-owned temporary file.
///
/// Uses `PHPickerViewController`, which does not require a photo-library
/// purpose string on modern Apple platforms. Apps that later adopt limited
/// library / older picker APIs still own those Info.plist strings themselves.
/// Camera capture is intentionally out of scope for this MVP.
@MainActor
public final class ImagePickerPlugin: NSObject, NativePlugin {
    public let name = "image-picker"
    public let methods: [UInt32] = [Wire.pick]
    private let context: NativePluginContext
    private var closed = false
    private var busy = false
    private let selection = ImageSelection()
    private var selectionID: UUID?
    private var picker: PHPickerViewController?
    private var pendingLimit = Wire.maxBytes

    public init(context: NativePluginContext) { self.context = context }

    public func close() {
        closed = true
        cancelSelection()
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard method == Wire.pick else { throw PluginError.unavailable }

        let options = try decodeOptions(payload)
        guard !busy else { throw PluginError.busy }

        try Task.checkCancellation()
        busy = true
        let identifier = UUID()
        selectionID = identifier
        pendingLimit = options.maxBytes
        defer {
            busy = false
            selectionID = nil
            pendingLimit = Wire.maxBytes
            releaseChooser()
        }

        let url = try await choose(identifier: identifier, media: options.media)
        try Task.checkCancellation()
        guard !closed else { throw PluginError.cancelled }

        return try encode(inspect(url, limit: options.maxBytes))
    }

    private func decodeOptions(_ payload: NativeBuffer) throws -> Options {
        try payload.withUnsafeBytes { bytes -> Options in
            guard bytes.count == Wire.requestBytes else { throw PluginError.invalidArgument }

            let media = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
            let maxBytes = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self))

            guard media <= Wire.all else { throw PluginError.invalidArgument }
            guard maxBytes <= UInt32(Wire.maxBytes) else { throw PluginError.tooLarge }

            return Options(media: media, maxBytes: Int(maxBytes))
        }
    }

    private func choose(identifier: UUID, media: UInt32) async throws -> URL {
        try await withTaskCancellationHandler {
            try await present(media: media)
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.cancel(identifier: identifier)
            }
        }
    }

    private func cancel(identifier: UUID) {
        guard selectionID == identifier else { return }

        cancelSelection()
    }

    private func cancelSelection() {
        dismissChooser()
        finish(.failure(PluginError.cancelled))
    }

    private func finish(_ result: Result<URL, Error>) {
        releaseChooser()
        selection.finish(result)
    }

    private func present(media: UInt32) async throws -> URL {
        let owner = try context.requirePresentationOwner()

        #if canImport(UIKit)
        guard owner.presentedViewController == nil else { throw PluginError.busy }
        #elseif canImport(AppKit)
        guard owner.view.window?.attachedSheet == nil else { throw PluginError.busy }
        #endif

        var configuration = PHPickerConfiguration()
        configuration.selectionLimit = 1
        configuration.filter = filter(for: media)

        let picker = PHPickerViewController(configuration: configuration)
        picker.delegate = self
        self.picker = picker

        return try await withCheckedThrowingContinuation { continuation in
            selection.begin(continuation)
            #if canImport(UIKit)
            owner.present(picker, animated: true)
            picker.presentationController?.delegate = self
            #elseif canImport(AppKit)
            owner.presentAsSheet(picker)
            #endif
        }
    }

    private func filter(for media: UInt32) -> PHPickerFilter {
        switch media {
        case Wire.videos: .videos
        case Wire.all: .any(of: [.images, .videos])
        default: .images
        }
    }

    private func dismissChooser() {
        #if canImport(UIKit)
        picker?.dismiss(animated: false)
        #elseif canImport(AppKit)
        if let picker {
            ownerDismiss(picker)
        }
        #endif
    }

    #if canImport(AppKit)
    private func ownerDismiss(_ picker: PHPickerViewController) {
        picker.presentingViewController?.dismiss(picker)
    }
    #endif

    private func releaseChooser() {
        picker?.delegate = nil
        picker = nil
    }

    /// Reads metadata from the owned temporary file without retaining gallery rights.
    private func inspect(_ url: URL, limit: Int) throws -> Asset {
        let values = try url.resourceValues(forKeys: [.fileSizeKey, .contentTypeKey])
        if let size = values.fileSize, size > limit { throw PluginError.tooLarge }

        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        let size = attributes[.size] as? UInt64 ?? 0
        if size > UInt64(limit) { throw PluginError.tooLarge }

        let mime = values.contentType?.preferredMIMEType ?? ""
        let dimensions = imageSize(at: url, mime: mime)
        return Asset(
            uri: url.absoluteString,
            mime: mime,
            width: dimensions?.width,
            height: dimensions?.height,
            size: size
        )
    }
}

private struct Asset {
    let uri: String
    let mime: String
    let width: UInt32?
    let height: UInt32?
    let size: UInt64
}

private func encode(_ asset: Asset) throws -> Data {
    guard !asset.uri.isEmpty, asset.uri.utf8.count <= Wire.maxURIBytes else {
        throw PluginError.invalidArgument
    }
    guard asset.mime.utf8.count <= Wire.maxMIMEBytes else { throw PluginError.tooLarge }

    var result = Data(capacity: 64)
    try write(&result, asset.uri)
    try write(&result, asset.mime)
    write(&result, asset.width ?? .max)
    write(&result, asset.height ?? .max)
    write(&result, UInt32(truncatingIfNeeded: asset.size))
    write(&result, UInt32(asset.size >> 32))

    return result
}

private func write(_ result: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
}

private func write(_ result: inout Data, _ text: String) throws {
    let bytes = Data(text.utf8)
    write(&result, UInt32(bytes.count))
    result.append(bytes)
}

private func imageSize(at url: URL, mime: String) -> (width: UInt32, height: UInt32)? {
    if mime.hasPrefix("video/") { return nil }

    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0
    else {
        return nil
    }

    return (UInt32(width), UInt32(height))
}

extension ImagePickerPlugin: PHPickerViewControllerDelegate {
    public func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        guard self.picker === picker else { return }

        guard let result = results.first else {
            finish(.failure(PluginError.cancelled))
            return
        }

        let provider = result.itemProvider
        let type = provider.registeredTypeIdentifiers.first ?? UTType.image.identifier
        let limit = pendingLimit
        provider.loadFileRepresentation(forTypeIdentifier: type) { [weak self] url, error in
            Task { @MainActor in
                if let error {
                    self?.finish(.failure(error))
                    return
                }

                guard let url else {
                    self?.finish(.failure(PluginError.cancelled))
                    return
                }

                do {
                    let owned = try Self.persist(url, limit: limit)
                    self?.finish(.success(owned))
                } catch {
                    self?.finish(.failure(error))
                }
            }
        }
    }

    /// Copies the provider file into the app temp directory before the callback returns.
    fileprivate static func persist(_ url: URL, limit: Int) throws -> URL {
        let values = try url.resourceValues(forKeys: [.fileSizeKey, .contentTypeKey])
        if let size = values.fileSize, size > limit { throw PluginError.tooLarge }

        let data = try Data(contentsOf: url, options: [.mappedIfSafe])
        if data.count > limit { throw PluginError.tooLarge }

        let ext = values.contentType?.preferredFilenameExtension ?? url.pathExtension
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("leptoskit-image-picker", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let destination = directory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension(ext.isEmpty ? "bin" : ext)
        try data.write(to: destination, options: .atomic)

        return destination
    }
}

#if canImport(UIKit)
extension ImagePickerPlugin: UIAdaptivePresentationControllerDelegate {
    public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        finish(.failure(PluginError.cancelled))
    }
}
#endif
