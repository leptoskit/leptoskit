import Contacts
import ContactsUI
import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Permission status, system contact picker, and bounded contact query.
///
/// Purpose strings stay app-owned (`NSContactsUsageDescription`). The system
/// picker does not require a prior grant; query and request do. Denied access
/// never invents contacts.
@MainActor
public final class ContactsPlugin: NSObject, NativePlugin {
    public let name = "contacts"
    public let methods: [UInt32] = [0, 1, 2, 3]
    private let context: NativePluginContext
    private let store = CNContactStore()
    private var closed = false
    private var busy = false
    private let selection = ContactsSelection()
    private var selectionID: UUID?

    #if canImport(UIKit)
    private var picker: CNContactPickerViewController?
    #elseif canImport(AppKit)
    private var picker: CNContactPicker?
    #endif

    public init(context: NativePluginContext) { self.context = context }

    public func close() {
        closed = true
        cancelSelection()
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard let method = ContactsMethod(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .status:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return encode(status())
        case .requestPermission:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return try await requestAccess()
        case .pick:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return try await pick()
        case .query:
            let limit = try payload.withUnsafeBytes(ContactsRecord.decodeQueryLimit)

            return try query(limit: limit)
        }
    }

    private func status() -> ContactsStatus {
        // Raw values keep iOS 18 `.limited` (4) as granted without an availability gate.
        switch CNContactStore.authorizationStatus(for: .contacts).rawValue {
        case 0: .prompt
        case 1: .restricted
        case 2: .denied
        case 3, 4: .granted
        default: .unknown
        }
    }

    private func requestAccess() async throws -> Data {
        try requireUsageDescription()
        let current = CNContactStore.authorizationStatus(for: .contacts)
        if current != .notDetermined {
            return encode(status())
        }

        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }
        _ = try await store.requestAccess(for: .contacts)
        guard !closed else { throw PluginError.cancelled }

        return encode(status())
    }

    private func pick() async throws -> Data {
        if busy { throw PluginError.busy }

        try Task.checkCancellation()
        busy = true
        let identifier = UUID()
        selectionID = identifier
        defer {
            busy = false
            selectionID = nil
            releaseChooser()
        }

        let contact = try await choose(identifier: identifier)
        try Task.checkCancellation()
        guard !closed else { throw PluginError.cancelled }

        return try contact.encode()
    }

    private func query(limit: Int) throws -> Data {
        guard status() == .granted else { throw PluginError.permissionDenied }

        try requireUsageDescription()
        if busy { throw PluginError.busy }

        busy = true
        defer { busy = false }

        let keys: [CNKeyDescriptor] = [
            CNContactIdentifierKey as CNKeyDescriptor,
            CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
            CNContactPhoneNumbersKey as CNKeyDescriptor,
            CNContactEmailAddressesKey as CNKeyDescriptor,
            CNContactOrganizationNameKey as CNKeyDescriptor
        ]
        let request = CNContactFetchRequest(keysToFetch: keys)
        request.unifyResults = true

        var contacts: [ContactsRecord] = []
        try store.enumerateContacts(with: request) { contact, stop in
            if contacts.count >= limit {
                stop.pointee = true

                return
            }

            if let record = try? Self.record(from: contact) {
                contacts.append(record)
            }
        }

        guard !closed else { throw PluginError.cancelled }

        return try ContactsRecord.encodeList(contacts)
    }

    private func choose(identifier: UUID) async throws -> ContactsRecord {
        try await withTaskCancellationHandler {
            try await present()
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.cancel(identifier: identifier)
            }
        }
    }

    private func present() async throws -> ContactsRecord {
        let owner = try context.requirePresentationOwner()

        #if canImport(UIKit)
        guard owner.presentedViewController == nil else { throw PluginError.busy }

        let picker = CNContactPickerViewController()
        picker.delegate = self
        self.picker = picker

        return try await withCheckedThrowingContinuation { continuation in
            selection.begin(continuation)
            owner.present(picker, animated: true)
            picker.presentationController?.delegate = self
        }
        #elseif canImport(AppKit)
        let picker = CNContactPicker()
        picker.delegate = self
        self.picker = picker

        return try await withCheckedThrowingContinuation { continuation in
            selection.begin(continuation)
            picker.showRelative(to: owner.view.bounds, of: owner.view, preferredEdge: .minY)
        }
        #endif
    }

    private func cancel(identifier: UUID) {
        guard selectionID == identifier else { return }

        cancelSelection()
    }

    private func cancelSelection() {
        dismissChooser()
        selection.finish(.failure(PluginError.cancelled))
    }

    private func dismissChooser() {
        #if canImport(UIKit)
        picker?.dismiss(animated: false)
        #elseif canImport(AppKit)
        picker?.close()
        #endif
    }

    private func releaseChooser() {
        #if canImport(UIKit)
        picker?.delegate = nil
        #elseif canImport(AppKit)
        picker?.delegate = nil
        #endif
        picker = nil
    }

    private func requireUsageDescription() throws {
        guard let description = Bundle.main.object(forInfoDictionaryKey: "NSContactsUsageDescription") as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.invalidArgument
        }
    }

    private func encode(_ status: ContactsStatus) -> Data {
        var raw = status.rawValue.littleEndian

        return withUnsafeBytes(of: &raw) { Data($0) }
    }

    fileprivate static func record(from contact: CNContact) throws -> ContactsRecord {
        let formatted = CNContactFormatter.string(from: contact, style: .fullName)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let organization = contact.organizationName.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName: String
        if let formatted, !formatted.isEmpty {
            displayName = formatted
        } else if !organization.isEmpty {
            displayName = organization
        } else {
            displayName = contact.identifier
        }

        let phone = contact.phoneNumbers.first.map { $0.value.stringValue }
        let email = contact.emailAddresses.first.map { $0.value as String }

        return try ContactsRecord(
            id: contact.identifier,
            displayName: displayName,
            phone: phone.flatMap { $0.isEmpty ? nil : $0 },
            email: email.flatMap { $0.isEmpty ? nil : $0 }
        )
    }
}

#if canImport(UIKit)
extension ContactsPlugin: @MainActor CNContactPickerDelegate {
    public func contactPickerDidCancel(_ picker: CNContactPickerViewController) {
        guard self.picker === picker else { return }

        selection.finish(.failure(PluginError.cancelled))
    }

    public func contactPicker(_ picker: CNContactPickerViewController, didSelect contact: CNContact) {
        guard self.picker === picker else { return }

        do {
            selection.finish(.success(try Self.record(from: contact)))
        } catch {
            selection.finish(.failure(error))
        }
    }
}

extension ContactsPlugin: @MainActor UIAdaptivePresentationControllerDelegate {
    public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        selection.finish(.failure(PluginError.cancelled))
    }
}
#elseif canImport(AppKit)
extension ContactsPlugin: @MainActor CNContactPickerDelegate {
    public func contactPickerDidClose(_ picker: CNContactPicker) {
        guard self.picker === picker else { return }

        // macOS closes after selection or cancel; only finish cancel when still waiting.
        selection.finish(.failure(PluginError.cancelled))
    }

    public func contactPicker(_ picker: CNContactPicker, didSelect contact: CNContact) {
        guard self.picker === picker else { return }

        do {
            selection.finish(.success(try Self.record(from: contact)))
        } catch {
            selection.finish(.failure(error))
        }
    }
}
#endif
