package dev.leptoskit.mobile

import android.content.Context
import android.widget.FrameLayout

/**
 * Lookup hooks the `maps` plugin installs so [NativeMapView] can attach a
 * Google [com.google.android.gms.maps.MapView] by host-owned id without shipping
 * Maps in every app.
 */
object NativeMapBinding {
    @Volatile
    var attach: ((FrameLayout, Long) -> Unit)? = null

    @Volatile
    var detach: ((FrameLayout) -> Unit)? = null
}

/** A `Map` element: a frame that hosts the plugin's map view. */
internal class NativeMapView(
    context: Context,
) : FrameLayout(context) {
    private var sessionId: Long? = null

    fun show(session: Long?) {
        if (sessionId == session) return
        NativeMapBinding.detach?.invoke(this)
        removeAllViews()
        sessionId = session
        if (session != null) {
            NativeMapBinding.attach?.invoke(this, session)
        }
    }

    fun close() {
        NativeMapBinding.detach?.invoke(this)
        removeAllViews()
        sessionId = null
    }
}
