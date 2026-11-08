import Foundation
import PhotosUI

/// Lookup hook the `live-photo` plugin installs so `NativeLivePhotoView` can
/// attach a `PHLivePhotoView` by host-owned id without linking the plugin into
/// every app.
@MainActor
public enum NativeLivePhotoBinding {
    /// Returns the Live Photo view for `id`, or `nil` when no plugin owns it.
    public static var view: ((UInt32) -> PHLivePhotoView?)?
}
