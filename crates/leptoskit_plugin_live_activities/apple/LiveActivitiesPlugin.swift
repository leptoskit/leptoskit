import Foundation
import LeptosKitBridge

#if canImport(ActivityKit) && os(iOS)
@preconcurrency import ActivityKit
#endif

/// Injectable ActivityKit surface so tests never start system Live Activities.
@MainActor
protocol LiveActivityService: AnyObject {
    var areEnabled: Bool { get }
    var frequentUpdatesEnabled: Bool { get }
    func start(
        id: String,
        kind: String,
        title: String,
        fields: [String: String],
        pushUpdates: Bool
    ) async throws
    func startScheduled(
        id: String,
        kind: String,
        title: String,
        fields: [String: String],
        pushUpdates: Bool,
        staleAfterSeconds: UInt32?,
        relevanceScore: Double
    ) async throws
    func update(id: String, fields: [String: String]) async throws
    func updateScheduled(
        id: String,
        fields: [String: String],
        staleAfterSeconds: UInt32?,
        relevanceScore: Double
    ) async throws
    func end(id: String, fields: [String: String]?, dismissal: DismissalKind, afterSeconds: UInt32) async throws
    func waitPushToken(id: String) async throws -> Data
    func waitPushToStartToken() async throws -> Data
    func list() -> [String]
}

extension LiveActivityService {
    func startScheduled(
        id: String,
        kind: String,
        title: String,
        fields: [String: String],
        pushUpdates: Bool,
        staleAfterSeconds: UInt32?,
        relevanceScore: Double
    ) async throws {
        _ = staleAfterSeconds
        _ = relevanceScore
        try await start(id: id, kind: kind, title: title, fields: fields, pushUpdates: pushUpdates)
    }

    func updateScheduled(
        id: String,
        fields: [String: String],
        staleAfterSeconds: UInt32?,
        relevanceScore: Double
    ) async throws {
        _ = staleAfterSeconds
        _ = relevanceScore
        try await update(id: id, fields: fields)
    }

    func waitPushToStartToken() async throws -> Data {
        throw PluginError.unavailable
    }
}

enum DismissalKind: UInt32 {
    case `default` = 0
    case immediate = 1
    case after = 2
}

#if canImport(ActivityKit) && os(iOS)
@available(iOS 16.1, *)
@MainActor
private final class SystemLiveActivityService: LiveActivityService {
    private var tokens: [String: Task<Void, Never>] = [:]
    private var latestTokens: [String: Data] = [:]
    private var waiters: [String: CheckedContinuation<Data, any Error>] = [:]

    var areEnabled: Bool { ActivityAuthorizationInfo().areActivitiesEnabled }
    var frequentUpdatesEnabled: Bool { ActivityAuthorizationInfo().frequentPushesEnabled }

    func start(
        id: String,
        kind: String,
        title: String,
        fields: [String: String],
        pushUpdates: Bool
    ) async throws {
        try await startScheduled(
            id: id,
            kind: kind,
            title: title,
            fields: fields,
            pushUpdates: pushUpdates,
            staleAfterSeconds: nil,
            relevanceScore: 0.0
        )
    }

    func startScheduled(
        id: String,
        kind: String,
        title: String,
        fields: [String: String],
        pushUpdates: Bool,
        staleAfterSeconds: UInt32?,
        relevanceScore: Double
    ) async throws {
        // Replace any prior ActivityKit activity with this app id so update/end
        // stay unambiguous and we never orphan a previous request.
        if let existing = activity(id: id) {
            await existing.end(nil, dismissalPolicy: .immediate)
            stopObserving(id: id)
        }

        let attributes = LeptosKitLiveActivityAttributes(id: id, kind: kind, title: title)
        let state = LeptosKitLiveActivityAttributes.ContentState(fields: fields)
        let staleDate = staleAfterSeconds.map { Date().addingTimeInterval(TimeInterval($0)) }
        let content = ActivityContent(
            state: state,
            staleDate: staleDate,
            relevanceScore: relevanceScore
        )
        let activity: Activity<LeptosKitLiveActivityAttributes>

        if pushUpdates {
            activity = try Activity.request(attributes: attributes, content: content, pushType: .token)
            observePushToken(id: id, activity: activity)
        } else {
            activity = try Activity.request(attributes: attributes, content: content, pushType: nil)
        }

        _ = activity
    }

