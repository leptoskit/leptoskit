import Foundation
import LeptosKitBridge
#if os(iOS)
import CoreMotion
import UIKit
#endif

/// Fused device-motion readings from CoreMotion on iOS. macOS reports
/// unavailable: there is no portable device-motion API there.
@MainActor
public final class DeviceMotionPlugin: NativePlugin {
    public let name = "device_motion"
    public let methods: [UInt32] = DeviceMotionWire.methods
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    #if os(iOS)
    private let motion = CMMotionManager()
    private var backgroundObserver: NSObjectProtocol?
    #endif

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }

        if method == DeviceMotionWire.available {
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return DeviceMotionWire.encodeAvailability(available)
        }

        let interval = try DeviceMotionWire.decodeInterval(payload)
        guard available else { throw PluginError.unavailable }

        #if os(iOS)
        return try await awaitSample(interval: interval)
        #else
        _ = interval
        throw PluginError.unavailable
        #endif
    }

    private var available: Bool {
        #if os(iOS)
        motion.isDeviceMotionAvailable
        #else
        false
        #endif
    }

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
    }

    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        #if os(iOS)
        motion.stopDeviceMotionUpdates()
        if let backgroundObserver {
            NotificationCenter.default.removeObserver(backgroundObserver)
        }
        backgroundObserver = nil
        #endif
        continuation.resume(with: result)
    }

    #if os(iOS)
    private func awaitSample(interval: TimeInterval) async throws -> Data {
        if pending != nil { throw PluginError.busy }

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                pending = continuation
                motion.deviceMotionUpdateInterval = interval
                motion.startDeviceMotionUpdates(to: .main) { [weak self] sample, error in
                    MainActor.assumeIsolated {
                        self?.deliver(sample: sample, error: error)
                    }
                }
                backgroundObserver = NotificationCenter.default.addObserver(
                    forName: UIApplication.didEnterBackgroundNotification,
                    object: nil,
                    queue: .main
                ) { [weak self] _ in
                    MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
                }
            }
        } onCancel: {
            Task { @MainActor in self.finish(.failure(PluginError.cancelled)) }
        }
    }

    private func deliver(sample: CMDeviceMotion?, error: (any Error)?) {
        if let error {
            let native = error as NSError
            let denied = native.domain == CMErrorDomain && (
                native.code == Int(CMErrorMotionActivityNotAuthorized.rawValue)
                    || native.code == Int(CMErrorNotAuthorized.rawValue)
            )
            finish(.failure(denied ? PluginError.permissionDenied : PluginError.native))
            return
        }

        guard let sample else {
            finish(.failure(PluginError.native))
            return
        }

        do {
            let scale = DeviceMotionWire.gravityToAndroid
            let data = try DeviceMotionWire.encodeReading(
                timestamp: sample.timestamp,
                accelX: sample.userAcceleration.x * scale,
                accelY: sample.userAcceleration.y * scale,
                accelZ: sample.userAcceleration.z * scale,
                gravityX: sample.gravity.x * scale,
                gravityY: sample.gravity.y * scale,
                gravityZ: sample.gravity.z * scale,
                rotationX: sample.rotationRate.x,
                rotationY: sample.rotationRate.y,
                rotationZ: sample.rotationRate.z,
                attitudeAlpha: sample.attitude.yaw,
                attitudeBeta: sample.attitude.pitch,
                attitudeGamma: sample.attitude.roll
            )
            finish(.success(data))
        } catch {
            finish(.failure(error))
        }
    }
    #endif
}
