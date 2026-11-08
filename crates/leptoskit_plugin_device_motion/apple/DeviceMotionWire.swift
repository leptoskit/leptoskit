import Foundation
import LeptosKitBridge

enum DeviceMotionWire {
    static let available: UInt32 = 0
    static let next: UInt32 = 1
    static let methods: [UInt32] = [available, next]
    static let readingBytes = 13 * 8
    static let intervalRange: ClosedRange<UInt32> = 5...60_000
    /// Scales CoreMotion units of gravity onto Android axes in m/s².
    static let gravityToAndroid = -9.80665

    static func decodeInterval(_ payload: NativeBuffer) throws -> TimeInterval {
        guard payload.count == 4 else { throw PluginError.invalidArgument }
        let milliseconds = payload.withUnsafeBytes { bytes -> UInt32 in
            var value: UInt32 = 0
            for index in 0..<4 {
                value |= UInt32(bytes[index]) << (8 * index)
            }
            return value
        }
        guard intervalRange.contains(milliseconds) else { throw PluginError.invalidArgument }
        return TimeInterval(milliseconds) / 1000.0
    }

    static func encodeAvailability(_ value: Bool) -> Data {
        Data([value ? 1 : 0, 0, 0, 0])
    }

    static func encodeReading(
        timestamp: Double,
        accelX: Double,
        accelY: Double,
        accelZ: Double,
        gravityX: Double,
        gravityY: Double,
        gravityZ: Double,
        rotationX: Double,
        rotationY: Double,
        rotationZ: Double,
        attitudeAlpha: Double,
        attitudeBeta: Double,
        attitudeGamma: Double
    ) throws -> Data {
        let values = [
            timestamp, accelX, accelY, accelZ, gravityX, gravityY, gravityZ,
            rotationX, rotationY, rotationZ, attitudeAlpha, attitudeBeta, attitudeGamma
        ]
        guard timestamp >= 0, values.allSatisfy(\.isFinite) else { throw PluginError.native }

        var data = Data(capacity: readingBytes)
        for value in values {
            var little = value.bitPattern.littleEndian
            withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
        }
        return data
    }
}
