package dev.leptoskit.mobile

import android.content.Context
import android.widget.FrameLayout

/**
 * Lookup hooks the `camera` plugin installs so [NativeCameraView] can attach a
 * CameraX preview surface by host-owned id without shipping CameraX in every app.
 */
object NativeCameraBinding {
    @Volatile
    var attach: ((FrameLayout, Long) -> Unit)? = null

    @Volatile
    var detach: ((FrameLayout) -> Unit)? = null
}

/** A `Camera` element: a frame that hosts the plugin's preview view. */
internal class NativeCameraView(
    context: Context,
) : FrameLayout(context) {
    private var sessionId: Long? = null

    fun show(session: Long?) {
        if (sessionId == session) return
        NativeCameraBinding.detach?.invoke(this)
        removeAllViews()
        sessionId = session
        if (session != null) {
            NativeCameraBinding.attach?.invoke(this, session)
        }
    }

    fun close() {
        NativeCameraBinding.detach?.invoke(this)
        removeAllViews()
        sessionId = null
    }
}
