package dev.leptoskit.mobile

import android.content.res.AssetManager
import android.graphics.Typeface
import org.json.JSONObject
import java.io.FileNotFoundException
import java.util.concurrent.ConcurrentHashMap

/**
 * The app's declared fonts: the faces `leptoskit.toml` declares under
 * `[theme.fonts]`, which the build lists in the APK's `fonts.json` and
 * bundles under `assets/`.
 *
 * [install] hands the manifest to the native library before the app is
 * created, so `font-[var(--name)]` resolves to the declared family and layout
 * measures in the face that draws. Measurement and drawing then take every
 * typeface from [typeface] or [face], so both use the same file at the same
 * weight. Faces load on first use and are shared by every thread.
 */
internal object NativeFonts {
    /** The manifest, at the root of the APK's assets beside `codepush.json`. */
    private const val MANIFEST = "fonts.json"

    /** Where the manifest's files are, inside the APK's assets. */
    private const val FOLDER = "assets/"

    private class Declared(
        val file: String,
        val variable: Boolean,
        val italic: Boolean,
    )

    @Volatile private var assets: AssetManager? = null

    @Volatile private var faces: List<Declared> = emptyList()

    private val loaded = ConcurrentHashMap<Int, Typeface>()

    /**
     * Installs the app's fonts for this process from [assets]; later calls
     * change nothing. An app built without `fonts.json` declares no fonts.
     */
    @Synchronized
    fun install(assets: AssetManager) {
        if (this.assets != null) return

        val manifest =
            try {
                assets.open(MANIFEST).use { it.readBytes() }
            } catch (_: FileNotFoundException) {
                null
            }

        if (manifest != null) {
            val status = NativeApp.installFonts(manifest)
            check(status == 0) { "The app's fonts could not be installed: $status" }
            faces = parse(manifest)
        }

        this.assets = assets
    }

    /**
     * The typeface for text at CSS [weight] and [italic] in declared [family]
     * (from a widget's `font_family`), or in the system font when [family] is
     * null or names no installed family.
     */
    fun typeface(
        family: Int?,
        weight: Int,
        italic: Boolean,
    ): Typeface {
        if (family != null) {
            val selected = NativeApp.selectFont(family, weight, italic)

            if (selected >= 0) return face(selected shr 16, selected and 0xFFFF)
        }

        return nativeTypeface(weight)
    }

    /**
     * The declared face at position [index] in `fonts.json`, drawn at
     * [weight]: the instance of a variable face's `wght` axis, or a static
     * face as it is.
     */
    fun face(
        index: Int,
        weight: Int,
    ): Typeface {
        val declared = faces.getOrNull(index) ?: error("No declared font face $index")
        val key = if (declared.variable) (index shl 16) or weight else index shl 16

        return loaded.getOrPut(key) { load(declared, weight) }
    }

    private fun load(
        declared: Declared,
        weight: Int,
    ): Typeface {
        val assets = this.assets ?: error("Fonts are drawn before they are installed")
        val builder =
            Typeface
                .Builder(assets, FOLDER + declared.file)
                .setWeight(weight)
                .setItalic(declared.italic)

        if (declared.variable) builder.setFontVariationSettings("'wght' $weight")

        // The build checks every declared file, so a face that does not load
        // means the APK was changed after it was built.
        return builder.build() ?: error("The font ${declared.file} could not be loaded")
    }

    private fun parse(manifest: ByteArray): List<Declared> {
        val families = JSONObject(manifest.toString(Charsets.UTF_8)).getJSONArray("families")

        return buildList {
            for (family in 0 until families.length()) {
                val faces = families.getJSONObject(family).getJSONArray("faces")

                for (face in 0 until faces.length()) {
                    val declared = faces.getJSONObject(face)
                    val weight = declared.getJSONArray("weight")
                    add(
                        Declared(
                            file = declared.getString("file"),
                            variable = weight.getInt(0) != weight.getInt(1),
                            italic = declared.optBoolean("italic"),
                        ),
                    )
                }
            }
        }
    }
}
