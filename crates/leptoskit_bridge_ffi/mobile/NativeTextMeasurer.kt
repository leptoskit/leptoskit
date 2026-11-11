package dev.leptoskit.mobile

import android.graphics.Paint
import android.graphics.Typeface
import android.os.Build
import android.text.BoringLayout
import android.text.Layout
import android.text.SpannableString
import android.text.Spanned
import android.text.StaticLayout
import android.text.TextDirectionHeuristics
import android.text.TextPaint
import android.text.style.LineHeightSpan
import android.util.DisplayMetrics
import android.util.TypedValue
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.roundToInt

/** The CSS weight a render transaction names, from 100 to 900. */
internal fun cssWeight(name: String?): Int =
    when (name) {
        "thin" -> 100
        "extra_light" -> 200
        "light" -> 300
        "medium" -> 500
        "semibold" -> 600
        "bold" -> 700
        "extra_bold" -> 800
        "black" -> 900
        else -> 400
    }

/**
 * The default family at CSS [weight]. Android 9 and later draw the nearest
 * weight the system font has; earlier releases have only regular and bold,
 * so semibold and heavier draw bold.
 */
internal fun nativeTypeface(weight: Int): Typeface =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
        Typeface.create(Typeface.DEFAULT, weight, false)
    } else {
        Typeface.defaultFromStyle(if (weight >= SEMIBOLD) Typeface.BOLD else Typeface.NORMAL)
    }

private const val SEMIBOLD = 600

/**
 * Sets every line to [height] pixels with its glyphs centered in it, as CSS's
 * `line-height` does: the extra space, or the overlap, is split above and
 * below. Line bottoms are rounded from the text's top, so `n` lines are
 * exactly `n * height` tall, rounded once, which is what layout reserved.
 */
internal class ExactLineHeightSpan(
    private val height: Float,
) : LineHeightSpan {
    override fun chooseHeight(
        text: CharSequence,
        start: Int,
        end: Int,
        spanstartv: Int,
        lineHeight: Int,
        fm: Paint.FontMetricsInt,
    ) {
        // `lineHeight` is this line's top, in pixels from the text's top.
        val line = ((lineHeight - spanstartv) / height).roundToInt()
        val target = ((line + 1) * height).roundToInt() - (line * height).roundToInt()
        val above = (target - (fm.descent - fm.ascent)) / 2
        fm.ascent -= above
        fm.descent = fm.ascent + target
        fm.top = fm.ascent
        fm.bottom = fm.descent
    }

    companion object {
        /** [text] with every line [height] pixels tall, or unchanged without a height. */
        fun lined(
            text: String,
            height: Float?,
        ): CharSequence {
            if (height == null || text.isEmpty()) return text

            return SpannableString(text).apply {
                setSpan(ExactLineHeightSpan(height), 0, text.length, Spanned.SPAN_INCLUSIVE_INCLUSIVE)
            }
        }
    }
}

/**
 * Measures the shared tree's text with the typeface and size [NativeWidgetProps]
 * draws it with, so layout gives each label and button the width Android needs.
 * The native library calls [measure] by name on whichever thread lays out.
 *
 * Every changed label is measured once per relayout, so a frame that rewrites a
 * screen of numbers measures hundreds of strings: the paint is reused per
 * size and weight, and text that fits on one line takes `BoringLayout`, which
 * skips line breaking entirely.
 */
internal class NativeTextMeasurer(
    private val metrics: DisplayMetrics,
) {
    /** The paint the last measurement used; labels in one list share a style. */
    private var painted: TextPaint? = null
    private var paintedSize = Float.NaN
    private var paintedWeight = 0

    /**
     * Returns the logical width and height of UTF-8 [text] at [fontSize] (in the
     * scaled points text views use), CSS [weight], and [rtl] direction within
     * [width] logical points: `-1` is unbounded and `-2` is the widest word.
     */
    @Suppress("unused") // Called by JNI.
    fun measure(
        text: ByteArray,
        fontSize: Float,
        weight: Int,
        width: Float,
        rtl: Boolean,
    ): FloatArray {
        val value = text.toString(Charsets.UTF_8)
        val paint = paint(fontSize, weight)
        val limit =
            when (width) {
                UNBOUNDED -> Layout.getDesiredWidth(value, paint)
                WIDEST_WORD -> value.split(Regex("\\s+")).maxOfOrNull { Layout.getDesiredWidth(it, paint) } ?: 0f
                else -> width * metrics.density
            }
        val bound = ceil(limit.toDouble()).toInt().coerceAtLeast(1)
        val direction = if (rtl) TextDirectionHeuristics.RTL else TextDirectionHeuristics.LTR
        // Text without tabs or bidi overrides that fits in the bound lays out
        // as one run, which `BoringLayout` measures without a line breaker.
        // It always lays out exactly one line, so text wider than the bound
        // must wrap through `StaticLayout`, as `TextView` does.
        val boring = BoringLayout.isBoring(value, paint, direction, true, null)?.takeIf { it.width <= bound }
        val measured =
            if (boring != null) {
                BoringLayout.make(
                    value,
                    paint,
                    bound,
                    Layout.Alignment.ALIGN_NORMAL,
                    1.0f,
                    0.0f,
                    boring,
                    false,
                )
            } else {
                StaticLayout.Builder
                    .obtain(value, 0, value.length, paint, bound)
                    .setIncludePad(false)
                    .setTextDirection(direction)
                    .setBreakStrategy(BREAK_STRATEGY)
                    .setHyphenationFrequency(HYPHENATION)
                    .build()
            }

        var widest = 0f
        repeat(measured.lineCount) { widest = max(widest, measured.getLineWidth(it)) }
        val pixels = ceil(widest.coerceAtMost(limit.coerceAtLeast(0f)).toDouble()).toFloat()

        return floatArrayOf(pixels / metrics.density, measured.height / metrics.density)
    }

    private fun paint(
        size: Float,
        weight: Int,
    ): TextPaint {
        val existing = painted

        if (existing != null && size == paintedSize && weight == paintedWeight) return existing

        val paint =
            (existing ?: TextPaint(TextPaint.ANTI_ALIAS_FLAG)).apply {
                textSize = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, size, metrics)

                typeface = nativeTypeface(weight)
                isAntiAlias = true
            }
        painted = paint
        paintedSize = size
        paintedWeight = weight

        return paint
    }

    internal companion object {
        /** How text breaks into lines, here and in the text views that draw it. */
        const val BREAK_STRATEGY = Layout.BREAK_STRATEGY_SIMPLE

        /** Whether text hyphenates, here and in the text views that draw it. */
        const val HYPHENATION = Layout.HYPHENATION_FREQUENCY_NONE

        private const val UNBOUNDED = -1f
        private const val WIDEST_WORD = -2f
    }
}
