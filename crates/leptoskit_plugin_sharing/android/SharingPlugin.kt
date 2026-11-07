package dev.leptoskit.plugins.sharing

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import android.os.Looper
import android.webkit.MimeTypeMap
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.content.IntentCompat
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File
import java.nio.ByteBuffer
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicInteger

/**
 * Shares text, a web URL, or files through Android's chooser, and reports the
 * component the user picked. The reply is never a delivery receipt.
 */
class SharingPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "sharing"
    override val methods = intArrayOf(SHARE)
    private val host = context as? NativeActivityResultHost
    private val staging =
        ShareStaging(
            File(context.filesDir, "files"),
            File(context.cacheDir, CACHE_DIRECTORY),
            ShareStaging::procPath,
            { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) },
        )
    private var job: Job? = null
    private var busy = false
    private var closed = false

    override fun close() {
        closed = true
        job?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Sharing requires the owning UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method != SHARE) throw PluginException(PluginException.UNAVAILABLE)

        if (busy) throw PluginException(PluginException.BUSY)

        val content = ShareContent.decode(payload)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)

        currentCoroutineContext().ensureActive()
        busy = true
        job = currentCoroutineContext()[Job]

        try {
            return share(host, content)
        } finally {
            busy = false
            job = null
        }
    }

    /** Copies files off the UI thread, presents the chooser, then reports the chosen component. */
    private suspend fun share(
        host: NativeActivityResultHost,
        content: ShareContent,
    ): ByteArray {
        val staged =
            if (content.files.isEmpty()) {
                emptyList()
            } else {
                withContext(Dispatchers.IO) {
                    val scope = currentCoroutineContext()
                    staging.stage(content.files) { scope.ensureActive() }
                }
            }

        currentCoroutineContext().ensureActive()

        if (closed) throw PluginException(PluginException.CANCELLED)

        val uris = staged.map { FileProvider.getUriForFile(context, context.packageName + AUTHORITY, it.file) }
        val target = ShareIntents.target(content, uris, staged.map(StagedFile::mime))

        return choose(host, target, content.title)
    }

    /**
     * Launches the chooser with a callback that names the chosen component. The
     * callback is a broadcast only this app can receive, matched by a one-time nonce.
     */
    private suspend fun choose(
        host: NativeActivityResultHost,
        target: Intent,
        title: String?,
    ): ByteArray {
        val action = context.packageName + CHOSEN_ACTION
        val nonce = ByteArray(NONCE_BYTES).also(random::nextBytes).joinToString("") { "%02x".format(it) }
        val chosen = CompletableDeferred<String?>()
        val receiver =
            object : BroadcastReceiver() {
                override fun onReceive(
                    context: Context,
                    intent: Intent,
                ) {
                    if (intent.getStringExtra(EXTRA_NONCE) != nonce) return

                    val component =
                        IntentCompat.getParcelableExtra(intent, Intent.EXTRA_CHOSEN_COMPONENT, ComponentName::class.java)
                    chosen.complete(component?.flattenToString())
                }
            }
        ContextCompat.registerReceiver(context, receiver, IntentFilter(action), ContextCompat.RECEIVER_NOT_EXPORTED)
        // Mutable so the chooser can add the chosen component; explicit by package.
        val callback =
            PendingIntent.getBroadcast(
                context,
                requests.incrementAndGet(),
                Intent(action).setPackage(context.packageName).putExtra(EXTRA_NONCE, nonce),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_ONE_SHOT,
            )

        try {
            val result = host.launchPluginActivity(Intent.createChooser(target, title, callback.intentSender))

            currentCoroutineContext().ensureActive()

            if (closed) throw PluginException(PluginException.CANCELLED)

            if (result.resultCode != Activity.RESULT_OK && result.resultCode != Activity.RESULT_CANCELED) {
                throw PluginException(PluginException.NATIVE)
            }

            // The broadcast can trail the chooser's result by a moment.
            val component = withTimeoutOrNull(CHOSEN_GRACE_MS) { chosen.await() }

            return if (component != null) ShareReply.success(component) else ShareReply.dismissed()
        } finally {
            context.unregisterReceiver(receiver)
            callback.cancel()
        }
    }

    private companion object {
        /** The only method: present the system share sheet. */
        const val SHARE = 0

        /** Must match `res/xml/leptoskit_sharing_paths.xml` and the Cargo metadata. */
        const val CACHE_DIRECTORY = "leptoskit-sharing"
        const val AUTHORITY = ".leptoskit.sharing"
        const val CHOSEN_ACTION = ".leptoskit.sharing.CHOSEN"
        const val EXTRA_NONCE = "dev.leptoskit.plugins.sharing.NONCE"
        const val NONCE_BYTES = 16
        const val CHOSEN_GRACE_MS = 500L

        val random = SecureRandom()
        val requests = AtomicInteger()
    }
}

/** The intent the chooser offers, built the way receiving apps expect. */
internal object ShareIntents {
    fun target(
        content: ShareContent,
        uris: List<Uri>,
        mimes: List<String>,
    ): Intent {
        val intent = Intent()

        when (uris.size) {
            0 -> {
                intent.action = Intent.ACTION_SEND
                intent.type = "text/plain"
                intent.putExtra(Intent.EXTRA_TEXT, content.url ?: content.text)
            }

            1 -> {
                intent.action = Intent.ACTION_SEND
                intent.type = mimes.first()
                intent.putExtra(Intent.EXTRA_STREAM, uris.first())
            }

            else -> {
                intent.action = Intent.ACTION_SEND_MULTIPLE
                intent.type = ShareStaging.commonMime(mimes)
                intent.putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
            }
        }

        if (uris.isNotEmpty()) {
            content.text?.let { intent.putExtra(Intent.EXTRA_TEXT, it) }
            // Read access goes only to the copies this share names, and only
            // through the intent the chosen app receives.
            val clip = ClipData.newRawUri(null, uris.first())
            uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
            intent.clipData = clip
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }

        content.subject?.let { intent.putExtra(Intent.EXTRA_SUBJECT, it) }
        content.title?.let { intent.putExtra(Intent.EXTRA_TITLE, it) }

        return intent
    }
}
