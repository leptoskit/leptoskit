import Foundation
import LeptosKitBridge
import Photos
import PhotosUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Host-owned `PHLivePhotoView` registry with hint/full playback.
@MainActor
public final class LivePhotoPlugin: NativePlugin {
    public let name = "live-photo"
    public let methods: [UInt32] = [0, 1, 2, 3, 4]

    private var sessions: [UInt32: Session] = [:]
    private var nextId: UInt32 = 1
    private var closed = false

    private enum Method: UInt32 {
        case create = 0
        case startPlayback = 1
        case stopPlayback = 2
        case status = 3
        case release = 4
    }

    private final class Session {
        let view: PHLivePhotoView
        var playing = false

        init(view: PHLivePhotoView) {
            self.view = view
        }
    }

    public init() {
        NativeLivePhotoBinding.view = { [weak self] id in self?.sessions[id]?.view }
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard let method = Method(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .create:
            return try await create(LivePhotoWire.create(payload))
        case .startPlayback:
            let request = try LivePhotoWire.startPlayback(payload)
            try startPlayback(request.id, style: request.style)
            return Data()
        case .stopPlayback:
            try stopPlayback(LivePhotoWire.session(payload))
            return Data()
        case .status:
            return try status(LivePhotoWire.session(payload))
        case .release:
            try release(LivePhotoWire.session(payload))
            return Data()
        }
    }

    public func close() {
        closed = true
        for id in Array(sessions.keys) {
            try? release(id)
        }
        NativeLivePhotoBinding.view = nil
    }

    private func create(_ uri: String) async throws -> Data {
        let livePhoto = try await loadLivePhoto(uri: uri)
        let view = PHLivePhotoView(frame: .zero)
        view.livePhoto = livePhoto
        let id = nextId
        nextId = nextId &+ 1
        if nextId == 0 { nextId = 1 }
        sessions[id] = Session(view: view)
        return LivePhotoWire.sessionId(id)
    }

    private func startPlayback(_ id: UInt32, style: UInt32) throws {
        guard let session = sessions[id], session.view.livePhoto != nil else {
            throw PluginError.invalidArgument
        }
        let playbackStyle: PHLivePhotoViewPlaybackStyle = style == 0 ? .hint : .full
        session.view.startPlayback(with: playbackStyle)
        session.playing = true
    }

    private func stopPlayback(_ id: UInt32) throws {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        session.view.stopPlayback()
        session.playing = false
    }

    private func status(_ id: UInt32) throws -> Data {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        return LivePhotoWire.status(active: true, playing: session.playing)
    }

    private func release(_ id: UInt32) throws {
        guard let session = sessions.removeValue(forKey: id) else {
            throw PluginError.invalidArgument
        }
        session.view.stopPlayback()
        session.view.livePhoto = nil
        session.view.removeFromSuperview()
    }

    private func loadLivePhoto(uri: String) async throws -> PHLivePhoto {
        if uri.hasPrefix("ph://") {
            let identifier = String(uri.dropFirst("ph://".count))
            guard !identifier.isEmpty else { throw PluginError.invalidArgument }
            let assets = PHAsset.fetchAssets(withLocalIdentifiers: [identifier], options: nil)
            guard let asset = assets.firstObject else { throw PluginError.unavailable }
            return try await withCheckedThrowingContinuation { continuation in
                PHImageManager.default().requestLivePhoto(
                    for: asset,
                    targetSize: PHImageManagerMaximumSize,
                    contentMode: .aspectFit,
                    options: nil
                ) { livePhoto, info in
                    if let cancelled = info?[PHImageCancelledKey] as? Bool, cancelled {
                        continuation.resume(throwing: PluginError.cancelled)
                    } else if let livePhoto {
                        continuation.resume(returning: livePhoto)
                    } else {
                        continuation.resume(throwing: PluginError.unavailable)
                    }
                }
            }
        }

        guard let url = URL(string: uri), url.isFileURL else {
            throw PluginError.invalidArgument
        }
        // File-based Live Photos need paired resources; without them the host
        // reports Unavailable rather than inventing motion.
        throw PluginError.unavailable
    }
}
