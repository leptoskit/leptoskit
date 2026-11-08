package dev.leptoskit.plugins.contacts

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.database.Cursor
import android.net.Uri
import android.os.Looper
import android.provider.ContactsContract
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Contacts permission, system pick-one, and a bounded query. Denied access never
 * invents contacts. Purpose / manifest declarations stay app-owned.
 */
class ContactsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "contacts"
    override val methods = intArrayOf(STATUS, REQUEST_PERMISSION, PICK, QUERY)
    private val activity = context as? Activity
    private val resultHost = context as? NativeActivityResultHost
    private val permissionHost = context as? NativePermissionHost
    private var busy = false
    private var closed = false
    private var pendingJob: Job? = null

    override fun close() {
        closed = true
        pendingJob?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Contacts UI requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            STATUS -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                encode(status())
            }

            REQUEST_PERMISSION -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                exclusive { requestPermission() }
            }

            PICK -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                exclusive { pick() }
            }

            else -> {
                val limit = ContactsRecord.decodeQueryLimit(payload)
                exclusive { query(limit) }
            }
        }
    }

    private suspend fun exclusive(operation: suspend () -> ByteArray): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)

        busy = true
        pendingJob = currentCoroutineContext()[Job]
        try {
            return operation()
        } finally {
            busy = false
            pendingJob = null
        }
    }

    private fun status(): Int {
        if (granted()) return GRANTED

        val activity = activity
        return if (activity?.shouldShowRequestPermissionRationale(Manifest.permission.READ_CONTACTS) == true) {
            DENIED
        } else {
            PROMPT
        }
    }

    private fun granted(): Boolean = context.checkSelfPermission(Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED

    private fun declaresPermission(): Boolean =
        context.packageManager
            .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            ?.contains(Manifest.permission.READ_CONTACTS) == true

    private suspend fun requestPermission(): ByteArray {
        if (granted()) return encode(GRANTED)

        if (!declaresPermission()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val host = permissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        val result = host.requestNativePermissions(arrayOf(Manifest.permission.READ_CONTACTS))
        if (result.size != 1) throw PluginException(PluginException.NATIVE)
        if (closed) throw PluginException(PluginException.CANCELLED)

        return encode(if (granted()) GRANTED else DENIED)
    }

    private suspend fun pick(): ByteArray {
        val host = resultHost ?: throw PluginException(PluginException.UNAVAILABLE)
        if (activeOwner() == null) throw PluginException(PluginException.UNAVAILABLE)

        currentCoroutineContext().ensureActive()
        val result =
            try {
                host.launchPluginActivity(
                    Intent(Intent.ACTION_PICK, ContactsContract.Contacts.CONTENT_URI),
                )
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (result.resultCode == Activity.RESULT_CANCELED) throw PluginException(PluginException.CANCELLED)

        if (result.resultCode != Activity.RESULT_OK) throw PluginException(PluginException.NATIVE)

        val uri = result.data?.data ?: throw PluginException(PluginException.CANCELLED)
        val contact =
            try {
                withContext(Dispatchers.IO) { readContact(uri) }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return contact.encode()
    }

    private suspend fun query(limit: Int): ByteArray {
        if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)

        currentCoroutineContext().ensureActive()
        val contacts =
            try {
                withContext(Dispatchers.IO) { listContacts(limit) }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return ContactsRecord.encodeList(contacts)
    }

    private suspend fun listContacts(limit: Int): List<ContactsRecord> {
        val resolver = context.contentResolver
        val contacts = ArrayList<ContactsRecord>(limit.coerceAtMost(ContactsRecord.MAX_QUERY))
        resolver
            .query(
                ContactsContract.Contacts.CONTENT_URI,
                arrayOf(
                    ContactsContract.Contacts._ID,
                    ContactsContract.Contacts.DISPLAY_NAME_PRIMARY,
                    ContactsContract.Contacts.HAS_PHONE_NUMBER,
                ),
                null,
                null,
                "${ContactsContract.Contacts.DISPLAY_NAME_PRIMARY} ASC",
            )?.use { cursor ->
                while (cursor.moveToNext() && contacts.size < limit) {
                    currentCoroutineContext().ensureActive()
                    val id = cursor.string(ContactsContract.Contacts._ID) ?: continue
                    val name =
                        cursor.string(ContactsContract.Contacts.DISPLAY_NAME_PRIMARY)?.ifBlank { null }
                            ?: continue
                    val hasPhone =
                        cursor.getInt(cursor.getColumnIndexOrThrow(ContactsContract.Contacts.HAS_PHONE_NUMBER)) > 0
                    val phone = if (hasPhone) primaryPhone(id) else null
                    val email = primaryEmail(id)
                    contacts.add(ContactsRecord.create(id, name, phone, email))
                }
            }

        return contacts
    }

    private fun readContact(uri: Uri): ContactsRecord {
        val resolver = context.contentResolver
        val projection = arrayOf(ContactsContract.Contacts._ID, ContactsContract.Contacts.DISPLAY_NAME_PRIMARY)
        resolver.query(uri, projection, null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) throw PluginException(PluginException.CANCELLED)

            val id =
                cursor.string(ContactsContract.Contacts._ID)
                    ?: throw PluginException(PluginException.NATIVE)
            val name =
                cursor.string(ContactsContract.Contacts.DISPLAY_NAME_PRIMARY)?.ifBlank { null }
                    ?: id
            return ContactsRecord.create(id, name, primaryPhone(id), primaryEmail(id))
        } ?: throw PluginException(PluginException.NATIVE)
    }

    private fun primaryPhone(contactId: String): String? =
        context.contentResolver
            .query(
                ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
                "${ContactsContract.CommonDataKinds.Phone.CONTACT_ID}=?",
                arrayOf(contactId),
                null,
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    cursor.string(ContactsContract.CommonDataKinds.Phone.NUMBER)?.ifBlank { null }
                } else {
                    null
                }
            }

    private fun primaryEmail(contactId: String): String? =
        context.contentResolver
            .query(
                ContactsContract.CommonDataKinds.Email.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Email.ADDRESS),
                "${ContactsContract.CommonDataKinds.Email.CONTACT_ID}=?",
                arrayOf(contactId),
                null,
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    cursor.string(ContactsContract.CommonDataKinds.Email.ADDRESS)?.ifBlank { null }
                } else {
                    null
                }
            }

    private fun Cursor.string(column: String): String? {
        val index = getColumnIndex(column)
        if (index < 0 || isNull(index)) return null

        return getString(index)
    }

    private fun activeOwner(): Activity? =
        activity?.takeIf {
            !it.isFinishing && !it.isDestroyed && it.window?.decorView?.isAttachedToWindow == true
        }

    private fun encode(status: Int): ByteArray =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(status)
            .array()

    private companion object {
        const val STATUS = 0
        const val REQUEST_PERMISSION = 1
        const val PICK = 2
        const val QUERY = 3
        const val PROMPT = 1
        const val DENIED = 2
        const val GRANTED = 3
    }
}
