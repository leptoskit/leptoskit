import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The request framing this host reads.
private enum Wire {
    /// The only method: present the system share UI.
    static let share: UInt32 = 0
}

/// Places the popover anchor inside the presenting view.
enum ShareAnchor {
    /// The guest's rectangle, clipped to `bounds`, or a point at the middle without one.
    /// An origin outside the view is refused rather than silently moved.
    static func rect(_ anchor: CGRect?, in bounds: CGRect) throws -> CGRect {
        guard let anchor else { return CGRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1) }

        guard (bounds.minX...bounds.maxX).contains(anchor.minX), (bounds.minY...bounds.maxY).contains(anchor.minY)
        else { throw PluginError.invalidArgument }

        return CGRect(x: anchor.minX, y: anchor.minY, width: min(anchor.width, bounds.maxX - anchor.minX),
                      height: min(anchor.height, bounds.maxY - anchor.minY))
    }
}

/// Shares text, a web URL, or files through the platform's own UI, and reports the choice.
@MainActor
public final class SharingPlugin: NSObject, NativePlugin {
    public let name = "sharing"
    public let methods: [UInt32] = [Wire.share]
    private let context: NativePluginContext
    private let staging: () throws -> ShareStaging
    private var session: ShareSession?
    private var preparing = false
    private var currentInvocation: UUID?
    private var closed = false

    #if canImport(UIKit)
    private var activity: UIActivityViewController?
    #elseif canImport(AppKit)
    private var picker: NSSharingServicePicker?
    private var service: NSSharingService?
    private var serviceDelegate: ShareServiceDelegate?
    private var subject: String?
    #endif

    public convenience init(context: NativePluginContext) {
        self.init(context: context, staging: ShareStaging.standard)
    }

    init(context: NativePluginContext, staging: @escaping () throws -> ShareStaging) {
        self.context = context
        self.staging = staging
    }

    public func close() {
        closed = true
        cancel()
        // AppKit cannot cancel a service after the user has chosen it. That
        // external compose UI now belongs to the service, not this runtime.
        finish(.failure(PluginError.cancelled))
    }

    /// Presents the share UI for one caller's content, and answers how it ended.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method == Wire.share else { throw PluginError.unavailable }

        guard session == nil, !preparing else { throw PluginError.busy }

        let content = try payload.withUnsafeBytes(ShareContent.decode)
        _ = try context.requirePresentationOwner()

        try Task.checkCancellation()
        let invocation = UUID()
        currentInvocation = invocation

