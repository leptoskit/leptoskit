import Foundation
import LeptosKitBridge

/// Named OS background-task registration. Distinct from the `background`
/// plugin's AppService keep-alive surface.
///
/// This Apple MVP returns Unavailable after validating the wire. Live
/// `BGTaskScheduler` scheduling needs declared `BGTaskSchedulerPermittedIdentifiers`
/// / `UIBackgroundModes` and honest register/submit error handling — later scope.
@MainActor
public final class BackgroundTaskPlugin: NativePlugin {
    public let name = "background-task"
    public let methods: [UInt32] = [0, 1, 2, 3]

    private var closed = false

    private enum Method: UInt32 {
        case status = 0
        case register = 1
        case unregister = 2
        case complete = 3
    }

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard let method = Method(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .status:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return BackgroundTaskWire.status(.unavailable)
        case .register:
            _ = try BackgroundTaskWire.register(payload)
            throw PluginError.unavailable
        case .unregister, .complete:
            _ = try BackgroundTaskWire.name(payload)
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
    }
}
