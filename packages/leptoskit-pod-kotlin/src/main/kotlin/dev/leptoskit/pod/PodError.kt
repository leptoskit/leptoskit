package dev.leptoskit.pod

sealed class PodError : Exception() {
	data object Truncated : PodError()
	data object Trailing : PodError()
	data object TooLarge : PodError()
	data object InvalidUtf8 : PodError()
	data class InvalidTag(val tag: UShort) : PodError()
	data object InvalidEnvelope : PodError()
	data class UnknownDiscriminator(val disc: UShort) : PodError()
	data class UnknownVersion(val version: UShort) : PodError()

	override val message: String
		get() =
			when (this) {
				Truncated -> "Truncated"
				Trailing -> "Trailing"
				TooLarge -> "TooLarge"
				InvalidUtf8 -> "InvalidUtf8"
				is InvalidTag -> "InvalidTag($tag)"
				InvalidEnvelope -> "InvalidEnvelope"
				is UnknownDiscriminator -> "UnknownDiscriminator($disc)"
				is UnknownVersion -> "UnknownVersion($version)"
			}
}
