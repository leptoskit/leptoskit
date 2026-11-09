import CLeptosKitBridge
import Foundation
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The Apple host's records, written through leptoskit's logger so they reach
/// the unified log at the runtime's level: subsystem `dev.leptoskit`, under
/// the same `leptoskit/<part>` categories as the runtime's own records.
///
/// A failure's record carries the whole error: each error's type, domain,
/// code, and description, and every error underneath it.
public enum NativeLog {
    public static let bridge = "leptoskit/bridge"
    public static let gpu = "leptoskit/gpu"
    public static let render = "leptoskit/render"

    /// How many underlying errors ``describe(_:_:)`` follows.
    private static let maximumDepth = 8

    public static func info(_ category: String, _ message: String) {
        write(level: 3, category: category, message: message)
    }

    public static func warn(_ category: String, _ message: String, _ error: Error? = nil) {
        write(level: 2, category: category, message: describe(message, error))
    }

    public static func error(_ category: String, _ message: String, _ error: Error? = nil) {
        write(level: 1, category: category, message: describe(message, error))
    }

    /// `message`, then `error` and each error under it, one per line.
    public static func describe(_ message: String, _ error: Error?) -> String {
        guard let error else { return message }
        var lines = ["\(message): \(line(error))"]
        var causes = underlying(error)
        var depth = 0

        while !causes.isEmpty, depth < maximumDepth {
            let cause = causes.removeFirst()
            lines.append("caused by \(line(cause))")
            causes.append(contentsOf: underlying(cause))
            depth += 1
        }

        return lines.joined(separator: "\n")
    }

    /// Logs the app's pause and resume at info level, once per process.
    @MainActor
    public static func followLifecycle() {
        _ = lifecycle
    }

    @MainActor
    private static let lifecycle: [NSObjectProtocol] = {
        #if canImport(UIKit)
        let (paused, resumed) = (
            UIApplication.willResignActiveNotification, UIApplication.didBecomeActiveNotification
        )
        #elseif canImport(AppKit)
        let (paused, resumed) = (
            NSApplication.didResignActiveNotification, NSApplication.didBecomeActiveNotification
        )
        #endif
        let center = NotificationCenter.default

        return [
            center.addObserver(forName: paused, object: nil, queue: .main) { _ in
                info(bridge, "paused")
            },
            center.addObserver(forName: resumed, object: nil, queue: .main) { _ in
                info(bridge, "resumed")
            }
        ]
    }()

    private static func line(_ error: Error) -> String {
        let bridged = error as NSError
        return "\(String(reflecting: type(of: error))) (\(bridged.domain) \(bridged.code)): "
            + error.localizedDescription
    }

    /// The errors `error` reports beneath it, through Foundation's keys.
    private static func underlying(_ error: Error) -> [Error] {
        let info = (error as NSError).userInfo
        var causes: [Error] = []

        if let cause = info[NSUnderlyingErrorKey] as? Error { causes.append(cause) }

        if let many = info[NSMultipleUnderlyingErrorsKey] as? [Error] { causes.append(contentsOf: many) }

        return causes
    }

    private static func write(level: UInt32, category: String, message: String) {
        var category = category
        var message = message
        category.withUTF8 { category in
            message.withUTF8 { message in
                lk_log_write(level, category.baseAddress, category.count, message.baseAddress, message.count)
            }
        }
    }
}
