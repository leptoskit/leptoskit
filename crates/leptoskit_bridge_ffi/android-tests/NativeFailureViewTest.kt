package dev.leptoskit.mobile

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.view.View
import android.widget.FrameLayout
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow

/**
 * A stopped app says why, readably, whatever the app's background and the
 * system's theme: a stopped Reapers once looked like a plain black screen.
 */
class NativeFailureViewTest {
    @Test
    fun theMessageReadsOnItsOwnBackground() =
        onMain {
            val view = NativeFailureView.create(context(), MESSAGE)

            assertEquals(MESSAGE, view.text.toString())
            assertEquals("leptoskit-error", view.contentDescription)
            assertEquals(NativeFailureView.TEXT, view.currentTextColor)
            assertTrue(contrast(NativeFailureView.TEXT, NativeFailureView.BACKGROUND) >= 7.0)
        }

    @Test
    fun itCoversTheAppItReplaces() =
        onMain {
            // The app's own dark background, as Reapers draws it.
            val parent = FrameLayout(context()).apply { setBackgroundColor(Color.rgb(7, 8, 11)) }
            val view = NativeFailureView.create(context(), MESSAGE)
            parent.addView(view)
            parent.measure(exactly(SIZE), exactly(SIZE))
            parent.layout(0, 0, SIZE, SIZE)
            val bitmap = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888)
            parent.draw(Canvas(bitmap))

            assertEquals(SIZE, view.width)
            assertEquals(SIZE, view.height)
            // A corner, away from the centred text, shows the view's own background.
            assertEquals(NativeFailureView.BACKGROUND, bitmap.getPixel(1, 1))
            assertTrue(
                "the message draws in its own color",
                (0 until SIZE).any { x -> (0 until SIZE).any { y -> bitmap.getPixel(x, y) == NativeFailureView.TEXT } },
            )
        }

    private fun context() = InstrumentationRegistry.getInstrumentation().targetContext

    private fun exactly(size: Int) = View.MeasureSpec.makeMeasureSpec(size, View.MeasureSpec.EXACTLY)

    private fun luminance(color: Int): Double {
        fun channel(value: Int): Double {
            val linear = value / 255.0

            return if (linear <= 0.03928) linear / 12.92 else ((linear + 0.055) / 1.055).pow(2.4)
        }

        return 0.2126 * channel(Color.red(color)) + 0.7152 * channel(Color.green(color)) +
            0.0722 * channel(Color.blue(color))
    }

    private fun contrast(
        first: Int,
        second: Int,
    ): Double {
        val a = luminance(first)
        val b = luminance(second)

        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    private fun <T> onMain(block: () -> T): T {
        var result: Result<T>? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { result = runCatching(block) }

        return checkNotNull(result).getOrThrow()
    }

    private companion object {
        const val MESSAGE = "host call failed: invalid type: map, expected f32 at line 1 column 433"
        const val SIZE = 400
    }
}
