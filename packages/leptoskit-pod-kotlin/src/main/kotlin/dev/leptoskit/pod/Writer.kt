package dev.leptoskit.pod

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

class Writer(capacity: Int = 0) {
	private val out = ByteArrayOutputStream(if (capacity > 0) capacity else 32)

	fun len(): Int = out.size()

	fun isEmpty(): Boolean = out.size() == 0

	fun raw(value: ByteArray) {
		out.write(value)
	}

	fun u8(value: UByte) {
		out.write(value.toInt())
	}

	private fun putLe(size: Int, write: (ByteBuffer) -> Unit) {
		val buf = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
		write(buf)
		out.write(buf.array())
	}

	fun u16(value: UShort) = putLe(2) { it.putShort(value.toShort()) }

	fun u32(value: UInt) = putLe(4) { it.putInt(value.toInt()) }

	fun u64(value: ULong) = putLe(8) { it.putLong(value.toLong()) }

	fun i8(value: Byte) = u8(value.toUByte())
	fun i16(value: Short) = u16(value.toUShort())
	fun i32(value: Int) = u32(value.toUInt())
	fun i64(value: Long) = u64(value.toULong())

	fun bool(value: Boolean) = u8(if (value) 1u else 0u)

	fun bytes(value: ByteArray): PodResult<Unit> {
		if (value.size.toLong() > 0xFFFF_FFFFL) return Result.failure(PodError.TooLarge)
		u32(value.size.toUInt())
		raw(value)
		return Result.success(Unit)
	}

	fun string(value: String): PodResult<Unit> = bytes(value.encodeToByteArray())

	fun vecLen(len: Int): PodResult<Unit> {
		if (len.toLong() > 0xFFFF_FFFFL) return Result.failure(PodError.TooLarge)
		u32(len.toUInt())
		return Result.success(Unit)
	}

	fun optionTag(some: Boolean) = u8(if (some) 1u else 0u)

	fun resultTag(ok: Boolean) = u8(if (ok) 0u else 1u)

	fun structHeader(version: UByte, flags: UByte) {
		u8(version)
		u8(flags)
	}

	fun variant(disc: UShort) = u16(disc)

	fun asSlice(): ByteArray = out.toByteArray()

	fun finish(): ByteArray {
		val slice = asSlice()
		out.reset()
		return slice
	}
}
