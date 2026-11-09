import AVFoundation
import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#endif

/// Continuous microphone recording as timed AAC-LC mono `.m4a` segments,
/// written through an `AVAudioEngine` input tap into an AAC `AVAudioFile`
/// that rotates at every boundary. Recording can only start in the
/// foreground; the `audio` background mode keeps it running afterwards. The
/// owner's music and calls keep working: the session is `.playAndRecord`
/// with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker`.
@MainActor
public final class RecorderPlugin: NativePlugin {
    public let name = "recorder"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6]

    #if os(iOS)
    private let core = RecorderCore()
    private var observers: [NSObjectProtocol] = []
    private var closed = false

    public init() {
        registerObservers()
    }

    /// Answers one call; only the permission request ever prompts.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case Method.permission:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return permissionReply()
        case Method.requestPermission:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try await requestAccess()
        case Method.start:
            return try start(StartRequest(payload.data()))
        case Method.stop:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            try stop()
            return Data()
        case Method.status:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return statusReply()
        case Method.nextSegment:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try await nextSegment()
        case Method.recover:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return try recover()
        default:
            throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        do {
            try stop()
        } catch {
            // Stopping on close is best-effort; finalised segments stay on
            // disk for the next recovery.
        }
        core.cancelWaiter()
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
        }
        observers.removeAll()
    }

    // MARK: Permissions

    /// The microphone permission, without prompting. The switch spells the
    /// states without the nested type, whose name moved between SDKs.
    private func permissionReply() -> Data {
        switch AVAudioApplication.shared.recordPermission {
        case .granted: return encodePermission(RecorderWire.permissionGranted)
        case .denied: return encodePermission(RecorderWire.permissionDenied)
        case .undetermined: return encodePermission(RecorderWire.permissionUnknown)
        @unknown default: return encodePermission(RecorderWire.permissionUnknown)
        }
    }

    /// Asks once, or reports the state when the system has already answered.
    private func requestAccess() async throws -> Data {
        let permission = AVAudioApplication.shared.recordPermission
        guard permission == .undetermined else { return permissionReply() }

        try requireUsageDescription()
        try requireForeground()
        let granted = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            AVAudioApplication.requestRecordPermission { granted in
                continuation.resume(returning: granted)
            }
        }

        let code: UInt32 = granted
            ? RecorderWire.permissionGranted
            : RecorderWire.permissionDenied
        return encodePermission(code)
    }

    /// A prompt needs the app's own words in its `Info.plist`.
    private func requireUsageDescription() throws {
        guard let description = Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription")
            as? String,
            !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { throw PluginError.invalidArgument }
    }

    /// Recording and prompts need the app in the foreground.
    private func requireForeground() throws {
        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }
    }

    // MARK: Recording

    /// Starts recording, delivering leftover files from an earlier session
    /// first, and reporting what that recovery found.
    private func start(_ request: StartRequest) throws -> Data {
        guard core.state == RecorderWire.stateIdle || core.state == RecorderWire.stateFailed else {
            throw PluginError.busy
        }
        try requireForeground()
        guard AVAudioApplication.shared.recordPermission == .granted else {
            throw PluginError.permissionDenied
        }

        let directory = try resolveDirectory(request.directory)
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(
            .playAndRecord,
            mode: .default,
            options: [.mixWithOthers, .allowBluetoothHFP, .defaultToSpeaker]
        )
        try session.setActive(true)

        let (recovered, discarded) = try core.recover(directory: directory)
        UserDefaults.standard.set(directory.path, forKey: RecorderCore.directoryKey)
        try core.start(
            directory: directory,
            segmentSecs: request.segmentSecs,
            sampleRate: request.sampleRate,
            bitrate: request.bitrate,
            thresholdDbfs: request.thresholdDbfs
        )
        return encodeRecovery(recovered: UInt32(recovered), discarded: UInt32(discarded))
    }

    /// Stops recording and finishes the current segment so it is delivered,
    /// never lost.
    private func stop() throws {
        try core.stop()
        try? AVAudioSession.sharedInstance().setActive(false, options: [])
    }

    private func statusReply() -> Data {
        let snapshot = core.status()
        var reply = StatusReply()
        reply.state = snapshot.state
        reply.currentSegmentStartedAtMs = snapshot.segmentStartedAtMs
        reply.reason = snapshot.reason
        do {
            return try reply.encode()
        } catch {
            // A failure reply keeps status answering even when the snapshot
            // itself is broken.
            var fallback = WireWriter()
            fallback.u32(RecorderWire.stateFailed)
            fallback.i64(0)
            try? fallback.optional("status unavailable")
            return fallback.bytes()
        }
    }

    /// Delivers one finished segment, waits for the next, or reports that
    /// the stream has ended.
    private func nextSegment() async throws -> Data {
        if let reply = core.dequeueSegment() {
            core.markDelivered(reply)
            return try reply.encode()
        }

        let waiting = core.state == RecorderWire.stateRecording
            || core.state == RecorderWire.stateSilenced
            || core.state == RecorderWire.stateInterrupted
        guard waiting else { return encodeAbsentSegment() }

        let reply = try await core.awaitSegment()
        core.markDelivered(reply)
        return try reply.encode()
    }

    /// Delivers leftover files from an earlier session, without recording.
    private func recover() throws -> Data {
        let (recovered, discarded) = try core.recoverLeftovers()
        return encodeRecovery(recovered: UInt32(recovered), discarded: UInt32(discarded))
    }

    /// An app-private directory inside the sandbox, created if missing.
    private func resolveDirectory(_ path: String) throws -> URL {
        let home = URL(fileURLWithPath: NSHomeDirectory()).resolvingSymlinksInPath().path
        let resolved = URL(fileURLWithPath: path).resolvingSymlinksInPath().path
        guard resolved.hasPrefix(home + "/") else { throw PluginError.invalidArgument }

        let url = URL(fileURLWithPath: resolved)
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        } catch {
            throw PluginError.invalidArgument
        }
        return url
    }

    // MARK: System events

    private func registerObservers() {
        let center = NotificationCenter.default
        observers.append(
            center.addObserver(
                forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
            ) { [weak self] note in
                // The payload is read here, on the posting thread, so only
                // sendable values cross into the task.
                let type = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt ?? 0
                let options = note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
                Task { @MainActor in self?.handleInterruption(type: type, options: options) }
            }
        )
        observers.append(
            center.addObserver(
                forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main
            ) { [weak self] note in
                let reason = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt ?? 0
                Task { @MainActor in self?.handleRouteChange(reason: reason) }
            }
        )
        observers.append(
            center.addObserver(
                forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main
            ) { [weak self] _ in
                Task { @MainActor in self?.handleMediaServicesReset() }
            }
        )
    }

    /// An interruption closes the segment and reports `interrupted`; when
    /// the system says the session should resume, recording resumes in a
    /// fresh segment.
    private func handleInterruption(type: UInt, options: UInt) {
        switch AVAudioSession.InterruptionType(rawValue: type) {
        case .began:
            core.interrupt()
        case .ended where core.state == RecorderWire.stateInterrupted:
            let resume = AVAudioSession.InterruptionOptions(rawValue: options)
            guard resume.contains(.shouldResume) else { return }
            do {
                try AVAudioSession.sharedInstance().setActive(true)
                try core.resume()
            } catch {
                core.fail(reason: "resuming after the interruption failed")
            }
        default:
            break
        }
    }

    /// A route change can move the input to a new format; when it does, the
    /// current segment rotates and the tap is rebuilt for the new format,
    /// without stopping the engine.
    private func handleRouteChange(reason: UInt) {
        guard core.state == RecorderWire.stateRecording || core.state == RecorderWire.stateSilenced else {
            return
        }

        let rebuilds: [AVAudioSession.RouteChangeReason] = [
            .newDeviceAvailable, .oldDeviceUnavailable, .categoryChange, .override
        ]
        guard let cause = AVAudioSession.RouteChangeReason(rawValue: reason),
            rebuilds.contains(cause)
        else { return }

        do {
            try core.rebuildForRouteChange()
        } catch {
            core.fail(reason: "rebuilding the tap after a route change failed")
        }
    }

    /// The media services daemon restarting destroys the engine; rebuild it
    /// and keep recording.
    private func handleMediaServicesReset() {
        guard core.state != RecorderWire.stateIdle else { return }

        do {
            try AVAudioSession.sharedInstance().setActive(true)
            try core.rebuildAfterMediaReset()
        } catch {
            core.fail(reason: "rebuilding after a media services reset failed")
        }
    }

    /// Native method codes, in wire order.
    private enum Method {
        static let permission: UInt32 = 0
        static let requestPermission: UInt32 = 1
        static let start: UInt32 = 2
        static let stop: UInt32 = 3
        static let status: UInt32 = 4
        static let nextSegment: UInt32 = 5
        static let recover: UInt32 = 6
    }
    #else
    public init() {}

    /// macOS has no continuous recording: every method is unavailable.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard methods.contains(method) else { throw PluginError.unavailable }
        _ = payload.data()
        _ = method
        throw PluginError.unavailable
    }
    #endif
}

