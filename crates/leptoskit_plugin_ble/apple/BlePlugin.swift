import Foundation
import LeptosKitBridge
#if canImport(CoreBluetooth)
import CoreBluetooth
#endif

/// BLE GATT central plugin for iOS, watchOS, and macOS via `CoreBluetooth`.
@MainActor
public final class BlePlugin: NSObject, NativePlugin {
    public let name = "ble"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7]

    #if canImport(CoreBluetooth)
    private var central: CBCentralManager?
    private var discovered: [UUID: (CBPeripheral, Int, [String])] = [:]
    private var connected: [UUID: CBPeripheral] = [:]
    private var connectWaiters: [UUID: CheckedContinuation<Void, Error>] = [:]
    private var notifications: [Data] = []
    private var dropped: UInt32 = 0
    private var waiter: CheckedContinuation<Data, Error>?
    private var closed = false

    public override init() {
        super.init()
    }

    private func ensureCentral() -> CBCentralManager {
        if let central { return central }
        let manager = CBCentralManager(delegate: self, queue: .main)
        central = manager
        return manager
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        let data = payload.withUnsafeBytes { Data($0) }
        switch method {
        case 0, 1:
            let manager = ensureCentral()
            return encodeU32(mapState(manager.state))
        case 2:
            return try await scan(data)
        case 3:
            try await connect(id: try readSingleString(data))
            return Data()
        case 4:
            disconnect(id: try readSingleString(data))
            return Data()
        case 5:
            try setNotify(data, enabled: true)
            return Data()
        case 6:
            try setNotify(data, enabled: false)
            return Data()
        case 7:
            return try await nextNotifications()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        if let central {
            central.stopScan()
            for peripheral in connected.values {
                central.cancelPeripheralConnection(peripheral)
            }
        }
        connected.removeAll()
        waiter?.resume(throwing: PluginError.cancelled)
        waiter = nil
    }

    private func mapState(_ state: CBManagerState) -> UInt32 {
        switch state {
        case .poweredOn: 0
        case .poweredOff, .resetting, .unknown: 1
        case .unauthorized: 2
        case .unsupported: 3
        @unknown default: 3
        }
    }

    private func scan(_ data: Data) async throws -> Data {
        let manager = ensureCentral()
        guard manager.state == .poweredOn else { throw PluginError.unavailable }
        var reader = WireReader(data)
        let timeoutMs = min(max(try reader.u32(), 500), 30_000)
        let count = Int(try reader.u32())
        var filters: [CBUUID] = []
        for _ in 0..<count {
            filters.append(CBUUID(string: try reader.string()))
        }
        try reader.finished()

        discovered.removeAll()
        manager.scanForPeripherals(withServices: filters.isEmpty ? nil : filters, options: nil)
        try await Task.sleep(nanoseconds: UInt64(timeoutMs) * 1_000_000)
        manager.stopScan()
        return encodeDevices()
    }

    private func connect(id: String) async throws {
        let manager = ensureCentral()
        guard let uuid = UUID(uuidString: id),
              let peripheral = discovered[uuid]?.0 ?? manager.retrievePeripherals(withIdentifiers: [uuid]).first
        else { throw PluginError.invalidArgument }
        peripheral.delegate = self
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            connectWaiters[uuid] = continuation
            manager.connect(peripheral, options: nil)
        }
    }

    private func disconnect(id: String) {
        guard let uuid = UUID(uuidString: id), let peripheral = connected.removeValue(forKey: uuid) else { return }
        central?.cancelPeripheralConnection(peripheral)
    }

    private func setNotify(_ data: Data, enabled: Bool) throws {
        var reader = WireReader(data)
        let id = try reader.string()
        let serviceId = CBUUID(string: try reader.string())
        let charId = CBUUID(string: try reader.string())
        try reader.finished()
        guard let uuid = UUID(uuidString: id),
              let peripheral = connected[uuid],
              let service = peripheral.services?.first(where: { $0.uuid == serviceId }),
              let characteristic = service.characteristics?.first(where: { $0.uuid == charId })
        else { throw PluginError.unavailable }
        peripheral.setNotifyValue(enabled, for: characteristic)
    }

    private func nextNotifications() async throws -> Data {
        if !notifications.isEmpty || dropped > 0 { return drain() }
        return try await withCheckedThrowingContinuation { continuation in
            waiter = continuation
        }
    }

    private func enqueue(deviceId: String, service: String, charUuid: String, value: Data) {
        var out = Data()
        appendBytes(&out, Data(deviceId.utf8))
        appendBytes(&out, Data(service.utf8))
        appendBytes(&out, Data(charUuid.utf8))
        appendI64(&out, Int64(Date().timeIntervalSince1970 * 1000))
        appendBytes(&out, value)
        if notifications.count >= 128 {
            notifications.removeFirst()
            dropped &+= 1
        }
        notifications.append(out)
        if let pending = waiter {
            waiter = nil
            pending.resume(returning: drain())
        }
    }

    private func drain() -> Data {
        var out = Data()
        appendU32(&out, dropped)
        appendU32(&out, UInt32(notifications.count))
        dropped = 0
        for item in notifications { out.append(item) }
        notifications.removeAll()
        return out
    }

    private func encodeDevices() -> Data {
        var out = Data()
        appendU32(&out, UInt32(discovered.count))
        for (uuid, (peripheral, rssi, services)) in discovered {
            appendBytes(&out, Data(uuid.uuidString.utf8))
            if let name = peripheral.name {
                appendBytes(&out, Data(name.utf8))
            } else {
                appendU32(&out, UInt32.max)
            }
            appendI32(&out, Int32(rssi))
            appendU32(&out, UInt32(services.count))
            for service in services { appendBytes(&out, Data(service.utf8)) }
        }
        return out
    }

    private func readSingleString(_ data: Data) throws -> String {
        var reader = WireReader(data)
        let text = try reader.string()
        try reader.finished()
        return text
    }
    #else
    public override init() { super.init() }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        _ = payload.withUnsafeBytes { $0.count }
        if method == 0 { return encodeU32(3) }
        throw PluginError.unavailable
    }
    #endif
}

