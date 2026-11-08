import Foundation

/// Host-installed wake that lets a background task reach the running app's
/// service. The generated host installs the wake on the main actor; background
/// plugins call [wakeBackground] from any queue and the host drains it on the
/// main actor.
public enum NativeServiceWakeBroker {
    private static let lock = NSLock()
    private nonisolated(unsafe) static var installed: (@MainActor () -> Void)?

    /// Installs or clears the wake. Called by the generated host on the main actor.
    @MainActor
    public static func install(_ wake: (@MainActor () -> Void)?) {
        lock.lock()
        installed = wake
        lock.unlock()
    }

    /// Posts a background tick from any queue; the host drains it on the main actor.
    public static func wakeBackground() {
        lock.lock()
        let wake = installed
        lock.unlock()

        guard let wake else { return }
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                wake()
            }
        }
    }
}