#if os(iOS)
/// All recording state, guarded by one lock the audio tap also takes, so
/// segment rotation never races a status read or a delivery.
final class RecorderCore: @unchecked Sendable {
    /// What a status call reports.
    struct Snapshot: Sendable {
        var state: UInt32 = RecorderWire.stateIdle
        var segmentStartedAtMs: Int64 = 0
        var reason: String?
    }

    /// The directory the app last asked to record into, so a recovery can
    /// find leftovers after a restart.
    static let directoryKey = "leptoskit.recorder.directory"

    /// One next-segment waiter, cancellable from any thread and delivering
    /// exactly once.
    private final class Waiter: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<SegmentReply, any Error>?
        private var cancelled = false

        /// Waits until a segment arrives or the call is cancelled.
        func wait() async throws -> SegmentReply {
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    lock.lock()
                    if cancelled {
                        lock.unlock()
                        continuation.resume(throwing: PluginError.cancelled)
                    } else {
                        self.continuation = continuation
                        lock.unlock()
                    }
                }
            } onCancel: {
                self.cancel()
            }
        }

        /// Delivers `reply`; false when the waiter already gave up, in which
        /// case the caller re-queues it.
        func deliver(_ reply: SegmentReply) -> Bool {
            lock.lock()
            guard !cancelled, let continuation = self.continuation else {
                lock.unlock()
                return false
            }
            self.continuation = nil
            lock.unlock()
            continuation.resume(returning: reply)
            return true
        }

        /// Cancels the waiter; a parked continuation resumes with an error.
        func cancel() {
            lock.lock()
            cancelled = true
            let continuation = self.continuation
            self.continuation = nil
            lock.unlock()
            continuation?.resume(throwing: PluginError.cancelled)
        }
    }

    private struct Configuration {
        var directory: URL
        var index: RecorderIndex
        var segmentFrames: Int
        var sampleRate: Int
        var bitrate: UInt32
        var thresholdDbfs: Double
    }

    private let lock = NSLock()
    private var configuration: Configuration?
    private var engine: AVAudioEngine?
    private var converter: AVAudioConverter?
    private var outputFormat: AVAudioFormat?
    private var writer: AVAudioFile?
    private var levels = SegmentLevels(thresholdDbfs: 0)
    private var pendingSamples: [Float] = []
    private var framesWritten = 0
    private var segmentFilename: String?
    private var segmentStartMs: Int64 = 0
    private var snapshot = Snapshot()
    private var waiter: Waiter?
    private var undelivered: [SegmentReply] = []

    /// The state a status call reports.
    var state: UInt32 {
        lock.withLock { snapshot.state }
    }

    /// The full status snapshot.
    func status() -> Snapshot {
        lock.withLock { snapshot }
    }

    // MARK: Session lifecycle

    /// Starts the engine and the first segment.
    func start(
        directory: URL,
        segmentSecs: UInt32,
        sampleRate: UInt32,
        bitrate: UInt32,
        thresholdDbfs: Double
    ) throws {
        try lock.withLock {
            guard snapshot.state == RecorderWire.stateIdle
                || snapshot.state == RecorderWire.stateFailed
            else { throw PluginError.busy }

            guard let format = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: Double(sampleRate),
                channels: 1,
                interleaved: false
            ) else { throw PluginError.native }

            configuration = Configuration(
                directory: directory,
                index: try RecorderIndex(url: directory.appendingPathComponent("recorder-index")),
                segmentFrames: Int(segmentSecs) * Int(sampleRate),
                sampleRate: Int(sampleRate),
                bitrate: bitrate,
                thresholdDbfs: thresholdDbfs
            )
            outputFormat = format

            do {
                try installEngineLocked()
                snapshot = Snapshot(state: RecorderWire.stateRecording)
            } catch {
                failLocked(reason: "starting the audio engine failed")
                throw PluginError.native
            }
        }
    }

    /// Stops the engine and finishes the current segment so it is delivered.
    func stop() throws {
        lock.withLock {
            guard snapshot.state != RecorderWire.stateIdle else { return }

            do {
                try finaliseLocked(endedBy: RecorderWire.endedStop)
            } catch {
                failLocked(reason: "finishing the last segment failed")
            }
            teardownLocked()
            snapshot = Snapshot()
        }
    }

    /// An interruption began: close the segment and wait for its end.
    func interrupt() {
        lock.withLock {
            guard snapshot.state == RecorderWire.stateRecording
                || snapshot.state == RecorderWire.stateSilenced
            else { return }

            do {
                try finaliseLocked(endedBy: RecorderWire.endedInterruption)
            } catch {
                failLocked(reason: "finishing the interrupted segment failed")
                return
            }
            engine?.pause()
            snapshot.state = RecorderWire.stateInterrupted
            snapshot.segmentStartedAtMs = 0
        }
    }

    /// The interruption ended and the system allows resuming.
    func resume() throws {
        try lock.withLock {
            guard snapshot.state == RecorderWire.stateInterrupted, let engine else {
                throw PluginError.unavailable
            }

            do {
                try openSegmentLocked()
                try engine.start()
                snapshot.state = RecorderWire.stateRecording
            } catch {
                failLocked(reason: "resuming the audio engine failed")
                throw PluginError.native
            }
        }
    }

    /// The input route changed: rotate and rebuild the tap for the new
    /// format, without stopping the engine.
    func rebuildForRouteChange() throws {
        try lock.withLock {
            guard snapshot.state == RecorderWire.stateRecording
                || snapshot.state == RecorderWire.stateSilenced,
                let engine
            else { return }

            do {
                try finaliseLocked(endedBy: RecorderWire.endedRotation)
                try installTapLocked(on: engine)
                try openSegmentLocked()
            } catch {
                failLocked(reason: "rebuilding the tap failed")
                throw PluginError.native
            }
        }
    }

    /// The media services were reset: rebuild the whole engine.
    func rebuildAfterMediaReset() throws {
        try lock.withLock {
            guard snapshot.state != RecorderWire.stateIdle else { return }

            let wasInterrupted = snapshot.state == RecorderWire.stateInterrupted
            // The reset already destroyed the writer; the journal's open
            // line keeps recovery honest about what was lost.
            try? finaliseLocked(endedBy: RecorderWire.endedInterruption)
            teardownLocked()

            do {
                try installEngineLocked()
                snapshot = Snapshot(
                    state: wasInterrupted
                        ? RecorderWire.stateInterrupted
                        : RecorderWire.stateRecording
                )
                if !wasInterrupted { try openSegmentLocked() }
            } catch {
                failLocked(reason: "rebuilding the audio engine failed")
                throw PluginError.native
            }
        }
    }

    /// Records a failure, tears the engine down, and keeps the reason for
    /// status calls. Finalised-but-undelivered segments stay deliverable.
    func fail(reason: String) {
        lock.withLock {
            failLocked(reason: reason)
        }
    }

    private func failLocked(reason: String) {
        if segmentFilename != nil {
            try? finaliseLocked(endedBy: RecorderWire.endedStop)
        }
        teardownLocked()
        snapshot = Snapshot(state: RecorderWire.stateFailed, reason: reason)
    }

    // MARK: Recovery

    /// Scans `directory` for an earlier session's leftovers and re-queues
    /// them, deleting files that never finished writing.
    func recover(directory: URL) throws -> (recovered: Int, discarded: Int) {
        try lock.withLock {
            guard snapshot.state == RecorderWire.stateIdle
                || snapshot.state == RecorderWire.stateFailed
            else { throw PluginError.busy }

            let index = try RecorderIndex(url: directory.appendingPathComponent("recorder-index"))
            return try recoverLocked(directory: directory, index: index)
        }
    }

    /// Recovers leftovers in the directory the app last used.
    func recoverLeftovers() throws -> (recovered: Int, discarded: Int) {
        try lock.withLock {
            guard snapshot.state == RecorderWire.stateIdle
                || snapshot.state == RecorderWire.stateFailed
            else { throw PluginError.busy }

            guard let remembered = UserDefaults.standard.string(forKey: Self.directoryKey) else {
                return (0, 0)
            }
            let directory = URL(fileURLWithPath: remembered)
            let index: RecorderIndex
            if let configuration {
                index = configuration.index
            } else {
                index = try RecorderIndex(url: directory.appendingPathComponent("recorder-index"))
            }
            return try recoverLocked(directory: directory, index: index)
        }
    }

    private func recoverLocked(
        directory: URL,
        index: RecorderIndex
    ) throws -> (recovered: Int, discarded: Int) {
        let files = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
        let plan = RecorderIndex.plan(lines: try index.read(), existingFiles: files)

        for record in plan.deliverable {
            let url = directory.appendingPathComponent(record.filename)
            enqueueLocked(
                SegmentReply(
                    path: url.path,
                    startedAtMs: record.startedAtMs,
                    durationMs: record.durationMs,
                    bytes: max(record.bytes, 1),
                    peakDbfs: record.peakDbfs,
                    activeMs: record.activeMs,
                    endedBy: RecorderWire.endedRecovered
                )
            )
        }

        for file in plan.discardable {
            // A truncated file cannot be read; delete it and report it once.
            try? FileManager.default.removeItem(at: directory.appendingPathComponent(file))
        }

        try index.compact(keeping: plan.deliverable)
        return (plan.deliverable.count, plan.discardable.count)
    }

    // MARK: Delivery

    /// Takes the next finished segment, when one is already waiting.
    func dequeueSegment() -> SegmentReply? {
        lock.withLock {
            guard !undelivered.isEmpty else { return nil }
            return undelivered.removeFirst()
        }
    }

    /// Waits for the next finished segment; cancelling abandons the wait
    /// without losing the segment.
    func awaitSegment() async throws -> SegmentReply {
        let launch = try lock.withLock { () -> Launch in
            if let reply = undelivered.first {
                undelivered.removeFirst()
                return .ready(reply)
            }
            guard waiter == nil else { throw PluginError.busy }
            let waiter = Waiter()
            self.waiter = waiter
            return .parking(waiter)
        }

        switch launch {
        case .ready(let reply):
            return reply
        case .parking(let waiter):
            defer {
                lock.withLock {
                    if waiter === self.waiter {
                        self.waiter = nil
                    }
                }
            }
            return try await waiter.wait()
        }
    }

    /// Whether a parked waiter or a ready reply starts the wait.
    private enum Launch {
        case ready(SegmentReply)
        case parking(Waiter)
    }

    /// Cancels a waiting next-segment call, if any.
    func cancelWaiter() {
        lock.withLock {
            waiter?.cancel()
            waiter = nil
        }
    }

    /// Marks the file behind a delivered segment, so a later recovery does
    /// not deliver it twice.
    func markDelivered(_ reply: SegmentReply) {
        let index = lock.withLock { configuration?.index }
        guard let index else { return }
        let filename = (reply.path as NSString).lastPathComponent
        try? index.recordDelivered(filename: filename)
    }

    /// Queues one finished segment for delivery and wakes a waiter.
    private func enqueueLocked(_ reply: SegmentReply) {
        undelivered.append(reply)
        guard let waiter else { return }
        self.waiter = nil
        let head = undelivered.removeFirst()
        if !waiter.deliver(head) {
            // The waiter already gave up; the segment stays queued.
            undelivered.insert(head, at: 0)
        }
    }

    // MARK: Engine plumbing

    private func installEngineLocked() throws {
        let engine = AVAudioEngine()
        self.engine = engine
        try installTapLocked(on: engine)
        try engine.start()
        // Opening the segment after the engine starts means the first tap
        // callback, which the lock serialises, always has a writer.
        try openSegmentLocked()
    }

    private func installTapLocked(on engine: AVAudioEngine) throws {
        let input = engine.inputNode
        let inputFormat = input.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0, inputFormat.channelCount >= 1, let outputFormat else {
            throw PluginError.native
        }

        converter = AVAudioConverter(from: inputFormat, to: outputFormat)
        input.installTap(onBus: 0, bufferSize: 4_096, format: inputFormat) { [weak self] buffer, _ in
            self?.process(buffer)
        }
    }

    private func teardownLocked() {
        if let engine {
            engine.inputNode.removeTap(onBus: 0)
            engine.stop()
        }
        engine = nil
        converter = nil
        writer = nil
        segmentFilename = nil
        snapshot.segmentStartedAtMs = 0
    }

    /// One tap callback: convert to the target format, measure each
    /// 30-millisecond frame, and write it to whichever segment covers it.
    private func process(_ buffer: AVAudioPCMBuffer) {
        lock.withLock {
            guard snapshot.state == RecorderWire.stateRecording
                || snapshot.state == RecorderWire.stateSilenced,
                let converter,
                let outputFormat
            else { return }

            guard let samples = convert(buffer, with: converter, to: outputFormat) else { return }
            pendingSamples.append(contentsOf: samples)

            let frameLength = RecorderLevels.frameSamples(sampleRate: Int(outputFormat.sampleRate))
            while pendingSamples.count >= frameLength {
                let frame = pendingSamples[0..<frameLength]
                levels.add(RecorderLevels.measure(frame))
                writeLocked(frame)
                pendingSamples.removeFirst(frameLength)
            }
        }
    }

    /// Converts one tap buffer to mono float samples at the target rate.
    private func convert(
        _ buffer: AVAudioPCMBuffer,
        with converter: AVAudioConverter,
        to format: AVAudioFormat
    ) -> [Float]? {
        let ratio = format.sampleRate / max(buffer.format.sampleRate, 1)
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 64
        guard let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else {
            return nil
        }

        var fed = false
        var failure: NSError?
        let status = converter.convert(to: output, error: &failure) { _, _ in
            if fed { return nil }
            fed = true
            return buffer
        }

        guard failure == nil, status != .error, output.frameLength > 0,
            let channel = output.floatChannelData?[0]
        else {
            if status == .error { failLocked(reason: "converting microphone audio failed") }
            return nil
        }
        return [Float](UnsafeBufferPointer(start: channel, count: Int(output.frameLength)))
    }

    /// Writes one measured frame, rotating at the boundary so no gap opens
    /// between segments.
    private func writeLocked(_ frame: ArraySlice<Float>) {
        guard let configuration else { return }
        var offset = frame.startIndex

        while offset < frame.endIndex {
            guard let writer else { return }
            let count = frame.endIndex - offset
            let take = RecorderLevels.split(
                written: framesWritten,
                count: count,
                segmentFrames: configuration.segmentFrames
            ) ?? count
            let chunk = frame[offset..<(offset + take)]

            do {
                try write(chunk, to: writer)
                framesWritten += take
                offset = chunk.endIndex

                if framesWritten >= configuration.segmentFrames {
                    try finaliseLocked(endedBy: RecorderWire.endedRotation)
                    _ = try openSegmentLocked()
                }
            } catch {
                failLocked(reason: "writing a segment failed")
                return
            }
        }
    }

    /// Writes raw samples into the current file.
    private func write(_ samples: ArraySlice<Float>, to file: AVAudioFile) throws {
        guard let outputFormat else { throw PluginError.native }
        let frames = AVAudioFrameCount(samples.count)
        guard frames > 0,
            let buffer = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: frames),
            let channel = buffer.floatChannelData?[0]
        else { throw PluginError.native }

        buffer.frameLength = frames
        samples.withUnsafeBufferPointer { source in
            if let base = source.baseAddress {
                channel.update(from: base, count: samples.count)
            }
        }
        try file.write(from: buffer)
    }

    /// Closes the current segment and queues it for delivery.
    private func finaliseLocked(endedBy: UInt32) throws {
        guard let configuration, let filename = segmentFilename else { return }
        let startedAtMs = segmentStartMs
        writer = nil
        segmentFilename = nil
        segmentStartMs = 0
        framesWritten = 0

        let url = configuration.directory.appendingPathComponent(filename)
        let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) ?? 0
        try configuration.index.recordFinalised(
            filename: filename,
            durationMs: levels.durationMs,
            bytes: Int64(size),
            peakDbfs: levels.peakDbfs,
            activeMs: levels.activeMs,
            endedBy: endedBy
        )
        enqueueLocked(
            SegmentReply(
                path: url.path,
                startedAtMs: startedAtMs,
                durationMs: levels.durationMs,
                bytes: Int64(max(size, 1)),
                peakDbfs: levels.peakDbfs,
                activeMs: levels.activeMs,
                endedBy: endedBy
            )
        )
        snapshot.segmentStartedAtMs = 0
    }

    /// Opens the next segment file and its journal line.
    @discardableResult
    private func openSegmentLocked() throws -> AVAudioFile {
        guard let configuration, let outputFormat else { throw PluginError.native }

        let startedMs = Int64(Date().timeIntervalSince1970 * 1000)
        var filename = "seg-\(startedMs).m4a"
        var url = configuration.directory.appendingPathComponent(filename)
        var attempt = 1
        while FileManager.default.fileExists(atPath: url.path) {
            attempt += 1
            filename = "seg-\(startedMs)-\(attempt).m4a"
            url = configuration.directory.appendingPathComponent(filename)
        }

        try configuration.index.recordOpen(
            filename: filename,
            startedMs: startedMs,
            plannedMs: Int64(configuration.segmentFrames) * 1_000 / Int64(configuration.sampleRate)
        )

        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: outputFormat.sampleRate,
            AVNumberOfChannelsKey: 1,
            AVEncoderBitRateKey: Int(configuration.bitrate),
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue
        ]
        let file = try AVAudioFile(forWriting: url, settings: settings)
        writer = file
        levels = SegmentLevels(thresholdDbfs: configuration.thresholdDbfs)
        framesWritten = 0
        segmentFilename = filename
        segmentStartMs = startedMs
        snapshot.segmentStartedAtMs = startedMs
        return file
    }
}
#endif

extension NativeBuffer {
    /// The whole request payload as `Data`.
    func data() -> Data {
        withUnsafeBytes { Data($0) }
    }
}
