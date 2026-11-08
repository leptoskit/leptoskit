import Foundation
import LeptosKitBridge
@MainActor
public final class BlobPlugin: NativePlugin {
    public let name = "blob"
    public let methods: [UInt32] = [0, 1, 2]
    private var closed = false
    public init(context: NativePluginContext) {}
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0: _ = try BlobWire.create(payload); throw PluginError.unavailable
        case 1: _ = try BlobWire.read(payload); throw PluginError.unavailable
        case 2: _ = try BlobWire.id(payload); throw PluginError.unavailable
        default: throw PluginError.unavailable
        }
    }
    public func close() { closed = true }
}
