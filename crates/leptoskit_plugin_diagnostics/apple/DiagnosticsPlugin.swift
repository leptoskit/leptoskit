import Foundation
import LeptosKitBridge

@MainActor
public final class DiagnosticsPlugin: NativePlugin {
    public let name = "diagnostics"
    public let methods: [UInt32] = [0, 1, 2, 3]
    private var closed = false
    private var nextId: UInt32 = 0
    private var open: Set<UInt32> = []
    private let maxOpen = 256

    public init(context: NativePluginContext) {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            try DiagnosticsWire.emptyPayload(payload)
            return DiagnosticsWire.capability(.available)
        case 1:
            _ = try DiagnosticsWire.name(payload)
            guard open.count < maxOpen else { throw PluginError.busy }
            guard nextId < UInt32.max else { throw PluginError.exhausted }
            nextId += 1
            open.insert(nextId)
            return DiagnosticsWire.encodeId(nextId)
        case 2:
            let id = try DiagnosticsWire.id(payload)
            guard open.remove(id) != nil else { throw PluginError.invalidArgument }
            return Data()
        case 3:
            _ = try DiagnosticsWire.name(payload)
            return Data()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() { closed = true }
}
