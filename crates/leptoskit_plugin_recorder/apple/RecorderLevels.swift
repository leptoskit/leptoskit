import Foundation

/// The level math shared by the engine and its tests: 30-millisecond frames
/// of converted samples, measured in dBFS, and the segment totals they add
/// up to. Digital silence floors at `floorDbfs` so replies stay finite.
public enum RecorderLevels {
    /// The quietest level a segment reports, in dBFS.
    public static let floorDbfs: Double = -120
    /// Every frame covers 30 milliseconds of audio.
    public static let frameMilliseconds: Int = 30
    /// Below this magnitude a level is digital silence.
    static let silence: Double = 1e-6

    /// One measured frame.
    public static func measure(_ samples: ArraySlice<Float>) -> LevelFrame {
        var sum: Double = 0
        var peak: Double = 0

        for sample in samples {
            let magnitude = abs(Double(sample))
            sum += magnitude * magnitude
            peak = max(peak, magnitude)
        }

        let rms = sqrt(sum / Double(samples.count))
        return LevelFrame(rmsDbfs: dbfs(rms), peakDbfs: dbfs(peak))
    }

    /// A magnitude in dBFS, floored for digital silence.
    public static func dbfs(_ magnitude: Double) -> Double {
        max(20 * log10(max(magnitude, silence)), floorDbfs)
    }

    /// How many samples one 30-millisecond frame holds at `sampleRate`.
    public static func frameSamples(sampleRate: Int) -> Int {
        max(sampleRate * frameMilliseconds / 1_000, 1)
    }

    /// Where a chunk of `count` samples, arriving after `written` samples of
    /// the current segment, splits at the `segmentFrames` boundary: the
    /// number of samples that still belong to the current segment, or `nil`
    /// when the whole chunk fits.
    public static func split(written: Int, count: Int, segmentFrames: Int) -> Int? {
        guard written + count > segmentFrames, written < segmentFrames else { return nil }
        return segmentFrames - written
    }
}

/// One 30-millisecond frame's RMS and peak levels, in dBFS.
public struct LevelFrame: Equatable {
    /// The frame's root-mean-square level, in dBFS.
    public let rmsDbfs: Double
    /// The frame's loudest sample, in dBFS.
    public let peakDbfs: Double

    /// Whether the frame counts as active at `thresholdDbfs`.
    public func isActive(thresholdDbfs: Double) -> Bool {
        rmsDbfs > thresholdDbfs
    }
}

/// The running totals of one segment: its peak level, its active time and
/// its duration, in the units the segment reply carries.
public struct SegmentLevels {
    /// The segment's loudest frame peak so far, in dBFS.
    public private(set) var peakDbfs: Double = RecorderLevels.floorDbfs
    /// Milliseconds of frames above the activity threshold so far.
    public private(set) var activeMs: Int64 = 0
    /// Milliseconds of audio so far.
    public private(set) var durationMs: Int64 = 0
    /// The activity threshold the app configured.
    public let thresholdDbfs: Double

    public init(thresholdDbfs: Double) {
        self.thresholdDbfs = thresholdDbfs
    }

    /// Adds one measured frame to the totals.
    public mutating func add(_ frame: LevelFrame) {
        peakDbfs = max(peakDbfs, frame.peakDbfs)
        durationMs += Int64(RecorderLevels.frameMilliseconds)
        if frame.isActive(thresholdDbfs: thresholdDbfs) {
            activeMs += Int64(RecorderLevels.frameMilliseconds)
        }
    }
}
