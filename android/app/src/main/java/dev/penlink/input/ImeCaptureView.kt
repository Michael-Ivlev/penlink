package dev.penlink.input

import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import dev.penlink.protocol.InputMsg

/**
 * An invisible text sink for the phone's normal keyboard. Typed characters go out as `text`
 * messages; Backspace and Enter go out as `key` messages (the Android IME may also send them
 * as key events or as deleteSurroundingText).
 */
class ImeCaptureView(context: Context) : View(context) {
    var sink: (String) -> Unit = {}

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor(): Boolean = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_ACTION_NONE
        return object : BaseInputConnection(this, false) {
            override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
                val s = text?.toString().orEmpty()
                if (s.isNotEmpty()) sendString(s)
                return true
            }

            override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean = true

            override fun finishComposingText(): Boolean = true

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                repeat(beforeLength.coerceAtLeast(1)) { tapVk(Vk.BACK) }
                return true
            }

            override fun sendKeyEvent(event: KeyEvent): Boolean {
                if (event.action == KeyEvent.ACTION_DOWN) {
                    when (event.keyCode) {
                        KeyEvent.KEYCODE_DEL -> tapVk(Vk.BACK)
                        KeyEvent.KEYCODE_ENTER -> tapVk(Vk.RETURN)
                        KeyEvent.KEYCODE_TAB -> tapVk(Vk.TAB)
                        else -> {
                            val u = event.unicodeChar
                            if (u != 0) sendString(String(Character.toChars(u)))
                        }
                    }
                }
                return true
            }

            override fun performEditorAction(actionCode: Int): Boolean {
                tapVk(Vk.RETURN)
                return true
            }
        }
    }

    private fun sendString(s: String) {
        // A newline in committed text is Enter, the rest is typed as Unicode text.
        val parts = s.split('\n')
        parts.forEachIndexed { i, part ->
            if (part.isNotEmpty()) sink(InputMsg.text(part))
            if (i < parts.lastIndex) tapVk(Vk.RETURN)
        }
    }

    private fun tapVk(vk: Int) {
        sink(InputMsg.key(true, vk))
        sink(InputMsg.key(false, vk))
    }

    fun show() {
        requestFocus()
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.restartInput(this)
        imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
    }

    fun hide() {
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(windowToken, 0)
        clearFocus()
    }
}
