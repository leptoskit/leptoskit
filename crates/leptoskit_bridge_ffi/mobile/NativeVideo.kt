package dev.leptoskit.mobile

import android.content.Context
import android.widget.FrameLayout

/**
 * Lookup hooks the `video` plugin installs so [NativeVideoView] can attach a
 * Media3 player surface by host-owned id without shipping Media3 in every app.
 */
object NativeVideoBinding {
    @Volatile
    var attach: ((FrameLayout, Long) -> Unit)? = null

    @Volatile
    var detach: ((FrameLayout) -> Unit)? = null
}

/** A `Video` element: a frame that hosts the plugin's player view. */
internal class NativeVideoView(
    context: Context,
) : FrameLayout(context) {
    private var playerId: Long? = null

    fun show(player: Long?) {
        if (playerId == player) return
        NativeVideoBinding.detach?.invoke(this)
        removeAllViews()
        playerId = player
        if (player != null) {
            NativeVideoBinding.attach?.invoke(this, player)
        }
    }

    fun close() {
        NativeVideoBinding.detach?.invoke(this)
        removeAllViews()
        playerId = null
    }
}
