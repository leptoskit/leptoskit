package dev.leptoskit.plugins.contacts

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun contactBytes(
    id: String,
    name: String,
    phone: String?,
    email: String?,
): ByteBuffer {
    val record = ContactsRecord.create(id, name, phone, email)

    return ByteBuffer.wrap(record.encode()).order(ByteOrder.LITTLE_ENDIAN)
}

/** Exercises contacts wire framing without opening the address book. */
fun main() {
    val decoded = ContactsRecord.decode(contactBytes("1", "Ada", "+1", "a@b.c"))
    check(decoded.id == "1")
    check(decoded.displayName == "Ada")
    check(decoded.phone == "+1")
    check(decoded.email == "a@b.c")

    val minimal = ContactsRecord.decode(contactBytes("x", "y", null, null))
    check(minimal.phone == null)
    check(minimal.email == null)

    for (bad in listOf("", "a\u0000b")) {
        try {
            ContactsRecord.create(bad, "n", null, null)
            error("Expected rejection: $bad")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT)
        }
    }

    fun limit(value: Int): ByteBuffer =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(value)
            .also { it.flip() }

    try {
        ContactsRecord.decodeQueryLimit(limit(0))
        error("Expected zero-limit rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    try {
        ContactsRecord.decodeQueryLimit(limit(33))
        error("Expected over-limit rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    check(ContactsRecord.decodeQueryLimit(limit(8)) == 8)

    val list = ContactsRecord.encodeList(listOf(ContactsRecord.create("1", "Ada", null, null)))
    val buffer = ByteBuffer.wrap(list).order(ByteOrder.LITTLE_ENDIAN)
    check(buffer.int == 1)
    check(ContactsRecord.decode(buffer).displayName == "Ada")
    check(!buffer.hasRemaining())

    println("Contacts synthetic wire and bounds checks passed")
}