    func update(id: String, fields: [String: String]) async throws {
        try await updateScheduled(
            id: id,
            fields: fields,
            staleAfterSeconds: nil,
            relevanceScore: 0.0
        )
    }

    func updateScheduled(
        id: String,
        fields: [String: String],
        staleAfterSeconds: UInt32?,
        relevanceScore: Double
    ) async throws {
        guard let activity = activity(id: id) else { throw PluginError.unavailable }
        let state = LeptosKitLiveActivityAttributes.ContentState(fields: fields)
        let staleDate = staleAfterSeconds.map { Date().addingTimeInterval(TimeInterval($0)) }
        await activity.update(
            ActivityContent(state: state, staleDate: staleDate, relevanceScore: relevanceScore)
        )
    }

    func end(
        id: String,
        fields: [String: String]?,
        dismissal: DismissalKind,
        afterSeconds: UInt32
    ) async throws {
        guard let activity = activity(id: id) else { throw PluginError.unavailable }
        let content: ActivityContent<LeptosKitLiveActivityAttributes.ContentState>?

        if let fields {
            content = ActivityContent(
                state: LeptosKitLiveActivityAttributes.ContentState(fields: fields),
                staleDate: nil
            )
        } else {
            content = nil
        }

        let policy: ActivityUIDismissalPolicy
        switch dismissal {
        case .default: policy = .default
        case .immediate: policy = .immediate
        case .after:
            policy = .after(.now + TimeInterval(afterSeconds))
        }
        await activity.end(content, dismissalPolicy: policy)
        stopObserving(id: id)
    }

    func waitPushToken(id: String) async throws -> Data {
        if let existing = latestTokens.removeValue(forKey: id) { return existing }

        guard activity(id: id) != nil else { throw PluginError.unavailable }

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Data, any Error>) in

                if let displaced = waiters.updateValue(continuation, forKey: id) {
                    displaced.resume(throwing: PluginError.cancelled)
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in

                guard let self else { return }

                if let waiter = self.waiters.removeValue(forKey: id) {
                    waiter.resume(throwing: PluginError.cancelled)
                }
            }
        }
    }

    func waitPushToStartToken() async throws -> Data {
        guard #available(iOS 17.2, *) else { throw PluginError.unavailable }
        for await token in Activity<LeptosKitLiveActivityAttributes>.pushToStartTokenUpdates {
            try Task.checkCancellation()
            return Data(token)
        }
        throw PluginError.unavailable
    }

    func list() -> [String] {
        Activity<LeptosKitLiveActivityAttributes>.activities.map(\.attributes.id)
    }

    private func activity(id: String) -> Activity<LeptosKitLiveActivityAttributes>? {
        Activity<LeptosKitLiveActivityAttributes>.activities.first { $0.attributes.id == id }
    }

    private func observePushToken(id: String, activity: Activity<LeptosKitLiveActivityAttributes>) {
        stopObserving(id: id)
        tokens[id] = Task { @MainActor in

            for await token in activity.pushTokenUpdates {
                if let waiter = waiters.removeValue(forKey: id) {
                    waiter.resume(returning: Data(token))
                } else {
                    latestTokens[id] = Data(token)
                }
            }
        }
    }

    private func stopObserving(id: String) {
        tokens.removeValue(forKey: id)?.cancel()

        if let waiter = waiters.removeValue(forKey: id) {
            waiter.resume(throwing: PluginError.cancelled)
        }

        latestTokens.removeValue(forKey: id)
    }
}
#endif

