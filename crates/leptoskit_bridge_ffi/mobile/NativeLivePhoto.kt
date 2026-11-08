package dev.leptoskit.mobile

import android.content.Context
import android.widget.FrameLayout

/**
 * Lookup hooks the `live-photo` plugin may install. Android has no Live Photo
 * surface in this MVP, so attach stays unused while the widget kind remains.
 */
object NativeLivePhotoBinding {
    @Volatile
    var attach: ((FrameLayout, Long) -> Unit)? = null

    @Volatile
    var detach: ((FrameLayout) -> Unit)? = null
}

/** A `LivePhoto` element placeholder frame. */
internal class NativeLivePhotoView(
    context: Context,
) : FrameLayout(context) {
    private var sessionId: Long? = null

    fun show(session: Long?) {
        if (sessionId == session) return
        NativeLivePhotoBinding.detach?.invoke(this)
        removeAllViews()
        sessionId = session
        if (session != null) {
            NativeLivePhotoBinding.attach?.invoke(this, session)
        }
    }

    fun close() {
        NativeLivePhotoBinding.detach?.invoke(this)
        removeAllViews()
        sessionId = null
    }
}
