package dev.penlink.input

import android.content.Context
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import dev.penlink.protocol.InputMsg
import dev.penlink.protocol.MouseAction
import dev.penlink.protocol.MouseButton
import dev.penlink.protocol.PenAction
import dev.penlink.protocol.TouchAction
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * Transparent view stacked on top of the video. It reads raw MotionEvents and turns them into
 * `input` data-channel messages via [sink]. It also owns the local pinch-zoom / pan of the view
 * (reported through [onTransform], which the owner applies to the video view).
 */
class InputOverlayView(context: Context) : View(context) {
    /** Receives each JSON message for the `input` data channel. */
    var sink: (String) -> Unit = {}

    /** Called whenever local zoom/pan change (scale, translationX, translationY). */
    var onTransform: (Float, Float, Float) -> Unit = { _, _, _ -> }

    /** Direct = finger acts as touch; trackpad = finger moves a relative cursor. */
    var trackpad: Boolean = false

    /** Video frame size in pixels (0 = unknown, then the whole view is used). */
    var videoW: Int = 0
    var videoH: Int = 0

    var zoom = 1f
        private set
    var panX = 0f
        private set
    var panY = 0f
        private set

    private val slop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    private val longPressMs = 450L
    private val tapMs = 280L

    private fun mapper() = VideoMapper(width.toFloat(), height.toFloat(), videoW, videoH, zoom, panX, panY)

    fun resetZoom() {
        zoom = 1f; panX = 0f; panY = 0f
        onTransform(zoom, panX, panY)
    }

    // ---------------------------------------------------------------- pen & mouse (absolute)

    private var penActive = false
    private var penPointerId = -1
    private var mouseActive = false

    private fun isPenTool(t: Int) = t == MotionEvent.TOOL_TYPE_STYLUS || t == MotionEvent.TOOL_TYPE_ERASER

    private fun sendPen(a: PenAction, e: MotionEvent, idx: Int, hist: Int, pressureOverride: Float? = null) {
        val m = mapper()
        val x: Float; val y: Float; val p: Float; val tilt: Float; val ori: Float
        if (hist >= 0) {
            x = e.getHistoricalX(idx, hist); y = e.getHistoricalY(idx, hist)
            p = e.getHistoricalAxisValue(MotionEvent.AXIS_PRESSURE, idx, hist)
            tilt = e.getHistoricalAxisValue(MotionEvent.AXIS_TILT, idx, hist)
            ori = e.getHistoricalAxisValue(MotionEvent.AXIS_ORIENTATION, idx, hist)
        } else {
            x = e.getX(idx); y = e.getY(idx)
            p = e.getAxisValue(MotionEvent.AXIS_PRESSURE, idx)
            tilt = e.getAxisValue(MotionEvent.AXIS_TILT, idx)
            ori = e.getAxisValue(MotionEvent.AXIS_ORIENTATION, idx)
        }
        val n = m.toNorm(x, y)
        val (tx, ty) = Tilt.toDegrees(tilt, ori)
        val side = (e.buttonState and MotionEvent.BUTTON_STYLUS_PRIMARY) != 0
        val eraser = e.getToolType(idx) == MotionEvent.TOOL_TYPE_ERASER
        sink(InputMsg.pen(a, n.x, n.y, (pressureOverride ?: p).toDouble(), tx, ty, side, eraser))
    }

