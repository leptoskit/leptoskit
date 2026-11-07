import Darwin
import Foundation
#if os(iOS)
import UIKit
#elseif os(macOS)
import IOKit
import SystemConfiguration
#endif

extension DeviceFacts {
    /// Reads this device. Nothing here is a persistent identifier.
    @MainActor
    static func current() -> DeviceFacts {
        let process = ProcessInfo.processInfo
        let disk = volume(NSHomeDirectory())
        var facts = DeviceFacts()
        facts.kind = kind
        facts.simulator = simulator
        facts.totalMemory = process.physicalMemory
        facts.availableMemory = freeMemory()
        facts.totalDisk = disk?.total ?? 0
        facts.freeDisk = disk?.free
        facts.cpuCount = UInt32(clamping: process.activeProcessorCount)
        facts.manufacturer = "Apple"
        facts.brand = "Apple"
        facts.modelIdentifier = modelIdentifier(environment: process.environment)
        facts.architecture = architecture
        facts.kernelVersion = kernelRelease()
        facts.iosAppOnMac = process.isiOSAppOnMac
        facts.macCatalyst = process.isMacCatalystApp
        facts.iosAppOnVision = iosAppOnVision(process)
        facts.cpuFrequency = sysctlNumber("hw.cpufrequency") ?? 0
        facts.osBuild = sysctlString("kern.osversion")
        #if os(iOS)
        let device = UIDevice.current
        facts.model = device.model
        facts.localizedModel = device.localizedModel
        facts.osName = device.systemName
        facts.osVersion = device.systemVersion
        #else
        let version = process.operatingSystemVersion
        facts.osName = "macOS"
        facts.osVersion = "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)"
        #endif
        return facts
    }

    /// `ProcessInfo.isiOSAppOnVision` exists from the iOS 26.1 SDK.
    private static func iosAppOnVision(_ process: ProcessInfo) -> Bool? {
        #if compiler(>=6.2.1)
        if #available(iOS 26.1, macOS 26.1, *) {
            return process.isiOSAppOnVision
        }
        #endif
        return nil
    }

    @MainActor
    private static var kind: UInt32 {
        #if os(iOS)
        switch UIDevice.current.userInterfaceIdiom {
        case .phone: 1
        case .pad: 2
        case .tv: 3
        case .mac: 4
        case .carPlay: 6
        case .vision: 7
        default: 0
        }
        #else
        4
        #endif
    }

    private static var simulator: Bool? {
        #if targetEnvironment(simulator)
        true
        #elseif os(iOS)
        false
        #else
        nil
        #endif
    }

    private static var architecture: String {
        #if arch(arm64)
        "aarch64"
        #elseif arch(x86_64)
        "x86_64"
        #else
        ""
        #endif
    }

    /// `iPhone15,2` on iOS — the simulated model in the simulator — and
    /// `Mac14,2` on macOS.
    static func modelIdentifier(environment: [String: String]) -> String? {
        #if targetEnvironment(simulator)
        return environment["SIMULATOR_MODEL_IDENTIFIER"]
        #elseif os(iOS)
        return sysctlString("hw.machine")
        #else
        return sysctlString("hw.model")
        #endif
    }

    /// The kernel release from `uname`, such as `24.1.0`.
    static func kernelRelease() -> String? {
        var names = utsname()

        guard uname(&names) == 0 else { return nil }

        return withUnsafeBytes(of: &names.release) { bytes in
            let text = bytes.prefix { $0 != 0 }
            return text.isEmpty ? nil : String(bytes: text, encoding: .utf8)
        }
    }

    /// Free pages times the page size. iOS reports only truly free pages, so
    /// this is lower than what the system could reclaim.
    static func freeMemory() -> UInt64? {
        var statistics = vm_statistics64_data_t()
        var count = mach_msg_type_number_t(
            MemoryLayout<vm_statistics64_data_t>.size / MemoryLayout<integer_t>.size
        )
        let status = withUnsafeMutablePointer(to: &statistics) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                host_statistics64(mach_host_self(), HOST_VM_INFO64, $0, &count)
            }
        }

        guard status == KERN_SUCCESS else { return nil }

        return UInt64(statistics.free_count) * UInt64(getpagesize())
    }

    /// The size of, and space this app can still use on, the volume at `path`.
    static func volume(_ path: String) -> (total: UInt64, free: UInt64)? {
        guard let attributes = try? FileManager.default.attributesOfFileSystem(forPath: path),
              let total = (attributes[.systemSize] as? NSNumber)?.uint64Value,
              let free = (attributes[.systemFreeSize] as? NSNumber)?.uint64Value
        else { return nil }

        return (total, min(free, total))
    }

    /// A NUL-terminated `sysctl` string, at most 1 KiB.
    static func sysctlString(_ name: String) -> String? {
        var size = 0

        guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0, size <= 1024 else { return nil }
        var buffer = [UInt8](repeating: 0, count: size)

        guard sysctlbyname(name, &buffer, &size, nil, 0) == 0 else { return nil }
        let text = buffer.prefix(size).prefix { $0 != 0 }

        return text.isEmpty ? nil : String(bytes: text, encoding: .utf8)
    }

    /// A 32- or 64-bit `sysctl` number.
    static func sysctlNumber(_ name: String) -> UInt64? {
        var value: UInt64 = 0
        var size = MemoryLayout<UInt64>.size

        guard sysctlbyname(name, &value, &size, nil, 0) == 0 else { return nil }

        switch size {
        case 8: return value
        case 4: return UInt64(UInt32(truncatingIfNeeded: value))
        default: return nil
        }
    }
}

extension DeviceIdentifiers {
    /// Reads the identifiers. Only called for method 1.
    @MainActor
    static func current() -> DeviceIdentifiers {
        #if os(iOS)
        let device = UIDevice.current

        return DeviceIdentifiers(
            deviceName: userAssignedName(device.name, model: device.model),
            vendorId: device.identifierForVendor?.uuidString,
            machineId: nil
        )
        #else
        return DeviceIdentifiers(
            deviceName: bounded(SCDynamicStoreCopyComputerName(nil, nil) as String?),
            vendorId: nil,
            machineId: platformUUID()
        )
        #endif
    }

    /// From iOS 16, `UIDevice.name` is the user's name for the device only
    /// with the user-assigned-device-name entitlement; without it iOS returns
    /// the model, which is not a name anyone gave the device.
    static func userAssignedName(_ name: String, model: String) -> String? {
        name == model ? nil : bounded(name)
    }

    /// Drops names the wire cannot carry rather than failing the call.
    static func bounded(_ value: String?) -> String? {
        guard let value, !value.isEmpty, value.utf8.count <= DeviceWire.maxIdentifier else { return nil }

        return value
    }

    #if os(macOS)
    /// `IOPlatformUUID`, the hardware UUID macOS shows in System Information.
    static func platformUUID() -> String? {
        let service = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"))

        guard service != 0 else { return nil }
        defer { IOObjectRelease(service) }
        let property = IORegistryEntryCreateCFProperty(service, "IOPlatformUUID" as CFString, kCFAllocatorDefault, 0)

        return bounded(property?.takeRetainedValue() as? String)
    }
    #endif
}
