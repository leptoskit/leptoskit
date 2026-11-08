package dev.leptoskit.plugins.maps

import android.content.Context
import android.content.ContextWrapper
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.lifecycle.LifecycleOwner
import com.google.android.gms.maps.CameraUpdateFactory
import com.google.android.gms.maps.GoogleMap
import com.google.android.gms.maps.MapView
import com.google.android.gms.maps.MapsInitializer
import com.google.android.gms.maps.model.LatLng
import com.google.android.gms.maps.model.MarkerOptions
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.NativeMapBinding
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import kotlin.coroutines.resume

/** Host-owned [MapView] registry. Surfaces attach through [NativeMapBinding]. */
class MapsPlugin(
    private val hostContext: Context,
) : NativePlugin {
    override val name: String = "maps"
    override val methods: IntArray = intArrayOf(0, 1, 2, 3, 4)

    private val handler = Handler(Looper.getMainLooper())
    private val sessions = linkedMapOf<Int, Session>()
    private val surfaces = linkedMapOf<Int, MapView>()
    private var nextId = 1
    private var closed = false

    private class Session(
        val mapView: MapView,
        var map: GoogleMap?,
        var camera: MapCamera,
        var markers: List<MapMarker> = emptyList(),
    )

    init {
        MapsInitializer.initialize(hostContext)
        NativeMapBinding.attach = { frame, id -> attach(frame, id.toInt()) }
        NativeMapBinding.detach = { frame -> detach(frame) }
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> {
                MapsWire.create(payload)
                create()
            }

            1 -> {
                val (id, camera) = MapsWire.setCamera(payload)
                setCamera(id, camera)
                ByteArray(0)
            }

            2 -> {
                val (id, markers) = MapsWire.setMarkers(payload)
                setMarkers(id, markers)
                ByteArray(0)
            }

            3 -> {
                status(MapsWire.session(payload))
            }

            4 -> {
                release(MapsWire.session(payload))
                ByteArray(0)
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        closed = true
        for (id in sessions.keys.toList()) {
            runCatching { release(id) }
        }
        if (NativeMapBinding.attach != null) {
            NativeMapBinding.attach = null
            NativeMapBinding.detach = null
        }
    }

    private suspend fun create(): ByteArray {
        lifecycleOwner()
        val mapView = MapView(hostContext)
        mapView.onCreate(null)
        mapView.onResume()
        val googleMap =
            suspendCancellableCoroutine { continuation ->
                mapView.getMapAsync { map ->
                    if (continuation.isActive) continuation.resume(map)
                }
            }
        val camera = MapCamera.DEFAULT
        googleMap.moveCamera(
            CameraUpdateFactory.newLatLngZoom(
                LatLng(camera.latitude, camera.longitude),
                camera.zoom.toFloat(),
            ),
        )
        val id = nextId
        nextId = if (nextId == Int.MAX_VALUE) 1 else nextId + 1
        sessions[id] = Session(mapView, googleMap, camera)
        return MapsWire.sessionId(id)
    }

    private fun setCamera(
        id: Int,
        camera: MapCamera,
    ) {
        val session = sessions[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        session.camera = camera
        session.map?.animateCamera(
            CameraUpdateFactory.newLatLngZoom(
                LatLng(camera.latitude, camera.longitude),
                camera.zoom.toFloat(),
            ),
        )
    }

    private fun setMarkers(
        id: Int,
        markers: List<MapMarker>,
    ) {
        val session = sessions[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        session.markers = markers
        val map = session.map ?: return
        map.clear()
        for (marker in markers) {
            val options =
                MarkerOptions()
                    .position(LatLng(marker.latitude, marker.longitude))
            if (marker.title.isNotEmpty()) {
                options.title(marker.title)
            }
            map.addMarker(options)
        }
    }

    private fun status(id: Int): ByteArray {
        val session = sessions[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        return MapsWire.status(session.camera, session.markers.size, active = true)
    }

    private fun release(id: Int) {
        val session = sessions.remove(id) ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        surfaces.remove(id)?.let { view ->
            (view.parent as? ViewGroup)?.removeView(view)
        }
        session.map?.clear()
        session.map = null
        session.mapView.onPause()
        session.mapView.onDestroy()
    }

    private fun lifecycleOwner(): LifecycleOwner {
        var current: Context? = hostContext
        while (current is ContextWrapper) {
            if (current is LifecycleOwner) return current
            current = current.baseContext
        }
        throw PluginException(PluginException.UNAVAILABLE)
    }

    private fun attach(
        frame: FrameLayout,
        id: Int,
    ) {
        val session = sessions[id] ?: return
        handler.post {
            val existing = surfaces[id]
            if (existing?.parent === frame) return@post
            (existing?.parent as? ViewGroup)?.removeView(existing)
            val view = session.mapView
            (view.parent as? ViewGroup)?.removeView(view)
            view.layoutParams =
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                )
            surfaces[id] = view
            frame.removeAllViews()
            frame.addView(view)
        }
    }

    private fun detach(frame: FrameLayout) {
        handler.post {
            val child = frame.getChildAt(0) as? MapView ?: return@post
            val id = surfaces.entries.firstOrNull { it.value === child }?.key
            frame.removeAllViews()
            if (id != null) {
                surfaces.remove(id)
            }
        }
    }
}
