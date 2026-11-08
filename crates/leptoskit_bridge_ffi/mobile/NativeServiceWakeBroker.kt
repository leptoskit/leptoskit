package dev.leptoskit.mobile

/**
 * Host-owned wake hook for background plugins. The active [LeptosKitView]
 * installs a wake that posts a background tick to its guest; a background
 * plugin (an Android foreground service) reads it through this host object
 * rather than reaching into the view. The hook is thread-safe so a service or
 * worker can call it from its own looper.
 */
object NativeServiceWakeBroker {
    @Volatile
    private var wake: (() -> Unit)? = null

    /** Installs or clears the wake. Only the owning view changes it. */
    internal fun install(wake: (() -> Unit)?) {
        this.wake = wake
    }

    /** Posts a background tick to the running app, or does nothing when closed. */
    fun wakeBackground() {
        wake?.invoke()
    }
}
