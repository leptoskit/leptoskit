import Foundation
import LeptosKitBridge

/// The installed host application's identity, install source and install
/// times. The byte layout is documented in the Rust crate and mirrored by
/// `ApplicationWire`.
@MainActor
public final class ApplicationPlugin: NativePlugin {
    public let name = "application"
    public let methods: [UInt32] = [0]

    private let facts: @MainActor () -> ApplicationFacts

    public convenience init() {
        self.init(facts: { ApplicationFacts.read(bundle: .main) })
    }

    /// Tests supply fixed facts instead of reading the bundle.
    init(facts: @escaping @MainActor () -> ApplicationFacts) {
        self.facts = facts
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == 0 else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return try ApplicationWire.snapshot(facts())
    }
}

/// Everything the plugin reports. `nil` means unknown.
struct ApplicationFacts: Equatable {
    var id: String?
    var name: String?
    var version: String?
    var build: String?
    var installerStore: String?
    var installTime: Date?
    var updateTime: Date?
}

/// The little-endian wire shared with Rust and Kotlin.
enum ApplicationWire {
    static let version: UInt32 = 2
    static let maxText = 4096
    static let maxInstaller = 256

    static func snapshot(_ facts: ApplicationFacts) throws -> Data {
        var data = Data(capacity: 160)
        append(&data, version)

        // Apple has no counterpart to Android's signing-certificate digest.
        let strings: [(String?, Int)] = [
            (facts.id, maxText), (facts.name, maxText), (facts.version, maxText), (facts.build, maxText),
            (nil, 0), (facts.installerStore, maxInstaller)
        ]

        for (text, max) in strings {
            let bytes = Data((text ?? "").utf8)

            guard bytes.count <= max else { throw PluginError.tooLarge }
            append(&data, UInt32(bytes.count))
            data.append(bytes)
        }

        for date in [facts.installTime, facts.updateTime] {
            let millis = date.flatMap(millis)
            append(&data, millis == nil ? 0 : 1)
            withUnsafeBytes(of: (millis ?? 0).littleEndian) { data.append(contentsOf: $0) }
        }

        return data
    }

    /// Whole milliseconds since 1970, or `nil` for earlier dates.
    static func millis(_ date: Date) -> UInt64? {
        let millis = (date.timeIntervalSince1970 * 1000).rounded(.down)

        return millis >= 0 && millis < 1.8e19 ? UInt64(millis) : nil
    }

    private static func append(_ data: inout Data, _ value: UInt32) {
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }
}

extension ApplicationFacts {
    /// Reads `bundle` and the app's containers.
    static func read(bundle: Bundle, fileManager: FileManager = .default) -> ApplicationFacts {
        let info = { (key: String) in bundle.object(forInfoDictionaryKey: key) as? String }
        #if os(iOS)
        let installer = installerStore(
            receipt: bundle.appStoreReceiptURL,
            hasProvisioningProfile: bundle.path(forResource: "embedded", ofType: "mobileprovision") != nil,
            simulator: isSimulator
        )
        let installed = installTime(fileManager)
        #else
        let installer: String? = nil
        // Only sandboxed Mac apps have a container whose creation marks the install.
        let sandboxed = ProcessInfo.processInfo.environment["APP_SANDBOX_CONTAINER_ID"] != nil
        let installed = sandboxed ? installTime(fileManager) : nil
        #endif

        return ApplicationFacts(
            id: bundle.bundleIdentifier,
            name: info("CFBundleDisplayName") ?? info("CFBundleName"),
            version: info("CFBundleShortVersionString"),
            build: info("CFBundleVersion"),
            installerStore: installer,
            installTime: installed,
            updateTime: modificationDate(bundle.bundleURL, fileManager)
        )
    }

    private static var isSimulator: Bool {
        #if targetEnvironment(simulator)
        true
        #else
        false
        #endif
    }

    /// Where an iOS app came from, judged from the App Store receipt's file
    /// name and whether the bundle carries a provisioning profile. Apple does
    /// not document this signal.
    ///
    /// - The simulator reports `com.apple.simulator`.
    /// - `receipt` means the App Store: `com.apple`.
    /// - `sandboxReceipt` with a provisioning profile means Xcode, ad hoc or
    ///   enterprise distribution: `com.apple.development`.
    /// - `sandboxReceipt` without one means TestFlight, which strips the
    ///   profile: `com.apple.testflight`.
    static func installerStore(receipt: URL?, hasProvisioningProfile: Bool, simulator: Bool) -> String? {
        if simulator { return "com.apple.simulator" }

        switch receipt?.lastPathComponent {
        case "receipt": return "com.apple"
        case "sandboxReceipt": return hasProvisioningProfile ? "com.apple.development" : "com.apple.testflight"
        default: return hasProvisioningProfile ? "com.apple.development" : nil
        }
    }

    /// The creation date of the app's Documents directory: the first install,
    /// surviving updates and reset by a reinstall.
    static func installTime(_ fileManager: FileManager) -> Date? {
        guard let documents = fileManager.urls(for: .documentDirectory, in: .userDomainMask).last,
              let attributes = try? fileManager.attributesOfItem(atPath: documents.path)
        else { return nil }

        return attributes[.creationDate] as? Date
    }

    /// The bundle's modification date, which an update replaces.
    static func modificationDate(_ url: URL, _ fileManager: FileManager) -> Date? {
        (try? fileManager.attributesOfItem(atPath: url.path))?[.modificationDate] as? Date
    }
}
