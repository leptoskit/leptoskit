package dev.leptoskit.plugins.application

import android.content.pm.PackageManager
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The same fixture and bytes as `android_fixture` in the Rust crate's tests. */
private val ANDROID_FIXTURE =
    ApplicationFacts(
        id = "dev.example.app",
        name = "Example",
        version = "1.2.3",
        build = "42",
        buildSignature = "0123456789ABCDEF".repeat(4),
        installerStore = "com.android.vending",
        installTime = 1_700_000_000_000L,
        updateTime = 1_710_000_000_456L,
    )

private val ANDROID_GOLDEN =
    listOf(
        "020000000f0000006465762e6578616d706c652e617070070000004578616d706c6505000000312e",
        "322e3302000000343240000000303132333435363738394142434445463031323334353637383941",
        "42434445463031323334353637383941424344454630313233343536373839414243444546130000",
        "00636f6d2e616e64726f69642e76656e64696e67010000000068e5cf8b01000001000000c84df123",
        "8e010000",
    ).joinToString("")

private fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }

class ApplicationPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun snapshotBytesMatchTheRustGolden() {
        assertEquals(ANDROID_GOLDEN, ApplicationWire.snapshot(ANDROID_FIXTURE).hex())
    }

    @Test
    fun reportsTheInstalledApplication() =
        runBlocking {
            val plugin = ApplicationPlugin(context)

            @Suppress("DEPRECATION")
            val installed = context.packageManager.getPackageInfo(context.packageName, 0)
            val bytes = ByteBuffer.wrap(plugin.invoke(0, ByteBuffer.allocate(0))).order(ByteOrder.LITTLE_ENDIAN)

            fun text(): String = ByteArray(bytes.int).also { bytes.get(it) }.toString(Charsets.UTF_8)

            fun time(): Long? {
                val present = bytes.int
                val millis = bytes.long
                if (present == 1) return millis
                assertEquals(0, present)
                assertEquals(0L, millis)
                return null
            }

            assertEquals(2, bytes.int)
            assertEquals(context.packageName, text())
            assertEquals(context.applicationInfo.loadLabel(context.packageManager).toString(), text())
            assertEquals(installed.versionName ?: "", text())
            val build =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    installed.longVersionCode
                } else {
                    @Suppress("DEPRECATION")
                    installed.versionCode.toLong()
                }
            assertEquals(build.toString(), text())
            val signature = text()
            assertTrue("a 64-digit uppercase SHA-256: $signature", signature.matches(Regex("[0-9A-F]{64}")))
            assertEquals(signature, buildSignature(context))
            assertEquals(installerStore(context) ?: "", text())
            assertEquals(installed.firstInstallTime, time())
            assertEquals(installed.lastUpdateTime, time())
            assertFalse(bytes.hasRemaining())
            plugin.close()
        }

    @Test
    fun signatureIsTheDigestOfTheSigningCertificate() {
        val certificate =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                context.packageManager
                    .getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
                    .signingInfo
                    ?.apkContentsSigners
                    ?.first()
            } else {
                @Suppress("DEPRECATION", "PackageManagerGetSignatures")
                context.packageManager
                    .getPackageInfo(context.packageName, PackageManager.GET_SIGNATURES)
                    .signatures
                    ?.first()
            }
        assertNotNull(certificate)
        assertEquals(ApplicationWire.sha256(certificate!!.toByteArray()), buildSignature(context))
        // SHA-256 of the empty input, a fixed reference value.
        assertEquals(
            "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
            ApplicationWire.sha256(ByteArray(0)),
        )
    }

    @Test
    fun rejectsUnknownMethodsPayloadsAndOversizedText() {
        val plugin = ApplicationPlugin { ANDROID_FIXTURE }

        runBlocking { assertEquals(ANDROID_GOLDEN, plugin.invoke(0, ByteBuffer.allocate(0)).hex()) }
        assertCode(PluginException.UNAVAILABLE) { plugin.invoke(1, ByteBuffer.allocate(0)) }
        assertCode(PluginException.INVALID_ARGUMENT) { plugin.invoke(0, ByteBuffer.wrap(byteArrayOf(1))) }
        val longInstaller = ANDROID_FIXTURE.copy(installerStore = "x".repeat(ApplicationWire.MAX_INSTALLER_BYTES + 1))
        assertCode(PluginException.TOO_LARGE) { ApplicationWire.snapshot(longInstaller) }
        val longName = ANDROID_FIXTURE.copy(name = "x".repeat(ApplicationWire.MAX_TEXT_BYTES + 1))
        assertCode(PluginException.TOO_LARGE) { ApplicationWire.snapshot(longName) }
        val negative = ApplicationWire.snapshot(ANDROID_FIXTURE.copy(installTime = -1, updateTime = null))
        assertEquals(ByteArray(24).toList(), negative.takeLast(24))
    }

    private fun assertCode(
        code: Int,
        block: suspend () -> Unit,
    ) {
        val error = assertThrows(PluginException::class.java) { runBlocking { block() } }
        assertEquals(code, error.code)
    }
}