/// Starts, updates, and ends iOS Live Activities through ActivityKit.
@MainActor
public final class LiveActivitiesPlugin: NativePlugin {
    public let name = "live-activities"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    private let service: (any LiveActivityService)?
    private var closed = false
    private var busy = Set<String>()

    public convenience init() {
        #if canImport(ActivityKit) && os(iOS)

        if #available(iOS 16.1, *) {
            self.init(service: SystemLiveActivityService())

            return
        }

        #endif
        self.init(service: nil)
    }

    init(service: (any LiveActivityService)?) {
        self.service = service
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()

        guard methods.contains(method) else { throw PluginError.unavailable }

        if method == 10 {
            _ = try payload.withUnsafeBytes { try AndroidLiveUpdateMessage($0) }
            throw PluginError.unavailable
        }
        if method == 11 {
            _ = try payload.withUnsafeBytes { bytes -> String in
                var reader = try Reader(bytes, maximum: 68)
                let id = try reader.identifier()
                try reader.finish()
                return id
            }
            throw PluginError.unavailable
        }

        guard let service else { throw PluginError.unavailable }

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return flag(service.areEnabled)
        case 1:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return flag(service.frequentUpdatesEnabled)
        case 2:
            let request = try payload.withUnsafeBytes { try StartMessage($0) }
            try await gated(request.id) {
                try await service.start(
                    id: request.id,
                    kind: request.kind,
                    title: request.title,
                    fields: request.fields,
                    pushUpdates: request.pushUpdates
                )
            }
            return try encodeIdentifier(request.id)
        case 3:
            let request = try payload.withUnsafeBytes { try UpdateMessage($0) }
            try await gated(request.id) {
                try await service.update(id: request.id, fields: request.fields)
            }
            return Data()
        case 4:
            let request = try payload.withUnsafeBytes { try EndMessage($0) }
            try await gated(request.id) {
                try await service.end(
                    id: request.id,
                    fields: request.fields,
                    dismissal: request.dismissal,
                    afterSeconds: request.afterSeconds
                )
            }
            return Data()
        case 5:
            let id = try payload.withUnsafeBytes { bytes -> String in
                var reader = try Reader(bytes, maximum: 68)
                let id = try reader.identifier()
                try reader.finish()
                return id
            }
            // Waiting for a push token must not hold the per-id mutation lock;
            // apps still need to update or end the activity while waiting.
            guard !closed else { throw PluginError.cancelled }
            try Task.checkCancellation()
            let token: Data

            do {
                token = try await service.waitPushToken(id: id)
            } catch is CancellationError {
                throw PluginError.cancelled
            } catch let error as PluginError {
                throw error
            } catch {
                throw PluginError.native
            }

            guard !token.isEmpty, token.count <= 512 else { throw PluginError.invalidArgument }
            return encodeBytes(token)
        case 6:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try encodeList(service.list())
        case 7:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            guard !closed else { throw PluginError.cancelled }
            try Task.checkCancellation()
            let token: Data

            do {
                token = try await service.waitPushToStartToken()
            } catch is CancellationError {
                throw PluginError.cancelled
            } catch let error as PluginError {
                throw error
            } catch {
                throw PluginError.native
            }

            guard !token.isEmpty, token.count <= 512 else { throw PluginError.invalidArgument }
            return encodeBytes(token)
        case 8:
            let request = try payload.withUnsafeBytes { try ScheduledStartMessage($0) }
            try await gated(request.start.id) {
                try await service.startScheduled(
                    id: request.start.id,
                    kind: request.start.kind,
                    title: request.start.title,
                    fields: request.start.fields,
                    pushUpdates: request.start.pushUpdates,
                    staleAfterSeconds: request.schedule.staleAfterSeconds,
                    relevanceScore: request.schedule.relevanceScore
                )
            }
            return try encodeIdentifier(request.start.id)
        case 9:
            let request = try payload.withUnsafeBytes { try ScheduledUpdateMessage($0) }
            try await gated(request.update.id) {
                try await service.updateScheduled(
                    id: request.update.id,
                    fields: request.update.fields,
                    staleAfterSeconds: request.schedule.staleAfterSeconds,
                    relevanceScore: request.schedule.relevanceScore
                )
            }
            return Data()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        busy.removeAll()
    }

    private func gated<T>(_ id: String, body: () async throws -> T) async throws -> T {
        guard busy.insert(id).inserted else { throw PluginError.busy }
        defer { busy.remove(id) }

        guard !closed else { throw PluginError.cancelled }
        try Task.checkCancellation()
        do {
            return try await body()
        } catch is CancellationError {
            throw PluginError.cancelled
        } catch let error as PluginError {
            throw error
        } catch {
            throw PluginError.native
        }
    }

    private func flag(_ value: Bool) -> Data {
        Data([value ? 1 : 0, 0, 0, 0])
    }

    private func encodeIdentifier(_ id: String) throws -> Data {
        var data = Data()
        try append(id, to: &data)

        return data
    }

    private func encodeBytes(_ bytes: Data) -> Data {
        var data = Data()
        var length = UInt32(bytes.count).littleEndian
        withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
        data.append(bytes)

        return data
    }

    private func encodeList(_ ids: [String]) throws -> Data {
        guard ids.count <= 64 else { throw PluginError.invalidArgument }
        var data = Data()
        var count = UInt32(ids.count).littleEndian
        withUnsafeBytes(of: &count) { data.append(contentsOf: $0) }

        for id in ids { try append(id, to: &data) }

        return data
    }

    private func append(_ value: String, to data: inout Data) throws {
        let utf8 = Array(value.utf8)
        var length = UInt32(utf8.count).littleEndian
        withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
        data.append(contentsOf: utf8)
    }
}

