package dev.leptoskit.pod

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.nio.file.Path
import kotlin.io.path.Path
import kotlin.io.path.exists
import kotlin.io.path.readText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.test.fail

class ConformanceTest {
	@Serializable
	private data class Vector(
		val name: String,
		val hex: String,
		val envelope: Boolean = false,
	)

	@Serializable
	private data class Malformed(
		val name: String,
		val hex: String,
		val expect: String,
		val envelope: Boolean = false,
	)

	@Serializable
	private data class Corpus(
		val vectors: List<Vector>,
		val malformed: List<Malformed>,
	)

	private fun hexDecode(hex: String): ByteArray =
		hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()

	private fun hexEncode(bytes: ByteArray): String =
		bytes.joinToString("") { "%02x".format(it.toInt() and 0xff) }

	private fun vectorsPath(): Path {
		val fromRepo =
			Path("crates/leptoskit_pod/conformance/vectors.json")
		if (fromRepo.exists()) return fromRepo
		return Path("../../crates/leptoskit_pod/conformance/vectors.json")
	}

	private fun loadCorpus(): Corpus {
		val raw = vectorsPath().readText()
		return Json { ignoreUnknownKeys = true }.decodeFromString(Corpus.serializer(), raw)
	}

	@Test
	fun conformanceVectors() {
		val corpus = loadCorpus()
		for (v in corpus.vectors) {
			val bytes = hexDecode(v.hex)
			if (v.envelope) {
				unwrap(bytes).getOrElse { fail("${v.name}: $it") }
			}
		}
	}

	@Test
	fun malformedVectors() {
		val corpus = loadCorpus()
		for (m in corpus.malformed) {
			val bytes = hexDecode(m.hex)
			val err =
				when {
					m.envelope -> unwrap(bytes).exceptionOrNull()
					m.name.contains("utf8") -> Reader(bytes).string().exceptionOrNull()
					m.name.contains("bool") -> Reader(bytes).bool().exceptionOrNull()
					else -> Reader(bytes).u16().exceptionOrNull()
				}
			requireNotNull(err) { "${m.name}: expected error" }
			assertTrue(
				err.toString().contains(m.expect),
				"${m.name}: expected ${m.expect}, got $err",
			)
		}
	}

	@Test
	fun encodeNoteMatchesVector() {
		val w = Writer()
		w.structHeader(0u, 0u)
		w.u8(7u)
		w.bool(true)
		w.string("hi").getOrElse { fail(it.toString()) }
		assertEquals("00000701020000006869", hexEncode(w.asSlice()))
	}

	@Test
	fun envelopeRpcEmpty() {
		val framed = wrap(MessageType.Rpc, 0u, ByteArray(0)).getOrElse { fail(it.toString()) }
		assertEquals("4c4b303101000100000000000000", hexEncode(framed))
	}

	@Test
	fun trailingAndTruncation() {
		var r = Reader(byteArrayOf(1, 2))
		r.u8().getOrElse { fail(it.toString()) }
		assertEquals(PodError.Trailing, r.finish().exceptionOrNull())

		r = Reader(byteArrayOf(1))
		assertEquals(PodError.Truncated, r.u16().exceptionOrNull())
	}
}
