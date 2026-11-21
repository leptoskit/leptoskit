import Foundation
import LeptosKitBridge
#if canImport(WatchConnectivity) && (os(iOS) || os(watchOS))
import WatchConnectivity
#endif

/// Bidirectional iPhone <-> Apple Watch companion transport backed by
/// `WatchConnectivity` (`WCSession`).
@MainActor
public final class WatchSyncPlugin: NSObject, NativePlugin {
    public let name = "watch-sync"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5]

    #if canImport(WatchConnectivity) && (os(iOS) || os(watchOS))
    private var incoming: [Data] = []
    private var unackedIds: Set<String> = []
    private var incomingWaiters: [CheckedContinuation<Data, Error>] = []
    private var statusWaiters: [CheckedContinuation<Data, Error>] = []
    private var closed = false

    public override init() {
        super.init()
        if WCSession.isSupported() {
            let session = WCSession.default
            session.delegate = self
            session.activate()
        }
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        let data = payload.withUnsafeBytes { Data($0) }
        switch method {
        case 0: return encodeCurrentStatus()
        case 1:
            try await sendMessage(data)
            return Data()
        case 2:
            try queueTransfer(data)
            return Data()
        case 3:
            return try await nextIncoming()
        case 4:
            try ackTransfer(data)
            return Data()
        case 5:
            return try await watchStatus(last: data)
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        for waiter in incomingWaiters { waiter.resume(throwing: PluginError.cancelled) }
        incomingWaiters.removeAll()
        for waiter in statusWaiters { waiter.resume(throwing: PluginError.cancelled) }
        statusWaiters.removeAll()
    }

    private func encodeCurrentStatus() -> Data {
        let supported = WCSession.isSupported()
        var flags: UInt32 = supported ? 1 : 0
        if supported {
            let session = WCSession.default
            #if os(iOS)
            if session.isPaired { flags |= 2 }
            if session.isWatchAppInstalled { flags |= 8 }
            #else
            flags |= 2
            if session.isCompanionAppInstalled { flags |= 8 }
            #endif
            if session.isReachable { flags |= 4 }
        }
        return encodeStatus(transport: supported ? 1 : 0, flags: flags, peerName: nil)
    }

    private func sendMessage(_ data: Data) async throws {
        guard WCSession.isSupported(),
              WCSession.default.activationState == .activated,
              WCSession.default.isReachable
        else {
            throw PluginError.unavailable
        }
        var reader = WireReader(data)
        let topic = try reader.string()
        let payload = try reader.bytes()
        try reader.finished()
        let message: [String: Any] = ["topic": topic, "payload": payload]
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            WCSession.default.sendMessage(message, replyHandler: { _ in
                continuation.resume()
            }, errorHandler: { _ in
                continuation.resume(throwing: PluginError.native)
            })
        }
    }

    private func queueTransfer(_ data: Data) throws {
        guard WCSession.isSupported(), WCSession.default.activationState == .activated else {
            throw PluginError.unavailable
        }
        var reader = WireReader(data)
        let id = try reader.string()
        let topic = try reader.string()
        let payload = try reader.bytes()
        try reader.finished()
        WCSession.default.transferUserInfo([
            "id": id,
            "topic": topic,
            "payload": payload
        ])
    }

    private func ackTransfer(_ data: Data) throws {
        var reader = WireReader(data)
        let id = try reader.string()
        try reader.finished()
        unackedIds.remove(id)
    }

    private func nextIncoming() async throws -> Data {
        if !incoming.isEmpty {
            return incoming.removeFirst()
        }
        return try await withCheckedThrowingContinuation { continuation in
            incomingWaiters.append(continuation)
        }
    }

    private func watchStatus(last: Data) async throws -> Data {
        let now = encodeCurrentStatus()
        if now != last { return now }
        return try await withCheckedThrowingContinuation { continuation in
            statusWaiters.append(continuation)
        }
    }

    private func deliver(id: String, topic: String, payload: Data) {
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let encoded = encodeEnvelope(id: id, topic: topic, receivedMs: nowMs, payload: payload)
        if !id.isEmpty { unackedIds.insert(id) }
        if !incomingWaiters.isEmpty {
            let waiter = incomingWaiters.removeFirst()
            waiter.resume(returning: encoded)
        } else {
            if incoming.count >= 64 { incoming.removeFirst() }
            incoming.append(encoded)
        }
    }

    private func notifyStatus() {
        guard !statusWaiters.isEmpty else { return }
        let current = encodeCurrentStatus()
        let waiters = statusWaiters
        statusWaiters.removeAll()
        for waiter in waiters {
            waiter.resume(returning: current)
        }
    }
    #else
    public override init() { super.init() }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        _ = payload.withUnsafeBytes { $0.count }
        if method == 0 {
            return encodeStatus(transport: 0, flags: 0, peerName: nil)
        }
        throw PluginError.unavailable
    }
    #endif
}