private struct StartMessage {
    let id: String
    let kind: String
    let title: String
    let pushUpdates: Bool
    let fields: [String: String]

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4096)
        try self.init(from: &reader)
        try reader.finish()
    }

    init(from reader: inout Reader) throws {
        id = try reader.identifier()
        kind = try reader.identifier()
        title = try reader.text(maximum: 128, required: true)
        switch try reader.u32() {
        case 0: pushUpdates = false
        case 1: pushUpdates = true
        default: throw PluginError.invalidArgument
        }
        fields = try reader.fields()
    }
}

private struct UpdateMessage {
    let id: String
    let fields: [String: String]

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4096)
        try self.init(from: &reader)
        try reader.finish()
    }

    init(from reader: inout Reader) throws {
        id = try reader.identifier()
        fields = try reader.fields()
    }
}

private struct ScheduleMessage {
    let staleAfterSeconds: UInt32?
    let relevanceScore: Double
    let allowPushToStart: Bool

    init(from reader: inout Reader) throws {
        guard try reader.u32() == 1 else { throw PluginError.invalidArgument }
        let hasStale = try reader.u32()
        let seconds = try reader.u32()
        switch hasStale {
        case 0:
            guard seconds == 0 else { throw PluginError.invalidArgument }
            staleAfterSeconds = nil
        case 1:
            guard (1...86_400).contains(seconds) else { throw PluginError.invalidArgument }
            staleAfterSeconds = seconds
        default:
            throw PluginError.invalidArgument
        }
        let score = Float(bitPattern: try reader.u32())
        guard score.isFinite, score >= 0.0, score <= 1.0 else {
            throw PluginError.invalidArgument
        }
        relevanceScore = Double(score)
        switch try reader.u32() {
        case 0: allowPushToStart = false
        case 1: allowPushToStart = true
        default: throw PluginError.invalidArgument
        }
    }
}

private struct ScheduledStartMessage {
    let start: StartMessage
    let schedule: ScheduleMessage

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4200)
        start = try StartMessage(from: &reader)
        schedule = try ScheduleMessage(from: &reader)
        try reader.finish()
    }
}

private struct ScheduledUpdateMessage {
    let update: UpdateMessage
    let schedule: ScheduleMessage

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4200)
        update = try UpdateMessage(from: &reader)
        schedule = try ScheduleMessage(from: &reader)
        try reader.finish()
    }
}

