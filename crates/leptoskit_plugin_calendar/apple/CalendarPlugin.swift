import EventKit
import Foundation
import LeptosKitBridge

/// Permission status plus bounded calendar and event access through EventKit.
///
/// Purpose strings stay app-owned (`NSCalendarsFullAccessUsageDescription` /
/// `NSCalendarsUsageDescription`). Denied access never invents calendars or
/// events.
@MainActor
public final class CalendarPlugin: NSObject, NativePlugin {
    public let name = "calendar"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5]
    private let context: NativePluginContext
    private let store = EKEventStore()
    private var closed = false
    private var busy = false

    public init(context: NativePluginContext) { self.context = context }

    public func close() {
        closed = true
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard let method = CalendarMethod(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .status:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return encode(status())
        case .requestPermission:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return try await requestAccess()
        case .listCalendars:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return try listCalendars()
        case .createEvent:
            let request = try payload.withUnsafeBytes(CalendarEventRecord.decodeCreate)

            return try createEvent(request)
        case .getEvent:
            let id = try payload.withUnsafeBytes(CalendarEventRecord.decodeEventID)

            return try getEvent(id: id)
        case .listEvents:
            let query = try payload.withUnsafeBytes(CalendarEventRecord.decodeListQuery)

            return try listEvents(query)
        }
    }

    private func status() -> CalendarStatus {
        // Raw values keep pre-iOS 17 `.authorized` (3) and iOS 17 `.fullAccess`
        // (3) as granted; `.writeOnly` (4) is incomplete for this MVP.
        switch EKEventStore.authorizationStatus(for: .event).rawValue {
        case 0: .prompt
        case 1: .restricted
        case 2: .denied
        case 3: .granted
        case 4: .restricted
        default: .unknown
        }
    }

    private func requestAccess() async throws -> Data {
        try requireUsageDescription()
        if EKEventStore.authorizationStatus(for: .event) != .notDetermined {
            return encode(status())
        }

        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }
        if #available(iOS 17.0, macOS 14.0, *) {
            _ = try await store.requestFullAccessToEvents()
        } else {
            _ = try await store.requestAccess(to: .event)
        }
        guard !closed else { throw PluginError.cancelled }

        return encode(status())
    }

    private func requireGranted() throws {
        guard status() == .granted else { throw PluginError.permissionDenied }

        try requireUsageDescription()
    }

    private func listCalendars() throws -> Data {
        try requireGranted()
        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }

        var calendars: [CalendarRecord] = []
        for calendar in store.calendars(for: .event) {
            if calendars.count >= CalendarRecord.maxCalendars { break }

            let title = calendar.title.trimmingCharacters(in: .whitespacesAndNewlines)
            let display = title.isEmpty ? calendar.calendarIdentifier : title
            if let record = try? CalendarRecord(
                id: calendar.calendarIdentifier,
                title: display,
                allowsModifications: calendar.allowsContentModifications
            ) {
                calendars.append(record)
            }
        }

        guard !closed else { throw PluginError.cancelled }

        return try CalendarRecord.encodeList(calendars)
    }

    private func createEvent(
        _ request: (
            calendarID: String,
            title: String,
            startMs: Int64,
            endMs: Int64,
            notes: String?,
            location: String?
        )
    ) throws -> Data {
        try requireGranted()
        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }

        guard let calendar = store.calendar(withIdentifier: request.calendarID) else {
            throw PluginError.invalidArgument
        }
        guard calendar.allowsContentModifications else { throw PluginError.permissionDenied }

        let event = EKEvent(eventStore: store)
        event.calendar = calendar
        event.title = request.title
        event.startDate = date(fromMs: request.startMs)
        event.endDate = date(fromMs: request.endMs)
        event.notes = request.notes
        event.location = request.location
        try store.save(event, span: .thisEvent, commit: true)
        guard !closed else { throw PluginError.cancelled }

        return try CalendarEventRecord.encodeEventID(event.eventIdentifier)
    }

    private func getEvent(id: String) throws -> Data {
        try requireGranted()
        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }

        guard let event = store.event(withIdentifier: id) else { throw PluginError.invalidArgument }
        guard !closed else { throw PluginError.cancelled }

        return try Self.record(from: event).encode()
    }

    private func listEvents(
        _ query: (startMs: Int64, endMs: Int64, limit: Int)
    ) throws -> Data {
        try requireGranted()
        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }

        let predicate = store.predicateForEvents(
            withStart: date(fromMs: query.startMs),
            end: date(fromMs: query.endMs),
            calendars: nil
        )
        var events: [CalendarEventRecord] = []
        for event in store.events(matching: predicate) {
            if events.count >= query.limit { break }

            if let record = try? Self.record(from: event) {
                events.append(record)
            }
        }

        guard !closed else { throw PluginError.cancelled }

        return try CalendarEventRecord.encodeList(events)
    }

    private func requireUsageDescription() throws {
        let full = Bundle.main.object(forInfoDictionaryKey: "NSCalendarsFullAccessUsageDescription") as? String
        let legacy = Bundle.main.object(forInfoDictionaryKey: "NSCalendarsUsageDescription") as? String
        let description = [full, legacy]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
        guard description != nil else { throw PluginError.invalidArgument }

        // Keep the context reference live so generated hosts that require a
        // presentation owner still construct this plugin honestly.
        _ = context
    }

    private func encode(_ status: CalendarStatus) -> Data {
        var raw = status.rawValue.littleEndian

        return withUnsafeBytes(of: &raw) { Data($0) }
    }

    private func date(fromMs value: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(value) / 1000.0)
    }

    fileprivate static func record(from event: EKEvent) throws -> CalendarEventRecord {
        guard let identifier = event.eventIdentifier else { throw PluginError.native }

        let title = event.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let display = title.isEmpty ? identifier : title
        let startMs = Int64((event.startDate.timeIntervalSince1970 * 1000.0).rounded())
        let endMs = Int64((event.endDate.timeIntervalSince1970 * 1000.0).rounded())
        let notes = event.notes?.trimmingCharacters(in: .whitespacesAndNewlines)
        let location = event.location?.trimmingCharacters(in: .whitespacesAndNewlines)

        return try CalendarEventRecord(
            id: identifier,
            calendarID: event.calendar.calendarIdentifier,
            title: display,
            startMs: startMs,
            endMs: max(endMs, startMs),
            notes: notes.flatMap { $0.isEmpty ? nil : $0 },
            location: location.flatMap { $0.isEmpty ? nil : $0 }
        )
    }
}
