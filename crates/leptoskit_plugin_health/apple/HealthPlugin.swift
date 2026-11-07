import Foundation
import LeptosKitBridge
#if canImport(HealthKit) && os(iOS)
import HealthKit
#endif

/// Reads HealthKit samples. Authorization is HealthKit's own sheet; denied
/// reads are indistinguishable from never asked, so grants are not exact.
@MainActor
public final class HealthPlugin: NativePlugin {
    public let name = "health"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6]

    #if os(iOS)
    private let store = HKHealthStore()
    private var closed = false
    private var queries: [ObjectIdentifier: HKQuery] = [:]

    public init() {}

    /// Answers availability, features, grants, permission, reads and changes.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }
        guard HKHealthStore.isHealthDataAvailable() else {
            if method == Method.availability { return encodeAvailability(Self.unsupported) }
            throw PluginError.unavailable
        }
        return try await answer(method, data: payload.data())
    }

    /// Dispatches a call the device's health store can answer.
    private func answer(_ method: UInt32, data: Data) async throws -> Data {
        switch method {
        case Method.availability: encodeAvailability(Self.available)
        case Method.features: encodeMask(HealthWire.appleTypes, flags: 0)
        case Method.granted: encodeGrants()
        case Method.request: try await request(TypesRequest(data))
        case Method.read: try await read(ReadRequest(data))
        case Method.changesToken: try await changesToken(TypesRequest(data))
        default: try await changes(try token(data))
        }
    }

    public func close() {
        closed = true
        for query in queries.values {
            store.stop(query)
        }
        queries.removeAll()
    }

    /// What the user allowed, as far as HealthKit says; reads it hides look
    /// like ones never asked for, so `exact` stays false.
    private func encodeGrants() -> Data {
        var mask: UInt32 = 0
        for code in Self.supportedCodes where store.authorizationStatus(for: objectType(code)) == .sharingAuthorized {
            mask |= 1 << UInt32(code)
        }
        return encodeMask(mask, flags: HealthWire.flagExact)
    }

    /// Shows HealthKit's sheet for the requested types and reports the state
    /// it left behind.
    private func request(_ request: TypesRequest) async throws -> Data {
        try await store.requestAuthorization(toShare: [], read: Set(request.types.map(objectType)))
        if closed { throw PluginError.cancelled }
        return encodeGrants()
    }

    /// One page of one type's samples, oldest first; the token carries how
    /// many samples earlier pages already delivered, because HealthKit has no
    /// page cursors.
    private func read(_ request: ReadRequest) async throws -> Data {
        let offset = try request.token.map(offset(of:)) ?? 0
        let predicate = HKQuery.predicateForSamples(
            withStart: date(request.startMs),
            end: date(request.endMs),
            options: [.strictStartDate, .strictEndDate]
        )
        let sort = [
            NSSortDescriptor(key: HKSampleSortIdentifierStartDate, ascending: true),
            NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: true)
        ]
        let found = try await sampleQuery(
            type: sampleType(request.type),
            predicate: predicate,
            limit: offset + request.pageSize,
            sort: sort
        )
        let page = found.dropFirst(offset)
        let next = page.count == request.pageSize ? String(offset + page.count) : nil
        return try encodePage(page.map(sample(of:)), token: next)
    }

    /// An anchor per requested type, serialised into one opaque token. The
    /// anchors are taken against a window that matches nothing; if HealthKit
    /// still replays history, the client receives it as upserts and stays
    /// correct.
    private func changesToken(_ request: TypesRequest) async throws -> Data {
        let predicate = HKQuery.predicateForSamples(
            withStart: Date(timeIntervalSinceNow: 1),
            end: Date(timeIntervalSinceNow: 2),
            options: [.strictStartDate, .strictEndDate]
        )
        var parts = ["1"]
        for code in request.types {
            let result = try await anchoredQuery(type: sampleType(code), predicate: predicate, anchor: nil)
            guard let anchor = result.anchor else { throw PluginError.native }
            parts.append("\(code):\(try Self.encode(anchor))")
        }
        var writer = WireWriter()
        try writer.string(parts.joined(separator: ";"))
        return writer.bytes()
    }

    /// The samples written and deleted after the token's anchors. Deleted
    /// samples only exist for sleep and workouts; other types report none.
    private func changes(_ token: String) async throws -> Data {
        var anchors = try Self.anchors(of: token)
        var reply = ChangesReply()
        for (code, anchor) in anchors.sorted(by: { $0.key < $1.key }) {
            let result = try await anchoredQuery(type: sampleType(code), predicate: nil, anchor: anchor)
            reply.upserted.append(contentsOf: result.samples.map(sample(of:)))
            reply.deleted.append(contentsOf: result.deleted.map(\.uuid.uuidString))
            if let next = result.anchor { anchors[code] = next }
        }
        reply.nextToken = Self.token(of: anchors)
        return try reply.encode()
    }

    // MARK: HealthKit plumbing

    /// The type codes the plugin reads.
    private static let supportedCodes = [
        HealthWire.heartRate, HealthWire.restingHeartRate, HealthWire.hrvSdnn, HealthWire.steps,
        HealthWire.sleep, HealthWire.oxygenSaturation, HealthWire.respiratoryRate, HealthWire.exercise
    ]

    private static let available = 0
    private static let unsupported = 3

    /// One query to stop when its caller goes away.
    private final class QueryHolder: @unchecked Sendable {
        var query: HKQuery?
    }

    /// Runs one sample query to completion; caller cancellation stops it.
    private func sampleQuery(
        type: HKSampleType,
        predicate: NSPredicate?,
        limit: Int,
        sort: [NSSortDescriptor]
    ) async throws -> [HKSample] {
        let holder = QueryHolder()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let query = HKSampleQuery(
                    sampleType: type,
                    predicate: predicate,
                    limit: limit,
                    sortDescriptors: sort
                ) { [weak self] query, samples, error in
                    Task { @MainActor in self?.forget(query) }
                    if error != nil {
                        continuation.resume(throwing: PluginError.native)
                    } else {
                        continuation.resume(returning: samples ?? [])
                    }
                }
                holder.query = query
                remember(query)
                store.execute(query)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.stop(holder.query) }
        }
    }

    /// Runs one anchored query to completion; caller cancellation stops it.
    private func anchoredQuery(
        type: HKSampleType,
        predicate: NSPredicate?,
        anchor: HKQueryAnchor?
    ) async throws -> AnchoredResult {
        let holder = QueryHolder()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let query = HKAnchoredObjectQuery(
                    type: type,
                    predicate: predicate,
                    anchor: anchor,
                    limit: HKObjectQueryNoLimit
                ) { [weak self] query, samples, deleted, anchor, error in
                    Task { @MainActor in self?.forget(query) }
                    if error != nil {
                        continuation.resume(throwing: PluginError.native)
                    } else {
                        continuation.resume(
                            returning: AnchoredResult(
                                samples: samples ?? [],
                                deleted: deleted ?? [],
                                anchor: anchor
                            )
                        )
                    }
                }
                holder.query = query
                remember(query)
                store.execute(query)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.stop(holder.query) }
        }
    }

    private func remember(_ query: HKQuery) {
        queries[ObjectIdentifier(query)] = query
    }

    private func forget(_ query: HKQuery) {
        queries.removeValue(forKey: ObjectIdentifier(query))
    }

    private func stop(_ query: HKQuery?) {
        guard let query else { return }
        store.stop(query)
        forget(query)
    }

    private func date(_ milliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(milliseconds) / 1000)
    }

    /// A read page token counts the samples already delivered. The token comes
    /// from the guest, so it is bounded: `offset + pageSize` must not overflow,
    /// which would trap and take the host app down.
    private func offset(of token: String) throws -> Int {
        guard let offset = Int(token), offset >= 0, offset <= Int.max - ReadRequest.maxPageSize else {
            throw PluginError.invalidArgument
        }
        return offset
    }

    /// The changes token the guest stored, without its framing.
    private func token(_ data: Data) throws -> String {
        var reader = WireReader(data)
        let value = try reader.string()
        try reader.finished()
        guard !value.isEmpty, value.utf8.count <= 64 * 1024 else { throw PluginError.invalidArgument }
        return value
    }

    private func objectType(_ code: Int) -> HKObjectType {
        if code == HealthWire.exercise { return HKObjectType.workoutType() }
        if code == HealthWire.sleep, let type = HKObjectType.categoryType(forIdentifier: .sleepAnalysis) {
            return type
        }
        guard let type = HKObjectType.quantityType(forIdentifier: quantityIdentifier(code)) else {
            preconditionFailure("unsupported health type code \(code)")
        }
        return type
    }

    private func sampleType(_ code: Int) -> HKSampleType {
        guard let type = objectType(code) as? HKSampleType else {
            preconditionFailure("unsupported health type code \(code)")
        }
        return type
    }

    private func quantityIdentifier(_ code: Int) -> HKQuantityTypeIdentifier {
        switch code {
        case HealthWire.heartRate: .heartRate
        case HealthWire.restingHeartRate: .restingHeartRate
        case HealthWire.hrvSdnn: .heartRateVariabilitySDNN
        case HealthWire.steps: .stepCount
        case HealthWire.oxygenSaturation: .oxygenSaturation
        case HealthWire.respiratoryRate: .respiratoryRate
        default: preconditionFailure("unsupported health type code \(code)")
        }
    }
    #else
    public init() {}

    /// macOS has no health store: availability reports unsupported and every
    /// other call fails without prompting.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard methods.contains(method) else { throw PluginError.unavailable }
        _ = payload.data()
        if method == Method.availability { return encodeAvailability(3) }
        throw PluginError.unavailable
    }
    #endif
}

