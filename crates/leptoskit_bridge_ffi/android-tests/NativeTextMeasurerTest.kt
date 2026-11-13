package dev.leptoskit.mobile

import android.view.View
import android.widget.FrameLayout
import android.widget.TextView
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.ceil

/**
 * Layout sizes every label with [NativeTextMeasurer], and the renderer's
 * `TextView` draws it in that frame, so the two must agree or text is clipped.
 * A sentence wider than its box was once measured as a single line, and every
 * paragraph on a Solana Seeker showed only its first line.
 */
class NativeTextMeasurerTest {
    @Test
    fun aSentenceWiderThanItsBoxWraps() {
        val measurer = NativeTextMeasurer(context().resources.displayMetrics)
        val line = measurer.measure(SENTENCE.toByteArray(), SIZE, REGULAR, UNBOUNDED, false)
        val box = line[0] / 3
        val wrapped = measurer.measure(SENTENCE.toByteArray(), SIZE, REGULAR, box, false)

        // Text layout rounds glyph advances per device density; a sub-pixel
        // overshoot is rounding, not a wrap failure, so allow one pixel.
        assertTrue(
            "the wrapped sentence fits its box: ${wrapped[0]} > $box",
            wrapped[0] <= box + 1f,
        )
        assertTrue(
            "a sentence three times its box's width takes three lines: ${wrapped[1]} < 3 × ${line[1]}",
            wrapped[1] >= 3 * line[1],
        )
    }

    @Test
    fun theNarrowestBoxIsTheWidestWord() {
        val measurer = NativeTextMeasurer(context().resources.displayMetrics)
        val line = measurer.measure(SENTENCE.toByteArray(), SIZE, REGULAR, UNBOUNDED, false)
        val widest =
            SENTENCE.split(' ').maxOf { word ->
                measurer.measure(word.toByteArray(), SIZE, REGULAR, UNBOUNDED, false)[0]
            }
        val narrowest = measurer.measure(SENTENCE.toByteArray(), SIZE, REGULAR, WIDEST_WORD, false)

        assertEquals(widest, narrowest[0], 1f)
        assertTrue("one word to a line: ${narrowest[1]} <= ${line[1]}", narrowest[1] > line[1])
    }

    @Test
    fun eachTextViewNeedsTheFrameItWasMeasuredTo() =
        onMain {
            val density = context().resources.displayMetrics.density
            val measurer = NativeTextMeasurer(context().resources.displayMetrics)

            for ((weight, cssWeight) in listOf("regular" to REGULAR, "bold" to BOLD)) {
                val view = text(weight)

                for (box in listOf(UNBOUNDED, 120f, 200f, 319f)) {
                    val size = measurer.measure(SENTENCE.toByteArray(), SIZE, cssWeight, box, false)
                    // The renderer snaps each frame outward to whole pixels.
                    val width = ceil(size[0] * density).toInt()
                    val height = ceil(size[1] * density).toInt()
                    view.measure(
                        View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                        View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
                    )

                    assertEquals(
                        "$weight text measured in a box $box wide, drawn $width px wide",
                        height.toFloat(),
                        view.measuredHeight.toFloat(),
                        1f,
                    )
                }
            }
        }

    /** The text view the renderer makes for a text node. */
    private fun text(weight: String): TextView {
        val root = FrameLayout(context())
        val renderer = NativeWidgetRenderer(root, { _, _, _ -> })
        val props = JSONObject().put("text", SENTENCE).put("font_size", SIZE.toDouble()).put("font_weight", weight)
        renderer.render(RenderBatch(1, listOf(RenderMutation.Create(1, "text", WidgetProps.fromJson(props)))))

        return (0 until root.childCount).map(root::getChildAt).filterIsInstance<TextView>().single()
    }

    private fun context() = InstrumentationRegistry.getInstrumentation().targetContext

    private fun onMain(block: () -> Unit) {
        var result: Result<Unit>? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { result = runCatching(block) }

        checkNotNull(result).getOrThrow()
    }

    private companion object {
        // Reapers' threshold tagline, which a Seeker showed as one clipped line.
        const val SENTENCE =
            "The dead market did not stay dead. What it left behind still drifts between worlds, and someone has to bring it in."
        const val SIZE = 16f
        const val REGULAR = 400
        const val BOLD = 700
        const val UNBOUNDED = -1f
        const val WIDEST_WORD = -2f
    }
}
