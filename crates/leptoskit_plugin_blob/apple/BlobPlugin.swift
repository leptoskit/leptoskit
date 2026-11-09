import Foundation
import LeptosKitBridge

@MainActor
public final class BlobPlugin: NativePlugin {
    public let name = "blob"
    public let methods: [UInt32] = [0, 1, 2, 3]
    private var closed = false
    private var nextId: UInt32 = 0
    private var entries: [UInt32: Data] = [:]
    private let maxLive = 256

    public init(context: NativePluginContext) {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            try BlobWire.emptyPayload(payload)
            return BlobWire.capability(.available)
        case 1:
            let bytes = try BlobWire.create(payload)
            guard entries.count < maxLive else { throw PluginError.busy }
            guard nextId < UInt32.max else { throw PluginError.exhausted }
            nextId += 1
            entries[nextId] = bytes
            return BlobWire.encodeId(nextId)
        case 2:
            let (id, offset, length) = try BlobWire.read(payload)
            guard let entry = entries[id] else { throw PluginError.invalidArgument }
            let start = Int(offset)
            if start >= entry.count {
                return try BlobWire.encodeSlice(Data())
            }
            let end = min(start + Int(length), entry.count)
            return try BlobWire.encodeSlice(entry.subdata(in: start ..< end))
        case 3:
            let id = try BlobWire.id(payload)
            guard entries.removeValue(forKey: id) != nil else { throw PluginError.invalidArgument }
            return Data()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() { closed = true }
}
