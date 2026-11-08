import Foundation
import LeptosKitBridge
import Photos
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

@MainActor
protocol MediaLibraryService {
    func status() -> MediaLibraryStatus
    func requestPermission() async -> MediaLibraryStatus
    func list(_ options: MediaLibraryListOptions) async throws -> [MediaLibraryAsset]
    func save(_ request: MediaLibrarySaveRequest) async throws -> MediaLibraryAsset
}

@MainActor
private final class PhotoKitMediaLibraryService: MediaLibraryService {
    func status() -> MediaLibraryStatus {
        MediaLibraryStatus.from(PHPhotoLibrary.authorizationStatus(for: .readWrite))
    }

    func requestPermission() async -> MediaLibraryStatus {
        let status = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
        return MediaLibraryStatus.from(status)
    }

    func list(_ options: MediaLibraryListOptions) async throws -> [MediaLibraryAsset] {
        guard status() == .granted else { throw PluginError.permissionDenied }
        let fetchOptions = PHFetchOptions()
        fetchOptions.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        fetchOptions.fetchLimit = Int(options.limit)
        let mediaType: PHAssetMediaType? = switch options.kind {
        case .images: .image
        case .videos: .video
        case .all: nil
        }
        let result: PHFetchResult<PHAsset>
        if let mediaType {
            result = PHAsset.fetchAssets(with: mediaType, options: fetchOptions)
        } else {
            result = PHAsset.fetchAssets(with: fetchOptions)
        }
        var assets: [MediaLibraryAsset] = []
        result.enumerateObjects { asset, _, stop in
            if assets.count >= Int(options.limit) {
                stop.pointee = true
                return
            }
            let kind: MediaLibraryKind = asset.mediaType == .video ? .videos : .images
            let creation = asset.creationDate.map { UInt64($0.timeIntervalSince1970 * 1000) }
            assets.append(
                MediaLibraryAsset(
                    id: asset.localIdentifier,
                    kind: kind,
                    uri: "ph://\(asset.localIdentifier)",
                    width: UInt32(asset.pixelWidth),
                    height: UInt32(asset.pixelHeight),
                    creationTimeMs: creation
                )
            )
        }
        return assets
    }

    func save(_ request: MediaLibrarySaveRequest) async throws -> MediaLibraryAsset {
        guard status() == .granted else { throw PluginError.permissionDenied }
        guard let url = URL(string: request.uri), url.isFileURL else {
            throw PluginError.invalidArgument
        }
        var localId: String?
        try await PHPhotoLibrary.shared().performChanges {
            let change: PHAssetChangeRequest?
            switch request.kind {
            case .images:
                change = PHAssetChangeRequest.creationRequestForAssetFromImage(atFileURL: url)
            case .videos:
                change = PHAssetChangeRequest.creationRequestForAssetFromVideo(atFileURL: url)
            case .all:
                change = nil
            }
            localId = change?.placeholderForCreatedAsset?.localIdentifier
        }
        guard let localId else { throw PluginError.native }
        return MediaLibraryAsset(
            id: localId,
            kind: request.kind,
            uri: "ph://\(localId)",
            width: nil,
            height: nil,
            creationTimeMs: nil
        )
    }
}

/// Lists and saves system photo-library assets with bounded replies.
@MainActor
public final class MediaLibraryPlugin: NativePlugin {
    public let name = "media-library"
    public let methods: [UInt32] = [0, 1, 2, 3]
    private let context: NativePluginContext
    private let service: any MediaLibraryService
    private var closed = false

    public init(context: NativePluginContext) {
        self.context = context
        service = PhotoKitMediaLibraryService()
    }

    init(context: NativePluginContext, service: any MediaLibraryService) {
        self.context = context
        self.service = service
    }

    public func close() {
        closed = true
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }
        _ = context

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return encode(service.status())
        case 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return encode(await service.requestPermission())
        case 2:
            let options = try payload.withUnsafeBytes(MediaLibraryListOptions.decode)
            try Task.checkCancellation()
            return try encodeAssets(await service.list(options))
        default:
            let request = try payload.withUnsafeBytes(MediaLibrarySaveRequest.decode)
            try Task.checkCancellation()
            return try await service.save(request).encode()
        }
    }

    private func encode(_ status: MediaLibraryStatus) -> Data {
        var raw = status.rawValue.littleEndian
        return withUnsafeBytes(of: &raw) { Data($0) }
    }
}
