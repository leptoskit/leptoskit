package dev.leptoskit.mobile

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeTextConstraintsTest {
    @Test
    fun fractionalPixelBoundsKeepTheLogicalConstraint() {
        val metrics =
            InstrumentationRegistry
                .getInstrumentation()
                .targetContext.resources.displayMetrics
        val measurer = NativeTextMeasurer(metrics)
        val text = "Fractional boxes must stay within their logical width.".toByteArray()

        for (pixels in listOf(0.25f, 4.25f, 120.25f)) {
            val width = pixels / metrics.density

            for (weight in listOf(400, 700)) {
                val measured = measurer.measure(text, 16f, weight, width, false)
                assertTrue(
                    "weight $weight in a $pixels px box measured ${measured[0]} points for $width points",
                    measured[0] > 0f && measured[0] <= width,
                )
            }
        }
    }
}
