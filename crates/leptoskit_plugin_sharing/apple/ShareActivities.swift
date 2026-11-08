#if canImport(UIKit)
import LinkPresentation
import UIKit

/// iOS activity types by wire code, in `share_plus`'s `CupertinoActivityType` order.
enum ShareActivities {
    static let all: [UIActivity.ActivityType] = [
        .postToFacebook, .postToTwitter, .postToWeibo, .message, .mail, .print, .copyToPasteboard,
        .assignToContact, .saveToCameraRoll, .addToReadingList, .postToFlickr, .postToVimeo,
        .postToTencentWeibo, .airDrop, .openInIBooks, .markupAsPDF, .sharePlay,
        .collaborationInviteWithLink, .collaborationCopyLink, .addToHomeScreen
    ]

    /// The excluded types; the decoder has already refused unknown codes.
    static func types(_ codes: [UInt32]) -> [UIActivity.ActivityType]? {
        codes.isEmpty ? nil : codes.compactMap { all.indices.contains(Int($0)) ? all[Int($0)] : nil }
    }
}

/// Wraps one sheet item to supply the email subject and the preview title.
/// UIKit may call these methods off the main thread; every field is immutable.
final class ShareItemSource: NSObject, UIActivityItemSource, @unchecked Sendable {
    private let item: Any
    private let subject: String?
    private let title: String?

    init(item: Any, subject: String?, title: String?) {
        self.item = item
        self.subject = subject
        self.title = title
    }

    func activityViewControllerPlaceholderItem(_ activityViewController: UIActivityViewController) -> Any {
        item
    }

    func activityViewController(
        _ activityViewController: UIActivityViewController, itemForActivityType activityType: UIActivity.ActivityType?
    ) -> Any? {
        item
    }

    func activityViewController(
        _ activityViewController: UIActivityViewController,
        subjectForActivityType activityType: UIActivity.ActivityType?
    ) -> String {
        subject ?? ""
    }

    /// A title replaces the preview's heading; without one the system builds its own.
    func activityViewControllerLinkMetadata(_ activityViewController: UIActivityViewController) -> LPLinkMetadata? {
        guard let title else { return nil }

        let metadata = LPLinkMetadata()
        metadata.title = title

        if let url = item as? URL {
            metadata.originalURL = url
            metadata.url = url
        }

        return metadata
    }
}
#endif
