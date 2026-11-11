import Foundation

/// The development companion asks the running app to follow a guest URL.
/// The app installs this hook; release builds still reject unsigned guests.
public enum DevelopmentSession {
    @MainActor public static var follow: (@MainActor (String) -> Bool)?
}
