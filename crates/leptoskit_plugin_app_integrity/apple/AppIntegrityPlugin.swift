import Foundation
import LeptosKitBridge

/// App attestation. This MVP validates the wire and returns Unavailable until
/// App Attest / DeviceCheck wiring lands.
@MainActor
public final class AppIntegrityPlugin: NativePlugin {
    public let name = "app-integrity"
    public let methods: [UInt32] = [0, 1]
    private var closed = false

    public init(context: NativePluginContext) {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return AppIntegrityWire.capability(.unavailable)
        case 1:
            _ = try AppIntegrityWire.challenge(payload)
            return AppIntegrityWire.attestationUnavailable()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() { closed = true }
}
