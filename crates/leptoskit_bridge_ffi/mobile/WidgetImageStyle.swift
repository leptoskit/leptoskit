import LeptosKitBridge
import UIKit

/// File-backed images remain synchronous and reject remote image sources.
@MainActor
enum WidgetImageStyle {
    static func apply(_ props: WidgetProps, to widget: UIView) throws {
        guard let image = widget as? UIImageView else { return }
        image.contentMode = .scaleAspectFit
        if image is NativeRiveImageView {
            image.isAccessibilityElement = props.aria?.ariaHidden != true
            image.accessibilityTraits = .image
            return
        }

        image.image = try props.source.flatMap(load)
    }

    private static func load(_ source: String) throws -> UIImage? {
        if let asset = try BundledAsset.url(source) {
            if let loaded = UIImage(contentsOfFile: asset.path) { return loaded }

            return try BundledAsset.missing(source, at: asset)
        }

        guard let url = URL(string: source), url.isFileURL,
            let loaded = UIImage(contentsOfFile: url.path)
        else {
            throw RuntimeFailure(message: "Image requires a readable file URL: \(source)")
        }

        return loaded
    }
}

/// `asset://<path>`: a file in the app's `assets/` folder, which `leptoskit build` copies into the
/// bundle's own `assets` folder. iOS 14 and later decode PNG, JPEG, and WebP.
enum BundledAsset {
    private static let scheme = "asset://"

    /// The bundled file `source` names, or nil when it is not an asset source.
    static func url(_ source: String) throws -> URL? {
        guard source.hasPrefix(scheme) else { return nil }
        let path = String(source.dropFirst(scheme.count))
        let segments = path.split(separator: "/", omittingEmptySubsequences: false)

        guard !path.isEmpty, !path.contains("\\"), !path.contains("\0"),
            segments.allSatisfy({ $0 != "" && $0 != "." && $0 != ".." })
        else {
            throw RuntimeFailure(message: "\(source) is not an asset path inside the app's assets folder")
        }
        guard let resources = Bundle.main.resourceURL else {
            throw RuntimeFailure(message: "The app bundle has no resources for \(source)")
        }

        return resources.appendingPathComponent("assets").appendingPathComponent(path)
    }

    /// A debug build stops on a missing asset; a release build logs it and shows nothing.
    static func missing(_ source: String, at url: URL) throws -> UIImage? {
        let message = "Image \(source) is not in the app bundle, or is not a PNG, JPEG, or WebP (looked at \(url.path))"
        #if DEBUG
        throw RuntimeFailure(message: message)
        #else
        NativeLog.error(NativeLog.render, message)
        return nil
        #endif
    }
}