#if canImport(CoreBluetooth)
extension BlePlugin: CBCentralManagerDelegate, CBPeripheralDelegate {
    public nonisolated func centralManagerDidUpdateState(_ central: CBCentralManager) {}

    public nonisolated func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi RSSI: NSNumber
    ) {
        let services = (advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID])?
            .map { $0.uuidString.lowercased() } ?? []
        let rssi = RSSI.intValue
        Task { @MainActor in
            self.discovered[peripheral.identifier] = (peripheral, rssi, services)
        }
    }

    public nonisolated func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        peripheral.discoverServices(nil)
    }

    public nonisolated func centralManager(
        _ central: CBCentralManager,
        didFailToConnect peripheral: CBPeripheral,
        error: Error?
    ) {
        let id = peripheral.identifier
        Task { @MainActor in
            self.connectWaiters.removeValue(forKey: id)?.resume(throwing: PluginError.native)
        }
    }

    public nonisolated func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        let id = peripheral.identifier
        if error != nil {
            Task { @MainActor in
                self.connectWaiters.removeValue(forKey: id)?.resume(throwing: PluginError.native)
            }
            return
        }
        for service in peripheral.services ?? [] {
            peripheral.discoverCharacteristics(nil, for: service)
        }
        Task { @MainActor in
            self.connected[id] = peripheral
            self.connectWaiters.removeValue(forKey: id)?.resume()
        }
    }

    public nonisolated func peripheral(
        _ peripheral: CBPeripheral,
        didUpdateValueFor characteristic: CBCharacteristic,
        error: Error?
    ) {
        guard error == nil, let value = characteristic.value else { return }
        let devId = peripheral.identifier.uuidString
        let serviceId = characteristic.service?.uuid.uuidString.lowercased() ?? ""
        let charId = characteristic.uuid.uuidString.lowercased()
        Task { @MainActor in
            self.enqueue(deviceId: devId, service: serviceId, charUuid: charId, value: value)
        }
    }
}
#endif

private struct WireReader {
    private let bytes: [UInt8]
    private var pos = 0

    init(_ data: Data) { self.bytes = [UInt8](data) }

    mutating func u32() throws -> UInt32 {
        guard bytes.count - pos >= 4 else { throw PluginError.invalidArgument }
        defer { pos += 4 }
        return bytes.withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: pos, as: UInt32.self))
        }
    }

    mutating func string() throws -> String {
        let len = Int(try u32())
        guard len >= 0, bytes.count - pos >= len else { throw PluginError.invalidArgument }
        defer { pos += len }
        guard let text = String(bytes: bytes[pos..<(pos + len)], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        return text
    }

    func finished() throws {
        guard pos == bytes.count else { throw PluginError.invalidArgument }
    }
}

private func encodeU32(_ value: UInt32) -> Data {
    var out = Data()
    appendU32(&out, value)
    return out
}

private func appendBytes(_ out: inout Data, _ data: Data) {
    appendU32(&out, UInt32(data.count))
    out.append(data)
}

private func appendU32(_ out: inout Data, _ value: UInt32) {
    var encoded = value.littleEndian
    withUnsafeBytes(of: &encoded) { out.append(contentsOf: $0) }
}

private func appendI32(_ out: inout Data, _ value: Int32) {
    var encoded = value.littleEndian
    withUnsafeBytes(of: &encoded) { out.append(contentsOf: $0) }
}

private func appendI64(_ out: inout Data, _ value: Int64) {
    var encoded = value.littleEndian
    withUnsafeBytes(of: &encoded) { out.append(contentsOf: $0) }
}
