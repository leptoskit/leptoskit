package dev.leptoskit.plugins.recorder

import java.io.File
import java.io.FileOutputStream

/**
 * The on-disk index that survives a crash: an append-only journal beside
 * the segments, one line per event, tab-separated.
 *
 * - `F` names a file the recorder opened, with its start time and planned
 *   length, written before the first audio lands in it.
 * - `C` completes a file: its real duration, size, peak level, active time
 *   and why it ended.
 * - `D` marks a file delivered to the app, after which the app owns it.
 *
 * A crash between `F` and `C` leaves a file that never finished writing:
 * recovery deletes it. A crash after `C` but before `D` leaves a finalised
 * file the app never saw: recovery re-queues it as a recovered segment.
 */
internal class RecorderIndex(
    directory: File,
) {
    /** One finalised-but-undelivered segment the journal describes. */
    data class Finalised(
        val filename: String,
        val startedAtMs: Long,
        val durationMs: Long,
        val bytes: Long,
        val peakDbfs: Double,
        val activeMs: Long,
        val endedBy: Int,
    )

    /** What a recovery pass found: files to deliver again and files to delete. */
    data class Plan(
        val deliverable: List<Finalised>,
        val discardable: List<String>,
    )

    /** One parsed journal line, in file order. */
    sealed interface Line {
        data class Open(
            val file: String,
            val startedMs: Long,
            val plannedMs: Long,
        ) : Line

        data class FinalisedLine(
            val file: String,
            val durationMs: Long,
            val bytes: Long,
            val peakDbfs: Double,
            val activeMs: Long,
            val endedBy: Int,
        ) : Line

        data class Delivered(
            val file: String,
        ) : Line
    }

    /** Segments are named so a recovery pass can spot them without the journal. */
    private val file = File(directory, "recorder-index")

    init {
        file.parentFile?.mkdirs()
        if (!file.exists()) file.createNewFile()
    }

    /** Records opening `filename`. */
    fun recordOpen(
        filename: String,
        startedMs: Long,
        plannedMs: Long,
    ) = append("F\t$filename\t$startedMs\t$plannedMs")

    /** Records `filename` finishing with its real metrics. */
    fun recordFinalised(
        filename: String,
        durationMs: Long,
        bytes: Long,
        peakDbfs: Double,
        activeMs: Long,
        endedBy: Int,
    ) = append("C\t$filename\t$durationMs\t$bytes\t$peakDbfs\t$activeMs\t$endedBy")

    /** Records `filename` leaving the plugin's care, delivered to the app. */
    fun recordDelivered(filename: String) = append("D\t$filename")

    /**
     * Rewrites the journal holding only the entries `keeping` names, each
     * as a fresh open+finalised pair, so a long-lived session cannot grow
     * it without bound. Everything else is settled history.
     */
    fun compact(keeping: List<Finalised>) {
        val text =
            buildString {
                for (record in keeping) {
                    appendLine("F\t${record.filename}\t${record.startedAtMs}\t${record.durationMs}")
                    appendLine(
                        "C\t${record.filename}\t${record.durationMs}\t${record.bytes}\t${record.peakDbfs}" +
                            "\t${record.activeMs}\t${record.endedBy}",
                    )
                }
            }

        synchronized(this) {
            val temporary = File(file.parentFile, "recorder-index.tmp")
            temporary.writeText(text)
            if (!temporary.renameTo(file)) {
                temporary.delete()
                file.writeText(text)
            }
        }
    }

    /** Reads the whole journal, for a recovery pass. */
    fun read(): List<Line> = synchronized(this) { parse(file.readText()) }

    /** Appends one durable line. */
    private fun append(line: String) {
        synchronized(this) {
            FileOutputStream(file, true).use { output ->
                output.write((line + "\n").toByteArray(Charsets.UTF_8))
                output.flush()
                output.fd.sync()
            }
        }
    }

    companion object {
        /** Parses journal text, skipping torn or malformed lines. */
        fun parse(text: String): List<Line> {
            val lines = mutableListOf<Line>()

            for (row in text.split("\n")) {
                val fields = row.split("\t")
                when (fields.firstOrNull()) {
                    "F" -> {
                        if (fields.size == 4 && fields[1].isNotEmpty()) {
                            val started = fields[2].toLongOrNull()
                            val planned = fields[3].toLongOrNull()
                            if (started != null && planned != null) {
                                lines.add(Line.Open(fields[1], started, planned))
                            }
                        }
                    }

                    "C" -> {
                        if (fields.size == 7 && fields[1].isNotEmpty()) {
                            val duration = fields[2].toLongOrNull()
                            val bytes = fields[3].toLongOrNull()
                            val peak = fields[4].toDoubleOrNull()
                            val active = fields[5].toLongOrNull()
                            val ended = fields[6].toIntOrNull()
                            if (duration != null && bytes != null && peak != null && active != null && ended != null) {
                                lines.add(Line.FinalisedLine(fields[1], duration, bytes, peak, active, ended))
                            }
                        }
                    }

                    "D" -> {
                        if (fields.size == 2 && fields[1].isNotEmpty()) {
                            lines.add(Line.Delivered(fields[1]))
                        }
                    }
                }
            }

            return lines
        }

        /**
         * Classifies a journal against the files that exist, so a later
         * start can deliver what a crash left behind and delete what never
         * finished.
         */
        fun plan(
            lines: List<Line>,
            existingFiles: List<String>,
        ): Plan {
            val opened = mutableMapOf<String, Long>()
            val finalised = mutableMapOf<String, Finalised?>()
            val delivered = mutableSetOf<String>()

            for (line in lines) {
                when (line) {
                    is Line.Open -> {
                        opened[line.file] = line.startedMs
                        finalised[line.file] = null
                    }

                    is Line.FinalisedLine -> {
                        val startedMs = opened[line.file]
                        if (startedMs != null) {
                            finalised[line.file] =
                                Finalised(
                                    line.file,
                                    startedMs,
                                    line.durationMs,
                                    line.bytes,
                                    line.peakDbfs,
                                    line.activeMs,
                                    line.endedBy,
                                )
                        }
                    }

                    is Line.Delivered -> {
                        delivered.add(line.file)
                    }
                }
            }

            val deliverable = mutableListOf<Finalised>()
            val discardable = mutableListOf<String>()

            // A finalised file the app never saw is delivered again.
            for ((file, record) in finalised) {
                if (record != null && file !in delivered) deliverable.add(record)
            }

            // An opened file that never finalised was cut short mid-write.
            for (file in opened.keys) {
                if (finalised[file] == null) discardable.add(file)
            }

            // Segment files the journal never opened: orphans from a torn
            // first line or an older run of the directory.
            for (file in existingFiles) {
                if (file.startsWith("seg-") && file.endsWith(".m4a") && file !in opened) {
                    discardable.add(file)
                }
            }

            return Plan(deliverable.sortedBy { it.startedAtMs }, discardable.sorted())
        }
    }
}
