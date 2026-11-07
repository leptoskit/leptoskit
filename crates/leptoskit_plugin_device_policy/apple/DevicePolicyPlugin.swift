import Foundation
import LeptosKitBridge

/// Apple platforms have no Device Admin / Device Policy Manager contract.
/// Absence is never an inferred grant or an MDM enrollment.
@MainActor
public final class DevicePolicyPlugin: NativePlugin {
    public let name = "device-policy"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7]
    private var closed = false

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
        // Validate empty vs non-empty payloads before Unavailable so malformed
        // calls stay InvalidArgument on every host.
        try payload.withUnsafeBytes { bytes in
            switch method {
            case 0, 2, 7:
                guard bytes.isEmpty else { throw PluginError.invalidArgument }
            case 1, 3, 4, 5, 6:
                guard !bytes.isEmpty else { throw PluginError.invalidArgument }
            default:
                break
            }
        }
        throw PluginError.unavailable
    }

    public func close() {
        closed = true
    }
}
