package dev.leptoskit.plugins.video

import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.NativeVideoBinding
import java.nio.ByteBuffer

/**
 * Host-owned Media3 ExoPlayer registry. Surfaces attach through
 * [NativeVideoBinding] into a [NativeVideoView] frame.
 */
class VideoPlugin(
    context: Context,
) : NativePlugin {
    override val name = "video"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)

    private val appContext = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private val players = LinkedHashMap<Int, ExoPlayer>()
    private val surfaces = HashMap<Int, PlayerView>()
    private var nextId = 1
    private var closed = false

    init {
        NativeVideoBinding.attach = { frame, id -> attach(frame, id.toInt()) }
        NativeVideoBinding.detach = { frame -> detach(frame) }
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        return when (method) {
            0 -> {
                create(VideoWire.source(payload))
            }

            1 -> {
                val (id, url) = VideoWire.replace(payload)
                replace(id, url)
                ByteArray(0)
            }

            2 -> {
                play(VideoWire.player(payload))
                ByteArray(0)
            }

            3 -> {
                pause(VideoWire.player(payload))
                ByteArray(0)
            }

            4 -> {
                val (id, position) = VideoWire.seek(payload)
                seek(id, position.toLong() and 0xFFFF_FFFFL)
                ByteArray(0)
            }

            5 -> {
                val (id, muted) = VideoWire.flag(payload)
                setMuted(id, muted)
                ByteArray(0)
            }

            6 -> {
                val (id, looping) = VideoWire.flag(payload)
                setLoop(id, looping)
                ByteArray(0)
            }

            7 -> {
                val (id, thousandths) = VideoWire.rate(payload)
                setRate(id, thousandths)
                ByteArray(0)
            }

            8 -> {
                status(VideoWire.player(payload))
            }

            else -> {
                release(VideoWire.player(payload))
                ByteArray(0)
            }
        }
    }

    override fun close() {
        closed = true
        ArrayList(players.keys).forEach { releaseQuiet(it) }
        if (NativeVideoBinding.attach != null) {
            NativeVideoBinding.attach = null
            NativeVideoBinding.detach = null
        }
    }

    private fun create(url: String): ByteArray {
        val id = nextId
        nextId = if (nextId == Int.MAX_VALUE) 1 else nextId + 1
        val player =
            ExoPlayer.Builder(appContext).build().also {
                it.setMediaItem(MediaItem.fromUri(Uri.parse(url)))
                it.prepare()
            }
        players[id] = player
        return VideoWire.playerId(id)
    }

    private fun replace(
        id: Int,
        url: String,
    ) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.setMediaItem(MediaItem.fromUri(Uri.parse(url)))
        player.prepare()
    }

    private fun play(id: Int) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.play()
    }

    private fun pause(id: Int) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.pause()
    }

    private fun seek(
        id: Int,
        positionMs: Long,
    ) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.seekTo(positionMs)
    }

    private fun setMuted(
        id: Int,
        muted: Boolean,
    ) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.volume = if (muted) 0f else 1f
    }

    private fun setLoop(
        id: Int,
        looping: Boolean,
    ) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.repeatMode = if (looping) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF
    }

    private fun setRate(
        id: Int,
        thousandths: Int,
    ) {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        player.setPlaybackSpeed(thousandths / 1000f)
    }

    private fun status(id: Int): ByteArray {
        val player = players[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        val duration = player.duration
        return VideoWire.status(
            playing = player.isPlaying,
            currentMs = player.currentPosition.coerceAtLeast(0).toInt(),
            durationMs =
                if (duration < 0) {
                    VideoWire.TIME_UNKNOWN_U
                } else {
                    duration.coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
                },
        )
    }

    private fun release(id: Int) {
        if (!players.containsKey(id)) throw PluginException(PluginException.INVALID_ARGUMENT)
        releaseQuiet(id)
    }

    private fun releaseQuiet(id: Int) {
        surfaces.remove(id)?.let { view ->
            (view.parent as? ViewGroup)?.removeView(view)
            view.player = null
        }
        players.remove(id)?.release()
    }

    private fun attach(
        frame: FrameLayout,
        id: Int,
    ) {
        val player = players[id] ?: return
        handler.post {
            val existing = surfaces[id]
            if (existing?.parent === frame) return@post
            (existing?.parent as? ViewGroup)?.removeView(existing)
            existing?.player = null
            val view =
                PlayerView(frame.context).apply {
                    useController = false
                    this.player = player
                    layoutParams =
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.MATCH_PARENT,
                        )
                }
            surfaces[id] = view
            frame.removeAllViews()
            frame.addView(view)
        }
    }

    private fun detach(frame: FrameLayout) {
        handler.post {
            val child = frame.getChildAt(0) as? PlayerView ?: return@post
            val id = surfaces.entries.firstOrNull { it.value === child }?.key
            child.player = null
            frame.removeAllViews()
            if (id != null) surfaces.remove(id)
        }
    }
}
