package dev.leptoskit.plugins.medialibrary

import android.Manifest
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Looper
import android.provider.MediaStore
import androidx.core.content.ContextCompat
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Lists and saves MediaStore assets with bounded replies. Denied access never
 * invents rows. Change observation remains out of scope.
 */
class MediaLibraryPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "media-library"
    override val methods = intArrayOf(STATUS, REQUEST_PERMISSION, LIST_ASSETS, SAVE)
    private val permissions = context as? NativePermissionHost
    private var closed = false

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
            STATUS -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                encode(status())
            }

            REQUEST_PERMISSION -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                encode(requestPermission())
            }

            LIST_ASSETS -> {
                list(ListOptions.decode(payload))
            }

            else -> {
                save(SaveRequest.decode(payload))
            }
        }
    }

    private fun status(): Int =
        when {
            hasReadPermission() -> GRANTED
            else -> PROMPT
        }

    private suspend fun requestPermission(): Int {
        if (hasReadPermission()) return GRANTED
        val host = permissions ?: throw PluginException(PluginException.UNAVAILABLE)
        val needed = readPermissions()
        val result = host.requestNativePermissions(needed)
        if (result.size != needed.size) throw PluginException(PluginException.NATIVE)
        if (closed) throw PluginException(PluginException.CANCELLED)
        return if (hasReadPermission()) GRANTED else DENIED
    }

    private suspend fun list(options: ListOptions): ByteArray {
        currentCoroutineContext().ensureActive()
        if (!hasReadPermission()) throw PluginException(PluginException.PERMISSION_DENIED)
        val collection =
            when (options.kind) {
                MediaKind.Images -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI
                MediaKind.Videos -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI
                MediaKind.All -> MediaStore.Files.getContentUri("external")
            }
        val projection =
            arrayOf(
                MediaStore.MediaColumns._ID,
                MediaStore.MediaColumns.WIDTH,
                MediaStore.MediaColumns.HEIGHT,
                MediaStore.MediaColumns.DATE_ADDED,
                MediaStore.Files.FileColumns.MEDIA_TYPE,
            )
        val selection =
            when (options.kind) {
                MediaKind.All -> {
                    "${MediaStore.Files.FileColumns.MEDIA_TYPE}=? OR ${MediaStore.Files.FileColumns.MEDIA_TYPE}=?"
                }

                else -> {
                    null
                }
            }
        val selectionArgs =
            when (options.kind) {
                MediaKind.All -> {
                    arrayOf(
                        MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
                            .toString(),
                        MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
                            .toString(),
                    )
                }

                else -> {
                    null
                }
            }
        val assets = ArrayList<Asset>(options.limit)
        context.contentResolver
            .query(
                collection,
                projection,
                selection,
                selectionArgs,
                "${MediaStore.MediaColumns.DATE_ADDED} DESC",
            )?.use { cursor ->
                val idIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                val widthIndex = cursor.getColumnIndex(MediaStore.MediaColumns.WIDTH)
                val heightIndex = cursor.getColumnIndex(MediaStore.MediaColumns.HEIGHT)
                val dateIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DATE_ADDED)
                val typeIndex = cursor.getColumnIndex(MediaStore.Files.FileColumns.MEDIA_TYPE)
                while (cursor.moveToNext() && assets.size < options.limit) {
                    val id = cursor.getLong(idIndex)
                    val kind =
                        when (options.kind) {
                            MediaKind.Images -> {
                                MediaKind.Images
                            }

                            MediaKind.Videos -> {
                                MediaKind.Videos
                            }

                            MediaKind.All -> {
                                if (typeIndex >= 0 &&
                                    cursor.getInt(typeIndex) ==
                                    MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
                                ) {
                                    MediaKind.Videos
                                } else {
                                    MediaKind.Images
                                }
                            }
                        }
                    val contentUri =
                        when (kind) {
                            MediaKind.Videos -> {
                                ContentUris.withAppendedId(
                                    MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
                                    id,
                                )
                            }

                            else -> {
                                ContentUris.withAppendedId(
                                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                                    id,
                                )
                            }
                        }
                    assets.add(
                        Asset(
                            id = id.toString(),
                            kind = kind,
                            uri = contentUri.toString(),
                            width = widthIndex.takeIf { it >= 0 }?.let { cursor.getInt(it) },
                            height = heightIndex.takeIf { it >= 0 }?.let { cursor.getInt(it) },
                            creationTimeMs =
                                dateIndex.takeIf { it >= 0 }?.let { cursor.getLong(it) * 1000L },
                        ),
                    )
                }
            }
        return encodeAssets(assets)
    }

    private suspend fun save(request: SaveRequest): ByteArray {
        currentCoroutineContext().ensureActive()
        if (!hasReadPermission()) throw PluginException(PluginException.PERMISSION_DENIED)
        val source = Uri.parse(request.uri)
        val values =
            ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, "leptoskit-${System.currentTimeMillis()}")
                put(
                    MediaStore.MediaColumns.MIME_TYPE,
                    if (request.kind == MediaKind.Videos) "video/mp4" else "image/jpeg",
                )
            }
        val collection =
            when (request.kind) {
                MediaKind.Videos -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI
                else -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI
            }
        val dest =
            context.contentResolver.insert(collection, values)
                ?: throw PluginException(PluginException.NATIVE)
        context.contentResolver.openOutputStream(dest)?.use { output ->
            val input =
                when (source.scheme) {
                    "file" -> {
                        File(source.path!!).inputStream()
                    }

                    else -> {
                        context.contentResolver.openInputStream(source)
                            ?: throw PluginException(PluginException.INVALID_ARGUMENT)
                    }
                }
            input.use { it.copyTo(output) }
        } ?: throw PluginException(PluginException.NATIVE)
        return Asset(
            id = ContentUris.parseId(dest).toString(),
            kind = request.kind,
            uri = dest.toString(),
            width = null,
            height = null,
            creationTimeMs = null,
        ).encode()
    }

    private fun hasReadPermission(): Boolean =
        readPermissions().all {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        }

    private fun readPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= 33) {
            arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO,
            )
        } else {
            arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
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
        const val STATUS = 0
        const val REQUEST_PERMISSION = 1
        const val LIST_ASSETS = 2
        const val SAVE = 3
        const val PROMPT = 1
        const val DENIED = 2
        const val GRANTED = 3
    }
}
