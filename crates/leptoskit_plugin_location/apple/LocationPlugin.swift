import CoreLocation
import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// swiftlint:disable file_length
// The plugin keeps its location manager delegate, bridge methods, and wire
// validation in one file because the bridge resolves the plugin by name.

/// Bounds the wire record enforces.
private enum Wire {
    /// One record is five u32 fields and eight doubles.
    static let recordBytes = 84
    /// One update request is a u32 accuracy, a distance double, a u32
    /// interval and three u32 flags.
    static let updateOptionsBytes = 28
    /// Fixes a native update buffer keeps before dropping the oldest.
    static let updateBufferCapacity = 64
    /// Coordinates are real degrees on the sphere.
    static let maxLatitude: Double = 90
    static let maxLongitude: Double = 180
    /// A course is a clockwise angle that stays under a full turn.
    static let fullTurn: Double = 360
}

@MainActor
// swiftlint:disable:next type_body_length
public final class LocationPlugin: NSObject, NativePlugin, @preconcurrency CLLocationManagerDelegate {
    public let name = "location"
    public let methods: [UInt32] = [0, 1, 2, 3]
    /// A permission prompt gets the longest timeout the Rust crate accepts.
    private static let promptTimeoutMilliseconds: UInt32 = 120_000
    /// The granted status code the permission reply carries.
    private static let granted: UInt32 = 3
    private let manager = CLLocationManager()
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    private var pendingID: UUID?
    private var authorizationRequest: AuthorizationRequest?
    private var options: LocationOptions?
    private var preciseAtStart = false
    private var started = 0.0
    private var timeout: Task<Void, Never>?
    private var backgroundObserver: NSObjectProtocol?
    private var updating = false
    private var visiting = false
    private var significant = false
    private var updateWaiter: CheckedContinuation<Data, any Error>?
    private var updateWaiterID: UUID?
    private var updateBuffer: [Data] = []
    private var updateDropped: UInt32 = 0

    /// The wire methods the Rust crate calls.
    private enum Method: UInt32 {
        case permission = 0
        case requestPermission = 1
        case current = 2
        case next = 3
        case requestBackgroundPermission = 4
        case startUpdates = 5
        case stopUpdates = 6
        case nextUpdates = 7
        case startVisits = 8
        case startSignificantChanges = 9
    }

    /// Which authorization a pending request is waiting for.
    private enum AuthorizationRequest {
        case whenInUse
        case always
    }

    public override init() {
        super.init()
        manager.delegate = self
    }

