import Foundation

/// The on-disk index that survives a crash: an append-only journal beside
/// the segments, one line per event, tab-separated.
///
/// - `F` names a file the recorder opened, with its start time and planned
///   length, written before the first audio lands in it.
/// - `C` completes a file: its real duration, size, peak level, active time
///   and why it ended.
/// - `D` marks a file delivered to the app, after which the app owns it.
///
/// A crash between `F` and `C` leaves a file that never finished writing:
/// recovery deletes it. A crash after `C` but before `D` leaves a finalised
/// file the app never saw: recovery re-queues it as a recovered segment.
public final class RecorderIndex: @unchecked Sendable {
    /// One finalised-but-undelivered segment the journal describes.
    public struct Finalised: Equatable {
        public let filename: String
        public let startedAtMs: Int64
        public let durationMs: Int64
        public let bytes: Int64
        public let peakDbfs: Double
        public let activeMs: Int64
        public let endedBy: UInt32
    }

    /// What a recovery pass found: files to deliver again and files to
    /// delete because they cannot be complete.
    public struct Plan: Equatable {
        public let deliverable: [Finalised]
        public let discardable: [String]
    }

    /// One parsed journal line, in file order.
    public enum Line: Equatable {
        case open(file: String, startedMs: Int64, plannedMs: Int64)
        case finalised(
            file: String, durationMs: Int64, bytes: Int64, peakDbfs: Double, activeMs: Int64, endedBy: UInt32
        )
        case delivered(file: String)
    }

    /// The journal's file URL.
    public let url: URL
    /// Segments are named so a recovery pass can spot them without the
    /// journal.
    public static let segmentPrefix = "seg-"
    public static let segmentSuffix = ".m4a"

    private let lock = NSLock()

    /// The journal for `url`, created empty when missing.
    public init(url: URL) throws {
        self.url = url
        if !FileManager.default.fileExists(atPath: url.path) {
            FileManager.default.createFile(atPath: url.path, contents: nil)
        }
    }

    /// Records opening `filename`.
    public func recordOpen(filename: String, startedMs: Int64, plannedMs: Int64) throws {
        try append("F\t\(filename)\t\(startedMs)\t\(plannedMs)")
    }

    /// Records `filename` finishing with its real metrics.
    public func recordFinalised(
        filename: String, durationMs: Int64, bytes: Int64, peakDbfs: Double, activeMs: Int64, endedBy: UInt32
    ) throws {
        try append(
            "C\t\(filename)\t\(durationMs)\t\(bytes)\t\(peakDbfs)\t\(activeMs)\t\(endedBy)"
        )
    }

    /// Records `filename` leaving the plugin's care, delivered to the app.
    public func recordDelivered(filename: String) throws {
        try append("D\t\(filename)")
    }

    /// Rewrites the journal holding only the entries `keeping` names, each
    /// as a fresh open+finalised pair, so a long-lived session cannot grow
    /// it without bound. Everything else is settled history.
    public func compact(keeping: [Finalised]) throws {
        var text = ""
        for record in keeping {
            text += "F\t\(record.filename)\t\(record.startedAtMs)\t\(record.durationMs)\n"
            text += "C\t\(record.filename)\t\(record.durationMs)\t\(record.bytes)\t\(record.peakDbfs)"
            text += "\t\(record.activeMs)\t\(record.endedBy)\n"
        }

        try lock.withLock {
            try text.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    /// Parses journal text, skipping torn or malformed lines.
    public static func parse(_ text: String) -> [Line] {
        var lines: [Line] = []

        for row in text.split(separator: "\n") {
            let fields = row.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
            switch fields.first {
            case "F" where fields.count == 4:
                if let started = Int64(fields[2]), let planned = Int64(fields[3]), !fields[1].isEmpty {
                    lines.append(.open(file: fields[1], startedMs: started, plannedMs: planned))
                }
            case "C" where fields.count == 7:
                if let duration = Int64(fields[2]), let bytes = Int64(fields[3]), let peak = Double(fields[4]),
                    let active = Int64(fields[5]), let ended = UInt32(fields[6]), !fields[1].isEmpty {
                    lines.append(
                        .finalised(
                            file: fields[1], durationMs: duration, bytes: bytes, peakDbfs: peak,
                            activeMs: active, endedBy: ended
                        )
                    )
                }
            case "D" where fields.count == 2:
                if !fields[1].isEmpty {
                    lines.append(.delivered(file: fields[1]))
                }
            default:
                break
            }
        }

        return lines
    }

    /// Classifies a journal against the files that exist, so a later start
    /// can deliver what a crash left behind and delete what never finished.
    public static func plan(lines: [Line], existingFiles: [String]) -> Plan {
        var opened: [String: Int64] = [:]
        var finalised: [String: Finalised?] = [:]
        var delivered: Set<String> = []

        for line in lines {
            switch line {
            case .open(let file, let startedMs, _):
                opened[file] = startedMs
                finalised[file] = nil
            case .finalised(let file, let durationMs, let bytes, let peakDbfs, let activeMs, let endedBy):
                if let startedMs = opened[file] {
                    finalised[file] = Finalised(
                        filename: file, startedAtMs: startedMs, durationMs: durationMs,
                        bytes: bytes, peakDbfs: peakDbfs, activeMs: activeMs, endedBy: endedBy
                    )
                }
            case .delivered(let file):
                delivered.insert(file)
            }
        }

        var deliverable: [Finalised] = []
        var discardable: [String] = []

        for (file, record) in finalised {
            // A finalised file the app never saw is delivered again.
            if let record, !delivered.contains(file) {
                deliverable.append(record)
            }
        }

        // An opened file that never finalised was cut short mid-write.
        for file in opened.keys where finalised[file] == nil {
            discardable.append(file)
        }

        // Segment files the journal never opened: orphans from a torn first
        // line or an older run of the directory.
        let known = Set(opened.keys)
        for file in existingFiles
        where file.hasPrefix(segmentPrefix) && file.hasSuffix(segmentSuffix) && !known.contains(file) {
            discardable.append(file)
        }

        return Plan(
            deliverable: deliverable.sorted { $0.startedAtMs < $1.startedAtMs },
            discardable: discardable.sorted()
        )
    }

    /// Reads the whole journal, for a recovery pass.
    public func read() throws -> [Line] {
        try lock.withLock {
            Self.parse(try String(contentsOf: url, encoding: .utf8))
        }
    }

    /// Appends one durable line.
    private func append(_ line: String) throws {
        try lock.withLock {
            let handle = try FileHandle(forWritingTo: url)
            defer { try? handle.close() }
            try handle.seekToEnd()
            try handle.write(contentsOf: Data((line + "\n").utf8))
            handle.synchronizeFile()
        }
    }
}
