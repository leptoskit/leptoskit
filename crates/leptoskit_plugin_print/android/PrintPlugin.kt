package dev.leptoskit.plugins.print

import android.app.Activity
import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintDocumentInfo
import android.print.PrintManager
import android.webkit.WebView
import android.webkit.WebViewClient
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Presents the system print dialog for HTML or a PDF URI. An empty reply means
 * the dialog finished or the print job was accepted — never that paper left a
 * printer.
 */
class PrintPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "print"
    override val methods = intArrayOf(AVAILABILITY, PRINT)
    private var busy = false
    private var closed = false
    private val mainHandler = Handler(Looper.getMainLooper())

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            AVAILABILITY -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                encode(availability())
            }

            else -> {
                print(PrintJob.decode(payload))
            }
        }
    }

    private fun availability(): Int =
        when {
            busy -> BUSY
            printManager() == null || activeOwner() == null -> INACTIVE
            else -> READY
        }

    private suspend fun print(job: PrintJob): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)

        val owner = activeOwner() ?: throw PluginException(PluginException.UNAVAILABLE)
        val manager = printManager() ?: throw PluginException(PluginException.UNAVAILABLE)

        currentCoroutineContext().ensureActive()
        busy = true

        try {
            when (job) {
                is PrintJob.Html -> printHtml(owner, manager, job.html)
                is PrintJob.PdfUri -> printPdf(manager, job.uri)
            }

            currentCoroutineContext().ensureActive()

            if (closed) throw PluginException(PluginException.CANCELLED)

            return byteArrayOf()
        } finally {
            busy = false
        }
    }

    private suspend fun printHtml(
        owner: Activity,
        manager: PrintManager,
        html: String,
    ) {
        suspendCancellableCoroutine { continuation ->
            val webView = WebView(owner)
            webView.webViewClient =
                object : WebViewClient() {
                    override fun onPageFinished(
                        view: WebView,
                        url: String?,
                    ) {
                        try {
                            val adapter: PrintDocumentAdapter =
                                view.createPrintDocumentAdapter("leptoskit-print")
                            manager.print(
                                "leptoskit-print",
                                adapter,
                                PrintAttributes.Builder().build(),
                            )
                            if (continuation.isActive) continuation.resume(Unit)
                        } catch (error: Throwable) {
                            if (continuation.isActive) {
                                continuation.resumeWithException(
                                    PluginException(PluginException.NATIVE, error),
                                )
                            }
                        }
                    }
                }
            continuation.invokeOnCancellation {
                mainHandler.post { webView.destroy() }
            }
            webView.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null)
        }
    }

    private fun printPdf(
        manager: PrintManager,
        uri: String,
    ) {
        val parsed = Uri.parse(uri)
        val adapter =
            object : PrintDocumentAdapter() {
                override fun onLayout(
                    oldAttributes: PrintAttributes?,
                    newAttributes: PrintAttributes,
                    cancellationSignal: android.os.CancellationSignal?,
                    callback: LayoutResultCallback,
                    extras: android.os.Bundle?,
                ) {
                    if (cancellationSignal?.isCanceled == true) {
                        callback.onLayoutCancelled()
                        return
                    }

                    callback.onLayoutFinished(
                        PrintDocumentInfo
                            .Builder("leptoskit-print.pdf")
                            .setContentType(PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
                            .build(),
                        true,
                    )
                }

                override fun onWrite(
                    pages: Array<out android.print.PageRange>,
                    destination: android.os.ParcelFileDescriptor,
                    cancellationSignal: android.os.CancellationSignal?,
                    callback: WriteResultCallback,
                ) {
                    try {
                        context.contentResolver.openInputStream(parsed).use { input ->
                            if (input == null) throw PluginException(PluginException.INVALID_ARGUMENT)

                            java.io.FileOutputStream(destination.fileDescriptor).use { output ->
                                input.copyTo(output)
                            }
                        }
                        callback.onWriteFinished(arrayOf(android.print.PageRange.ALL_PAGES))
                    } catch (error: Throwable) {
                        callback.onWriteFailed(error.message)
                    }
                }
            }
        // Job acceptance is enough; dialog completion is not separately awaited.
        manager.print("leptoskit-print-$parsed", adapter, PrintAttributes.Builder().build())
    }

    private fun printManager(): PrintManager? = context.getSystemService(Context.PRINT_SERVICE) as? PrintManager

    private fun activeOwner(): Activity? =
        (context as? Activity)?.takeIf {
            !it.isFinishing && !it.isDestroyed && it.window?.decorView?.isAttachedToWindow == true
        }

    private fun encode(status: Int): ByteArray =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(status)
            .array()

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        const val AVAILABILITY = 0
        const val PRINT = 1
        const val READY = 1
        const val INACTIVE = 2
        const val BUSY = 3
    }
}
