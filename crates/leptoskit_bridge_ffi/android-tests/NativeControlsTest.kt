package dev.leptoskit.mobile

import android.text.InputType
import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.CompoundButton
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ProgressBar
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** Exercises the same renderer generated APKs use, with real Android controls. */
class NativeControlsTest {
    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    @Test
    fun togglesEmitGuestPayloadsAndReconciliationIsSilent() =
        onMain {
            for (kind in listOf("switch", "checkbox", "radio")) {
                val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
                val events = mutableListOf<Pair<Int, String>>()
                val renderer = NativeWidgetRenderer(root, { _, kind, payload -> events.add(kind to payload) })
                renderer.render(RenderBatch(1, listOf(RenderMutation.Create(1, kind, props("{\"checked\":false}")))))
                val control = root.getChildAt(0) as CompoundButton
                assertTrue(events.isEmpty())
                control.performClick()
                assertEquals(listOf(6 to "{\"checked\":true}"), events)
                assertFalse("Unacknowledged changes must not override guest state", control.isChecked)

                events.clear()
                renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"checked\":true}")))))
                assertTrue(events.isEmpty())
                control.performClick()

                if (kind == "radio") {
                    assertTrue(control.isChecked)
                    assertTrue(events.isEmpty())
                } else {
                    assertEquals(listOf(6 to "{\"checked\":false}"), events)
                }

                renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"checked\":false,\"enabled\":false}")))))
                assertFalse(control.isEnabled)

                if (kind != "switch") {
                    control.layout(0, 0, 40, 40)
                    val bitmap = android.graphics.Bitmap.createBitmap(40, 40, android.graphics.Bitmap.Config.ARGB_8888)
                    control.draw(android.graphics.Canvas(bitmap))
                    assertTrue(checkNotNull(control.buttonDrawable).bounds.right <= control.width)
                    assertTrue(checkNotNull(control.buttonDrawable).bounds.bottom <= control.height)
                    bitmap.recycle()
                }
            }
        }

    @Test
    fun sliderMapsRangeAndStepsAndProgressCanStop() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<String>()
            val renderer = NativeWidgetRenderer(root, { _, _, payload -> events.add(payload) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(
                            1,
                            "slider",
                            props(
                                """
            {"slider":{"min":10,"max":20,"value":14,"step":2},"accessibility_label":"Volume"}
        """,
                            ),
                        ),
                    ),
                ),
            )
            val slider = root.getChildAt(0) as NativeSlider
            assertEquals(4000, slider.progress)
            assertEquals(16.0, slider.proposedValue(5500), 0.0)
            assertEquals(20.0, slider.proposedValue(10000), 0.0)
            assertTrue(events.isEmpty())
            val arguments =
                android.os.Bundle().apply {
                    putFloat(android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, 8000f)
                }
            assertTrue(
                slider.performAccessibilityAction(
                    android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS.id,
                    arguments,
                ),
            )
            assertEquals(18.0, JSONObject(events.single()).getDouble("value"), 0.0)
            val progressRoot = FrameLayout(root.context)
            val progressRenderer = NativeWidgetRenderer(progressRoot, { _, _, _ -> fail("Progress is not interactive") })
            progressRenderer.render(
                RenderBatch(
                    1,
                    listOf(RenderMutation.Create(1, "linear_progress_indicator", props("{\"progress\":{\"determinate\":0.6}}"))),
                ),
            )
            val progress = progressRoot.getChildAt(0) as ProgressBar
            assertFalse(progress.isIndeterminate)
            assertEquals(6000, progress.progress)
            progressRenderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"running\":false}")))))
            assertTrue(progress.isIndeterminate)
            assertEquals(View.INVISIBLE, progress.visibility)
        }

    @Test
    fun outsideTouchAndSubmitClearFocusWithoutSwallowingPresses() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Int>()
            val renderer = NativeWidgetRenderer(root, { _, kind, _ -> events.add(kind) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "text_input", WidgetProps(value = "hello")),
                        RenderMutation.InsertChild(1, 2, 0),
                    ),
                ),
            )
            val editor = (root.getChildAt(0) as FrameLayout).getChildAt(0) as EditText
            editor.isFocusableInTouchMode = true
            assertTrue(editor.requestFocus())
            events.clear()
            val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 200f, 200f, 0)
            dismissEditorOutside(root, down)
            down.recycle()
            assertFalse(editor.hasFocus())
            assertEquals(listOf(4), events)
            assertTrue(editor.requestFocus())
            events.clear()
            editor.onEditorAction(EditorInfo.IME_ACTION_DONE)
            assertFalse(editor.hasFocus())
            assertEquals(listOf(4, 2), events)
        }

    @Test
    fun nextMovesToPasswordWithoutSubmitting() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Int>()
            val renderer = NativeWidgetRenderer(root, { _, kind, _ -> events.add(kind) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "text_input", WidgetProps()),
                        RenderMutation.Create(3, "password_input", WidgetProps()),
                        RenderMutation.InsertChild(1, 2, 0),
                        RenderMutation.InsertChild(1, 3, 1),
                    ),
                ),
            )
            val content = root.getChildAt(0) as FrameLayout
            val email = content.getChildAt(0) as EditText
            val password = content.getChildAt(1) as EditText
            assertEquals(EditorInfo.IME_ACTION_NEXT, email.imeOptions)
            assertEquals(EditorInfo.IME_ACTION_DONE, password.imeOptions)
            assertTrue(email.requestFocus())
            events.clear()
            email.onEditorAction(EditorInfo.IME_ACTION_NEXT)
            assertTrue(password.hasFocus())
            assertFalse(events.contains(2))
            assertTrue(email.requestFocus())
            events.clear()
            email.dispatchKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, android.view.KeyEvent.KEYCODE_ENTER))
            email.dispatchKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, android.view.KeyEvent.KEYCODE_ENTER))
            assertTrue(password.hasFocus())
            assertFalse(events.contains(2))
        }

    @Test
    fun aTextAreaKeepsEnterForANewLine() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Pair<Int, String>>()
            val renderer = NativeWidgetRenderer(root, { _, kind, text -> events.add(kind to text) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "text_input", WidgetProps()),
                        RenderMutation.Create(3, "text_area", WidgetProps(value = "one\ntwo")),
                        RenderMutation.InsertChild(1, 2, 0),
                        RenderMutation.InsertChild(1, 3, 1),
                    ),
                ),
            )
            val content = root.getChildAt(0) as FrameLayout
            val line = content.getChildAt(0) as EditText
            val area = content.getChildAt(1) as EditText
            assertEquals("one\ntwo", area.text.toString())
            assertTrue(area.inputType and InputType.TYPE_TEXT_FLAG_MULTI_LINE != 0)
            // The field before it still moves on to it; the area itself names no action.
            assertEquals(EditorInfo.IME_ACTION_NEXT, line.imeOptions)
            assertEquals(EditorInfo.IME_FLAG_NO_ENTER_ACTION, area.imeOptions)
            events.clear()
            area.setText("one\ntwo\nthree")
            assertEquals(listOf(1 to "one\ntwo\nthree"), events)
        }

    @Test
    fun anUnknownKindStaysInTheTree() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            // A trailing lambda binds canvasChanged, which left event missing.
            val renderer = NativeWidgetRenderer(root, { _, _, _ -> })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "custom:chart", WidgetProps()),
                        RenderMutation.InsertChild(1, 2, 0),
                    ),
                ),
            )
            assertEquals(1, root.childCount)
            val parent = root.getChildAt(0) as android.view.ViewGroup
            assertEquals(1, parent.childCount)
        }

    @Test
    fun aFocusRequestMovesFocusOnce() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val renderer = NativeWidgetRenderer(root, { _, _, _ -> })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "text_input", WidgetProps()),
                        RenderMutation.Create(3, "text_input", WidgetProps()),
                        RenderMutation.InsertChild(1, 2, 0),
                        RenderMutation.InsertChild(1, 3, 1),
                    ),
                ),
            )
            val content = root.getChildAt(0) as FrameLayout
            val email = content.getChildAt(0) as EditText
            val code = content.getChildAt(1) as EditText
            assertFalse(code.isFocused)

            renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(3, props("""{"focus_request":7}""")))))
            assertTrue(code.isFocused)

            // The person moves on, and a later change to the field leaves them there.
            email.requestFocus()
            renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(3, props("""{"value":"12"}""")))))
            assertTrue(email.isFocused)
            assertFalse(code.isFocused)
        }

    @Test
    fun anOverlayKeepsItsPlaceAndWaitsForAWindow() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Int>()
            val renderer = NativeWidgetRenderer(root, { _, kind, _ -> events.add(kind) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "modal", props("""{"overlay":{"open":true}}""")),
                        RenderMutation.Create(3, "view", WidgetProps()),
                        RenderMutation.Create(4, "popover", props("""{"overlay":{"open":false,"anchor":"menu"}}""")),
                        RenderMutation.Create(5, "view", WidgetProps()),
                        RenderMutation.InsertChild(1, 2, 0),
                        RenderMutation.InsertChild(2, 3, 0),
                        RenderMutation.InsertChild(1, 4, 1),
                        RenderMutation.InsertChild(1, 5, 2),
                    ),
                ),
            )
            val content = root.getChildAt(0) as FrameLayout
            // Each overlay holds a place, so the view after them is still the third child.
            assertEquals(3, content.childCount)
            val modal = content.getChildAt(0) as NativeOverlayView
            val popover = content.getChildAt(1) as NativeOverlayView
            assertEquals(View.GONE, modal.visibility)
            assertEquals(0, modal.childCount)
            assertEquals(1, modal.content.childCount)
            assertEquals(0, popover.content.childCount)
            assertTrue(checkNotNull(modal.state).open)
            assertFalse(checkNotNull(popover.state).open)

            // The root is in no window, so an open overlay has nowhere to show and nothing asks to close.
            assertFalse(modal.showing)
            assertFalse(popover.showing)
            renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(4, props("""{"overlay":{"open":true}}""")))))
            assertFalse(popover.showing)
            renderer.render(RenderBatch(null, listOf(RenderMutation.RemoveChild(1, 2), RenderMutation.Delete(2))))
            assertEquals(2, content.childCount)
            assertTrue(events.isEmpty())
        }

    private fun props(json: String) = WidgetProps.fromJson(JSONObject(json))
}
