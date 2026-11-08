package dev.leptoskit.pod

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

class Reader(input: ByteArray) {
	private var remaining = input

	fun remaining(): ByteArray = remaining

	private fun take(len: Int): PodResult<ByteArray> {
		if (remaining.size < len) return Result.failure(PodError.Truncated)
		val slice = remaining.copyOfRange(0, len)
		remaining = remaining.copyOfRange(len, remaining.size)
		return Result.success(slice)
	}

	fun u8(): PodResult<UByte> = take(1).map { it[0].toUByte() }

	fun u16(): PodResult<UShort> =
		take(2).map {
			ByteBuffer.wrap(it).order(ByteOrder.LITTLE_ENDIAN).short.toUShort()
		}

	fun u32(): PodResult<UInt> =
		take(4).map {
			ByteBuffer.wrap(it).order(ByteOrder.LITTLE_ENDIAN).int.toUInt()
		}

	fun u64(): PodResult<ULong> =
		take(8).map {
			ByteBuffer.wrap(it).order(ByteOrder.LITTLE_ENDIAN).long.toULong()
		}

	fun i8(): PodResult<Byte> = u8().map { it.toByte() }
	fun i16(): PodResult<Short> = u16().map { it.toShort() }
	fun i32(): PodResult<Int> = u32().map { it.toInt() }
	fun i64(): PodResult<Long> = u64().map { it.toLong() }

	fun bool(): PodResult<Boolean> {
		val tag = u8().getOrElse { return Result.failure(it) }
		return when (tag) {
			0u.toUByte() -> Result.success(false)
			1u.toUByte() -> Result.success(true)
			else -> Result.failure(PodError.InvalidTag(tag.toUShort()))
		}
	}

	fun bytes(): PodResult<ByteArray> {
		val len = u32().getOrElse { return Result.failure(it) }.toInt()
		return take(len)
	}

	fun string(): PodResult<String> {
		val slice = bytes().getOrElse { return Result.failure(it) }
		val decoder =
			StandardCharsets.UTF_8
				.newDecoder()
				.onMalformedInput(java.nio.charset.CodingErrorAction.REPORT)
				.onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT)
		return try {
			Result.success(decoder.decode(ByteBuffer.wrap(slice)).toString())
		} catch (_: CharacterCodingException) {
			Result.failure(PodError.InvalidUtf8)
		}
	}

	fun vecLen(): PodResult<Int> = u32().map { it.toInt() }

	fun optionTag(): PodResult<Boolean> {
		val tag = u8().getOrElse { return Result.failure(it) }
		return when (tag) {
			0u.toUByte() -> Result.success(false)
			1u.toUByte() -> Result.success(true)
			else -> Result.failure(PodError.InvalidTag(tag.toUShort()))
		}
	}

	fun resultTag(): PodResult<Boolean> {
		val tag = u8().getOrElse { return Result.failure(it) }
		return when (tag) {
			0u.toUByte() -> Result.success(true)
			1u.toUByte() -> Result.success(false)
			else -> Result.failure(PodError.InvalidTag(tag.toUShort()))
		}
	}

	fun structHeader(): PodResult<UShort> = u16()

	fun variant(): PodResult<UShort> = u16()

	fun finish(): PodResult<Unit> =
		if (remaining.isEmpty()) {
			Result.success(Unit)
		} else {
			Result.failure(PodError.Trailing)
		}
}

private fun UByte.toUShort(): UShort = this.toUInt().toUShort()
