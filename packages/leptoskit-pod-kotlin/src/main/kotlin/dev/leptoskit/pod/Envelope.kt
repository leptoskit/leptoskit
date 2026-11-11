package dev.leptoskit.pod

val POD_MAGIC = byteArrayOf(0x4c, 0x4b, 0x30, 0x31) // "LK01"
const val POD_CONTENT_TYPE = "application/vnd.leptoskit.rpc.v1+bin"
const val POD_CODEC_VERSION: UShort = 1u

enum class MessageType(val raw: UShort) {
	Rpc(1u),
	Live(2u),
	;

	companion object {
		fun fromU16(value: UShort): PodResult<MessageType> =
			when (value) {
				Rpc.raw -> Result.success(Rpc)
				Live.raw -> Result.success(Live)
				else -> Result.failure(PodError.InvalidTag(value))
			}
	}
}

data class Envelope(
	val codecVer: UShort,
	val msgType: MessageType,
	val flags: UShort,
	val body: ByteArray,
) {
	override fun equals(other: Any?): Boolean {
		if (this === other) return true
		if (other !is Envelope) return false
		return codecVer == other.codecVer &&
			msgType == other.msgType &&
			flags == other.flags &&
			body.contentEquals(other.body)
	}

	override fun hashCode(): Int {
		var result = codecVer.hashCode()
		result = 31 * result + msgType.hashCode()
		result = 31 * result + flags.hashCode()
		result = 31 * result + body.contentHashCode()
		return result
	}
}

fun wrap(
	msgType: MessageType,
	flags: UShort,
	body: ByteArray,
): PodResult<ByteArray> {
	if (body.size.toLong() > 0xFFFF_FFFFL) return Result.failure(PodError.TooLarge)
	val w = Writer(14 + body.size)
	w.raw(POD_MAGIC)
	w.u16(POD_CODEC_VERSION)
	w.u16(msgType.raw)
	w.u16(flags)
	w.u32(body.size.toUInt())
	w.raw(body)
	return Result.success(w.asSlice())
}

fun unwrap(bytes: ByteArray): PodResult<Envelope> {
	if (bytes.size < 14) return Result.failure(PodError.Truncated)
	if (!bytes.copyOfRange(0, 4).contentEquals(POD_MAGIC)) {
		return Result.failure(PodError.InvalidEnvelope)
	}
	val r = Reader(bytes.copyOfRange(4, bytes.size))
	val codecVer = r.u16().getOrElse { return Result.failure(it) }
	if (codecVer != POD_CODEC_VERSION) return Result.failure(PodError.InvalidEnvelope)
	val msgType = MessageType.fromU16(r.u16().getOrElse { return Result.failure(it) })
		.getOrElse { return Result.failure(it) }
	val flags = r.u16().getOrElse { return Result.failure(it) }
	val bodyLen = r.u32().getOrElse { return Result.failure(it) }.toInt()
	val body = r.remaining()
	if (body.size != bodyLen) return Result.failure(PodError.Truncated)
	return Result.success(Envelope(codecVer, msgType, flags, body))
}
