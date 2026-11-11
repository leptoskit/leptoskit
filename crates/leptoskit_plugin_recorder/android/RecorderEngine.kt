package dev.leptoskit.plugins.recorder

import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.media.MediaRecorder
import java.io.File
import kotlin.math.max

/**
 * The recording thread: an [AudioRecord] reading 16 kHz mono PCM into a
 * MediaCodec AAC-LC encoder writing `.m4a` through a [MediaMuxer], rotating
 * the encoder and muxer at each segment boundary with no gap between
 * segments, because the stream itself never stops.
 */
internal class RecorderEngine(
    private val directory: File,
    private val index: RecorderIndex,
    private val sampleRate: Int,
    private val bitrate: Int,
    private val segmentFrames: Long,
    private val thresholdDbfs: Double,
    private val listener: Listener,
) {
    /** What the engine tells its plugin. */
    interface Listener {
        /** A segment finished and is waiting for delivery. */
        fun onSegment(
            path: String,
            startedAtMs: Long,
            durationMs: Long,
            bytes: Long,
            peakDbfs: Double,
            activeMs: Long,
            endedBy: Int,
        )

        /** A new segment opened, with its start time. */
        fun onOpened(startedAtMs: Long)

        /** Recording failed; the reason is reported through status. */
        fun onFailure(reason: String)
    }

    private var worker: Thread? = null

    @Volatile
    private var stopping = false

    /** Starts the recording thread. */
    fun start() {
        stopping = false
        worker = Thread({ record() }, "leptoskit.recorder").apply { start() }
    }

    /** Asks the loop to finish the current segment and exit. */
    fun stop() {
        stopping = true
    }

    /** Waits for the loop to finish the current segment. */
    fun join(timeoutMillis: Long) {
        worker?.join(timeoutMillis)
    }

    private fun record() {
        val bufferSize =
            AudioRecord.getMinBufferSize(
                sampleRate,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
            )
        if (bufferSize <= 0) {
            listener.onFailure("no microphone input available")
            return
        }

        val audioRecord =
            try {
                AudioRecord(
                    MediaRecorder.AudioSource.MIC,
                    sampleRate,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    max(bufferSize * 2, sampleRate / 2),
                )
            } catch (error: Exception) {
                listener.onFailure("opening the microphone failed: ${error.message ?: error.javaClass.simpleName}")
                return
            }
        if (audioRecord.state != AudioRecord.STATE_INITIALIZED) {
            audioRecord.release()
            listener.onFailure("initialising the microphone failed")
            return
        }

        try {
            audioRecord.startRecording()
            if (audioRecord.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                listener.onFailure("the microphone did not start")
                return
            }

            val chunk = ShortArray(2_048)
            var segment: SegmentWriter? = null

            while (!stopping) {
                val read = audioRecord.read(chunk, 0, chunk.size)
                if (read < 0) {
                    listener.onFailure("reading the microphone failed")
                    break
                }
                if (read == 0) continue

                if (segment == null) segment = openSegment() ?: return
                var offset = 0
                while (offset < read) {
                    val writer = segment
                    if (writer == null) return

                    offset += writer.feed(chunk, offset, read - offset)
                    if (writer.isComplete()) {
                        writer.finish(ENDED_ROTATION)
                        segment = openSegment() ?: return
                    }
                }
            }

            segment?.finish(ENDED_STOP)
        } catch (error: Exception) {
            listener.onFailure("the recording thread failed: ${error.message ?: error.javaClass.simpleName}")
        } finally {
            try {
                audioRecord.stop()
            } catch (_: Exception) {
                // The stream was already gone.
            }
            audioRecord.release()
        }
    }

    /** Opens the next segment file and its journal line. */
    private fun openSegment(): SegmentWriter? {
        val startedMs = System.currentTimeMillis()
        var name = "seg-$startedMs.m4a"
        var file = File(directory, name)
        var attempt = 1
        while (file.exists()) {
            attempt += 1
            name = "seg-$startedMs-$attempt.m4a"
            file = File(directory, name)
        }

        return try {
            SegmentWriter(file, startedMs)
        } catch (error: Exception) {
            listener.onFailure("opening a segment failed: ${error.message ?: error.javaClass.simpleName}")
            null
        }
    }

    private fun fail(reason: String) {
        listener.onFailure(reason)
    }

    /**
     * One segment: its encoder, its muxer, its level totals, and the frame
     * buffer that measures 30-millisecond frames as they pass through.
     */
    private inner class SegmentWriter(
        val file: File,
        val startedAtMs: Long,
    ) {
        private val encoder: MediaCodec
        private var muxer: MediaMuxer? = null
        private var track = -1
        private var muxerStarted = false
        private var inputEnded = false
        private var outputDone = false
        private var samplesWritten = 0L
        private val levels = SegmentLevels(thresholdDbfs)
        private val frameBuffer = ShortArray(RecorderLevels.frameSamples(sampleRate))
        private var frameFill = 0
        private val info = MediaCodec.BufferInfo()

        init {
            val format =
                MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, sampleRate, 1).apply {
                    setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
                    setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
                    setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, -1)
                }
            encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC)
            encoder.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            encoder.start()
            index.recordOpen(file.name, startedAtMs, segmentFrames * 1_000 / sampleRate)
            listener.onOpened(startedAtMs)
        }

        /** Whether the boundary arrived and the segment is full. */
        fun isComplete(): Boolean = samplesWritten >= segmentFrames

        /**
         * Measures and encodes as many samples as still belong to this
         * segment, returning how many were consumed; the boundary splits a
         * chunk so no sample is lost to a gap.
         */
        fun feed(
            chunk: ShortArray,
            offset: Int,
            count: Int,
        ): Int {
            val take = RecorderLevels.split(samplesWritten, count, segmentFrames) ?: count

            var position = offset
            var remaining = take
            while (remaining > 0) {
                val copy = minOf(frameBuffer.size - frameFill, remaining)
                System.arraycopy(chunk, position, frameBuffer, frameFill, copy)
                frameFill += copy
                position += copy
                remaining -= copy
                if (frameFill == frameBuffer.size) {
                    levels.add(RecorderLevels.measure(frameBuffer))
                    frameFill = 0
                }
            }

            try {
                encode(chunk, offset, take)
            } catch (error: Exception) {
                fail("encoding audio failed: ${error.message ?: error.javaClass.simpleName}")
            }
            return take
        }

        /** Closes the segment and reports it, whatever its length reached. */
        fun finish(endedBy: Int) {
            try {
                if (!inputEnded) {
                    val index = encoder.dequeueInputBuffer(50_000)
                    if (index >= 0) {
                        encoder.queueInputBuffer(
                            index,
                            0,
                            0,
                            samplesWritten * 1_000_000 / sampleRate,
                            MediaCodec.BUFFER_FLAG_END_OF_STREAM,
                        )
                        inputEnded = true
                    }
                    drain(endOfStream = true)
                }
            } catch (error: Exception) {
                fail("closing a segment failed: ${error.message ?: error.javaClass.simpleName}")
            } finally {
                try {
                    if (muxerStarted) muxer?.stop()
                } catch (_: Exception) {
                    // A segment the muxer never started stays unreported.
                }
                muxer?.release()
                try {
                    encoder.stop()
                } catch (_: Exception) {
                    // Already stopped.
                }
                encoder.release()

                val size = if (file.exists()) file.length() else 0L
                if (muxerStarted && size > 0) {
                    index.recordFinalised(file.name, levels.durationMs, size, levels.peakDbfs, levels.activeMs, endedBy)
                    listener.onSegment(
                        file.absolutePath,
                        startedAtMs,
                        levels.durationMs,
                        size,
                        levels.peakDbfs,
                        levels.activeMs,
                        endedBy,
                    )
                } else {
                    // A file the muxer never finished cannot be read; the
                    // journal's open line keeps recovery from counting it.
                    file.delete()
                }
            }
        }

        /** Queues PCM into the encoder, splitting across its input buffers. */
        private fun encode(
            chunk: ShortArray,
            offset: Int,
            count: Int,
        ) {
            var position = offset
            var remaining = count
            var attempts = 0

            while (remaining > 0) {
                val index = encoder.dequeueInputBuffer(10_000)
                if (index < 0) {
                    attempts += 1
                    if (attempts > 50) throw IllegalStateException("the encoder stopped accepting audio")
                    drain(endOfStream = false)
                    continue
                }
                attempts = 0

                val input = encoder.getInputBuffer(index) ?: throw IllegalStateException("no input buffer")
                input.clear()
                val capacity = input.remaining() / 2
                val take = minOf(remaining, capacity)
                input.asShortBuffer().put(chunk, position, take)
                input.position(0)
                input.limit(take * 2)
                encoder.queueInputBuffer(
                    index,
                    0,
                    take * 2,
                    samplesWritten * 1_000_000 / sampleRate,
                    0,
                )
                samplesWritten += take
                position += take
                remaining -= take
                drain(endOfStream = false)
            }
        }

        /** Moves encoded audio into the muxer until it runs dry. */
        private fun drain(endOfStream: Boolean) {
            var attempts = 0

            while (!outputDone) {
                val index =
                    encoder.dequeueOutputBuffer(
                        info,
                        if (endOfStream) 50_000 else 0,
                    )
                when {
                    index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        val muxer =
                            this.muxer ?: MediaMuxer(file.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
                                .also { muxer = it }
                        track = muxer.addTrack(encoder.outputFormat)
                        muxer.start()
                        muxerStarted = true
                    }

                    index == MediaCodec.INFO_TRY_AGAIN_LATER -> {
                        if (!endOfStream) return
                        attempts += 1
                        if (attempts > 100) throw IllegalStateException("the encoder never finished")
                    }

                    index >= 0 -> {
                        val output = encoder.getOutputBuffer(index)
                        if (output != null) {
                            if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                                info.size = 0
                            } else if (muxerStarted && info.size > 0) {
                                output.position(info.offset)
                                output.limit(info.offset + info.size)
                                muxer?.writeSampleData(track, output, info)
                            }
                        }
                        encoder.releaseOutputBuffer(index, false)
                        if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) outputDone = true
                    }
                }
            }
        }
    }
}
