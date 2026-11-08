import Foundation
import LeptosKitBridge
import LocalAuthentication
import Security

/// Byte limits the Rust crate enforces on the wire values.
enum DeviceKeysLimits {
    /// Longest accepted alias, in ASCII bytes.
    static let aliasBytes = 64
    /// Longest accepted message, in bytes.
    static let messageBytes = 4096
    /// Longest accepted reason, in UTF-8 bytes.
    static let reasonBytes = 200
}

/// Terminal status codes the replies carry.
enum KeyStatus {
    static let ok: UInt32 = 0
    static let notFound: UInt32 = 1
    static let alreadyExists: UInt32 = 2
    static let cancelled: UInt32 = 3
    static let lockedOut: UInt32 = 4
    static let notEnrolled: UInt32 = 5
    static let invalidated: UInt32 = 6
    static let unavailable: UInt32 = 7
    static let failed: UInt32 = 8
}

/// Hardware kinds the availability reply carries.
enum HardwareKind {
    static let secureEnclave: UInt32 = 0
    static let none: UInt32 = 3
}

/// Biometric hardware kinds the availability reply carries.
enum BiometryKind {
    static let touchId: UInt32 = 0
    static let faceId: UInt32 = 1
    static let opticId: UInt32 = 2
    static let none: UInt32 = 6
}

/// Authentication policies the requests carry.
enum PolicyKind {
    static let biometry: UInt32 = 0
    static let biometryOrPasscode: UInt32 = 1
}

/// The fixed SubjectPublicKeyInfo prefix for a P-256 key, before the point.
/// `SEQUENCE` wrapping `SEQUENCE (ecPublicKey, prime256v1)` and a BIT STRING
/// whose first byte is the unused-bit count `0x00`.
private let p256SpkiPrefix: [UInt8] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01,
    0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00
]

/// Appends every field as a little-endian u32.
func deviceKeysFields(_ fields: [UInt32]) -> Data {
    var output = Data(capacity: fields.count * 4)

    for field in fields {
        var little = field.littleEndian
        withUnsafeBytes(of: &little) { output.append(contentsOf: $0) }
    }

    return output
}

/// Length-prefixes one byte blob.
func lengthPrefixed(_ value: Data) -> Data {
    var output = deviceKeysFields([UInt32(value.count)])
    output.append(value)
    return output
}

/// Builds the SubjectPublicKeyInfo DER for a P-256 point.
func spki(sec1: Data) -> Data {
    Data(p256SpkiPrefix) + sec1
}

/// The terminal status for a SecKey signing failure.
func keyStatus(for error: NSError?) -> UInt32 {
    guard let error, error.domain == LAError.errorDomain else {
        return keyStatusForOSStatus(error.map { Int32($0.code) } ?? errSecAuthFailed)
    }

    switch error.code {
    case LAError.userCancel.rawValue, LAError.appCancel.rawValue, LAError.systemCancel.rawValue:
        return KeyStatus.cancelled
    case LAError.biometryLockout.rawValue:
        return KeyStatus.lockedOut
    case LAError.biometryNotEnrolled.rawValue, LAError.passcodeNotSet.rawValue:
        return KeyStatus.notEnrolled
    case LAError.biometryNotAvailable.rawValue:
        return KeyStatus.unavailable
    default:
        return KeyStatus.failed
    }
}

/// The terminal status for one raw OSStatus signing failure.
func keyStatusForOSStatus(_ code: Int32) -> UInt32 {
    switch code {
    case errSecUserCanceled:
        return KeyStatus.cancelled
    case errSecItemNotFound:
        return KeyStatus.notFound
    case errSecInteractionNotAllowed, errSecNotAvailable:
        return KeyStatus.unavailable
    default:
        return KeyStatus.failed
    }
}

/// The biometric hardware kind the OS reports.
func biometryKind(_ type: LABiometryType) -> UInt32 {
    switch type {
    case .touchID: BiometryKind.touchId
    case .faceID: BiometryKind.faceId
    case .opticID: BiometryKind.opticId
    default: BiometryKind.none
    }
}

