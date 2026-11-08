import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

@MainActor
protocol PrintService {
    func present(_ job: PrintJob, from owner: NativePresentationOwner) async throws
}

#if canImport(UIKit)
@MainActor
private final class SystemPrintService: PrintService {
    func present(_ job: PrintJob, from owner: NativePresentationOwner) async throws {
        let controller = UIPrintInteractionController.shared

        switch job {
        case let .html(html):
            let formatter = UIMarkupTextPrintFormatter(markupText: html)
            controller.printFormatter = formatter
            controller.printingItem = nil
        case let .pdfUri(uri):
            guard let url = URL(string: uri) else { throw PluginError.invalidArgument }
            controller.printingItem = url as NSURL
            controller.printFormatter = nil
        }

        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            controller.present(animated: true) { _, _, error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume()
                }
            }
        }
    }
}
#elseif canImport(AppKit)
@MainActor
private final class SystemPrintService: PrintService {
    func present(_ job: PrintJob, from owner: NativePresentationOwner) async throws {
        let info = NSPrintInfo.shared
        let view: NSView
        switch job {
        case let .html(html):
            let text = NSTextView(frame: NSRect(x: 0, y: 0, width: 612, height: 792))
            if let data = html.data(using: .utf8) {
                text.replaceCharacters(
                    in: NSRange(location: 0, length: 0),
                    with: NSAttributedString(
                        html: data,
                        options: [.documentType: NSAttributedString.DocumentType.html],
                        documentAttributes: nil
                    ) ?? NSAttributedString(string: html)
                )
            } else {
                text.string = html
            }
            view = text
        case let .pdfUri(uri):
            guard let url = URL(string: uri), let pdf = NSPDFImageRep(contentsOf: url) else {
                throw PluginError.invalidArgument
            }
            let image = NSImage(size: pdf.size)
            image.addRepresentation(pdf)
            let imageView = NSImageView(frame: NSRect(origin: .zero, size: pdf.size))
            imageView.image = image
            view = imageView
        }

        let operation = NSPrintOperation(view: view, printInfo: info)
        operation.showsPrintPanel = true
        operation.showsProgressPanel = true

        let ok = operation.run()
        if !ok {
            // User cancelled or the panel failed; still a completed dialog callback.
        }
        _ = owner
    }
}
#else
@MainActor
private struct UnavailablePrintService: PrintService {
    func present(_ job: PrintJob, from owner: NativePresentationOwner) async throws {
        throw PluginError.unavailable
    }
}
#endif

/// Presents the system print dialog for HTML or a PDF URI.
/// Completion is dialog finish / job acceptance only, never proof of paper output.
@MainActor
public final class PrintPlugin: NativePlugin {
    public let name = "print"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private let service: any PrintService
    private var busy = false
    private var closed = false

    public init(context: NativePluginContext) {
        self.context = context
        #if canImport(UIKit) || canImport(AppKit)
        service = SystemPrintService()
        #else
        service = UnavailablePrintService()
        #endif
    }

    init(context: NativePluginContext, service: any PrintService) {
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

        let job = try payload.withUnsafeBytes(PrintJob.decode)
        try Task.checkCancellation()

        return try await present(job)
    }

    private func availability() -> PrintAvailability {
        if busy { return .busy }

        do {
            _ = try context.requirePresentationOwner()

            return .ready
        } catch PluginError.unavailable {
            return .inactive
        } catch {
            return .inactive
        }
    }

    private func present(_ job: PrintJob) async throws -> Data {
        if busy { throw PluginError.busy }

        let owner = try context.requirePresentationOwner()

        busy = true
        defer { busy = false }
        try await service.present(job, from: owner)

        return Data()
    }

    private func encode(_ value: PrintAvailability) -> Data {
        var raw = value.rawValue.littleEndian

        return withUnsafeBytes(of: &raw) { Data($0) }
    }
}
