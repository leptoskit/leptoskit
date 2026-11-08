package dev.leptoskit.mobile

import android.app.Activity
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper

/**
 * The immersive activity a Meta Quest app opens for an `xr` headset session.
 *
 * Horizon OS shows an OpenXR session only from an activity in the `com.oculus.intent.category.VR` category, and a
 * panel app's launcher activity is a 2D panel. The Quest build declares this activity in that category; Rust opens it
 * through [NativeImmersive] when the guest starts a headset session, draws the session from its own render thread
 * while the activity runs, and closes it when the session ends. The guest keeps running in the panel's activity.
 */
class NativeImmersiveActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        NativeImmersive.attach(this)
    }

    override fun onDestroy() {
        NativeImmersive.detach(this)
        super.onDestroy()
    }
}

/** What Rust calls, through the app's class loader, to open and close [NativeImmersiveActivity]. */
object NativeImmersive {
    /** The extra Horizon Home reads to open an activity in Home rather than in the immersive space. */
    private const val LAUNCH_IN_HOME = "extra_launch_in_home_pending_intent"

    @Volatile private var activity: Activity? = null

    /** The running immersive activity, or null before it starts and after it finishes. */
    @JvmStatic
    fun current(): Activity? = activity

    internal fun attach(activity: Activity) {
        this.activity = activity
    }

    internal fun detach(activity: Activity) {
        if (this.activity === activity) this.activity = null
    }

    /** Opens the immersive activity over the panel; false when the app doesn't declare it (any build but Quest's). */
    @JvmStatic
    fun open(context: Context): Boolean {
        val intent =
            Intent(context, NativeImmersiveActivity::class.java)
                .setAction(Intent.ACTION_MAIN)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (intent.resolveActivity(context.packageManager) == null) return false
        Handler(Looper.getMainLooper()).post { context.startActivity(intent) }

        return true
    }

    /** Finishes the immersive activity and brings the app's launcher activity, the panel, back in Horizon Home. */
    @JvmStatic
    fun close(context: Context) {
        Handler(Looper.getMainLooper()).post {
            val immersive = activity
            val panel =
                context.packageManager.getLaunchIntentForPackage(context.packageName)?.apply {
                    action = Intent.ACTION_MAIN
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }

            if (panel != null) {
                val pending =
                    PendingIntent.getActivity(
                        context,
                        0,
                        panel,
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                    )
                val home =
                    Intent(Intent.ACTION_MAIN)
                        .addCategory(Intent.CATEGORY_HOME)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        .putExtra(LAUNCH_IN_HOME, pending)

                try {
                    context.startActivity(home)
                } catch (error: ActivityNotFoundException) {
                    context.startActivity(panel)
                }
            }

            immersive?.finish()
        }
    }
}
