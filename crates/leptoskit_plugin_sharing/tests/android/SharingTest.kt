package dev.leptoskit.plugins.sharing

import android.content.ClipData
import android.content.Intent
import android.net.Uri
import android.system.Os
import androidx.core.content.FileProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID

/**
 * The request the Rust and Swift suites use byte for byte: text "hi", subject "S",
 * title "T", anchor (1, 2, 3, 4), Mail excluded, app file `a.txt` as `text/plain`,
 * and the bytes 1, 2, 3 named `b.bin`.
 */
private val GOLDEN_REQUEST =
    hex(
        "01000000" + "020000006869" + "00000000" + "0100000053" + "0100000054" +
            "01000000" + "0000803f" + "00000040" + "00004040" + "00008040" +
            "0100000004000000" + "02000000" +
            "00000000" + "05000000612e747874" + "0a000000746578742f706c61696e" + "00000000" +
            "01000000" + "03000000010203" + "00000000" + "05000000622e62696e",
    )

private fun hex(value: String) = ByteArray(value.length / 2) { value.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

private fun bytes(vararg values: Int) = ByteArray(values.size) { values[it].toByte() }

/** One file of a hand-built request. */
private class WireFile(
    val source: Int,
    val payload: ByteArray,
    val mime: String = "",
    val name: String = "",
)

private fun app(
    path: String,
    mime: String = "",
    name: String = "",
) = WireFile(0, path.toByteArray(), mime, name)

/** Little-endian request with only the given text, URL, labels, exclusions, and files. */
private fun request(
    text: String = "",
    url: String = "",
    subject: String = "",
    excluded: List<Int> = emptyList(),
    files: List<WireFile> = emptyList(),
): ByteBuffer {
    val buffer = ByteBuffer.allocate(1 shl 24).order(ByteOrder.LITTLE_ENDIAN)
    val field = { value: ByteArray -> buffer.putInt(value.size).put(value) }
    buffer.putInt(1)
    listOf(text, url, subject, "").forEach { field(it.toByteArray()) }
    buffer.putInt(0).putInt(excluded.size)
    excluded.forEach { buffer.putInt(it) }
    buffer.putInt(files.size)

    for (file in files) {
        buffer.putInt(file.source)
        field(file.payload)
        field(file.mime.toByteArray())
        field(file.name.toByteArray())
    }

    buffer.flip()

    return buffer
}

private fun rejects(
    payload: ByteBuffer,
    code: Int = PluginException.INVALID_ARGUMENT,
) {
    try {
        ShareContent.decode(payload)
        fail("Expected rejection")
    } catch (error: PluginException) {
        assertEquals(code, error.code)
    }
}

private fun rejects(
    payload: ByteArray,
    code: Int = PluginException.INVALID_ARGUMENT,
) = rejects(ByteBuffer.wrap(payload), code)

@RunWith(AndroidJUnit4::class)
class SharingTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private lateinit var root: File
    private lateinit var staging: ShareStaging

    @Before
    fun createDirectories() {
        root = File(context.cacheDir, "sharing-test-${UUID.randomUUID()}")
        File(root, "files/docs").mkdirs()
        File(root, "files/docs/report.txt").writeText("report")
        File(root, "secret.txt").writeText("secret")
        staging = staging()
    }

    @After
    fun removeDirectories() {
        root.deleteRecursively()
    }

    private fun staging(
        maxFile: Long = ShareStaging.MAX_FILE_BYTES,
        maxTotal: Long = ShareStaging.MAX_TOTAL_BYTES,
    ) = ShareStaging(File(root, "files"), File(root, "leptoskit-sharing"), ShareStaging::procPath, { null }, maxFile, maxTotal)

    private fun appFile(
        path: String,
        name: String? = null,
    ) = ShareFile(path, null, null, name)

    private fun stagingRejects(
        files: List<ShareFile>,
        code: Int,
        staging: ShareStaging = this.staging,
    ) {
        try {
            staging.stage(files) {}
            fail("Expected rejection")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    @Test
    fun goldenRequestDecodesLikeRust() {
        val content = ShareContent.decode(ByteBuffer.wrap(GOLDEN_REQUEST))
        assertEquals("hi", content.text)
        assertNull(content.url)
        assertEquals("S", content.subject)
        assertEquals("T", content.title)
        assertEquals(ShareAnchor(1f, 2f, 3f, 4f), content.anchor)
        assertEquals(listOf(4), content.excluded)
        assertEquals(
            listOf(ShareFile("a.txt", null, "text/plain", null), ShareFile(null, bytes(1, 2, 3), null, "b.bin")),
            content.files,
        )

        for (count in GOLDEN_REQUEST.indices) rejects(GOLDEN_REQUEST.copyOf(count))

        rejects(GOLDEN_REQUEST + 0)
    }

    @Test
    fun goldenRepliesEncodeLikeRust() {
        assertArrayEquals(bytes(1, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 120, 46, 121), ShareReply.success("x.y"))
        assertArrayEquals(bytes(1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0), ShareReply.dismissed())
        assertArrayEquals(bytes(1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0), ShareReply.unavailable())

        // An identifier the guest would refuse is reported as unavailable instead.
        for (raw in listOf("", "a\nb", "a".repeat(1025))) {
            assertArrayEquals(ShareReply.unavailable(), ShareReply.success(raw))
        }
    }

    @Test
    fun textUrlsAndLabelsAreBounded() {
        assertEquals("hello\n世界", ShareContent.decode(request(text = "hello\n世界")).text)
        assertEquals("https://example.test/a%20b", ShareContent.decode(request(url = "https://example.test/a%20b")).url)

        for (url in listOf(
            "file:///secret",
            "content://provider/item",
            "https://user:pass@example.test",
            "https:///host",
            "https://example.test/a b",
        )) {
            rejects(request(url = url))
        }

        rejects(request(text = "a".repeat(65537)), PluginException.TOO_LARGE)
        rejects(request(text = "a\u0000b"))
        rejects(request(text = "t", subject = "line\nbreak"))
        rejects(request(text = "t", subject = "s".repeat(1025)), PluginException.TOO_LARGE)
        // Nothing to share, a URL with text, and a URL with files.
        rejects(request())
        rejects(request(text = "t", url = "https://example.test"))
        rejects(request(url = "https://example.test", files = listOf(app("a"))))
        rejects(request(text = "a").apply { put(4, 0) })
    }

    @Test
    fun previousProtocolAndUnknownVersionsAreRefused() {
        rejects(bytes(0, 0, 0, 0, 1, 0, 0, 0, 120))
        rejects(GOLDEN_REQUEST.copyOf().also { it[0] = 2 })
        rejects(ByteArray(ShareContent.MAX_REQUEST_BYTES + 1), PluginException.TOO_LARGE)
    }

    @Test
    fun anchorsAndExcludedActivitiesAreValidated() {
        val nan = GOLDEN_REQUEST.copyOf()
        ByteBuffer.wrap(nan).order(ByteOrder.LITTLE_ENDIAN).putFloat(28, Float.NaN)
        rejects(nan)
        val negative = GOLDEN_REQUEST.copyOf()
        ByteBuffer.wrap(negative).order(ByteOrder.LITTLE_ENDIAN).putFloat(36, -1f)
        rejects(negative)
        rejects(GOLDEN_REQUEST.copyOf().also { it[24] = 2 })
        rejects(request(text = "t", excluded = listOf(20)))
        rejects(request(text = "t", excluded = listOf(4, 4)))
        rejects(request(text = "t", excluded = List(21) { 0 }), PluginException.TOO_LARGE)
        assertEquals((0 until 20).toList(), ShareContent.decode(request(text = "t", excluded = (0 until 20).toList())).excluded)
    }

    @Test
    fun filePathsNamesAndMimeTypesFollowTheRustRules() {
        for (path in listOf(
            "",
            "/etc/passwd",
            "a/../b",
            "../secret",
            "a//b",
            "a/",
            "a\\b",
            "con",
            "a/LPT9",
            "name.",
            ".leptoskit-tmp-x",
            "a?",
        )) {
            rejects(request(files = listOf(app(path))))
        }

        rejects(request(files = listOf(WireFile(0, bytes(0xFF)))))
        rejects(request(files = listOf(WireFile(1, bytes(1)))))
        rejects(request(files = listOf(WireFile(2, "a".toByteArray()))))
        rejects(request(files = listOf(app("a", name = "x/y"))))

        for (mime in listOf("image", "*/png", "image/png; q=1", "image/ png", "-a/b")) {
            rejects(request(files = listOf(app("a", mime = mime))))
        }

        for (mime in listOf("image/png", "image/*", "*/*", "text/x-c++")) {
            assertEquals(mime, ShareContent.decode(request(files = listOf(app("a", mime = mime)))).files[0].mime)
        }

        rejects(request(files = List(33) { app("a") }), PluginException.TOO_LARGE)
        val half = ByteArray(ShareContent.MAX_INLINE_BYTES / 2)
        ShareContent.decode(request(files = listOf(WireFile(1, half, name = "a"), WireFile(1, half, name = "b"))))
        rejects(
            request(files = listOf(WireFile(1, half, name = "a"), WireFile(1, half, name = "b"), WireFile(1, bytes(0), name = "c"))),
            PluginException.TOO_LARGE,
        )
    }

    @Test
    fun stagingCopiesAppFilesAndBytesAndClearsTheNextTime() {
        val staged =
            staging.stage(
                listOf(
                    appFile("docs/report.txt"),
                    appFile("docs/report.txt", "renamed.txt"),
                    ShareFile(null, bytes(1, 2, 3), "image/png", "b.png"),
                ),
            ) {}
        assertEquals(listOf("report.txt", "renamed.txt", "b.png"), staged.map { it.file.name })
        assertEquals(listOf("application/octet-stream", "application/octet-stream", "image/png"), staged.map { it.mime })
        assertEquals("report", staged[0].file.readText())
        assertEquals("report", staged[1].file.readText())
        assertArrayEquals(bytes(1, 2, 3), staged[2].file.readBytes())
        assertTrue(staged.all { it.file.canonicalPath.startsWith(staging.cache.canonicalPath) })

        val next = staging.stage(listOf(appFile("docs/report.txt"))) {}
        assertFalse(staged[2].file.exists())
        assertEquals(1, next.size)
        staging.clear()
        assertFalse(staging.cache.exists())
    }

    @Test
    fun symlinksCannotEscapeTheFilesDirectory() {
        val files = File(root, "files")
        Os.symlink(File(root, "secret.txt").path, File(files, "link.txt").path)
        Os.symlink(root.path, File(files, "outside").path)
        Os.symlink(File(files, "docs").path, File(files, "inside").path)

        stagingRejects(listOf(appFile("link.txt")), PluginException.INVALID_ARGUMENT)
        stagingRejects(listOf(appFile("outside/secret.txt")), PluginException.INVALID_ARGUMENT)
        // A directory symlink that stays inside the files directory still resolves.
        assertEquals("report", staging.stage(listOf(appFile("inside/report.txt"))) {}[0].file.readText())
    }

    @Test
    fun missingDirectoriesAndOversizedFilesFail() {
        stagingRejects(listOf(appFile("missing.txt")), PluginException.NATIVE)
        stagingRejects(listOf(appFile("docs")), PluginException.INVALID_ARGUMENT)
        stagingRejects(listOf(appFile("docs/report.txt")), PluginException.TOO_LARGE, staging(maxFile = 3))
        stagingRejects(
            listOf(appFile("docs/report.txt"), appFile("docs/report.txt")),
            PluginException.TOO_LARGE,
            staging(maxTotal = 8),
        )
    }

    @Test
    fun commonMimeTypesNarrowToWhatEveryFileShares() {
        assertEquals("*/*", ShareStaging.commonMime(emptyList()))
        assertEquals("image/png", ShareStaging.commonMime(listOf("image/png", "image/png")))
        assertEquals("image/*", ShareStaging.commonMime(listOf("image/png", "image/jpeg")))
        assertEquals("*/*", ShareStaging.commonMime(listOf("image/png", "application/pdf")))
    }

    /** The generated manifest's provider serves the share cache and nothing else. */
    @Test
    fun fileProviderServesOnlyTheShareCache() {
        val cache = File(context.cacheDir, "leptoskit-sharing/0")
        cache.mkdirs()
        val file = File(cache, "shared.txt").apply { writeText("shared") }
        val authority = context.packageName + ".leptoskit.sharing"

        try {
            val uri = FileProvider.getUriForFile(context, authority, file)
            assertEquals("content", uri.scheme)
            assertEquals(authority, uri.authority)
            assertEquals("shared", context.contentResolver.openInputStream(uri)!!.use { it.readBytes().decodeToString() })

            try {
                FileProvider.getUriForFile(context, authority, File(context.filesDir, "outside.txt"))
                fail("Files outside the share cache must not be served")
            } catch (expected: IllegalArgumentException) {
                // The provider's paths XML names only the share cache.
            }
        } finally {
            File(context.cacheDir, "leptoskit-sharing").deleteRecursively()
        }
    }

    @Test
    fun intentsCarryTextSubjectTitleAndReadGrantsOnlyForSharedFiles() {
        val text = ShareContent.decode(request(text = "body", subject = "subject"))
        val plain = ShareIntents.target(text, emptyList(), emptyList())
        assertEquals(Intent.ACTION_SEND, plain.action)
        assertEquals("text/plain", plain.type)
        assertEquals("body", plain.getStringExtra(Intent.EXTRA_TEXT))
        assertEquals("subject", plain.getStringExtra(Intent.EXTRA_SUBJECT))
        assertEquals(0, plain.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION)
        assertNull(plain.clipData)

        val first = Uri.parse("content://example/0/a.png")
        val second = Uri.parse("content://example/1/b.jpg")
        val files = ShareContent.decode(request(text = "caption", files = listOf(app("a"), app("b"))))
        val many = ShareIntents.target(files, listOf(first, second), listOf("image/png", "image/jpeg"))
        assertEquals(Intent.ACTION_SEND_MULTIPLE, many.action)
        assertEquals("image/*", many.type)
        assertEquals("caption", many.getStringExtra(Intent.EXTRA_TEXT))
        assertEquals(Intent.FLAG_GRANT_READ_URI_PERMISSION, many.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION)
        assertEquals(0, many.flags and Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        val clip: ClipData = many.clipData!!
        assertEquals(listOf(first, second), (0 until clip.itemCount).map { clip.getItemAt(it).uri })

        val one = ShareIntents.target(files, listOf(first), listOf("image/png"))
        assertEquals(Intent.ACTION_SEND, one.action)
        assertEquals("image/png", one.type)
    }

    @Test
    fun invalidPayloadsAndMissingHostFailBeforePresentation() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = SharingPlugin(context)

                for ((method, payload, code) in listOf(
                    Triple(0, ByteBuffer.wrap(bytes(0, 0, 0, 0, 1, 0, 0, 0, 120)), PluginException.INVALID_ARGUMENT),
                    Triple(1, request(text = "t"), PluginException.UNAVAILABLE),
                    // The instrumentation context is not an activity-result host.
                    Triple(0, request(text = "t"), PluginException.UNAVAILABLE),
                )) {
                    try {
                        plugin.invoke(method, payload)
                        fail("Expected rejection")
                    } catch (error: PluginException) {
                        assertEquals(code, error.code)
                    }
                }

                plugin.close()

                try {
                    plugin.invoke(0, request(text = "t"))
                    fail("Closed plugin must fail")
                } catch (error: PluginException) {
                    assertEquals(PluginException.CANCELLED, error.code)
                }
            }
        }
}
