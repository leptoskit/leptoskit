import Darwin
import Foundation
import LeptosKitBridge
import Network
#if os(iOS)
import NetworkExtension
#elseif os(macOS)
import CoreWLAN
#endif

/// One address an interface carries, as `getifaddrs` reports it.
struct InterfaceAddress: Equatable {
    enum Family: Equatable { case ipv4, ipv6 }

    let interface: String
    let family: Family
    let address: [UInt8]
    let netmask: [UInt8]?
    /// Present only when the interface sets `IFF_BROADCAST`.
    let broadcast: [UInt8]?
}

/// What the system reveals about the associated Wi-Fi network.
struct WifiAssociation: Equatable, Sendable {
    /// Raw SSID bytes; iOS hands over a decoded string, sent here as UTF-8.
    var ssid: Data?
    /// The BSSID text the system reports, such as `a4:2b:b0:3:4:5`.
    var bssid: String?
}

/// The system calls the plugin reads, so tests can substitute fixed values.
@MainActor
protocol NetworkInfoSystem {
    /// The Wi-Fi interface's BSD name, such as `en0`, when Wi-Fi is up.
    func wifiInterface() async -> String?
    /// The associated network; empty without the entitlement or location authorization.
    func association() async -> WifiAssociation
    func addresses() -> [InterfaceAddress]
    /// A `NET_RT_FLAGS` sysctl dump of IPv4 routes through a gateway.
    func routeDump() -> Data?
    func interfaceIndex(_ name: String) -> UInt32
}

/// Reads the Wi-Fi name, BSSID, and the Wi-Fi interface's addresses. It never
/// asks for permission and reports nothing the system withholds.
@MainActor
public final class NetworkInfoPlugin: NativePlugin {
    public let name = "network-info"
    public let methods: [UInt32] = [NetworkInfoWire.snapshotMethod]
    private let system: NetworkInfoSystem
    private var closed = false

    public convenience init() {
        self.init(system: LiveNetworkInfoSystem())
    }

    init(system: NetworkInfoSystem) {
        self.system = system
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method == NetworkInfoWire.snapshotMethod else { throw PluginError.unavailable }

        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return try await snapshot()
    }

    public func close() {
        closed = true
    }

    /// Reads every field; a close or cancellation during the reads wins.
    func snapshot() async throws -> Data {
        let interface = await system.wifiInterface()
        let association = await system.association()

        guard !closed else { throw PluginError.cancelled }

        try Task.checkCancellation()
        let gateway = interface.flatMap { name in
            system.routeDump().flatMap {
                NetworkInfoWire.defaultGateway(in: $0, interfaceIndex: system.interfaceIndex(name))
            }
        }

        return NetworkInfoWire.reading(
            interface: interface,
            association: association,
            addresses: interface == nil ? [] : system.addresses(),
            gateway: gateway
        )
    }
}

/// The live system: `NWPathMonitor` for the Wi-Fi interface, `NEHotspotNetwork`
/// on iOS or `CoreWLAN` on macOS for the association, and BSD calls for addresses.
@MainActor
struct LiveNetworkInfoSystem: NetworkInfoSystem {
    func wifiInterface() async -> String? {
        await withCheckedContinuation { continuation in
            let monitor = NWPathMonitor(requiredInterfaceType: .wifi)
            let once = Once()
            monitor.pathUpdateHandler = { path in
                guard once.claim() else { return }

                monitor.cancel()
                continuation.resume(returning: path.availableInterfaces.first { $0.type == .wifi }?.name)
            }
            monitor.start(queue: DispatchQueue(label: "dev.leptoskit.network-info"))
        }
    }

    func association() async -> WifiAssociation {
        await currentAssociation()
    }

    func addresses() -> [InterfaceAddress] {
        var list: UnsafeMutablePointer<ifaddrs>?

        guard getifaddrs(&list) == 0, let first = list else { return [] }

        defer { freeifaddrs(list) }

        return sequence(first: first, next: { $0.pointee.ifa_next }).compactMap { InterfaceAddress($0.pointee) }
    }

    func routeDump() -> Data? {
        var mib: [Int32] = [CTL_NET, PF_ROUTE, 0, AF_INET, NetworkInfoWire.netRouteFlags, NetworkInfoWire.routeGateway]

        // The table can grow between sizing and reading, so retry a few times.
        for _ in 0..<3 {
            var length = 0

            guard sysctl(&mib, UInt32(mib.count), nil, &length, nil, 0) == 0 else { return nil }

            var buffer = [UInt8](repeating: 0, count: length)

            if sysctl(&mib, UInt32(mib.count), &buffer, &length, nil, 0) == 0 {
                return Data(buffer.prefix(length))
            }

            guard errno == ENOMEM else { return nil }
        }

        return nil
    }

    func interfaceIndex(_ name: String) -> UInt32 {
        if_nametoindex(name)
    }
}

/// Lets exactly one of several racing callbacks act.
private final class Once: @unchecked Sendable {
    private let lock = NSLock()
    private var claimed = false

    func claim() -> Bool {
        lock.withLock {
            defer { claimed = true }

            return !claimed
        }
    }
}

extension InterfaceAddress {
    /// Reads one `ifaddrs` entry, skipping families other than IPv4 and IPv6.
    init?(_ entry: ifaddrs) {
        guard let address = entry.ifa_addr else { return nil }

        let family: Family
        let length: Int
        let offset: Int

        switch Int32(address.pointee.sa_family) {
        case AF_INET:
            (family, length, offset) = (.ipv4, 4, MemoryLayout.offset(of: \sockaddr_in.sin_addr) ?? 4)
        case AF_INET6:
            (family, length, offset) = (.ipv6, 16, MemoryLayout.offset(of: \sockaddr_in6.sin6_addr) ?? 8)
        default:
            return nil
        }

        func bytes(_ sockaddr: UnsafeMutablePointer<sockaddr>?) -> [UInt8]? {
            guard let sockaddr, Int(sockaddr.pointee.sa_len) >= offset + length else { return nil }

            let raw = UnsafeRawPointer(sockaddr) + offset
            return Array(UnsafeRawBufferPointer(start: raw, count: length))
        }

        let broadcasts = entry.ifa_flags & UInt32(IFF_BROADCAST) != 0
        self.init(
            interface: String(cString: entry.ifa_name),
            family: family,
            address: bytes(address) ?? [],
            netmask: bytes(entry.ifa_netmask),
            broadcast: broadcasts && family == .ipv4 ? bytes(entry.ifa_dstaddr) : nil
        )
    }
}

#if os(iOS)
/// Needs the Access Wi-Fi Information entitlement and location authorization
/// (or an app-configured hotspot or VPN); otherwise iOS answers nothing.
@MainActor
private func currentAssociation() async -> WifiAssociation {
    await withCheckedContinuation { continuation in
        NEHotspotNetwork.fetchCurrent { network in
            continuation.resume(
                returning: WifiAssociation(ssid: network.map { Data($0.ssid.utf8) }, bssid: network?.bssid)
            )
        }
    }
}
#elseif os(macOS)
/// macOS withholds the SSID and BSSID until the app has location authorization.
@MainActor
private func currentAssociation() async -> WifiAssociation {
    let interface = CWWiFiClient.shared().interface()
    return WifiAssociation(ssid: interface?.ssidData(), bssid: interface?.bssid())
}
#else
@MainActor
private func currentAssociation() async -> WifiAssociation {
    WifiAssociation()
}
#endif
