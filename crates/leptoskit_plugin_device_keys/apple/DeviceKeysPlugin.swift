import Foundation
import LeptosKitBridge
import LocalAuthentication
import Security

/// Hardware-bound P-256 signing keys in the Secure Enclave, gated by biometrics
/// or the device passcode. The private key never leaves the Secure Enclave, so
/// a server can treat a signature as proof the device owner authenticated on
/// this device just now.
@MainActor
public final class DeviceKeysPlugin: NSObject, NativePlugin {
    public let name = "device-keys"
    public let methods: [UInt32] = [0, 1, 2, 3, 4]
    private var closed = false

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case 0: return try generate(DeviceKeysGenerateRequest(payload: payload))
        case 1: return try publicKey(decodeAlias(payload))
        case 2: return try sign(DeviceKeysSignRequest(payload: payload))
        case 3: return try delete(decodeAlias(payload))
        case 4:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return available()
        default: throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
    }

    /// Reports hardware, biometric hardware, and enrollment without prompting.
    private func available() -> Data {
        let context = LAContext()
        var error: NSError?
        let enrolled = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        let hardware = Self.secureEnclaveAvailable() ? HardwareKind.secureEnclave : HardwareKind.none

        return deviceKeysFields([1, hardware, biometryKind(context.biometryType), enrolled ? 1 : 0])
    }

    /// Creates a permanent, non-exportable P-256 key in the Secure Enclave.
    private func generate(_ request: DeviceKeysGenerateRequest) throws -> Data {
        guard let application = Bundle.main.bundleIdentifier else {
            return deviceKeysFields([1, KeyStatus.unavailable])
        }

        if let refusal = generateStatus(request) {
            return deviceKeysFields([1, refusal])
        }

        let flags: SecAccessControlCreateFlags = request.policy == PolicyKind.biometry
            ? [.privateKeyUsage, .biometryCurrentSet]
            : [.privateKeyUsage, .userPresence]
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, &error
        ) else {
            return deviceKeysFields([1, KeyStatus.unavailable])
        }

        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
            kSecAttrTokenID as String: kSecAttrTokenIDSecureEnclave,
            kSecAttrApplicationTag as String: Self.applicationTag(application, alias: request.alias),
            kSecAttrAccessControl as String: access,
            kSecAttrIsPermanent as String: true
        ]
        guard let privateKey = SecKeyCreateRandomKey(attributes as CFDictionary, &error) else {
            return deviceKeysFields([1, Self.generateStatus(for: error)])
        }

        return try publicKeyFields(privateKey: privateKey)
    }

    /// Reads a key's public key, or reports absence.
    private func publicKey(_ alias: String) throws -> Data {
        guard let application = Bundle.main.bundleIdentifier else {
            return deviceKeysFields([1, KeyStatus.unavailable])
        }

        var item: CFTypeRef?
        let status = SecItemCopyMatching(Self.keyQuery(application, alias: alias) as CFDictionary, &item)

        if status == errSecItemNotFound { return deviceKeysFields([1, KeyStatus.ok, 0]) }
        try Self.check(status)

        guard let item, let privateKey = item as? SecKey else {
            return deviceKeysFields([1, KeyStatus.failed])
        }

        return try publicKeyFields(privateKey: privateKey)
    }

    /// Shows the system prompt with `reason`, then signs the message.
    private func sign(_ request: DeviceKeysSignRequest) throws -> Data {
        guard let application = Bundle.main.bundleIdentifier else {
            return deviceKeysFields([1, KeyStatus.unavailable])
        }

        let context = LAContext()
        context.localizedReason = request.reason

        var query = Self.keyQuery(application, alias: request.alias)
        query[kSecUseAuthenticationContext as String] = context

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)

        if status == errSecItemNotFound { return deviceKeysFields([1, KeyStatus.notFound]) }
        try Self.check(status)

        guard let item, let privateKey = item as? SecKey else {
            return deviceKeysFields([1, KeyStatus.failed])
        }

        var error: Unmanaged<CFError>?
        guard let signature = SecKeyCreateSignature(
            privateKey, .ecdsaSignatureMessageX962SHA256, request.message as CFData, &error
        ) as Data? else {
            return deviceKeysFields([1, keyStatus(for: error?.takeRetainedValue() as NSError?)])
        }

        return deviceKeysFields([1, KeyStatus.ok]) + lengthPrefixed(signature)
    }

    /// Deletes a key. Deleting an absent alias succeeds.
    private func delete(_ alias: String) throws -> Data {
        guard let application = Bundle.main.bundleIdentifier else {
            return deviceKeysFields([1, KeyStatus.unavailable])
        }

        let query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: Self.applicationTag(application, alias: alias)
        ]
        let status = SecItemDelete(query as CFDictionary)

        if status != errSecItemNotFound { try Self.check(status) }

        return deviceKeysFields([1, KeyStatus.ok])
    }

    /// Encodes a public key as its SEC1 point and SPKI DER, after a success status.
    private func publicKeyFields(privateKey: SecKey) throws -> Data {
        guard let publicKey = SecKeyCopyPublicKey(privateKey) else {
            return deviceKeysFields([1, KeyStatus.failed])
        }
        var error: Unmanaged<CFError>?
        guard let sec1 = SecKeyCopyExternalRepresentation(publicKey, &error) as Data? else {
            return deviceKeysFields([1, KeyStatus.failed])
        }

        return deviceKeysFields([1, KeyStatus.ok, 1]) + lengthPrefixed(sec1) + lengthPrefixed(spki(sec1: sec1))
    }

    /// Why a biometry-only generate must not start, or nil when it may.
    private func generateStatus(_ request: DeviceKeysGenerateRequest) -> UInt32? {
        guard request.policy == PolicyKind.biometry else { return nil }

        let context = LAContext()
        var error: NSError?

        if context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error) {
            return nil
        }

        return keyStatus(for: error)
    }

    /// The status for a failed key generation.
    private static func generateStatus(for error: Unmanaged<CFError>?) -> UInt32 {
        guard let cfError = error?.takeRetainedValue() else { return KeyStatus.unavailable }
        let code = CFErrorGetCode(cfError)

        if code == errSecDuplicateItem { return KeyStatus.alreadyExists }

        return KeyStatus.unavailable
    }

    /// The application-scoped tag that names one alias.
    private static func applicationTag(_ application: String, alias: String) -> Data {
        Data("\(application).device-keys.\(alias)".utf8)
    }

    /// A query that finds a stored private key by its application tag.
    private static func keyQuery(_ application: String, alias: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: applicationTag(application, alias: alias),
            kSecReturnRef as String: true
        ]
    }

    /// Whether the Secure Enclave is available, probed with an ephemeral key.
    private static func secureEnclaveAvailable() -> Bool {
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, [.privateKeyUsage], &error
        ) else { return false }
        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
            kSecAttrTokenID as String: kSecAttrTokenIDSecureEnclave,
            kSecAttrIsPermanent as String: false,
            kSecAttrAccessControl as String: access
        ]

        return SecKeyCreateRandomKey(attributes as CFDictionary, &error) != nil
    }

    /// Maps a Keychain status onto a bridge error.
    private static func check(_ status: OSStatus) throws {
        switch status {
        case errSecSuccess: return
        case errSecAuthFailed, errSecInteractionNotAllowed, errSecMissingEntitlement:
            throw PluginError.permissionDenied
        case errSecNotAvailable: throw PluginError.unavailable
        default: throw PluginError.native
        }
    }
}