    /// Answers one call: a permission query never prompts, and a fix request needs
    /// access the OS already granted.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch Method(rawValue: method) {
        case .permission: return try permissionReply(payload)
        case .requestPermission: return try await requestAccess(payload)
        case .current: return try await locate(payload, hasPrevious: false)
        case .next: return try await locate(payload, hasPrevious: true)
        case .requestBackgroundPermission: return try await requestBackgroundAccess(payload)
        case .startUpdates: return try startUpdatesCall(payload)
        case .stopUpdates: return try stopUpdatesCall(payload)
        case .nextUpdates: return try await nextUpdatesCall(payload)
        case .startVisits, .startSignificantChanges:
            #if os(iOS)
            return try startMonitoringCall(method, payload: payload)
            #else
            // Visit and significant-change monitoring exist only on iOS.
            throw PluginError.unavailable
            #endif
        case nil: throw PluginError.unavailable
        }
    }

    /// Reports the current scope without prompting.
    private func permissionReply(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return permission()
    }

    /// Asks the OS once for "When In Use", or reports the scope when it has
    /// already answered.
    private func requestAccess(_ payload: NativeBuffer) async throws -> Data {
        try payload.withUnsafeBytes { bytes in

            guard bytes.count == 4, UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) <= 1 else {
                throw PluginError.invalidArgument
            }
        }

        guard manager.authorizationStatus == .notDetermined else { return permission() }

        try requireUsageDescription()
        try requireForeground()

        return try await wait(options: nil, request: .whenInUse)
    }

    /// Upgrades "When In Use" to "Always", the separate step iOS requires.
    private func requestBackgroundAccess(_ payload: NativeBuffer) async throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        #if os(iOS)
        // "Always" can only follow the foreground grant.
        guard manager.authorizationStatus == .authorizedAlways ||
                manager.authorizationStatus == .authorizedWhenInUse else {
            throw PluginError.invalidArgument
        }
        #else
        // macOS has a single always grant; nothing to upgrade.
        guard manager.authorizationStatus == .authorizedAlways else { throw PluginError.invalidArgument }
        #endif

        guard manager.authorizationStatus != .authorizedAlways else { return backgroundScope() }

        try requireAlwaysUsageDescription()
        try requireForeground()

        return try await wait(options: nil, request: .always)
    }

    /// Starts continuous updates into the bounded buffer.
    private func startUpdatesCall(_ payload: NativeBuffer) throws -> Data {
        let options = try payload.withUnsafeBytes { try UpdateOptions(bytes: $0) }

        guard pending == nil else { throw PluginError.busy }
        guard isAuthorized else { throw PluginError.permissionDenied }

        try requireUsageDescription()

        #if os(iOS)
        if options.background {
            // Background updates need the "Always" grant and the location
            // background mode, which the plugin registry adds.
            guard manager.authorizationStatus == .authorizedAlways else { throw PluginError.permissionDenied }
        }
        #endif

        switch options.accuracy {
        case .navigation: manager.desiredAccuracy = kCLLocationAccuracyBestForNavigation
        case .balanced: manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
        case .best: manager.desiredAccuracy = kCLLocationAccuracyBest
        }

        manager.distanceFilter =
            options.distanceFilterM <= 0 ? kCLDistanceFilterNone : CLLocationDistance(options.distanceFilterM)

        #if os(iOS)
        manager.pausesLocationUpdatesAutomatically = options.pausesAutomatically
        manager.allowsBackgroundLocationUpdates = options.background
        manager.showsBackgroundLocationIndicator = options.showBackgroundIndicator
        #endif

        manager.startUpdatingLocation()
        updating = true

        return Data()
    }

    /// Stops every running update source. Buffered fixes stay buffered.
    private func stopUpdatesCall(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        if updating {
            manager.stopUpdatingLocation()
            updating = false
        }

        #if os(iOS)
        if visiting {
            manager.stopMonitoringVisits()
            visiting = false
        }

        if significant {
            manager.stopMonitoringSignificantLocationChanges()
            significant = false
        }
        #endif

        // A waiter that can never see another fix gets what is buffered, or
        // the stream's end.
        answerUpdateWaiter()

        return Data()
    }

    /// Delivers the buffered batch, waits while a source runs, or ends it.
    private func nextUpdatesCall(_ payload: NativeBuffer) async throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        guard updateBuffer.isEmpty && sourceRunning else { return deliverBatch() }
        guard updateWaiter == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                updateWaiter = continuation
                updateWaiterID = identifier
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.cancelUpdateWaiter(identifier) }
        }
    }

    #if os(iOS)
    /// Starts one of the iOS-only monitoring sources that feed the buffer.
    private func startMonitoringCall(_ method: UInt32, payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }
        guard isAuthorized else { throw PluginError.permissionDenied }

        try requireUsageDescription()

        if method == Method.startVisits.rawValue {
            manager.startMonitoringVisits()
            visiting = true
        } else {
            manager.startMonitoringSignificantLocationChanges()
            significant = true
        }

        return Data()
    }
    #endif

    /// Whether any source still feeds the update buffer.
    private var sourceRunning: Bool {
        updating || visiting || significant
    }

    /// Buffers the fixes a source reported, dropping the oldest when full.
    private func bufferLocations(_ locations: [CLLocation]) {
        for location in locations {
            guard let record = try? fixRecord(location) else { continue }

            updateBuffer.append(record)

            if updateBuffer.count > Wire.updateBufferCapacity {
                updateBuffer.removeFirst()
                updateDropped &+= 1
            }
        }
    }

    /// Hands a waiting update caller whatever is buffered, or the stream's end.
    private func answerUpdateWaiter() {
        guard let waiter = updateWaiter else { return }

        updateWaiter = nil
        updateWaiterID = nil
        waiter.resume(returning: deliverBatch())
    }

    /// Abandons the waiter a cancelled caller left behind, if it still owns
    /// the slot.
    private func cancelUpdateWaiter(_ identifier: UUID) {
        guard updateWaiterID == identifier else { return }

        updateWaiter = nil
        updateWaiterID = nil
    }

    /// Answers one batch: the drops since the previous one, then every
    /// buffered fix.
    private func deliverBatch() -> Data {
        let reply = batch(dropped: updateDropped, records: updateBuffer)
        updateBuffer.removeAll()
        updateDropped = 0

        return reply
    }

    /// Reports the background grant's own scope: granted only under Always.
    private func backgroundScope() -> Data {
        let always = manager.authorizationStatus == .authorizedAlways
        let status: UInt32 = always ? Self.granted : 2
        // Accuracy is meaningful only to a granted scope; elsewhere unknown.
        let accuracy: UInt32 = always ? (manager.accuracyAuthorization == .fullAccuracy ? 2 : 1) : 0

        return permissionData(status: status, accuracy: accuracy)
    }

    /// Frames one permission reply from its two little-endian fields.
    private func permissionData(status: UInt32, accuracy: UInt32) -> Data {
        var result = Data()

        for var field in [status.littleEndian, accuracy.littleEndian] {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }

        return result
    }

    /// Waits for a bounded fix the caller's options describe.
    private func locate(_ payload: NativeBuffer, hasPrevious: Bool) async throws -> Data {
        let options = try payload.withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: hasPrevious) }

        guard isAuthorized else { throw PluginError.permissionDenied }

        try requireUsageDescription()
        try requireForeground()

        return try await wait(options: options, request: nil)
    }

    #if os(iOS)
    /// A prompt needs the app in the foreground, where iOS will show it.
    private func requireForeground() throws {
        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }
    }
    #elseif os(macOS)
    /// A prompt needs the app in the foreground, where macOS will show it.
    private func requireForeground() throws {
        guard NSApplication.shared.isActive else { throw PluginError.cancelled }
    }
    #endif

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
        manager.delegate = nil

        if updating {
            manager.stopUpdatingLocation()
            updating = false
        }

        #if os(iOS)
        if visiting {
            manager.stopMonitoringVisits()
            visiting = false
        }

        if significant {
            manager.stopMonitoringSignificantLocationChanges()
            significant = false
        }
        #endif

        if let waiter = updateWaiter {
            updateWaiter = nil
            updateWaiterID = nil
            waiter.resume(throwing: PluginError.cancelled)
        }
    }

    private var isAuthorized: Bool {
        #if os(macOS)
        manager.authorizationStatus == .authorizedAlways
        #else
        manager.authorizationStatus == .authorizedAlways || manager.authorizationStatus == .authorizedWhenInUse
        #endif
    }

    /// The usage description Apple requires before it shows the permission prompt.
    private func requireUsageDescription() throws {
        #if os(macOS)
        let key = "NSLocationUsageDescription"
        #else
        let key = "NSLocationWhenInUseUsageDescription"
        #endif

        guard let description = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.invalidArgument
        }
    }

    /// The usage description the "Always" upgrade prompt needs.
    private func requireAlwaysUsageDescription() throws {
        #if os(macOS)
        // macOS has a single location usage description and an always grant.
        try requireUsageDescription()
        #else
        let key = "NSLocationAlwaysAndWhenInUseUsageDescription"
        guard let description = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.invalidArgument
        }
        #endif
    }

    private func wait(options: LocationOptions?, request: AuthorizationRequest?) async throws -> Data {
        guard pending == nil, !sourceRunning else { throw PluginError.busy }

        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                begin(continuation, options: options, request: request, identifier: identifier)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.cancelIfCurrent(identifier) }
        }
    }

    /// Registers the waiter and starts the acquisition it belongs to.
    private func begin(
        _ continuation: CheckedContinuation<Data, any Error>,
        options: LocationOptions?,
        request: AuthorizationRequest?,
        identifier: UUID
    ) {
        pending = continuation
        pendingID = identifier
        authorizationRequest = request
        self.options = options
        preciseAtStart = manager.accuracyAuthorization == .fullAccuracy

        if Task.isCancelled {
            finish(.failure(CancellationError()))

            return
        }

        started = Date().timeIntervalSince1970 * 1000
        scheduleTimeout(
            milliseconds: options?.timeout ?? Self.promptTimeoutMilliseconds,
            identifier: identifier
        )
        observeBackground()

        if let options {
            startUpdates(options)
        } else if request == .always {
            manager.requestAlwaysAuthorization()
        } else {
            manager.requestWhenInUseAuthorization()
        }
    }

    /// Starts the updates the request's accuracy choice asks for.
    private func startUpdates(_ options: LocationOptions) {
        manager.desiredAccuracy = options.highAccuracy ? kCLLocationAccuracyBest : kCLLocationAccuracyHundredMeters
        manager.distanceFilter = kCLDistanceFilterNone

        manager.startUpdatingLocation()
    }

    /// Cancels the waiter the caller abandoned, if it is still the one that runs.
    private func cancelIfCurrent(_ identifier: UUID) {
        guard pendingID == identifier else { return }

        finish(.failure(CancellationError()))
    }

    private func scheduleTimeout(milliseconds: UInt32, identifier: UUID) {
        timeout = Task { @MainActor [weak self] in

            guard await Self.sleep(milliseconds) else { return }

            guard let self, self.pendingID == identifier else { return }

            self.finish(.success(self.timeoutAnswer()))
        }
    }

    /// Sleeps for the wait's deadline, saying whether it ran to completion.
    private static func sleep(_ milliseconds: UInt32) async -> Bool {
        do {
            try await Task.sleep(for: .milliseconds(milliseconds))
        } catch {
            return false
        }

        return true
    }

    /// The answer a waiter gets when its deadline passes first.
    private func timeoutAnswer() -> Data {
        switch authorizationRequest {
        case .always: return backgroundScope()
        case .whenInUse: return permission()
        case nil: return Data([1, 0, 0, 0])
        }
    }

    /// The notification the host posts when the app can no longer show a prompt.
    private var departureNotification: Notification.Name {
        #if os(iOS)

        return UIApplication.didEnterBackgroundNotification
        #else
        // A system authorization dialog can itself remove focus; hiding the
        // app still cancels its waiter without attempting to dismiss that dialog.
        return authorizationRequest != nil
            ? NSApplication.didHideNotification : NSApplication.didResignActiveNotification
        #endif
    }

    private func observeBackground() {
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: departureNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
        }
    }

    /// Answers a permission change; a downgrade cannot satisfy a waiting fix.
    public func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        if authorizationRequest != nil {
            answeredAuthorization(manager)

            return
        }

        if pending != nil, !canDeliverLocation { finish(.failure(PluginError.permissionDenied)) }
    }

    /// Answers a pending permission request once the OS has made a choice.
    private func answeredAuthorization(_ manager: CLLocationManager) {
        guard manager.authorizationStatus != .notDetermined else { return }

        finish(.success(authorizationRequest == .always ? backgroundScope() : permission()))
    }

    /// Answers with the newest fix inside the request's age and cursor window,
    /// and buffers every fix an update source reported.
    public func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        if sourceRunning {
            bufferLocations(locations)
            answerUpdateWaiter()
        }

        guard let options else { return }

        guard canDeliverLocation else {
            finish(.failure(PluginError.permissionDenied))

            return
        }

        guard let location = newest(locations, options) else { return }

        deliver(location)
    }

    #if os(iOS)
    /// Answers a visit by buffering one fix: at arrival while it is ongoing,
    /// at departure once it ended.
    public func locationManager(_ manager: CLLocationManager, didVisit visit: CLVisit) {
        guard visiting else { return }

        bufferLocations(visitLocations(visit))
        answerUpdateWaiter()
    }

    /// The fix one visit report stands for. iOS reports an ongoing visit with
    /// a `distantFuture` departure and an unknown arrival as `distantPast`,
    /// so a report carries an arrival or a departure, never both, and a
    /// visit iOS reports twice is not buffered twice.
    private func visitLocations(_ visit: CLVisit) -> [CLLocation] {
        if visit.departureDate != .distantFuture {
            return [visitLocation(visit, at: visit.departureDate)]
        }

        return visit.arrivalDate == .distantPast ? [] : [visitLocation(visit, at: visit.arrivalDate)]
    }

    /// The visit's coordinate as a fix at one of its timestamps.
    private func visitLocation(_ visit: CLVisit, at timestamp: Date) -> CLLocation {
        CLLocation(
            coordinate: visit.coordinate,
            altitude: 0,
            horizontalAccuracy: visit.horizontalAccuracy,
            verticalAccuracy: -1,
            timestamp: timestamp
        )
    }
    #endif

    /// Answers with an encoded fix, or the encoding failure.
    private func deliver(_ location: CLLocation) {
        do {
            finish(.success(try encodeLocation(location)))
        } catch {
            finish(.failure(error))
        }
    }

    /// The newest reported fix that is fresh enough and past the caller's cursor.
    private func newest(_ locations: [CLLocation], _ options: LocationOptions) -> CLLocation? {
        let oldest = options.maximumAge == 0
            ? started : Date().timeIntervalSince1970 * 1000 - Double(options.maximumAge)

        return locations.last(where: { location in
            let timestamp = location.timestamp.timeIntervalSince1970 * 1000

            if timestamp < oldest { return false }

            return options.after.map { timestamp > $0 } ?? true
        })
    }

    public func locationManager(_ manager: CLLocationManager, didFailWithError error: any Error) {
        guard pending != nil else { return }
        let native = error as NSError

        if native.domain == kCLErrorDomain && native.code == CLError.locationUnknown.rawValue {
            // A transient unavailable fix can recover within the caller's bounded deadline.
            return
        }

        let denied = native.domain == kCLErrorDomain && native.code == CLError.denied.rawValue
        finish(.failure(denied ? PluginError.permissionDenied : PluginError.native))
    }

    /// Reports the authorization state and the accuracy left to the app.
    private func permission() -> Data {
        let status = statusCode(manager.authorizationStatus)
        let accuracy = grantAccuracy(status)
        var result = Data()

        for var field in [status.littleEndian, accuracy.littleEndian] {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }

        return result
    }

    /// Accuracy matters only once access is granted; until then it is unknown.
    private func grantAccuracy(_ status: UInt32) -> UInt32 {
        guard status == Self.granted else { return 0 }

        return manager.accuracyAuthorization == .fullAccuracy ? 2 : 1
    }

    #if os(iOS)
    /// Reads the authorization state, including iOS's "when in use" grant.
    private func statusCode(_ status: CLAuthorizationStatus) -> UInt32 {
        switch status {
        case .notDetermined: 1
        case .denied: 2
        case .authorizedAlways, .authorizedWhenInUse: 3
        case .restricted: 4
        @unknown default: 0
        }
    }
    #else
    /// Reads the authorization state; macOS has no "when in use" grant.
    private func statusCode(_ status: CLAuthorizationStatus) -> UInt32 {
        switch status {
        case .notDetermined: 1
        case .denied: 2
        case .authorizedAlways: 3
        case .restricted: 4
        @unknown default: 0
        }
    }
    #endif

    private var canDeliverLocation: Bool {
        permitsLocationDelivery(
            startedPrecise: preciseAtStart,
            authorized: isAuthorized,
            precise: manager.accuracyAuthorization == .fullAccuracy
        )
    }

    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil
        options = nil
        preciseAtStart = false
        authorizationRequest = nil
        timeout?.cancel()
        timeout = nil

        // A running update stream owns the updates; an acquisition does not
        // stop them.
        if !sourceRunning { manager.stopUpdatingLocation() }

        if let backgroundObserver { NotificationCenter.default.removeObserver(backgroundObserver) }

        backgroundObserver = nil
        continuation.resume(with: result)
    }
}

