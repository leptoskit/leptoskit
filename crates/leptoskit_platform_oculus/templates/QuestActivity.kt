package __PACKAGE_ID__

/**
 * Experimental Meta Quest activity stub.
 *
 * A real host loads the leptoskit Wasm guest, applies render transactions to an
 * OpenXR panel (or compositor layer), and forwards input as NativeEvents.
 * Packaging and the OpenXR session loop are not wired yet.
 */
class QuestActivity {
	companion object {
		const val STATUS: String = "experimental-stub"
		const val PLATFORM: String = "oculus"
	}
}
