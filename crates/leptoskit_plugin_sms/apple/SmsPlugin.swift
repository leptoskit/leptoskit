import Foundation
import LeptosKitBridge
#if canImport(MessageUI) && os(iOS)
import MessageUI
import UIKit
#endif

@MainActor
protocol SmsComposerService {
    var canSend: Bool { get }
    func present(_ compose: SmsCompose, from owner: NativePresentationOwner) async throws
}

#if canImport(MessageUI) && os(iOS)
@MainActor
private final class SystemSmsComposer: NSObject, SmsComposerService {
    var canSend: Bool { MFMessageComposeViewController.canSendText() }
    private var continuation: CheckedContinuation<Void, Error>?

    func present(_ compose: SmsCompose, from owner: NativePresentationOwner) async throws {
        guard canSend else { throw PluginError.unavailable }

        guard owner.presentedViewController == nil else { throw PluginError.busy }

        let controller = MFMessageComposeViewController()
        controller.messageComposeDelegate = self
        controller.recipients = compose.recipients
        controller.body = compose.body

        try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            owner.present(controller, animated: true)
        }
    }
}

extension SystemSmsComposer: @MainActor MFMessageComposeViewControllerDelegate {
    func messageComposeViewController(
        _ controller: MFMessageComposeViewController,
        didFinishWith result: MessageComposeResult
    ) {
        controller.dismiss(animated: true)
        let pending = continuation
        continuation = nil

        switch result {
        case .cancelled:
            pending?.resume(throwing: PluginError.cancelled)
        case .sent, .failed:
            // OS callback only: failed/sent never mean delivery confirmation.
            pending?.resume()
        @unknown default:
            pending?.resume(throwing: PluginError.native)
        }
    }
}
#else
@MainActor
private struct UnavailableSmsComposer: SmsComposerService {
    var canSend: Bool { false }
    func present(_ compose: SmsCompose, from owner: NativePresentationOwner) async throws {
        throw PluginError.unavailable
    }
}
#endif

/// Presents the system SMS composer. Completion never confirms delivery.
@MainActor
public final class SmsPlugin: NativePlugin {
    public let name = "sms"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private let service: any SmsComposerService
    private var busy = false
    private var closed = false

    public init(context: NativePluginContext) {
        self.context = context
        #if canImport(MessageUI) && os(iOS)
        service = SystemSmsComposer()
        #else
        service = UnavailableSmsComposer()
        #endif
    }

    init(context: NativePluginContext, service: any SmsComposerService) {
        self.context = context
        self.service = service
    }

    public func close() {
        closed = true
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return encode(availability())
        }

        let compose = try payload.withUnsafeBytes(SmsCompose.decode)
        try Task.checkCancellation()

        return try await present(compose)
    }

    private func availability() -> SmsAvailability {
        #if canImport(MessageUI) && os(iOS)
        if busy { return .busy }

        if !service.canSend { return .absent }

        do {
            _ = try context.requirePresentationOwner()

            return .ready
        } catch PluginError.unavailable {
            return .inactive
        } catch {
            return .inactive
        }
        #else
        return .unsupported
        #endif
    }

    private func present(_ compose: SmsCompose) async throws -> Data {
        if busy { throw PluginError.busy }

        let owner = try context.requirePresentationOwner()

        guard service.canSend else { throw PluginError.unavailable }

        busy = true
        defer { busy = false }
        try await service.present(compose, from: owner)

        return Data()
    }

    private func encode(_ value: SmsAvailability) -> Data {
        var raw = value.rawValue.littleEndian

        return withUnsafeBytes(of: &raw) { Data($0) }
    }
}
