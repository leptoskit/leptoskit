import Foundation
import LeptosKitBridge
import SafariServices
#if canImport(UIKit)
import UIKit
#endif

@MainActor
protocol WebBrowserService {
    func open(_ url: URL, from owner: NativePresentationOwner) async throws
}

#if canImport(UIKit)
@MainActor
private final class SafariBrowserService: NSObject, WebBrowserService {
    private var continuation: CheckedContinuation<Void, Error>?
    private weak var controller: SFSafariViewController?

    func open(_ url: URL, from owner: NativePresentationOwner) async throws {
        guard owner.presentedViewController == nil else { throw PluginError.busy }

        let controller = SFSafariViewController(url: url)
        controller.delegate = self
        self.controller = controller

        try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            owner.present(controller, animated: true)
        }
    }
}

extension SafariBrowserService: @MainActor SFSafariViewControllerDelegate {
    func safariViewControllerDidFinish(_ controller: SFSafariViewController) {
        guard self.controller === controller else { return }

        finish()
    }

    func dismiss() {
        controller?.dismiss(animated: false)
        finish()
    }

    private func finish() {
        let pending = continuation
        continuation = nil
        controller = nil
        pending?.resume()
    }
}
#else
@MainActor
private struct UnavailableBrowserService: WebBrowserService {
    func open(_ url: URL, from owner: NativePresentationOwner) async throws {
        throw PluginError.unavailable
    }
}
#endif

/// Opens HTTP(S) in Safari View Controller and answers on dismiss.
/// Distinct from web-auth (no return address) and linking.open (no dismiss wait).
@MainActor
public final class WebBrowserPlugin: NativePlugin {
    public let name = "web-browser"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private let service: any WebBrowserService
    private var busy = false
    private var closed = false
    private var invocation: UUID?
    #if canImport(UIKit)
    private var safari: SafariBrowserService?
    #endif

    public init(context: NativePluginContext) {
        self.context = context
        #if canImport(UIKit)
        let safari = SafariBrowserService()
        self.safari = safari
        service = safari
        #else
        service = UnavailableBrowserService()
        #endif
    }

    init(context: NativePluginContext, service: any WebBrowserService) {
        self.context = context
        self.service = service
    }

    public func close() {
        closed = true
        #if canImport(UIKit)
        safari?.dismiss()
        #endif
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return encode(availability())
        }

        let open = try payload.withUnsafeBytes(WebBrowserOpen.decode)
        try Task.checkCancellation()

        return try await present(open.url)
    }

    private func availability() -> WebBrowserAvailability {
        #if canImport(UIKit)
        if busy { return .busy }

        do {
            _ = try context.requirePresentationOwner()

            return .ready
        } catch {
            return .inactive
        }
        #else
        return .unsupported
        #endif
    }

    private func present(_ url: URL) async throws -> Data {
        if busy { throw PluginError.busy }

        let owner = try context.requirePresentationOwner()
        let invocation = UUID()
        self.invocation = invocation
        busy = true
        defer {
            busy = false
            self.invocation = nil
        }

        try await withTaskCancellationHandler {
            try await service.open(url, from: owner)
        } onCancel: {
            Task { @MainActor [weak self] in
                guard let self, self.invocation == invocation else { return }
                #if canImport(UIKit)
                self.safari?.dismiss()
                #endif
            }
        }

        if closed { throw PluginError.cancelled }

        return Data()
    }

    private func encode(_ value: WebBrowserAvailability) -> Data {
        var raw = value.rawValue.littleEndian

        return withUnsafeBytes(of: &raw) { Data($0) }
    }
}