/// A length-checked field reader over a borrowed request payload.
struct DeviceKeysReader {
    let bytes: UnsafeRawBufferPointer
    var offset = 0
    var finished: Bool { offset == bytes.count }

    mutating func integer() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        defer { offset += 4 }

        return bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self).littleEndian
    }

    mutating func text(limit: Int) throws -> String {
        let count = Int(try integer())

        guard count <= limit else { throw PluginError.tooLarge }

        guard count <= bytes.count - offset,
              let value = String(bytes: bytes[offset..<(offset + count)], encoding: .utf8),
              !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !value.contains("\0") else { throw PluginError.invalidArgument }
        offset += count

        return value
    }

    mutating func alias() throws -> String {
        let count = Int(try integer())

        guard count <= DeviceKeysLimits.aliasBytes else { throw PluginError.tooLarge }

        guard count > 0, count <= bytes.count - offset else { throw PluginError.invalidArgument }
        let slice = bytes[offset..<(offset + count)]
        offset += count

        let valid = slice.allSatisfy { byte in
            (65...90).contains(byte) || (97...122).contains(byte) || (48...57).contains(byte)
                || byte == 45 || byte == 46 || byte == 95
        }

        guard valid, let value = String(bytes: slice, encoding: .ascii) else {
            throw PluginError.invalidArgument
        }

        return value
    }

    mutating func data(limit: Int, nonEmpty: Bool = false) throws -> Data {
        let count = Int(try integer())

        guard count <= limit else { throw PluginError.tooLarge }

        if nonEmpty, count == 0 { throw PluginError.invalidArgument }

        guard count <= bytes.count - offset else { throw PluginError.invalidArgument }
        let value = Data(bytes[offset..<(offset + count)])
        offset += count

        return value
    }
}

/// The decoded `generate` request.
struct DeviceKeysGenerateRequest {
    let policy: UInt32
    let alias: String

    init(payload: NativeBuffer) throws {
        self = try payload.withUnsafeBytes { bytes in
            var input = DeviceKeysReader(bytes: bytes)

            guard try input.integer() == 1 else { throw PluginError.invalidArgument }
            let policy = try input.integer()

            guard policy == PolicyKind.biometry || policy == PolicyKind.biometryOrPasscode else {
                throw PluginError.invalidArgument
            }
            let alias = try input.alias()

            guard input.finished else { throw PluginError.invalidArgument }

            return DeviceKeysGenerateRequest(policy: policy, alias: alias)
        }
    }

    private init(policy: UInt32, alias: String) {
        self.policy = policy
        self.alias = alias
    }
}

/// The decoded `sign` request.
struct DeviceKeysSignRequest {
    let alias: String
    let message: Data
    let reason: String

    init(payload: NativeBuffer) throws {
        self = try payload.withUnsafeBytes { bytes in
            var input = DeviceKeysReader(bytes: bytes)

            guard try input.integer() == 1 else { throw PluginError.invalidArgument }
            let alias = try input.alias()
            let message = try input.data(limit: DeviceKeysLimits.messageBytes, nonEmpty: true)
            let reason = try input.text(limit: DeviceKeysLimits.reasonBytes)

            guard input.finished else { throw PluginError.invalidArgument }

            return DeviceKeysSignRequest(alias: alias, message: message, reason: reason)
        }
    }

    private init(alias: String, message: Data, reason: String) {
        self.alias = alias
        self.message = message
        self.reason = reason
    }
}

/// Decodes the length-prefixed alias a public-key or delete request carries.
func decodeAlias(_ payload: NativeBuffer) throws -> String {
    try payload.withUnsafeBytes { bytes -> String in
        var input = DeviceKeysReader(bytes: bytes)
        let alias = try input.alias()

        guard input.finished else { throw PluginError.invalidArgument }

        return alias
    }
}
