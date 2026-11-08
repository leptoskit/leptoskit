import Foundation
import LeptosKitBridge

@MainActor
public final class AssetPlugin: NativePlugin {
    public let name = "asset"
    public let methods: [UInt32] = [0, 1]
    private var closed = false
    public init(context: NativePluginContext) {}
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return AssetWire.capability(.unavailable)
        case 1:
            _ = try AssetWire.name(payload)
            return AssetWire.resolvedUnavailable()
        default:
            throw PluginError.unavailable
        }
    }
    public func close() { closed = true }
}
