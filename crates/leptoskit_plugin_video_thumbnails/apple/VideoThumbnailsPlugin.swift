import AVFoundation
import Foundation
import LeptosKitBridge
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Extracts one JPEG frame from an app-owned video URI.
@MainActor
public final class VideoThumbnailsPlugin: NativePlugin {
    public let name = "video-thumbnails"
    public let methods: [UInt32] = [0]

    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard method == 0 else { throw PluginError.unavailable }
        let request = try VideoThumbnailsWire.getThumbnail(payload)
        return try await extract(uri: request.uri, timeMs: request.timeMs, quality: request.quality)
    }

    public func close() {
        closed = true
    }

    private func extract(uri: String, timeMs: UInt64, quality: UInt32) async throws -> Data {
        guard let url = URL(string: uri), url.isFileURL else { throw PluginError.invalidArgument }
        let asset = AVURLAsset(url: url)
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        let time = CMTime(value: CMTimeValue(timeMs), timescale: 1000)
        let cgImage = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<CGImage, Error>) in
            generator.generateCGImagesAsynchronously(forTimes: [NSValue(time: time)]) { _, image, _, result, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if result == .succeeded, let image {
                    continuation.resume(returning: image)
                } else {
                    continuation.resume(throwing: PluginError.unavailable)
                }
            }
        }
        #if os(macOS)
        let bitmap = NSBitmapImageRep(cgImage: cgImage)
        guard let jpeg = bitmap.representation(
            using: .jpeg,
            properties: [.compressionFactor: Double(quality) / 100.0]
        ) else {
            throw PluginError.unavailable
        }
        #else
        let image = UIImage(cgImage: cgImage)
        guard let jpeg = image.jpegData(compressionQuality: CGFloat(quality) / 100.0) else {
            throw PluginError.unavailable
        }
        #endif
        let directory = FileManager.default.temporaryDirectory
        let file = directory.appendingPathComponent("lk-video-thumb-\(UUID().uuidString).jpg")
        try jpeg.write(to: file, options: .atomic)
        return try VideoThumbnailsWire.thumbnail(
            uri: file.absoluteString,
            width: UInt32(cgImage.width),
            height: UInt32(cgImage.height)
        )
    }
}
