import AVFoundation
import Foundation

/// Lookup hook the `camera` plugin installs so `NativeCameraView` can attach an
/// `AVCaptureSession` by host-owned id without linking the plugin into every app.
@MainActor
public enum NativeCameraBinding {
    /// Returns the capture session for `id`, or `nil` when no plugin owns it.
    public static var session: ((UInt32) -> AVCaptureSession?)?
}
