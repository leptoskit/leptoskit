import BackgroundTasks
import Foundation
import LeptosKitBridge

/// Registers named `BGAppRefreshTask` identifiers. Distinct from the
/// `background` plugin's AppService keep-alive surface.
@MainActor
public final class BackgroundTaskPlugin: NativePlugin {
    public let name = "background-task"
    public let methods: [UInt32] = [0, 1, 2, 3]

    private var tasks: [String: UInt32] = [:]
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
            #if os(iOS)
            return BackgroundTaskWire.status(.available)
            #else
            return BackgroundTaskWire.status(.unavailable)
            #endif
        case .register:
            #if os(iOS)
            let request = try BackgroundTaskWire.register(payload)
            BGTaskScheduler.shared.register(forTaskWithIdentifier: request.name, using: nil) { task in
                task.setTaskCompleted(success: true)
            }
            var submit = BGAppRefreshTaskRequest(identifier: request.name)
            submit.earliestBeginDate = Date(timeIntervalSinceNow: TimeInterval(request.interval))
            try? BGTaskScheduler.shared.submit(submit)
            tasks[request.name] = request.interval
            return Data()
            #else
            _ = try BackgroundTaskWire.register(payload)
            throw PluginError.unavailable
            #endif
        case .unregister:
            #if os(iOS)
            let name = try BackgroundTaskWire.name(payload)
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: name)
            tasks.removeValue(forKey: name)
            return Data()
            #else
            _ = try BackgroundTaskWire.name(payload)
            throw PluginError.unavailable
            #endif
        case .complete:
            #if os(iOS)
            let name = try BackgroundTaskWire.name(payload)
            guard tasks[name] != nil else { throw PluginError.invalidArgument }
            // Host-owned handlers already call setTaskCompleted; guest COMPLETE
            // acknowledges the expiration window without inventing a second run.
            return Data()
            #else
            _ = try BackgroundTaskWire.name(payload)
            throw PluginError.unavailable
            #endif
        }
    }

    public func close() {
        closed = true
        #if os(iOS)
        for name in tasks.keys {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: name)
        }
        #endif
        tasks.removeAll()
    }
}