#if canImport(WatchConnectivity) && (os(iOS) || os(watchOS))
extension WatchSyncPlugin: WCSessionDelegate {
    public nonisolated func session(
        _ session: WCSession,
        activationDidCompleteWith activationState: WCSessionActivationState,
        error: Error?
    ) {
        Task { @MainActor in self.notifyStatus() }
    }

    #if os(iOS)
    public nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}
    public nonisolated func sessionDidDeactivate(_ session: WCSession) {
        session.activate()
    }
    public nonisolated func sessionWatchStateDidChange(_ session: WCSession) {
        Task { @MainActor in self.notifyStatus() }
    }
    #else
    public nonisolated func sessionCompanionAppInstalledDidChange(_ session: WCSession) {
        Task { @MainActor in self.notifyStatus() }
    }
    #endif

    public nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        Task { @MainActor in self.notifyStatus() }
    }

    public nonisolated func session(_ session: WCSession, didReceiveMessage message: [String: Any]) {
        let topic = (message["topic"] as? String) ?? "/message"
        let payload = (message["payload"] as? Data) ?? Data()
        Task { @MainActor in self.deliver(id: "", topic: topic, payload: payload) }
    }

    public nonisolated func session(
        _ session: WCSession,
        didReceiveMessage message: [String: Any],
        replyHandler: @escaping ([String: Any]) -> Void
    ) {
        let topic = (message["topic"] as? String) ?? "/message"
        let payload = (message["payload"] as? Data) ?? Data()
        Task { @MainActor in self.deliver(id: "", topic: topic, payload: payload) }
        replyHandler([:])
    }

    public nonisolated func session(_ session: WCSession, didReceiveUserInfo userInfo: [String: Any]) {
        let id = (userInfo["id"] as? String) ?? ""
        let topic = (userInfo["topic"] as? String) ?? "/transfer"
        let payload = (userInfo["payload"] as? Data) ?? Data()
        Task { @MainActor in self.deliver(id: id, topic: topic, payload: payload) }
    }
}
#endif

private struct WireReader {
    private let buffer: [UInt8]
    private var pos = 0

    init(_ data: Data) { self.buffer = [UInt8](data) }

    mutating func u32() throws -> UInt32 {
        guard buffer.count - pos >= 4 else { throw PluginError.invalidArgument }
        let offset = pos
        pos += 4
        return buffer.withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        }
    }

    mutating func bytes() throws -> Data {
        let len = Int(try u32())
        guard len >= 0, buffer.count - pos >= len else { throw PluginError.invalidArgument }
        let start = pos
        pos += len
        return Data(buffer[start..<(start + len)])
    }

    mutating func string() throws -> String {
        let data = try bytes()
        guard let text = String(data: data, encoding: .utf8) else { throw PluginError.invalidArgument }
        return text
    }

    func finished() throws {
        guard pos == buffer.count else { throw PluginError.invalidArgument }
    }
}

private func encodeStatus(transport: UInt32, flags: UInt32, peerName: String?) -> Data {
    var out = Data()
    appendU32(&out, transport)
    appendU32(&out, flags)
    if let peerName {
        appendBytes(&out, Data(peerName.utf8))
    } else {
        appendU32(&out, UInt32.max)
    }
    return out
}

private func encodeEnvelope(id: String, topic: String, receivedMs: Int64, payload: Data) -> Data {
    var out = Data()
    appendBytes(&out, Data(id.utf8))
    appendBytes(&out, Data(topic.utf8))
    appendI64(&out, receivedMs)
    appendBytes(&out, payload)
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

private func appendI64(_ out: inout Data, _ value: Int64) {
    var encoded = value.littleEndian
    withUnsafeBytes(of: &encoded) { out.append(contentsOf: $0) }
}
