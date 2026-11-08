import AVFoundation
import Foundation
import LeptosKitBridge

/// Host-owned `AVPlayer` registry with create/replace/play/pause/seek and status.
@MainActor
public final class VideoPlugin: NativePlugin {
    public let name = "video"
    public let methods: [UInt32] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

    private var players: [UInt32: AVPlayer] = [:]
    private var looping: [UInt32: Bool] = [:]
    private var endObservers: [UInt32: NSObjectProtocol] = [:]
    private var nextId: UInt32 = 1
    private var closed = false

    private enum Method: UInt32 {
        case create = 0
        case replace = 1
        case play = 2
        case pause = 3
        case seek = 4
        case setMuted = 5
        case setLoop = 6
        case setRate = 7
        case status = 8
        case release = 9
    }

    public init() {
        NativeVideoBinding.player = { [weak self] id in self?.players[id] }
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard let method = Method(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .create: return try create(VideoWire.source(payload))
        case .replace:
            let (id, source) = try VideoWire.replace(payload)
            try replace(id, source)
            return Data()
        case .play:
            try play(VideoWire.player(payload))
            return Data()
        case .pause:
            try pause(VideoWire.player(payload))
            return Data()
        case .seek:
            let seek = try VideoWire.seek(payload)
            try self.seek(seek.id, seek.positionMs)
            return Data()
        case .setMuted:
            let flag = try VideoWire.flag(payload)
            try setMuted(flag.id, flag.enabled)
            return Data()
        case .setLoop:
            let flag = try VideoWire.flag(payload)
            try setLoop(flag.id, flag.enabled)
            return Data()
        case .setRate:
            let rate = try VideoWire.rate(payload)
            try setRate(rate.id, rate.thousandths)
            return Data()
        case .status: return try status(VideoWire.player(payload))
        case .release:
            try release(VideoWire.player(payload))
            return Data()
        }
    }

    public func close() {
        closed = true
        for id in Array(players.keys) {
            try? release(id)
        }
        NativeVideoBinding.player = nil
    }

    private func create(_ url: URL) throws -> Data {
        let id = nextId
        nextId = nextId &+ 1
        if nextId == 0 { nextId = 1 }
        let player = AVPlayer(url: url)
        players[id] = player
        looping[id] = false
        observeEnd(id, player)
        return VideoWire.playerId(id)
    }

    private func replace(_ id: UInt32, _ url: URL) throws {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        player.replaceCurrentItem(with: AVPlayerItem(url: url))
        observeEnd(id, player)
    }

    private func play(_ id: UInt32) throws {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        player.play()
    }

    private func pause(_ id: UInt32) throws {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        player.pause()
    }

    private func seek(_ id: UInt32, _ positionMs: UInt32) throws {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        let seconds = Double(positionMs) / 1_000
        let time = CMTime(seconds: seconds, preferredTimescale: 1_000)
        player.seek(to: time, toleranceBefore: .zero, toleranceAfter: .zero)
    }

    private func setMuted(_ id: UInt32, _ muted: Bool) throws {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        player.isMuted = muted
    }

    private func setLoop(_ id: UInt32, _ enabled: Bool) throws {
        guard players[id] != nil else { throw PluginError.invalidArgument }
        looping[id] = enabled
    }

    private func setRate(_ id: UInt32, _ thousandths: UInt32) throws {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        player.rate = Float(thousandths) / 1_000
    }

    private func status(_ id: UInt32) throws -> Data {
        guard let player = players[id] else { throw PluginError.invalidArgument }
        let item = player.currentItem
        let current = Self.milliseconds(player.currentTime())
        let duration: UInt32
        if let item, item.duration.isNumeric {
            duration = Self.milliseconds(item.duration)
        } else {
            duration = VideoWire.timeUnknown
        }
        let playing = player.rate != 0 && player.error == nil
        return VideoWire.status(playing: playing, currentMs: current, durationMs: duration)
    }

    private func release(_ id: UInt32) throws {
        guard players.removeValue(forKey: id) != nil else { throw PluginError.invalidArgument }
        looping.removeValue(forKey: id)
        if let observer = endObservers.removeValue(forKey: id) {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    private func observeEnd(_ id: UInt32, _ player: AVPlayer) {
        if let observer = endObservers.removeValue(forKey: id) {
            NotificationCenter.default.removeObserver(observer)
        }
        guard let item = player.currentItem else { return }
        let observer = NotificationCenter.default.addObserver(
            forName: .AVPlayerItemDidPlayToEndTime,
            object: item,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.looping[id] == true, let player = self.players[id] else { return }
                player.seek(to: .zero)
                player.play()
            }
        }
        endObservers[id] = observer
    }

    private static func milliseconds(_ time: CMTime) -> UInt32 {
        guard time.isNumeric else { return VideoWire.timeUnknown }
        let value = time.seconds * 1_000
        guard value.isFinite, value >= 0, value < Double(UInt32.max) else {
            return VideoWire.timeUnknown
        }
        return UInt32(value.rounded())
    }
}