/// Rejects queued precise coordinates after authorization is downgraded.
func permitsLocationDelivery(startedPrecise: Bool, authorized: Bool, precise: Bool) -> Bool {
    authorized && (!startedPrecise || precise)
}

struct LocationOptions {
    /// Byte counts and value bounds the request enforces.
    private enum Bound {
        static let bytes = 12
        static let previousBytes = 20
        static let maxTimeout: UInt32 = 120_000
        static let maxAge: UInt32 = 3_600_000
    }

    /// Where each field starts in the request bytes.
    private enum Field {
        static let timeout = 4
        static let maximumAge = 8
        static let after = 12
    }

    let highAccuracy: Bool
    let timeout: UInt32
    let maximumAge: UInt32
    let after: Double?

    init(bytes: UnsafeRawBufferPointer, hasPrevious: Bool) throws {
        let expected = hasPrevious ? Bound.previousBytes : Bound.bytes

        guard bytes.count == expected else { throw PluginError.invalidArgument }
        let accuracy = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        timeout = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.timeout, as: UInt32.self))
        maximumAge = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.maximumAge, as: UInt32.self))

        guard accuracy <= 1, (1...Bound.maxTimeout).contains(timeout), maximumAge <= Bound.maxAge else {
            throw PluginError.invalidArgument
        }

        highAccuracy = accuracy == 1

        if hasPrevious {
            let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.after, as: UInt64.self))

            after = try timestamp(bits)
        } else {
            after = nil
        }
    }
}