    override fun onGenericMotionEvent(e: MotionEvent): Boolean {
        val tool = e.getToolType(0)
        when {
            isPenTool(tool) -> when (e.actionMasked) {
                MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
                    for (h in 0 until e.historySize) sendPen(PenAction.Hover, e, 0, h, 0f)
                    sendPen(PenAction.Hover, e, 0, -1, 0f)
                    return true
                }
                MotionEvent.ACTION_HOVER_EXIT -> {
                    sendPen(PenAction.Leave, e, 0, -1, 0f)
                    return true
                }
            }
            tool == MotionEvent.TOOL_TYPE_MOUSE -> when (e.actionMasked) {
                MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
                    val n = mapper().toNorm(e.x, e.y)
                    sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, MouseButton.Left))
                    return true
                }
                MotionEvent.ACTION_SCROLL -> {
                    val dy = e.getAxisValue(MotionEvent.AXIS_VSCROLL)
                    val dx = e.getAxisValue(MotionEvent.AXIS_HSCROLL)
                    sink(InputMsg.wheel((dx * 120).roundToInt(), (dy * 120).roundToInt()))
                    return true
                }
            }
        }
        return super.onGenericMotionEvent(e)
    }

    private fun buttonOf(state: Int): MouseButton = when {
        state and MotionEvent.BUTTON_SECONDARY != 0 -> MouseButton.Right
        state and MotionEvent.BUTTON_TERTIARY != 0 -> MouseButton.Middle
        else -> MouseButton.Left
    }

    private var mouseButton = MouseButton.Left

    private fun handlePen(e: MotionEvent): Boolean {
        val idx = e.findPointerIndex(penPointerId).takeIf { it >= 0 } ?: 0
        when (e.actionMasked) {
            MotionEvent.ACTION_MOVE -> {
                for (h in 0 until e.historySize) sendPen(PenAction.Move, e, idx, h)
                sendPen(PenAction.Move, e, idx, -1)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                sendPen(PenAction.Up, e, idx, -1)
                penActive = false
            }
            else -> {} // other pointers while the pen is down are ignored (palm rejection)
        }
        return true
    }

    private fun handleMouse(e: MotionEvent): Boolean {
        val n = mapper().toNorm(e.x, e.y)
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                mouseButton = buttonOf(e.buttonState)
                sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, mouseButton))
                sink(InputMsg.mouse(MouseAction.Down, n.x, n.y, mouseButton))
            }
            MotionEvent.ACTION_MOVE -> sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, mouseButton))
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                sink(InputMsg.mouse(MouseAction.Up, n.x, n.y, mouseButton))
                mouseActive = false
            }
        }
        return true
    }

    // ---------------------------------------------------------------- finger

    private enum class Mode { Idle, Single, Two, Ignore }

    private var mode = Mode.Idle
    private var primaryId = 0
    private var startX = 0f
    private var startY = 0f
    private var lastX = 0f
    private var lastY = 0f
    private var startT = 0L
    private var moved = false
    private var touchSent = false
    private var longPressed = false

    // trackpad state
    private var cursorX = 0.5
    private var cursorY = 0.5
    private var dragging = false
    private var lastTapUpT = 0L
    private var lastTapX = 0f
    private var lastTapY = 0f

    // two finger state
    private enum class Two { Undecided, Scroll, Pinch }

    private var twoKind = Two.Undecided
    private var twoStartT = 0L
    private var twoStartDist = 0f
    private var twoStartCx = 0f
    private var twoStartCy = 0f
    private var lastDist = 0f
    private var lastCx = 0f
    private var lastCy = 0f
    private var wheelAccX = 0f
    private var wheelAccY = 0f

    private val longPress = Runnable {
        if (mode == Mode.Single && !moved && !trackpad) {
            longPressed = true
            if (touchSent) {
                sink(InputMsg.touch(TouchAction.Cancel, primaryId, 0.0, 0.0))
                touchSent = false
            }
            val n = mapper().toNorm(startX, startY)
            sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, MouseButton.Right))
            sink(InputMsg.mouse(MouseAction.Down, n.x, n.y, MouseButton.Right))
            sink(InputMsg.mouse(MouseAction.Up, n.x, n.y, MouseButton.Right))
            performHapticFeedback(android.view.HapticFeedbackConstants.LONG_PRESS)
        }
    }

    private fun clickAtCursor(btn: MouseButton) {
        sink(InputMsg.mouse(MouseAction.Move, cursorX, cursorY, btn))
        sink(InputMsg.mouse(MouseAction.Down, cursorX, cursorY, btn))
        sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, btn))
    }

    private fun handleFinger(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                mode = Mode.Single
                primaryId = e.getPointerId(0)
                startX = e.x; startY = e.y; lastX = e.x; lastY = e.y
                startT = e.eventTime
                moved = false; longPressed = false; touchSent = false; dragging = false
                if (trackpad) {
                    if (e.eventTime - lastTapUpT < 300 &&
                        abs(e.x - lastTapX) < slop * 4 && abs(e.y - lastTapY) < slop * 4
                    ) {
                        dragging = true
                        sink(InputMsg.mouse(MouseAction.Down, cursorX, cursorY, MouseButton.Left))
                    }
                } else {
                    val n = mapper().toNorm(e.x, e.y)
                    sink(InputMsg.touch(TouchAction.Down, primaryId, n.x, n.y))
                    touchSent = true
                    postDelayed(longPress, longPressMs)
                }
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                if (mode == Mode.Single && e.pointerCount == 2) {
                    removeCallbacks(longPress)
                    if (touchSent) {
                        sink(InputMsg.touch(TouchAction.Cancel, primaryId, 0.0, 0.0))
                        touchSent = false
                    }
                    if (dragging) {
                        sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, MouseButton.Left))
                        dragging = false
                    }
                    mode = Mode.Two
                    twoKind = Two.Undecided
                    twoStartT = e.eventTime
                    twoStartDist = Gesture.distance(e.getX(0), e.getY(0), e.getX(1), e.getY(1))
                    twoStartCx = (e.getX(0) + e.getX(1)) / 2f
                    twoStartCy = (e.getY(0) + e.getY(1)) / 2f
                    lastDist = twoStartDist; lastCx = twoStartCx; lastCy = twoStartCy
                    wheelAccX = 0f; wheelAccY = 0f
                } else if (mode != Mode.Two) {
                    mode = Mode.Ignore
                }
            }
            MotionEvent.ACTION_MOVE -> when (mode) {
                Mode.Single -> moveSingle(e)
                Mode.Two -> if (e.pointerCount >= 2) moveTwo(e)
                else -> {}
            }
            MotionEvent.ACTION_POINTER_UP -> {
                if (mode == Mode.Two) {
                    if (twoKind == Two.Undecided && e.eventTime - twoStartT < 300) {
                        if (trackpad) {
                            clickAtCursor(MouseButton.Right)
                        } else {
                            val n = mapper().toNorm(twoStartCx, twoStartCy)
                            sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, MouseButton.Right))
                            sink(InputMsg.mouse(MouseAction.Down, n.x, n.y, MouseButton.Right))
                            sink(InputMsg.mouse(MouseAction.Up, n.x, n.y, MouseButton.Right))
                        }
                    }
                    mode = Mode.Ignore
                }
            }
            MotionEvent.ACTION_UP -> {
                removeCallbacks(longPress)
                if (mode == Mode.Single) {
                    if (trackpad) {
                        if (dragging) {
                            sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, MouseButton.Left))
                        } else if (!moved && e.eventTime - startT < tapMs) {
                            clickAtCursor(MouseButton.Left)
                            lastTapUpT = e.eventTime; lastTapX = e.x; lastTapY = e.y
                        }
                    } else if (touchSent) {
                        val n = mapper().toNorm(e.x, e.y)
                        sink(InputMsg.touch(TouchAction.Up, primaryId, n.x, n.y))
                    }
                }
                resetFinger()
            }
            MotionEvent.ACTION_CANCEL -> {
                removeCallbacks(longPress)
                if (touchSent) sink(InputMsg.touch(TouchAction.Cancel, primaryId, 0.0, 0.0))
                if (dragging) sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, MouseButton.Left))
                resetFinger()
            }
        }
        return true
    }

    private fun resetFinger() {
        mode = Mode.Idle
        touchSent = false
        dragging = false
        longPressed = false
    }

    private fun moveSingle(e: MotionEvent) {
        val idx = e.findPointerIndex(primaryId).takeIf { it >= 0 } ?: return
        val x = e.getX(idx)
        val y = e.getY(idx)
        if (!moved && (abs(x - startX) > slop || abs(y - startY) > slop)) {
            moved = true
            removeCallbacks(longPress)
        }
        if (longPressed) return
        if (trackpad) {
            val dxPx = x - lastX
            val dyPx = y - lastY
            if (dxPx != 0f || dyPx != 0f) {
                val m = mapper()
                val unit = max(m.displayedPixelSize, 0.001f)
                val gain = 1.5f
                val dxV = dxPx / unit * gain
                val dyV = dyPx / unit * gain
                sink(InputMsg.mouseRel(dxV.toDouble(), dyV.toDouble()))
                val vw = if (videoW > 0) videoW.toDouble() else m.fitW.toDouble()
                val vh = if (videoH > 0) videoH.toDouble() else m.fitH.toDouble()
                cursorX = (cursorX + dxV / vw).coerceIn(0.0, 1.0)
                cursorY = (cursorY + dyV / vh).coerceIn(0.0, 1.0)
            }
        } else if (touchSent) {
            val n = mapper().toNorm(x, y)
            sink(InputMsg.touch(TouchAction.Move, primaryId, n.x, n.y))
        }
        lastX = x
        lastY = y
    }

    private fun moveTwo(e: MotionEvent) {
        val cx = (e.getX(0) + e.getX(1)) / 2f
        val cy = (e.getY(0) + e.getY(1)) / 2f
        val dist = Gesture.distance(e.getX(0), e.getY(0), e.getX(1), e.getY(1))
        if (twoKind == Two.Undecided) {
            val dDist = abs(dist - twoStartDist)
            val dCen = Gesture.distance(cx, cy, twoStartCx, twoStartCy)
            if (dDist > slop * 1.5f || dCen > slop * 1.5f) {
                twoKind = if (dDist > dCen) Two.Pinch else Two.Scroll
                lastDist = dist; lastCx = cx; lastCy = cy
                if (twoKind == Two.Scroll) { lastCx = twoStartCx; lastCy = twoStartCy }
                else { lastDist = twoStartDist; lastCx = twoStartCx; lastCy = twoStartCy }
            } else return
        }
        when (twoKind) {
            Two.Scroll -> {
                wheelAccY += Gesture.wheelY(cy - lastCy)
                wheelAccX += Gesture.wheelX(cx - lastCx)
                val wy = wheelAccY.toInt()
                val wx = wheelAccX.toInt()
                if (wx != 0 || wy != 0) {
                    sink(InputMsg.wheel(wx, wy))
                    wheelAccX -= wx
                    wheelAccY -= wy
                }
            }
            Two.Pinch -> {
                val factor = if (lastDist > 0f) dist / lastDist else 1f
                val newZoom = (zoom * factor).coerceIn(1f, 6f)
                // Keep the content point under the fingers fixed: pan follows the centroid.
                val oldZoom = zoom
                zoom = newZoom
                val k = if (oldZoom > 0f) newZoom / oldZoom else 1f
                val fx = cx - width / 2f
                val fy = cy - height / 2f
                panX = (fx - (fx - panX) * k) + (cx - lastCx)
                panY = (fy - (fy - panY) * k) + (cy - lastCy)
                clampPan()
                onTransform(zoom, panX, panY)
            }
            else -> {}
        }
        lastDist = dist
        lastCx = cx
        lastCy = cy
    }

    private fun clampPan() {
        val m = mapper()
        panX = m.clampPanX(panX)
        panY = m.clampPanY(panY)
        if (zoom <= 1.0001f) { panX = 0f; panY = 0f }
    }

    override fun onSizeChanged(w: Int, h: Int, ow: Int, oh: Int) {
        super.onSizeChanged(w, h, ow, oh)
        clampPan()
        onTransform(zoom, panX, panY)
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        if (e.actionMasked == MotionEvent.ACTION_DOWN) {
            // A new gesture: decide who owns it from the first pointer's tool.
            val tool = e.getToolType(0)
            penActive = isPenTool(tool)
            mouseActive = tool == MotionEvent.TOOL_TYPE_MOUSE
            if (penActive) {
                penPointerId = e.getPointerId(0)
                requestUnbufferedDispatch(e)
                sendPen(PenAction.Down, e, 0, -1)
                return true
            }
            if (mouseActive) return handleMouse(e)
        }
        return when {
            penActive -> handlePen(e)
            mouseActive -> handleMouse(e)
            else -> handleFinger(e)
        }
    }

}
