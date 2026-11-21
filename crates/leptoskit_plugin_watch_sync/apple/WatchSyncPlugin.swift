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
    private var incomingWaiter: CheckedContinuation<Data, Error>?
    private var statusWaiter: CheckedContinuation<Data, Error>?
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
            return Data()
        case 5:
            return try await watchStatus(last: data)
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        incomingWaiter?.resume(throwing: PluginError.cancelled)
        incomingWaiter = nil
        statusWaiter?.resume(throwing: PluginError.cancelled)
        statusWaiter = nil
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
            flags |= 2 | 8
            #endif
            if session.isReachable { flags |= 4 }
        }
        return encodeStatus(transport: supported ? 1 : 0, flags: flags, peerName: nil)
    }

    private func sendMessage(_ data: Data) async throws {
        guard WCSession.isSupported(), WCSession.default.isReachable else {
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
        guard WCSession.isSupported() else { throw PluginError.unavailable }
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

    private func nextIncoming() async throws -> Data {
        if !incoming.isEmpty {
            return incoming.removeFirst()
        }
        return try await withCheckedThrowingContinuation { continuation in
            incomingWaiter = continuation
        }
    }

    private func watchStatus(last: Data) async throws -> Data {
        let now = encodeCurrentStatus()
        if now != last { return now }
        return try await withCheckedThrowingContinuation { continuation in
            statusWaiter = continuation
        }
    }

    private func deliver(id: String, topic: String, payload: Data) {
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let encoded = encodeEnvelope(id: id, topic: topic, receivedMs: nowMs, payload: payload)
        if let waiter = incomingWaiter {
            incomingWaiter = nil
            waiter.resume(returning: encoded)
        } else {
            if incoming.count >= 64 { incoming.removeFirst() }
            incoming.append(encoded)
        }
    }

    private func notifyStatus() {
        if let waiter = statusWaiter {
            statusWaiter = nil
            waiter.resume(returning: encodeCurrentStatus())
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
    #endif

    public nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        Task { @MainActor in self.notifyStatus() }
    }

    public nonisolated func session(_ session: WCSession, didReceiveMessage message: [String: Any]) {
        let topic = (message["topic"] as? String) ?? "/message"
        let payload = (message["payload"] as? Data) ?? Data()
        Task { @MainActor in self.deliver(id: "", topic: topic, payload: payload) }
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
    private let bytes: [UInt8]
    private var pos = 0

    init(_ data: Data) { self.bytes = [UInt8](data) }

    mutating func u32() throws -> UInt32 {
        guard bytes.count - pos >= 4 else { throw PluginError.invalidArgument }
        let offset = pos
        pos += 4
        return bytes.withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        }
    }

    mutating func bytes() throws -> Data {
        let len = Int(try u32())
        guard len >= 0, bytes.count - pos >= len else { throw PluginError.invalidArgument }
        let start = pos
        pos += len
        return Data(bytes[start..<(start + len)])
    }

    mutating func string() throws -> String {
        let data = try bytes()
        guard let text = String(data: data, encoding: .utf8) else { throw PluginError.invalidArgument }
        return text
    }

    func finished() throws {
        guard pos == bytes.count else { throw PluginError.invalidArgument }
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