/// Reads a follow-up cursor, which must be a real timestamp.
private func timestamp(_ bits: UInt64) throws -> Double {
    let value = Double(bitPattern: bits)

    guard value.isFinite, value >= 0 else { throw PluginError.invalidArgument }

    return value
}

/// Pure conversion also used by tests with synthetic coordinates; never starts location acquisition.
func encodeLocation(_ location: CLLocation) throws -> Data {
    var result = Data(capacity: Wire.recordBytes)

    for var field in [UInt32(0)].map(\.littleEndian) {
        withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
    }

    result.append(try fixRecord(location))

    return result
}

/// The 80-byte fix record without the acquisition tag, shared by update
/// batches. Pure conversion; never starts location acquisition.
func fixRecord(_ location: CLLocation) throws -> Data {
    let optional = optionalValues(location)
    let values = requiredValues(location) + optional.map { $0 ?? 0 }

    try requireDeliverable(values)
    var mask: UInt32 = 0

    for (index, value) in optional.enumerated() where value != nil {
        mask |= 1 << index
    }

    let simulated: UInt32 = location.sourceInformation.map { $0.isSimulatedBySoftware ? 2 : 1 } ?? 0
    var result = Data(capacity: Wire.recordBytes - 4)

    for var field in [mask, 0, simulated, 0].map(\.littleEndian) {
        withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
    }

    for value in values {
        var bits = value.bitPattern.littleEndian
        withUnsafeBytes(of: &bits) { result.append(contentsOf: $0) }
    }

    return result
}

