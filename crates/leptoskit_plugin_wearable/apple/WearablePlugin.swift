import Foundation
import LeptosKitBridge
#if canImport(HealthKit) && os(watchOS)
import HealthKit
#endif
#if canImport(CoreMotion) && os(watchOS)
import CoreMotion
#endif

/// Direct on-watch biometric sensor plugin for Apple Watch (watchOS).
///
/// Uses `HKWorkoutSession` + `HKLiveWorkoutBuilder` for continuous live wrist
/// biometrics (heart rate, active calories, distance, SpO2, respiratory rate),
/// `HKObserverQuery` + `enableBackgroundDelivery` for passive updates, and
/// `CMPedometer` for live steps and floors climbed.
@MainActor
public final class WearablePlugin: NSObject, NativePlugin {
    public let name = "wearable"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7]

    #if os(watchOS)
    private let healthStore = HKHealthStore()
    private let pedometer = CMPedometer()
    private var workoutSession: HKWorkoutSession?
    private var workoutBuilder: HKLiveWorkoutBuilder?
    private var observerQueries: [HKObserverQuery] = []
    private var buffer: [Data] = []
    private var dropped: UInt32 = 0
    private var waiter: CheckedContinuation<Data, Error>?
    private var closed = false

    public override init() {
        super.init()
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        let data = payload.withUnsafeBytes { Data($0) }
        switch method {
        case 0: return encodeCapabilities()
        case 1: return encodeU32(0) // HealthKit does not disclose read grant state.
        case 2:
            try await requestPermissions(mask: try readMask(data))
            return encodeU32(0)
        case 3:
            try await startPassive(mask: try readMask(data))
            return Data()
        case 4:
            stopPassive()
            return Data()
        case 5:
            try await startLive(data: data)
            return Data()
        case 6:
            stopLive()
            return Data()
        case 7:
            return try await nextBatch()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        stopPassive()
        stopLive()
        waiter?.resume(throwing: PluginError.cancelled)
        waiter = nil
    }

    private func encodeCapabilities() -> Data {
        let mask: UInt32 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 7)
        let flags: UInt32 = 1 | 2 // passive + live workout sessions
        var out = Data()
        appendU32(&out, 2) // PlatformKind::WatchOs
        appendU32(&out, mask)
        appendU32(&out, flags)
        return out
    }

    private func requestPermissions(mask: UInt32) async throws {
        var readTypes = Set<HKObjectType>()
        if mask & (1 << 0) != 0, let hr = HKObjectType.quantityType(forIdentifier: .heartRate) { readTypes.insert(hr) }
        if mask & (1 << 1) != 0, let hrv = HKObjectType.quantityType(forIdentifier: .heartRateVariabilitySDNN) { readTypes.insert(hrv) }
        if mask & (1 << 2) != 0, let spo2 = HKObjectType.quantityType(forIdentifier: .oxygenSaturation) { readTypes.insert(spo2) }
        if mask & (1 << 4) != 0, let steps = HKObjectType.quantityType(forIdentifier: .stepCount) { readTypes.insert(steps) }
        if mask & (1 << 5) != 0, let floors = HKObjectType.quantityType(forIdentifier: .flightsClimbed) { readTypes.insert(floors) }
        if mask & (1 << 6) != 0, let cal = HKObjectType.quantityType(forIdentifier: .activeEnergyBurned) { readTypes.insert(cal) }
        if mask & (1 << 7) != 0, let dist = HKObjectType.quantityType(forIdentifier: .distanceWalkingRunning) { readTypes.insert(dist) }
        let shareTypes: Set<HKSampleType> = [HKObjectType.workoutType()]
        try await healthStore.requestAuthorization(toShare: shareTypes, read: readTypes)
    }

    private func startPassive(mask: UInt32) async throws {
        stopPassive()
        let ids: [(UInt32, HKQuantityTypeIdentifier)] = [
            (1 << 0, .heartRate),
            (1 << 2, .oxygenSaturation),
            (1 << 4, .stepCount),
            (1 << 6, .activeEnergyBurned),
        ]
        for (bit, identifier) in ids where mask & bit != 0 {
            guard let sampleType = HKObjectType.quantityType(forIdentifier: identifier) else { continue }
            try await healthStore.enableBackgroundDelivery(for: sampleType, frequency: .immediate)
            let query = HKObserverQuery(sampleType: sampleType, predicate: nil) { [weak self] _, completion, _ in
                Task { @MainActor in
                    await self?.fetchLatest(identifier: identifier)
                    completion()
                }
            }
            observerQueries.append(query)
            healthStore.execute(query)
        }
    }

    private func stopPassive() {
        for query in observerQueries {
            healthStore.stop(query)
        }
        observerQueries.removeAll()
    }

    private func startLive(data: Data) async throws {
        guard data.count == 12 else { throw PluginError.invalidArgument }
        let mask = data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self)) }
        let intervalMs = data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt32.self)) }
        guard mask != 0, intervalMs >= 200 else { throw PluginError.invalidArgument }

        stopLive()
        let config = HKWorkoutConfiguration()
        config.activityType = .other
        config.locationType = .unknown
        let session = try HKWorkoutSession(healthStore: healthStore, configuration: config)
        let builder = session.associatedWorkoutBuilder()
        builder.dataSource = HKLiveWorkoutDataSource(healthStore: healthStore, workoutConfiguration: config)
        builder.delegate = self
        workoutSession = session
        workoutBuilder = builder
        let now = Date()
        session.startActivity(with: now)
        try await builder.beginCollection(at: now)

        if mask & (1 << 4) != 0, CMPedometer.isStepCountingAvailable() {
            pedometer.startUpdates(from: now) { [weak self] data, _ in
                guard let data else { return }
                let startMs = Int64(data.startDate.timeIntervalSince1970 * 1000)
                let endMs = Int64(data.endDate.timeIntervalSince1970 * 1000)
                let steps = data.numberOfSteps.int64Value
                Task { @MainActor in
                    self?.enqueue(Self.encodeSteps(startMs: startMs, endMs: endMs, count: steps))
                }
            }
        }
    }

    private func stopLive() {
        pedometer.stopUpdates()
        workoutSession?.end()
        workoutSession = nil
        workoutBuilder = nil
    }

    private func fetchLatest(identifier: HKQuantityTypeIdentifier) async {
        guard let sampleType = HKObjectType.quantityType(forIdentifier: identifier) else { return }
        let sort = [NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)]
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            let query = HKSampleQuery(sampleType: sampleType, predicate: nil, limit: 1, sortDescriptors: sort) { [weak self] _, samples, _ in
                if let sample = samples?.first as? HKQuantitySample {
                    Task { @MainActor in
                        self?.enqueueQuantity(sample)
                    }
                }
                continuation.resume()
            }
            healthStore.execute(query)
        }
    }

    private func enqueueQuantity(_ sample: HKQuantitySample) {
        let startMs = Int64(sample.startDate.timeIntervalSince1970 * 1000)
        let endMs = Int64(sample.endDate.timeIntervalSince1970 * 1000)
        switch sample.quantityType.identifier {
        case HKQuantityTypeIdentifier.heartRate.rawValue:
            let bpm = sample.quantity.doubleValue(for: HKUnit.count().unitDivided(by: .minute()))
            enqueue(Self.encodeHeartRate(timeMs: endMs, bpm: bpm, accuracy: 4))
        case HKQuantityTypeIdentifier.oxygenSaturation.rawValue:
            let pct = sample.quantity.doubleValue(for: .percent()) * 100.0
            enqueue(Self.encodeInstantDouble(kind: 2, timeMs: endMs, value: pct))
        case HKQuantityTypeIdentifier.stepCount.rawValue:
            let steps = Int64(sample.quantity.doubleValue(for: .count()))
            enqueue(Self.encodeSteps(startMs: startMs, endMs: endMs, count: steps))
        case HKQuantityTypeIdentifier.activeEnergyBurned.rawValue:
            let kcal = sample.quantity.doubleValue(for: .kilocalorie())
            enqueue(Self.encodeIntervalDouble(kind: 6, startMs: startMs, endMs: endMs, value: kcal))
        case HKQuantityTypeIdentifier.distanceWalkingRunning.rawValue:
            let meters = sample.quantity.doubleValue(for: .meter())
            enqueue(Self.encodeIntervalDouble(kind: 7, startMs: startMs, endMs: endMs, value: meters))
        default:
            break
        }
    }

    private func enqueue(_ reading: Data) {
        if buffer.count >= 128 {
            buffer.removeFirst()
            dropped &+= 1
        }
        buffer.append(reading)
        if let pending = waiter {
            waiter = nil
            pending.resume(returning: drainBatch())
        }
    }

    private func nextBatch() async throws -> Data {
        if !buffer.isEmpty || dropped > 0 {
            return drainBatch()
        }
        return try await withCheckedThrowingContinuation { continuation in
            waiter = continuation
        }
    }

    private func drainBatch() -> Data {
        var out = Data()
        appendU32(&out, dropped)
        appendU32(&out, UInt32(buffer.count))
        dropped = 0
        for item in buffer {
            out.append(item)
        }
        buffer.removeAll()
        return out
    }

    private func readMask(_ data: Data) throws -> UInt32 {
        guard data.count == 4 else { throw PluginError.invalidArgument }
        let mask = data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
        guard mask != 0 else { throw PluginError.invalidArgument }
        return mask
    }

    private static func encodeHeartRate(timeMs: Int64, bpm: Double, accuracy: UInt32) -> Data {
        var out = Data()
        appendU32(&out, 0)
        appendI64(&out, timeMs)
        appendF64(&out, bpm)
        appendU32(&out, accuracy)
        return out
    }

    private static func encodeInstantDouble(kind: UInt32, timeMs: Int64, value: Double) -> Data {
        var out = Data()
        appendU32(&out, kind)
        appendI64(&out, timeMs)
        appendF64(&out, value)
        return out
    }

    private static func encodeSteps(startMs: Int64, endMs: Int64, count: Int64) -> Data {
        var out = Data()
        appendU32(&out, 4)
        appendI64(&out, startMs)
        appendI64(&out, endMs)
        appendI64(&out, count)
        return out
    }

    private static func encodeIntervalDouble(kind: UInt32, startMs: Int64, endMs: Int64, value: Double) -> Data {
        var out = Data()
        appendU32(&out, kind)
        appendI64(&out, startMs)
        appendI64(&out, endMs)
        appendF64(&out, value)
        return out
    }
    #else
    public override init() {
        super.init()
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        _ = payload.withUnsafeBytes { $0.count }
        if method == 0 {
            var out = Data()
            appendU32(&out, 0) // Unsupported on non-watchOS Apple hosts
            appendU32(&out, 0)
            appendU32(&out, 0)
            return out
        }
        if method == 1 {
            return encodeU32(4) // Unsupported
        }
        throw PluginError.unavailable
    }
    #endif
}