#if os(iOS)
#if os(iOS)
private extension HealthPlugin {
    /// Serialises an anchor so the token the guest stores is plain text.
    static func encode(_ anchor: HKQueryAnchor) throws -> String {
        try NSKeyedArchiver
            .archivedData(withRootObject: anchor, requiringSecureCoding: true)
            .base64EncodedString()
    }

    /// Parses a token's per-type anchors; an unreadable one is caller error.
    static func anchors(of token: String) throws -> [Int: HKQueryAnchor] {
        var anchors: [Int: HKQueryAnchor] = [:]
        for part in token.components(separatedBy: ";").dropFirst() {
            let fields = part.split(separator: ":", maxSplits: 1).map(String.init)
            guard fields.count == 2,
                let code = Int(fields[0]),
                supportedCodes.contains(code),
                let data = Data(base64Encoded: fields[1]),
                let anchor = try? NSKeyedUnarchiver.unarchivedObject(ofClass: HKQueryAnchor.self, from: data)
            else { throw PluginError.invalidArgument }
            anchors[code] = anchor
        }
        guard !anchors.isEmpty else { throw PluginError.invalidArgument }
        return anchors
    }

    /// Rebuilds the opaque token around fresh anchors.
    static func token(of anchors: [Int: HKQueryAnchor]) -> String? {
        let parts = anchors
            .sorted(by: { $0.key < $1.key })
            .compactMap { code, anchor in
                (try? encode(anchor)).map { "\(code):\($0)" }
            }
        return parts.isEmpty ? nil : "1;" + parts.joined(separator: ";")
    }
}
#endif

/// What one anchored query observed: upserts, deletions and where to continue.
struct AnchoredResult {
    let samples: [HKSample]
    let deleted: [HKDeletedObject]
    let anchor: HKQueryAnchor?
}
#endif

extension NativeBuffer {
    /// The whole request payload as `Data`.
    func data() -> Data {
        withUnsafeBytes { Data($0) }
    }
}

/// Native method codes, in wire order.
private enum Method {
    static let availability: UInt32 = 0
    static let features: UInt32 = 1
    static let granted: UInt32 = 2
    static let request: UInt32 = 3
    static let read: UInt32 = 4
    static let changesToken: UInt32 = 5
}
