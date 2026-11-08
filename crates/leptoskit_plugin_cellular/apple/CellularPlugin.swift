import Foundation
import LeptosKitBridge
#if canImport(CoreTelephony)
import CoreTelephony
#endif

/// Wire codes shared with the Rust codec.
private enum Wire {
    static let snapshot: UInt32 = 0

    static let unknown: UInt32 = 0
    static let unavailable: UInt32 = 1
    static let restricted: UInt32 = 2
    static let ready: UInt32 = 3

    static let voipFalse: UInt32 = 3
    static let voipTrue: UInt32 = 4

    static let twoG: UInt32 = 3
    static let threeG: UInt32 = 4
    static let fourG: UInt32 = 5
    static let fiveG: UInt32 = 6

    static let maxCarrierBytes = 256
}

/// Carrier / radio snapshot from CoreTelephony when the platform exposes it.
@MainActor
public final class CellularPlugin: NativePlugin {
    public let name = "cellular"
    public let methods: [UInt32] = [Wire.snapshot]

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == Wire.snapshot else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return try encode(snapshot())
    }

    #if os(iOS) && canImport(CoreTelephony)
    /// Reads permission-light CoreTelephony fields without inventing values.
    private func snapshot() -> Snapshot {
        let info = CTTelephonyNetworkInfo()
        let carriers = info.serviceSubscriberCellularProviders ?? [:]
        let technologies = info.serviceCurrentRadioAccessTechnology ?? [:]

        // Prefer the data service when present; otherwise any reported service.
        let service = info.dataServiceIdentifier
            ?? carriers.keys.first
            ?? technologies.keys.first

        guard let service else {
            return Snapshot.unknownRadio()
        }

        let carrier = carriers[service]
        let technology = technologies[service]

        return Snapshot(
            carrierName: textField(carrier?.carrierName),
            mcc: codeField(carrier?.mobileCountryCode),
            mnc: codeField(carrier?.mobileNetworkCode),
            generation: generation(from: technology),
            allowsVoip: voip(from: carrier)
        )
    }

    private func textField(_ value: String?) -> TextField {
        guard let value else { return .unknown }

        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)

        return trimmed.isEmpty ? .unknown : .ready(trimmed)
    }

    private func codeField(_ value: String?) -> CodeField {
        guard let value else { return .unknown }

        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)

        guard !trimmed.isEmpty, let code = UInt16(trimmed) else { return .unknown }

        return .ready(code)
    }

    private func voip(from carrier: CTCarrier?) -> UInt32 {
        guard let carrier else { return Wire.unknown }

        return carrier.allowsVOIP ? Wire.voipTrue : Wire.voipFalse
    }

    private func generation(from technology: String?) -> UInt32 {
        guard let technology else { return Wire.unknown }

        switch technology {
        case CTRadioAccessTechnologyGPRS,
             CTRadioAccessTechnologyEdge,
             CTRadioAccessTechnologyCDMA1x:
            return Wire.twoG
        case CTRadioAccessTechnologyWCDMA,
             CTRadioAccessTechnologyHSDPA,
             CTRadioAccessTechnologyHSUPA,
             CTRadioAccessTechnologyCDMAEVDORev0,
             CTRadioAccessTechnologyCDMAEVDORevA,
             CTRadioAccessTechnologyCDMAEVDORevB,
             CTRadioAccessTechnologyeHRPD:
            return Wire.threeG
        case CTRadioAccessTechnologyLTE:
            return Wire.fourG
        default:
            if #available(iOS 14.1, *) {
                if technology == CTRadioAccessTechnologyNRNSA
                    || technology == CTRadioAccessTechnologyNR
                {
                    return Wire.fiveG
                }
            }

            return Wire.unknown
        }
    }
    #else
    /// macOS and non-telephony hosts have no cellular radio.
    private func snapshot() -> Snapshot {
        Snapshot.unavailable()
    }
    #endif
}

/// One validated cellular reading before packing.
private struct Snapshot {
    var carrierName: TextField
    var mcc: CodeField
    var mnc: CodeField
    var generation: UInt32
    var allowsVoip: UInt32

    static func unavailable() -> Snapshot {
        Snapshot(
            carrierName: .unavailable,
            mcc: .unavailable,
            mnc: .unavailable,
            generation: Wire.unavailable,
            allowsVoip: Wire.unavailable
        )
    }

    static func unknownRadio() -> Snapshot {
        Snapshot(
            carrierName: .unknown,
            mcc: .unknown,
            mnc: .unknown,
            generation: Wire.unknown,
            allowsVoip: Wire.unknown
        )
    }
}

private enum TextField {
    case unknown
    case unavailable
    case restricted
    case ready(String)

    var status: UInt32 {
        switch self {
        case .unknown: Wire.unknown
        case .unavailable: Wire.unavailable
        case .restricted: Wire.restricted
        case .ready: Wire.ready
        }
    }

    var text: String {
        if case let .ready(value) = self { return value }

        return ""
    }
}

private enum CodeField {
    case unknown
    case unavailable
    case restricted
    case ready(UInt16)

    var status: UInt32 {
        switch self {
        case .unknown: Wire.unknown
        case .unavailable: Wire.unavailable
        case .restricted: Wire.restricted
        case .ready: Wire.ready
        }
    }

    var value: UInt32 {
        if case let .ready(code) = self { return UInt32(code) }

        return 0
    }
}

private func encode(_ snapshot: Snapshot) throws -> Data {
    var result = Data(capacity: 64)

    write(&result, snapshot.generation)
    write(&result, snapshot.allowsVoip)
    write(&result, snapshot.mcc.status)
    write(&result, snapshot.mcc.value)
    write(&result, snapshot.mnc.status)
    write(&result, snapshot.mnc.value)
    write(&result, snapshot.carrierName.status)
    try write(&result, snapshot.carrierName.text)

    return result
}

private func write(_ result: inout Data, _ value: UInt32) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
}

private func write(_ result: inout Data, _ text: String) throws {
    let bytes = Data(text.utf8)

    guard bytes.count <= Wire.maxCarrierBytes else { throw PluginError.tooLarge }
    write(&result, UInt32(bytes.count))
    result.append(bytes)
}
