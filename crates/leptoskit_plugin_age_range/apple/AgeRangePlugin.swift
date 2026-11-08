import Foundation
import LeptosKitBridge

/// Declared age-range prompts. This MVP returns Unavailable on every Apple
/// host until Declared Age Range wiring lands; wire validation stays honest.
@MainActor
public final class AgeRangePlugin: NativePlugin {
    public let name = "age-range"
    public let methods: [UInt32] = [0, 1]
    private var closed = false

    public init(context: NativePluginContext) {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return AgeRangeWire.capability(.unavailable)
        case 1:
            _ = try AgeRangeWire.threshold(payload)
            return AgeRangeWire.responseUnavailable()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
    }
}
