import AVFoundation
import Foundation

/// Lookup hook the `video` plugin installs so `NativeVideoView` can attach an
/// `AVPlayer` by host-owned id without linking the plugin into every app.
@MainActor
public enum NativeVideoBinding {
    /// Returns the player for `id`, or `nil` when no plugin owns it.
    public static var player: ((UInt32) -> AVPlayer?)?
}