/// The optional fields in wire order: altitude, vertical accuracy, course, speed.
private func optionalValues(_ location: CLLocation) -> [Double?] {
    return [
        location.verticalAccuracy >= 0 ? location.altitude : nil,
        location.verticalAccuracy >= 0 ? location.verticalAccuracy : nil,
        location.course >= 0 && location.course < Wire.fullTurn ? location.course : nil,
        location.speed >= 0 ? location.speed : nil
    ]
}

/// The timestamp, latitude, longitude, and horizontal accuracy every fix carries.
private func requiredValues(_ location: CLLocation) -> [Double] {
    return [location.timestamp.timeIntervalSince1970 * 1000,
            location.coordinate.latitude,
            location.coordinate.longitude,
            location.horizontalAccuracy]
}

/// Rejects a fix whose fields the wire cannot carry.
private func requireDeliverable(_ values: [Double]) throws {
    guard values.allSatisfy(\.isFinite),
          values[0] >= 0,
          (-Wire.maxLatitude...Wire.maxLatitude).contains(values[1]),
          (-Wire.maxLongitude...Wire.maxLongitude).contains(values[2]),
          values[3] >= 0 else { throw PluginError.native }
}

/// Frames one native update batch: two little-endian u32 header fields,
/// then the fix records.
func batch(dropped: UInt32, records: [Data]) -> Data {
    var result = Data(capacity: 8 + records.reduce(0) { $0 + $1.count })

    for var field in [dropped, UInt32(records.count)].map(\.littleEndian) {
        withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
    }

    for record in records {
        result.append(record)
    }

    return result
}