#if os(watchOS)
extension WearablePlugin: HKLiveWorkoutBuilderDelegate {
    public nonisolated func workoutBuilder(
        _ workoutBuilder: HKLiveWorkoutBuilder,
        didCollectDataOf collectedTypes: Set<HKSampleType>
    ) {
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        for sampleType in collectedTypes {
            guard let quantityType = sampleType as? HKQuantityType,
                  let stats = workoutBuilder.statistics(for: quantityType) else { continue }
            if quantityType.identifier == HKQuantityTypeIdentifier.heartRate.rawValue,
               let bpm = stats.mostRecentQuantity()?.doubleValue(for: HKUnit.count().unitDivided(by: .minute())) {
                Task { @MainActor in
                    self.enqueue(Self.encodeHeartRate(timeMs: nowMs, bpm: bpm, accuracy: 4))
                }
            } else if quantityType.identifier == HKQuantityTypeIdentifier.activeEnergyBurned.rawValue,
                      let kcal = stats.sumQuantity()?.doubleValue(for: .kilocalorie()) {
                Task { @MainActor in
                    self.enqueue(Self.encodeIntervalDouble(kind: 6, startMs: nowMs - 1000, endMs: nowMs, value: kcal))
                }
            }
        }
    }

    public nonisolated func workoutBuilderDidCollectEvent(_ workoutBuilder: HKLiveWorkoutBuilder) {}
}
#endif

private func encodeU32(_ value: UInt32) -> Data {
    var out = Data()
    appendU32(&out, value)
    return out
}

private func appendU32(_ out: inout Data, _ value: UInt32) {
    var v = value.littleEndian
    withUnsafeBytes(of: &v) { out.append(contentsOf: $0) }
}

private func appendI64(_ out: inout Data, _ value: Int64) {
    var v = value.littleEndian
    withUnsafeBytes(of: &v) { out.append(contentsOf: $0) }
}

private func appendF64(_ out: inout Data, _ value: Double) {
    var v = value.bitPattern.littleEndian
    withUnsafeBytes(of: &v) { out.append(contentsOf: $0) }
}
