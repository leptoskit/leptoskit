import Foundation
#if canImport(HealthKit) && os(iOS)
import HealthKit

/// Beats per minute, the unit heart rate and respiratory rate report.
let beatsPerMinute = HKUnit.count().unitDivided(by: .minute())

/// Milliseconds, the unit heart-rate variability reports.
let milliseconds = HKUnit.secondUnit(with: .milli)

/// One HealthKit sample as the wire's sample layout.
func sample(of sample: HKSample) -> HealthSample {
    HealthSample(
        id: sample.uuid.uuidString,
        startMs: ms(sample.startDate),
        endMs: ms(sample.endDate),
        source: HealthSource(
            appId: sample.sourceRevision.source.bundleIdentifier,
            appName: sample.sourceRevision.source.name,
            device: sample.device?.name
        ),
        value: value(of: sample)
    )
}

private func ms(_ date: Date) -> Int64 {
    Int64(date.timeIntervalSince1970 * 1000)
}

private func value(of sample: HKSample) -> HealthValue {
    switch sample {
    case let quantity as HKQuantitySample:
        value(ofQuantity: quantity)
    case let category as HKCategorySample
        where category.categoryType.identifier == HKCategoryTypeIdentifier.sleepAnalysis.rawValue:
        .sleep(stages: [
            SleepStage(
                startMs: ms(category.startDate),
                endMs: ms(category.endDate),
                stage: SleepStage.Stage(category.value)
            )
        ])
    case let workout as HKWorkout:
        .exercise(
            activity: Activity(workout.workoutActivityType).code,
            platformCode: Int32(bitPattern: UInt32(workout.workoutActivityType.rawValue)),
            title: nil
        )
    default:
        .sleep(stages: [])
    }
}

private func value(ofQuantity sample: HKQuantitySample) -> HealthValue {
    switch sample.quantityType.identifier {
    case HKQuantityTypeIdentifier.heartRate.rawValue:
        .heartRate([HeartRatePoint(timeMs: ms(sample.endDate), bpm: sample.quantity.doubleValue(for: beatsPerMinute))])
    case HKQuantityTypeIdentifier.restingHeartRate.rawValue:
        .restingHeartRate(bpm: sample.quantity.doubleValue(for: beatsPerMinute))
    case HKQuantityTypeIdentifier.heartRateVariabilitySDNN.rawValue:
        .hrvSdnn(milliseconds: sample.quantity.doubleValue(for: milliseconds))
    case HKQuantityTypeIdentifier.stepCount.rawValue:
        .steps(count: Int64(sample.quantity.doubleValue(for: HKUnit.count())))
    case HKQuantityTypeIdentifier.oxygenSaturation.rawValue:
        .oxygenSaturation(percent: sample.quantity.doubleValue(for: HKUnit.percent()))
    case HKQuantityTypeIdentifier.respiratoryRate.rawValue:
        .respiratoryRate(breathsPerMinute: sample.quantity.doubleValue(for: beatsPerMinute))
    default:
        .sleep(stages: [])
    }
}

/// The common activities HealthKit and Health Connect share; anything else
/// stays `other` with the platform's own code alongside.
private enum Activity {
    case other
    case walking
    case running
    case cycling
    case swimming
    case hiking
    case strengthTraining
    case yoga
    case rowing
    case elliptical
    case stairClimbing
    case dancing
    case pilates
    case martialArts

    /// The workout types each common activity covers.
    private static let workoutTypes: [HKWorkoutActivityType: Activity] = [
        .walking: .walking,
        .running: .running,
        .cycling: .cycling,
        .swimming: .swimming,
        .hiking: .hiking,
        .traditionalStrengthTraining: .strengthTraining,
        .yoga: .yoga,
        .rowing: .rowing,
        .elliptical: .elliptical,
        .stairClimbing: .stairClimbing,
        .dance: .dancing,
        .danceInspiredTraining: .dancing,
        .pilates: .pilates,
        .martialArts: .martialArts
    ]

    var code: Int {
        switch self {
        case .other: HealthActivity.other
        case .walking: HealthActivity.walking
        case .running: HealthActivity.running
        case .cycling: HealthActivity.cycling
        case .swimming: HealthActivity.swimming
        case .hiking: HealthActivity.hiking
        case .strengthTraining: HealthActivity.strengthTraining
        case .yoga: HealthActivity.yoga
        case .rowing: HealthActivity.rowing
        case .elliptical: HealthActivity.elliptical
        case .stairClimbing: HealthActivity.stairClimbing
        case .dancing: HealthActivity.dancing
        case .pilates: HealthActivity.pilates
        case .martialArts: HealthActivity.martialArts
        }
    }

    init(_ type: HKWorkoutActivityType) {
        self = Activity.workoutTypes[type] ?? .other
    }
}

/// Activity codes, in wire order.
private enum HealthActivity {
    static let other = 0
    static let walking = 1
    static let running = 2
    static let cycling = 3
    static let swimming = 4
    static let hiking = 5
    static let strengthTraining = 6
    static let yoga = 7
    static let rowing = 8
    static let elliptical = 9
    static let stairClimbing = 10
    static let dancing = 11
    static let pilates = 12
    static let martialArts = 13
}

extension SleepStage.Stage {
    /// HealthKit's sleep values keep their stage, in bed stays its own, and
    /// anything unknown reports unknown rather than a guess.
    init(_ value: Int) {
        guard let analysis = HKCategoryValueSleepAnalysis(rawValue: value) else {
            self = .unknown
            return
        }
        switch analysis {
        case .inBed: self = .inBed
        case .asleepUnspecified, .asleep: self = .asleep
        case .awake: self = .awake
        case .asleepCore: self = .light
        case .asleepDeep: self = .deep
        case .asleepREM: self = .rem
        @unknown default: self = .unknown
        }
    }
}
#endif