/// Validated continuous-update request; the bounds mirror the Rust crate.
struct UpdateOptions {
    /// Byte counts and value bounds the request enforces.
    private enum Bound {
        static let bytes = 28
        static let maxDistance: Double = 10_000
        static let maxInterval: UInt32 = 3_600_000
    }

    /// Where each field starts in the request bytes.
    private enum Field {
        static let distance = 4
        static let interval = 12
        static let background = 16
        static let pauses = 20
        static let indicator = 24
    }

    /// The accuracy a stream asks the platform for.
    enum Accuracy: UInt32 {
        case best = 0
        case navigation = 1
        case balanced = 2
    }

    let accuracy: Accuracy
    let distanceFilterM: Double
    let intervalMs: UInt32
    let background: Bool
    let pausesAutomatically: Bool
    let showBackgroundIndicator: Bool

    init(bytes: UnsafeRawBufferPointer) throws {
        guard bytes.count == Bound.bytes else { throw PluginError.invalidArgument }
        guard let accuracy = Accuracy(rawValue: UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))) else {
            throw PluginError.invalidArgument
        }

        let distance = Double(bitPattern: UInt64(littleEndian: bytes.loadUnaligned(
            fromByteOffset: Field.distance,
            as: UInt64.self
        )))

        guard distance.isFinite, (0...Bound.maxDistance).contains(distance) else { throw PluginError.invalidArgument }

        let interval = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.interval, as: UInt32.self))
        guard (1...Bound.maxInterval).contains(interval) else { throw PluginError.invalidArgument }

        var flags: [UInt32] = []

        for offset in [Field.background, Field.pauses, Field.indicator] {
            let flag = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
            guard flag <= 1 else { throw PluginError.invalidArgument }
            flags.append(flag)
        }

        self.accuracy = accuracy
        distanceFilterM = distance
        intervalMs = interval
        background = flags[0] == 1
        pausesAutomatically = flags[1] == 1
        showBackgroundIndicator = flags[2] == 1
    }
}
// swiftlint:enable file_length
