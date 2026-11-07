import Foundation

/// The 80-byte snapshot the Rust crate decodes. Absent fields stay all zero,
/// and nothing the system withholds is replaced by a placeholder.
enum NetworkInfoWire {
    static let snapshotMethod: UInt32 = 0
    static let snapshotBytes = 80
    static let maxSsidBytes = 32

    /// `NET_RT_FLAGS` and `RTF_GATEWAY`, which the iOS SDK headers omit.
    static let netRouteFlags: Int32 = 2
    static let routeGateway: Int32 = 0x2

    /// Byte offsets of each field.
    private enum Offset {
        static let ssidLength = 4
        static let ssid = 8
        static let bssid = 40
        static let ipv4 = 48
        static let ipv6 = 52
        static let subnetMask = 68
        static let broadcast = 72
        static let gateway = 76
    }

    /// Presence bits, one per field.
    private enum Present {
        static let ssid: UInt32 = 1 << 0
        static let bssid: UInt32 = 1 << 1
        static let ipv4: UInt32 = 1 << 2
        static let ipv6: UInt32 = 1 << 3
        static let subnetMask: UInt32 = 1 << 4
        static let broadcast: UInt32 = 1 << 5
        static let gateway: UInt32 = 1 << 6
    }

    /// Assembles the reading for the Wi-Fi interface's addresses and the association.
    static func reading(
        interface: String?,
        association: WifiAssociation,
        addresses: [InterfaceAddress],
        gateway: [UInt8]?
    ) -> Data {
        let own = addresses.filter { $0.interface == interface && !isLoopback($0) }
        let ipv4 = preferred(own.filter { $0.family == .ipv4 && $0.address.count == 4 && !isZero($0.address) })
        let ipv6 = preferred(own.filter { $0.family == .ipv6 && $0.address.count == 16 && !isZero($0.address) })

        return encode(
            Fields(
                ssid: ssid(association.ssid),
                bssid: bssid(association.bssid),
                ipv4: ipv4?.address,
                ipv6: ipv6?.address,
                subnetMask: ipv4?.netmask.flatMap(mask),
                broadcast: ipv4?.broadcast.flatMap(nonzero),
                gateway: gateway.flatMap(nonzero)
            )
        )
    }

    /// Every optional field; each must already be valid or nil.
    struct Fields {
        var ssid: Data?
        var bssid: [UInt8]?
        var ipv4: [UInt8]?
        var ipv6: [UInt8]?
        var subnetMask: [UInt8]?
        var broadcast: [UInt8]?
        var gateway: [UInt8]?
    }

    /// One field's presence bit, offset and bytes.
    private struct Slot {
        let bit: UInt32
        let offset: Int
        let bytes: [UInt8]?
    }

    static func encode(_ fields: Fields) -> Data {
        var output = Data(count: snapshotBytes)
        var presence: UInt32 = 0
        let slots = [
            Slot(bit: Present.ssid, offset: Offset.ssid, bytes: fields.ssid.map { Array($0) }),
            Slot(bit: Present.bssid, offset: Offset.bssid, bytes: fields.bssid),
            Slot(bit: Present.ipv4, offset: Offset.ipv4, bytes: fields.ipv4),
            Slot(bit: Present.ipv6, offset: Offset.ipv6, bytes: fields.ipv6),
            Slot(bit: Present.subnetMask, offset: Offset.subnetMask, bytes: fields.subnetMask),
            Slot(bit: Present.broadcast, offset: Offset.broadcast, bytes: fields.broadcast),
            Slot(bit: Present.gateway, offset: Offset.gateway, bytes: fields.gateway)
        ]

        for slot in slots {
            guard let bytes = slot.bytes else { continue }

            presence |= slot.bit
            output.replaceSubrange(slot.offset..<slot.offset + bytes.count, with: bytes)
        }

        write(presence, into: &output, at: 0)
        write(UInt32(fields.ssid?.count ?? 0), into: &output, at: Offset.ssidLength)
        return output
    }

    /// An SSID is 1 to 32 bytes; anything else is treated as withheld.
    static func ssid(_ raw: Data?) -> Data? {
        guard let raw, (1...maxSsidBytes).contains(raw.count) else { return nil }

        return raw
    }

    /// Parses `aa:bb:cc:dd:ee:ff`, allowing Apple's single-digit components,
    /// and refuses the zero, redacted, and group addresses.
    static func bssid(_ text: String?) -> [UInt8]? {
        guard let parts = text?.split(separator: ":", omittingEmptySubsequences: false), parts.count == 6 else {
            return nil
        }

        let octets = parts.compactMap { part in (1...2).contains(part.count) ? UInt8(part, radix: 16) : nil }

        guard octets.count == 6, !isZero(octets), octets != [2, 0, 0, 0, 0, 0], octets[0] & 1 == 0 else {
            return nil
        }

        return octets
    }

