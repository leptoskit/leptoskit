import Foundation
import LeptosKitBridge
#if canImport(MessageUI) && os(iOS)
import MessageUI
import UIKit
#endif

@MainActor
protocol MailComposerService {
    var canSend: Bool { get }
    func present(_ compose: MailCompose, from owner: NativePresentationOwner) async throws
}

#if canImport(MessageUI) && os(iOS)
@MainActor
private final class SystemMailComposer: NSObject, MailComposerService {
    var canSend: Bool { MFMailComposeViewController.canSendMail() }
    private var continuation: CheckedContinuation<Void, Error>?

    func present(_ compose: MailCompose, from owner: NativePresentationOwner) async throws {
        guard canSend else { throw PluginError.unavailable }

        guard owner.presentedViewController == nil else { throw PluginError.busy }

        let controller = MFMailComposeViewController()
        controller.mailComposeDelegate = self
        controller.setToRecipients(compose.to)
        controller.setCcRecipients(compose.cc)
        controller.setBccRecipients(compose.bcc)
        controller.setSubject(compose.subject)
        controller.setMessageBody(compose.body, isHTML: false)

        try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            owner.present(controller, animated: true)
        }
    }
}

extension SystemMailComposer: @MainActor MFMailComposeViewControllerDelegate {
    func mailComposeController(
        _ controller: MFMailComposeViewController,
        didFinishWith result: MFMailComposeResult,
        error: (any Error)?
    ) {
        controller.dismiss(animated: true)
        let pending = continuation
        continuation = nil

        if error != nil {
            pending?.resume(throwing: PluginError.native)

            return
        }

        switch result {
        case .cancelled:
            pending?.resume(throwing: PluginError.cancelled)
        case .saved, .sent, .failed:
            // OS callback only: never a delivery receipt. Attachments are deferred.
            pending?.resume()
        @unknown default:
            pending?.resume(throwing: PluginError.native)
        }
    }
}
#else
@MainActor
private struct UnavailableMailComposer: MailComposerService {
    var canSend: Bool { false }
    func present(_ compose: MailCompose, from owner: NativePresentationOwner) async throws {
        throw PluginError.unavailable
    }
}
#endif

/// Presents the system mail composer. Attachments are deferred in this MVP.
@MainActor
public final class MailComposerPlugin: NativePlugin {
    public let name = "mail-composer"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private let service: any MailComposerService
    private var busy = false
    private var closed = false

    public init(context: NativePluginContext) {
        self.context = context
        #if canImport(MessageUI) && os(iOS)
        service = SystemMailComposer()
        #else
        service = UnavailableMailComposer()
        #endif
    }

    init(context: NativePluginContext, service: any MailComposerService) {
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

        let compose = try payload.withUnsafeBytes(MailCompose.decode)
        try Task.checkCancellation()

        return try await present(compose)
    }

    private func availability() -> MailAvailability {
        #if canImport(MessageUI) && os(iOS)
        if busy { return .busy }

        if !service.canSend { return .absent }

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

    private func present(_ compose: MailCompose) async throws -> Data {
        if busy { throw PluginError.busy }

        let owner = try context.requirePresentationOwner()

        guard service.canSend else { throw PluginError.unavailable }

        busy = true
        defer { busy = false }
        try await service.present(compose, from: owner)

        return Data()
    }

    private func encode(_ value: MailAvailability) -> Data {
        var raw = value.rawValue.littleEndian

        return withUnsafeBytes(of: &raw) { Data($0) }
    }
}
