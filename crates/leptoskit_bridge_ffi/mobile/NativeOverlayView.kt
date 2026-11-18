package dev.leptoskit.mobile

import android.app.Activity
import android.app.Dialog
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Color
import android.graphics.Rect
import android.graphics.drawable.ColorDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.Window
import android.widget.FrameLayout
import android.widget.PopupWindow
import org.json.JSONObject

/** What a `Modal` or `Popover` shows, as `Element::modal` and `Element::popover` set it. */
internal class OverlayState(
    val open: Boolean,
    /** The `id` of the element inside that takes focus when the overlay opens. */
    val initialFocus: String?,
) {
    companion object {
        fun fromJson(overlay: JSONObject) =
            OverlayState(
                overlay.optBoolean("open"),
                if (overlay.isNull("initial_focus")) null else overlay.getString("initial_focus"),
            )
    }
}

/**
 * Back, Escape and a touch outside all end in `cancel`, which a dialog answers
 * by closing. A modal only asks: the app closes it with the next tree.
 */
internal class NativeModalDialog(
    context: Context,
    private val onCancel: () -> Unit,
) : Dialog(context) {
    init {
        requestWindowFeature(Window.FEATURE_NO_TITLE)
        setCanceledOnTouchOutside(true)
        // The app draws the dialog's own panel, corners and all.
        window?.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
    }

    override fun cancel() = onCancel()
}

/** The view an overlay's children go into, which knows the overlay it belongs to. */
internal class NativeOverlayContent(
    context: Context,
    val owner: NativeOverlayView,
) : FrameLayout(context)

/**
 * A `Modal` or `Popover`'s place in the tree.
 *
 * The view itself is gone from layout and holds nothing. What the overlay
 * holds goes into [content], which Android shows in a window of its own: a
 * modal's in a dialog, and a popover's in a popup window. Keeping a view in
 * the tree keeps every sibling at the index the batch gives it, and tells the
 * overlay when the tree has a window to show over.
 */
internal class NativeOverlayView(
    context: Context,
    private val hangs: Boolean,
    private val dismissed: () -> Unit,
) : FrameLayout(context) {
    /** Where the overlay's children go, and what its own style is applied to. */
    val content = NativeOverlayContent(context, this)
    var state: OverlayState? = null

    /** The view joined a window, so an open overlay may now have somewhere to show. */
    var onWindowChange: (() -> Unit)? = null

    /** A dialog's content view, as large as what the modal holds. */
    private val holder = FrameLayout(context).apply { addView(content) }

    /** The overlay's frame, in pixels from the viewport's corner. */
    private val frame = Rect()

    /** Set while the renderer takes the window down, so that is not reported as a request to close. */
    private var closing = false
    var dialog: Dialog? = null
        private set
    var popup: PopupWindow? = null
        private set

    init {
        visibility = View.GONE
    }

    val showing: Boolean get() = dialog?.isShowing == true || popup?.isShowing == true

    /** Sets where layout put the overlay. A dialog shows only what the modal holds; a popup is this large. */
    fun place(
        left: Int,
        top: Int,
        width: Int,
        height: Int,
    ) {
        frame.set(left, top, left + width, top + height)
    }

    /** A dialog dims the screen behind it, so the backdrop a modal draws on the web has nothing to cover. */
    fun styled() {
        if (!hangs) content.background = null
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        // Not during the attach itself: a window cannot be added from inside one.
        post { onWindowChange?.invoke() }
    }

    /**
     * Shows the overlay if it is open and has somewhere to show, and takes it
     * down otherwise. Returns whether this call showed it. [viewport] is the
     * view whose corner layout measures from.
     */
    fun present(viewport: View): Boolean {
        if (state?.open != true || !isAttachedToWindow) {
            hide()

            return false
        }

        return if (hangs) showPopup(viewport) else showDialog()
    }

    /** A modal covers the viewport. The dialog is as large as what it holds, and Android centres it. */
    private fun showDialog(): Boolean {
        val held = Rect()

        for (index in 0 until content.childCount) {
            val child = content.getChildAt(index)
            val params = child.layoutParams as? LayoutParams ?: continue

            if (child.visibility != View.VISIBLE || params.width <= 0 || params.height <= 0) continue
            held.union(params.leftMargin, params.topMargin, params.leftMargin + params.width, params.topMargin + params.height)
        }
        val activity = generateSequence(context) { (it as? ContextWrapper)?.baseContext }.filterIsInstance<Activity>().firstOrNull()

        if (activity == null || held.isEmpty) {
            hide()

            return false
        }
        content.layoutParams =
            LayoutParams(frame.width(), frame.height(), Gravity.TOP or Gravity.LEFT).apply {
                leftMargin = -held.left
                topMargin = -held.top
            }
        val dialog =
            dialog ?: NativeModalDialog(activity, dismissed).also {
                it.setContentView(holder, ViewGroup.LayoutParams(held.width(), held.height()))
                dialog = it
            }
        holder.layoutParams =
            holder.layoutParams.apply {
                width = held.width()
                height = held.height()
            }
        dialog.window?.setLayout(held.width(), held.height())

        if (dialog.isShowing) return false
        dialog.show()

        return true
    }

    /** Layout has already placed the popover beside its anchor, so the popup window goes exactly there. */
    private fun showPopup(viewport: View): Boolean {
        if (frame.isEmpty) {
            hide()

            return false
        }
        content.layoutParams = LayoutParams(frame.width(), frame.height())
        val popup =
            popup ?: PopupWindow(content, frame.width(), frame.height(), true).also { popup ->
                popup.setBackgroundDrawable(null)
                popup.isOutsideTouchable = true
                // A touch outside asks the app, and the popup stays until the app closes it.
                popup.setTouchInterceptor { view, event ->
                    val outside =
                        event.actionMasked == MotionEvent.ACTION_OUTSIDE ||
                            (
                                event.actionMasked == MotionEvent.ACTION_DOWN &&
                                    (event.x < 0 || event.y < 0 || event.x >= view.width || event.y >= view.height)
                            )

                    if (outside) dismissed()
                    outside
                }
                // Back closes a popup window itself. The app is told, and the
                // popup comes back with the next batch if the app keeps it open.
                popup.setOnDismissListener { if (!closing) dismissed() }
                this.popup = popup
            }
        // Inside a dialog, the viewport's corner is that modal's content.
        val below = enclosing() ?: viewport
        val origin = IntArray(2)
        below.getLocationInWindow(origin)
        val left = origin[0] + frame.left
        val top = origin[1] + frame.top

        if (popup.isShowing) {
            popup.update(left, top, frame.width(), frame.height())

            return false
        }
        popup.showAtLocation(below, Gravity.NO_GRAVITY, left, top)

        return true
    }

    /** The content of the overlay this one is inside, when that overlay is on screen. */
    private fun enclosing(): View? {
        var current = parent

        while (current != null) {
            if (current is NativeOverlayContent && current.owner.showing) return current
            current = current.parent
        }

        return null
    }

    /** Takes the overlay's window down without asking anyone. */
    fun hide() {
        closing = true
        dialog?.takeIf { it.isShowing }?.dismiss()
        popup?.takeIf { it.isShowing }?.dismiss()
        closing = false
    }
}