    /// A contiguous, non-empty IPv4 mask, or nil.
    static func mask(_ bytes: [UInt8]) -> [UInt8]? {
        guard bytes.count == 4 else { return nil }

        let bits = bytes.reduce(UInt32(0)) { $0 << 8 | UInt32($1) }

        guard bits != 0, (~bits).leadingZeroBitCount + bits.trailingZeroBitCount == 32 else { return nil }

        return bytes
    }

    /// Finds the IPv4 default route leaving through `interfaceIndex`, scoped
    /// or not, and returns its gateway.
    static func defaultGateway(in dump: Data, interfaceIndex: UInt32) -> [UInt8]? {
        let bytes = [UInt8](dump)
        var start = 0

        while start + RouteMessage.headerBytes <= bytes.count {
            let length = Int(bytes[start]) | Int(bytes[start + 1]) << 8

            guard length >= RouteMessage.headerBytes, start + length <= bytes.count else { return nil }

            let message = RouteMessage(bytes: Array(bytes[start..<start + length]))

            if message.index == interfaceIndex, let gateway = message.defaultGateway() { return gateway }

            start += length
        }

        return nil
    }

    private static func preferred(_ candidates: [InterfaceAddress]) -> InterfaceAddress? {
        candidates.first { !isLinkLocal($0) } ?? candidates.first
    }

    private static func isLinkLocal(_ entry: InterfaceAddress) -> Bool {
        switch entry.family {
        case .ipv4: entry.address.starts(with: [169, 254])
        case .ipv6: entry.address[0] == 0xFE && entry.address[1] & 0xC0 == 0x80
        }
    }

    private static func isLoopback(_ entry: InterfaceAddress) -> Bool {
        switch entry.family {
        case .ipv4: entry.address.first == 127
        case .ipv6: entry.address == [UInt8](repeating: 0, count: 15) + [1]
        }
    }

    private static func isZero(_ bytes: [UInt8]) -> Bool {
        bytes.allSatisfy { $0 == 0 }
    }

    private static func nonzero(_ bytes: [UInt8]) -> [UInt8]? {
        bytes.count == 4 && !isZero(bytes) ? bytes : nil
    }

    private static func write(_ value: UInt32, into data: inout Data, at offset: Int) {
        withUnsafeBytes(of: value.littleEndian) { data.replaceSubrange(offset..<offset + 4, with: $0) }
    }
}

/// One Darwin `rt_msghdr` message and the socket addresses after it, read by
/// offset in host byte order.
private struct RouteMessage {
    static let headerBytes = 92
    private static let upAndGateway: UInt32 = 0x3
    private static let addressSlots = 8
    private static let inet: UInt8 = 2

    let bytes: [UInt8]

    var index: UInt32 { UInt32(bytes[4]) | UInt32(bytes[5]) << 8 }

    private func word(_ offset: Int) -> UInt32 {
        (0..<4).reduce(UInt32(0)) { $0 | UInt32(bytes[offset + $1]) << (8 * $1) }
    }

    /// The gateway when this is an up IPv4 route to 0.0.0.0/0.
    func defaultGateway() -> [UInt8]? {
        guard word(8) & Self.upAndGateway == Self.upAndGateway else { return nil }

        let present = word(12)
        var slots = [ArraySlice<UInt8>?](repeating: nil, count: Self.addressSlots)
        var offset = Self.headerBytes

        for slot in 0..<Self.addressSlots where present & (1 << slot) != 0 {
            guard offset < bytes.count else { return nil }

            let length = Int(bytes[offset])

            guard offset + max(length, 1) <= bytes.count else { return nil }

            slots[slot] = bytes[offset..<offset + max(length, 1)]
            // Darwin pads each address to four bytes; an empty one still takes four.
            offset += length == 0 ? 4 : (length + 3) & ~3
        }

        guard let destination = slots[0].flatMap(inet), let gateway = slots[1].flatMap(inet),
              destination.allSatisfy({ $0 == 0 }),
              slots[2].map({ $0.dropFirst(4).allSatisfy { $0 == 0 } }) ?? true
        else { return nil }

        return gateway
    }

    private func inet(_ sockaddr: ArraySlice<UInt8>) -> [UInt8]? {
        let start = sockaddr.startIndex

        guard sockaddr.count >= 8, sockaddr[start + 1] == Self.inet else { return nil }

        return Array(sockaddr[start + 4..<start + 8])
    }
}
