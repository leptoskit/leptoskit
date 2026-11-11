import AVFoundation
import Foundation
import LeptosKitBridge
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Host-owned `AVCaptureSession` registry with preview binding, torch, and still capture.
@MainActor
public final class CameraPlugin: NativePlugin {
    public let name = "camera"
    public let methods: [UInt32] = [0, 1, 2, 3, 4]

    private var sessions: [UInt32: Session] = [:]
    private var nextId: UInt32 = 1
    private var closed = false

    private enum Method: UInt32 {
        case create = 0
        case setTorch = 1
        case takePicture = 2
        case status = 3
        case release = 4
    }

    private final class Session {
        let facing: UInt32
        let capture: AVCaptureSession
        let photo: AVCapturePhotoOutput
        var device: AVCaptureDevice
        var torchOn = false

        init(facing: UInt32, capture: AVCaptureSession, photo: AVCapturePhotoOutput, device: AVCaptureDevice) {
            self.facing = facing
            self.capture = capture
            self.photo = photo
            self.device = device
        }

        var torchAvailable: Bool {
            device.hasTorch && device.isTorchAvailable
        }
    }

    public init() {
        NativeCameraBinding.session = { [weak self] id in self?.sessions[id]?.capture }
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard let method = Method(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .create: return try create(CameraWire.create(payload))
        case .setTorch:
            let torch = try CameraWire.torch(payload)
            try setTorch(torch.id, torch.enabled)
            return Data()
        case .takePicture:
            let request = try CameraWire.takePicture(payload)
            return try await takePicture(request.id, maxBytes: request.maxBytes)
        case .status: return try status(CameraWire.session(payload))
        case .release:
            try release(CameraWire.session(payload))
            return Data()
        }
    }

    public func close() {
        closed = true
        for id in Array(sessions.keys) {
            try? release(id)
        }
        NativeCameraBinding.session = nil
    }

    private func create(_ facing: UInt32) throws -> Data {
        try requireCameraPermission()
        let position: AVCaptureDevice.Position = facing == 0 ? .back : .front
        guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: position)
            ?? AVCaptureDevice.default(for: .video)
        else {
            throw PluginError.unavailable
        }
        let input = try AVCaptureDeviceInput(device: device)
        let capture = AVCaptureSession()
        capture.beginConfiguration()
        guard capture.canAddInput(input) else { throw PluginError.unavailable }
        capture.addInput(input)
        let photo = AVCapturePhotoOutput()
        guard capture.canAddOutput(photo) else { throw PluginError.unavailable }
        capture.addOutput(photo)
        capture.commitConfiguration()
        capture.startRunning()

        let id = nextId
        nextId = nextId &+ 1
        if nextId == 0 { nextId = 1 }
        sessions[id] = Session(facing: facing, capture: capture, photo: photo, device: device)
        return CameraWire.sessionId(id)
    }

    private func setTorch(_ id: UInt32, _ enabled: Bool) throws {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        guard session.torchAvailable else { throw PluginError.unavailable }
        try session.device.lockForConfiguration()
        defer { session.device.unlockForConfiguration() }
        if enabled {
            try session.device.setTorchModeOn(level: 1.0)
        } else {
            session.device.torchMode = .off
        }
        session.torchOn = enabled
    }

    private func takePicture(_ id: UInt32, maxBytes: UInt32) async throws -> Data {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        try requireCameraPermission()
        let settings = AVCapturePhotoSettings()
        let data = try await capturePhoto(session.photo, settings: settings)
        guard data.count <= Int(maxBytes) else { throw PluginError.tooLarge }
        let url = try writeTemp(data)
        #if os(macOS)
        let image = NSImage(data: data)
        let size = image?.size ?? .zero
        let width = UInt32(max(size.width, 0).rounded())
        let height = UInt32(max(size.height, 0).rounded())
        #else
        let image = UIImage(data: data)
        let width = UInt32(image?.size.width.rounded() ?? 0)
        let height = UInt32(image?.size.height.rounded() ?? 0)
        #endif
        return try CameraWire.asset(
            uri: url.absoluteString,
            mime: "image/jpeg",
            width: width == 0 ? nil : width,
            height: height == 0 ? nil : height,
            size: UInt64(data.count)
        )
    }

    private func status(_ id: UInt32) throws -> Data {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        return CameraWire.status(
            facing: session.facing,
            torchOn: session.torchOn,
            torchAvailable: session.torchAvailable,
            active: session.capture.isRunning
        )
    }

    private func release(_ id: UInt32) throws {
        guard let session = sessions.removeValue(forKey: id) else { throw PluginError.invalidArgument }
        if session.torchOn, session.torchAvailable {
            try? session.device.lockForConfiguration()
            session.device.torchMode = .off
            session.device.unlockForConfiguration()
        }
        session.capture.stopRunning()
    }

    private func requireCameraPermission() throws {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            return
        case .denied, .restricted:
            throw PluginError.permissionDenied
        case .notDetermined:
            // Apps request through the `permissions` plugin; do not prompt here.
            throw PluginError.permissionDenied
        @unknown default:
            throw PluginError.unavailable
        }
    }

    private func capturePhoto(
        _ output: AVCapturePhotoOutput,
        settings: AVCapturePhotoSettings
    ) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            let delegate = PhotoDelegate { result in
                continuation.resume(with: result)
            }
            // Retain the delegate for the capture lifetime.
            PhotoDelegate.retain(delegate)
            output.capturePhoto(with: settings, delegate: delegate)
        }
    }

    private func writeTemp(_ data: Data) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("leptoskit-camera", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("\(UUID().uuidString).jpg")
        try data.write(to: url, options: .atomic)
        return url
    }
}

private final class PhotoDelegate: NSObject, AVCapturePhotoCaptureDelegate {
    private static let live = PhotoDelegateStore()
    private let finish: (Result<Data, Error>) -> Void

    init(finish: @escaping (Result<Data, Error>) -> Void) {
        self.finish = finish
    }

    static func retain(_ delegate: PhotoDelegate) {
        live.insert(delegate)
    }

    func photoOutput(
        _ output: AVCapturePhotoOutput,
        didFinishProcessingPhoto photo: AVCapturePhoto,
        error: Error?
    ) {
        defer { Self.live.remove(self) }
        if let error {
            finish(.failure(error))
            return
        }
        guard let data = photo.fileDataRepresentation() else {
            finish(.failure(PluginError.native))
            return
        }
        finish(.success(data))
    }
}

private final class PhotoDelegateStore: @unchecked Sendable {
    private let lock = NSLock()
    private var delegates: [ObjectIdentifier: PhotoDelegate] = [:]

    func insert(_ delegate: PhotoDelegate) {
        lock.lock()
        defer { lock.unlock() }
        delegates[ObjectIdentifier(delegate)] = delegate
    }

    func remove(_ delegate: PhotoDelegate) {
        lock.lock()
        defer { lock.unlock() }
        delegates.removeValue(forKey: ObjectIdentifier(delegate))
    }
}