        return try await withTaskCancellationHandler {
            let items = try await prepare(content)
            let owner = try context.requirePresentationOwner()

            guard !closed, !Task.isCancelled else { throw PluginError.cancelled }

            return try await present(content, items: items, owner: owner)
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.cancel(invocation: invocation)
            }
        }
    }

    /// Copies files into the private share cache off the main actor, then lists the sheet's items.
    private func prepare(_ content: ShareContent) async throws -> [Any] {
        var files: [URL] = []

        if !content.files.isEmpty {
            preparing = true
            defer { preparing = false }

            let staging = try staging()
            let task = Task.detached { try staging.stage(content.files) }
            files = try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
        }

        var items: [Any] = []

        if let text = content.text { items.append(text) }

        if let url = content.url { items.append(url) }

        return items + files.map { $0 as Any }
    }

    /// Cancels the presentation only while this call still owns it.
    private func cancel(invocation: UUID) {
        guard currentInvocation == invocation else { return }

        cancel()
    }

    private func cancel() {
        dismissChooser()
    }

    private func finish(_ result: Result<Data, Error>) {
        let pending = session
        session = nil
        currentInvocation = nil
        releaseChooser()
        pending?.complete(result)
    }

    #if canImport(UIKit)
    /// Presents the activity sheet and answers when it completes.
    private func present(_ content: ShareContent, items: [Any], owner: NativePresentationOwner) async throws -> Data {
        guard isIdle(owner) else { throw PluginError.busy }

        let activity = try makeActivity(content, items: items, owner: owner)
        self.activity = activity

        return try await withCheckedThrowingContinuation { continuation in
            let session = ShareSession(continuation)
            self.session = session
            activity.completionWithItemsHandler = { [weak self] type, completed, _, error in
                self?.completed(session, type: type?.rawValue, completed: completed, error: error)
            }
            owner.present(activity, animated: true)
            activity.presentationController?.delegate = self
        }
    }

    /// A presenter that is already showing, dismissing, or presenting cannot host the sheet.
    private func isIdle(_ owner: NativePresentationOwner) -> Bool {
        owner.presentedViewController == nil && !owner.isBeingDismissed && !owner.isBeingPresented
            && owner.transitionCoordinator == nil
    }

    /// Builds the sheet; only iPad presents it as a popover, anchored where the guest asked.
    private func makeActivity(
        _ content: ShareContent, items: [Any], owner: NativePresentationOwner
    ) throws -> UIActivityViewController {
        let sources = items.enumerated().map { index, item in
            ShareItemSource(item: item, subject: content.subject, title: index == 0 ? content.title : nil)
        }
        let activity = UIActivityViewController(activityItems: sources, applicationActivities: nil)
        activity.excludedActivityTypes = ShareActivities.types(content.excluded)

        // On iPhone the controller can expose a non-popover presentation controller;
        // configuring it would push the sheet onto the anchored path.
        if owner.traitCollection.userInterfaceIdiom == .pad, let popover = activity.popoverPresentationController {
            popover.sourceView = owner.view
            popover.sourceRect = try ShareAnchor.rect(content.anchor, in: owner.view.bounds)
        }

        return activity
    }

    /// Answers the one session the sheet belongs to.
    private func completed(_ session: ShareSession, type: String?, completed: Bool, error: (any Error)?) {
        guard self.session?.id == session.id else { return }

        finish(Result { try ShareReply.activity(type: type, completed: completed, failed: error != nil).encoded() })
    }

    private func dismissChooser() {
        activity?.dismiss(animated: false)
        finish(.failure(PluginError.cancelled))
    }

    private func releaseChooser() {
        activity?.completionWithItemsHandler = nil
        activity = nil
    }
    #elseif canImport(AppKit)
    /// Presents the sharing picker at the anchor, or beside the view's centre.
    private func present(_ content: ShareContent, items: [Any], owner: NativePresentationOwner) async throws -> Data {
        guard let window = owner.view.window, window.attachedSheet == nil else { throw PluginError.busy }

        guard !NSSharingService.sharingServices(forItems: items).isEmpty else {
            throw PluginError.unavailable
        }

        let anchor = try ShareAnchor.rect(content.anchor, in: owner.view.bounds)
        let picker = NSSharingServicePicker(items: items)
        picker.delegate = self
        self.picker = picker
        // macOS services have one subject field; a title stands in when there is no subject.
        subject = content.subject ?? content.title

        return try await withCheckedThrowingContinuation { continuation in
            session = ShareSession(continuation)
            picker.show(relativeTo: anchor, of: owner.view, preferredEdge: .minY)
        }
    }

    /// AppKit cannot cancel a service after the user has chosen it.
    private func dismissChooser() {
        guard service == nil else {
            // Retain busy admission until the service ends; cancellation is not
            // a promise to retract content already handed to that service.
            session?.complete(.failure(PluginError.cancelled))

            return
        }

        picker?.close()
        finish(.failure(PluginError.cancelled))
    }

    private func releaseChooser() {
        picker?.delegate = nil
        picker = nil
        service?.delegate = nil
        service = nil
        serviceDelegate = nil
        subject = nil
    }
    #else
    private func present(_ content: ShareContent, items: [Any], owner: NativePresentationOwner) async throws -> Data {
        throw PluginError.unavailable
    }

    private func dismissChooser() {}

    private func releaseChooser() {}
    #endif
}

#if canImport(UIKit)
extension SharingPlugin: UIAdaptivePresentationControllerDelegate {
    public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        guard presentationController.presentedViewController === activity else { return }

        finish(.success(ShareReply.dismissed.encoded()))
    }
}
#elseif canImport(AppKit)
// AppKit invokes picker delegates on its UI thread; the older protocol lacks
// actor annotations. Swift checks this main-actor isolation at runtime.
extension SharingPlugin: @preconcurrency NSSharingServicePickerDelegate {
    public func sharingServicePicker(
        _ sharingServicePicker: NSSharingServicePicker,
        delegateFor sharingService: NSSharingService
    ) -> (any NSSharingServiceDelegate)? {
        guard sharingServicePicker === picker, let id = session?.id else { return nil }

        service = sharingService

        if let subject { sharingService.subject = subject }

        let delegate = ShareServiceDelegate(context: context) { [weak self] result in
            self?.serviceEnded(id, result: result)
        }
        serviceDelegate = delegate

        return delegate
    }

    /// Answers the session the chosen service belongs to.
    private func serviceEnded(_ id: UUID, result: Result<Data, Error>) {
        guard session?.id == id else { return }

        finish(result)
    }

    public func sharingServicePicker(
        _ sharingServicePicker: NSSharingServicePicker,
        didChoose sharingService: NSSharingService?
    ) {
        guard sharingServicePicker === picker else { return }

        if sharingService == nil { finish(.success(ShareReply.dismissed.encoded())) }
    }
}
#endif
