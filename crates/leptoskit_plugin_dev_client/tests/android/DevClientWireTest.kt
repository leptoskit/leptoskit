package dev.leptoskit.plugins.devclient

import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.ByteBuffer

/** The same addresses the Rust parser accepts and refuses, without a camera. */
fun main() {
    assertEquals(
        "http://127.0.0.1:9/_leptoskit/dev/android/guest",
        DevClientWire.canonical("  http://127.0.0.1:9/_leptoskit/dev  "),
    )
    assertEquals(
        "http://10.0.2.2:43123/_leptoskit/dev/android/guest",
        DevClientWire.canonical("http://10.0.2.2:43123/_leptoskit/dev/android/guest"),
    )
    assertEquals(
        "http://192.168.0.10:80/_leptoskit/dev/android/guest",
        DevClientWire.canonical("http://192.168.0.10"),
    )
    assertEquals(
        "https://dev.local:443/_leptoskit/dev/ios/guest",
        DevClientWire.canonical("https://dev.local/_leptoskit/dev/ios/guest"),
    )
    assertEquals(
        "http://[::1]:3000/_leptoskit/dev/android/guest",
        DevClientWire.canonical("http://[::1]:3000/_leptoskit/dev/android/guest/"),
    )
    for (refused in listOf(
        "",
        "ftp://127.0.0.1:1",
        "http://user:pw@127.0.0.1:1/_leptoskit/dev",
        "http://127.0.0.1:1/_leptoskit/dev?x=1",
        "http://8.8.8.8:1/_leptoskit/dev",
        "http://evil.com:1/_leptoskit/dev",
        "http://127.0.0.1:1/other",
        "http://127.0.0.1:0",
    )) {
        try {
            DevClientWire.canonical(refused)
            error("expected $refused to be refused")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }
    val encoded = DevClientWire.encode("ok")
    assertEquals("ok", DevClientWire.text(ByteBuffer.wrap(encoded)))
    assertEquals(4, DevClientWire.status(true).size)
}

class DevClientWireTest {
    @Test
    fun localAddressesMatchTheRustParser() {
        main()
    }
}