private struct AndroidLiveUpdateMessage {
    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4096)
        _ = try reader.identifier()
        guard try reader.u32() == 1 else { throw PluginError.invalidArgument }
        _ = try reader.text(maximum: 128, required: true)
        _ = try reader.text(maximum: 256, required: false)
        _ = try reader.text(maximum: 32, required: false)
        let progress = Float(bitPattern: try reader.u32())
        guard progress.isFinite, progress >= 0.0, progress <= 1.0 else {
            throw PluginError.invalidArgument
        }
        let segmentCount = Int(try reader.u32())
        guard segmentCount <= 8 else { throw PluginError.tooLarge }
        for _ in 0..<segmentCount {
            let length = try reader.u32()
            guard (1...10_000).contains(length) else { throw PluginError.invalidArgument }
            switch try reader.u32() {
            case 0: break
            case 1:
                let hex = try reader.text(maximum: 7, required: true)
                guard isValidHexColor(hex) else { throw PluginError.invalidArgument }
            default:
                throw PluginError.invalidArgument
            }
        }
        try reader.finish()
    }
}

private func isValidHexColor(_ value: String) -> Bool {
    let utf8 = Array(value.utf8)
    guard utf8.count == 7, utf8[0] == 35 else { return false }
    return utf8[1...].allSatisfy {
        (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
    }
}

private struct EndMessage {
    let id: String
    let dismissal: DismissalKind
    let afterSeconds: UInt32
    let fields: [String: String]?

    init(_ bytes: UnsafeRawBufferPointer) throws {
        var reader = try Reader(bytes, maximum: 4096)
        id = try reader.identifier()
        let kind = try reader.u32()
        afterSeconds = try reader.u32()
        switch kind {
        case 0:
            guard afterSeconds == 0 else { throw PluginError.invalidArgument }
            dismissal = .default
        case 1:
            guard afterSeconds == 0 else { throw PluginError.invalidArgument }
            dismissal = .immediate
        case 2:
            guard (1...86_400).contains(afterSeconds) else { throw PluginError.invalidArgument }
            dismissal = .after
        default:
            throw PluginError.invalidArgument
        }
        switch try reader.u32() {
        case 0: fields = nil
        case 1: fields = try reader.fields()
        default: throw PluginError.invalidArgument
        }
        try reader.finish()
    }
}

private struct Reader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0

    init(_ bytes: UnsafeRawBufferPointer, maximum: Int) throws {
        guard bytes.count <= maximum else { throw PluginError.invalidArgument }
        self.bytes = bytes
    }

    mutating func u32() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let value = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        offset += 4

        return value
    }

    mutating func identifier() throws -> String {
        let value = try text(maximum: 64, required: true)
        guard value.utf8.allSatisfy({
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
                || [45, 46, 95].contains($0)
        }) else { throw PluginError.invalidArgument }

        return value
    }

    mutating func text(maximum: Int, required: Bool) throws -> String {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        let length = Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
        offset += 4

        guard length <= maximum, length <= bytes.count - offset,
              let value = String(bytes: bytes[offset..<(offset + length)], encoding: .utf8)
        else { throw PluginError.invalidArgument }
        offset += length

        guard !required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,

              !value.unicodeScalars.contains(where: { $0.value <= 31 || (127...159).contains($0.value) })
        else { throw PluginError.invalidArgument }

        return value
    }

    mutating func fields() throws -> [String: String] {
        let count = Int(try u32())

        guard count <= 16 else { throw PluginError.invalidArgument }
        var fields: [String: String] = [:]

        for _ in 0..<count {
            let key = try identifier()
            let value = try text(maximum: 256, required: false)

            if fields.updateValue(value, forKey: key) != nil {
                throw PluginError.invalidArgument
            }
        }

        return fields
    }

    func finish() throws {
        guard offset == bytes.count else { throw PluginError.invalidArgument }
    }
}
