package dev.leptoskit.plugins.sharing

import android.os.ParcelFileDescriptor
import android.system.ErrnoException
import android.system.Os
import dev.leptoskit.bridge.PluginException
import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.nio.file.attribute.BasicFileAttributes

/** A copy in the private share cache, with the MIME type receivers see. */
internal data class StagedFile(
    val file: File,
    val mime: String,
)

/**
 * Copies shared files into a private cache directory the FileProvider serves.
 *
 * App files are refused when their last segment is a symlink or not a regular
 * file, and the opened file must still resolve inside the app's files directory,
 * so neither a symlink nor a swapped directory exposes another file. The cache is
 * cleared at the start of the next share, because a destination may read it
 * after the chooser closes.
 */
internal class ShareStaging(
    /** The app's files directory: the filesystem plugin's root. */
    val filesRoot: File,
    /** The private cache this plugin owns. */
    val cache: File,
    /** Reports the path the kernel resolved for an open file. */
    private val openedPath: (FileInputStream) -> Path,
    private val mimeFor: (String) -> String?,
    private val maxFileBytes: Long = MAX_FILE_BYTES,
    private val maxTotalBytes: Long = MAX_TOTAL_BYTES,
) {
    /** Replaces the previous share's copies with this share's files, in order. */
    fun stage(
        files: List<ShareFile>,
        checkActive: () -> Unit,
    ): List<StagedFile> {
        clear()
        createPrivate(cache)
        var total = 0L

        return files.mapIndexed { index, file ->
            checkActive()
            val directory = File(cache, index.toString())
            createPrivate(directory)
            val name = file.name ?: file.path!!.substringAfterLast('/')
            val destination = File(directory, name)
            val path = file.path

            if (path != null) {
                total += copyAppFile(path, destination, maxTotalBytes - total, checkActive)
            } else {
                val bytes = file.bytes!!
                total += bytes.size

                if (total > maxTotalBytes) throw PluginException(PluginException.TOO_LARGE)

                write(destination) { it.write(bytes) }
            }

            StagedFile(destination, file.mime ?: mimeFor(name.substringAfterLast('.', "").lowercase()) ?: FALLBACK)
        }
    }

    /** Removes every earlier copy, never following a link out of the cache. */
    fun clear() {
        if (Files.exists(cache.toPath(), LinkOption.NOFOLLOW_LINKS)) delete(cache.toPath())
    }

    private fun delete(path: Path) {
        if (Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) {
            Files.newDirectoryStream(path).use { entries -> entries.forEach(::delete) }
        }

        Files.deleteIfExists(path)
    }

    private fun createPrivate(directory: File) {
        if (!directory.mkdir() && !directory.isDirectory) throw PluginException(PluginException.NATIVE)

        directory.setReadable(false, false)
        directory.setWritable(false, false)
        directory.setExecutable(false, false)
        directory.setReadable(true, true)
        directory.setWritable(true, true)
        directory.setExecutable(true, true)
    }

    private fun copyAppFile(
        path: String,
        destination: File,
        budget: Long,
        checkActive: () -> Unit,
    ): Long {
        try {
            val root = filesRoot.toPath().toRealPath()
            val candidate = filesRoot.toPath().resolve(path)
            val attributes = Files.readAttributes(candidate, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)

            if (!attributes.isRegularFile) throw ShareContent.invalid()

            val limit = minOf(maxFileBytes, budget)

            if (attributes.size() > limit) throw PluginException(PluginException.TOO_LARGE)

            return FileInputStream(candidate.toFile()).use { input ->
                val opened = openedPath(input)

                if (!opened.startsWith(root) || opened == root) throw ShareContent.invalid()

                write(destination) { output -> copy(input, output, limit, checkActive) }
            }
        } catch (error: IOException) {
            throw PluginException(PluginException.NATIVE, error)
        }
    }

    private fun copy(
        input: InputStream,
        output: java.io.OutputStream,
        limit: Long,
        checkActive: () -> Unit,
    ): Long {
        val buffer = ByteArray(CHUNK_BYTES)
        var copied = 0L

        while (true) {
            checkActive()
            val count = input.read(buffer)

            if (count < 0) return copied

            copied += count

            // The file may grow after the size check.
            if (copied > limit) throw PluginException(PluginException.TOO_LARGE)

            output.write(buffer, 0, count)
        }
    }

    private fun <T> write(
        destination: File,
        body: (java.io.OutputStream) -> T,
    ): T =
        try {
            Files
                .newOutputStream(
                    destination.toPath(),
                    StandardOpenOption.CREATE_NEW,
                    StandardOpenOption.WRITE,
                    LinkOption.NOFOLLOW_LINKS,
                ).use(body)
        } catch (error: IOException) {
            throw PluginException(PluginException.NATIVE, error)
        }

    companion object {
        const val MAX_FILE_BYTES = 512L * 1024 * 1024
        const val MAX_TOTAL_BYTES = 1024L * 1024 * 1024
        private const val CHUNK_BYTES = 64 * 1024
        private const val FALLBACK = "application/octet-stream"

        /** Reduces several MIME types to the narrowest type that covers them all. */
        fun commonMime(types: List<String>): String {
            if (types.isEmpty()) return "*/*"

            var common = types.first()

            for (type in types.drop(1)) {
                if (type == common) continue

                val base = type.substringBefore('/')

                if (base != common.substringBefore('/')) return "*/*"

                common = "$base/*"
            }

            return common
        }

        /** The Linux kernel's record of what an open descriptor points at. */
        fun procPath(input: FileInputStream): Path =
            try {
                ParcelFileDescriptor.dup(input.fd).use { descriptor ->
                    File(Os.readlink("/proc/self/fd/${descriptor.fd}")).toPath()
                }
            } catch (error: ErrnoException) {
                throw PluginException(PluginException.NATIVE, error)
            }
    }
}
