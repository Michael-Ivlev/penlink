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

    /**
     * Called whenever local zoom/pan change (scale, translationX, translationY). The translation already includes
     * the keyboard pan, so the owner just applies it to the video view.
     */
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

    // ---- keyboard pan: the picture slides up so the last tap stays above the phone keyboard (no PC resize)

    /** Current extra Y translation (<= 0) that keeps the focus above the keyboard. Added to [panY] for display and mapping. */
    var keyboardPanY = 0f
        private set
    private var covered = 0f
    private var marginPx = 0f

    /** Last pressed point as a fraction of the video (the protocol has no caret position, so the last tap stands in). */
    private var focusNormY = Double.NaN

    /** The mapper for the picture as it is displayed right now (zoom, user pan and keyboard pan). */
    private fun mapper() = VideoMapper(width.toFloat(), height.toFloat(), videoW, videoH, zoom, panX, panY + keyboardPanY)

    /** The mapper without the keyboard pan: where things would be if the keyboard were hidden. */
    private fun restMapper() = VideoMapper(width.toFloat(), height.toFloat(), videoW, videoH, zoom, panX, panY)

    private fun notifyTransform() = onTransform(zoom, panX, panY + keyboardPanY)

    /**
     * The keyboard now covers [coveredPx] of the bottom of this view (called every frame of the keyboard animation, 0 =
     * hidden). Moves the picture so the focus stays [margin] above it; back to 0 as the keyboard goes away.
     */
    fun setKeyboardCovered(coveredPx: Float, margin: Float) {
        covered = coveredPx.coerceAtLeast(0f)
        marginPx = margin
        updateKeyboardPan()
    }

    private fun updateKeyboardPan() {
        val h = height.toFloat()
        val pan = if (h <= 0f || focusNormY.isNaN()) 0f else {
            val m = restMapper()
            -KeyboardPan.panUp(m.toViewY(focusNormY), m.toViewY(1.0), h, covered, marginPx)
        }
        if (pan != keyboardPanY) {
            keyboardPanY = pan
            notifyTransform()
        }
    }

    /** Focus of the press in progress; applied when the gesture ends so the picture never moves under a finger that is down. */
    private var pendingFocusNormY = Double.NaN

    /**
     * A press at view point ([x], [y]) (as displayed now). Only remembered here: moving the picture mid-gesture would make
     * the Move/Up of the same gesture map to a different spot than its Down (a tap would become a drag).
     */
    private fun noteFocus(x: Float, y: Float) = noteFocusNorm(mapper().toNorm(x, y).y)

    private fun noteFocusNorm(ny: Double) { pendingFocusNormY = ny }

    /**
     * Gesture over: the remembered press becomes the focus. While the keyboard is up it only moves the focus further
     * down (needing more pan), never back up, so tapping higher in the visible area does not make the picture jump.
     */
    private fun commitFocus() {
        val ny = pendingFocusNormY
        pendingFocusNormY = Double.NaN
        if (ny.isNaN()) return
        if (covered > 0f && !focusNormY.isNaN() && ny <= focusNormY) return
        focusNormY = ny
        updateKeyboardPan()
    }

    fun resetZoom() {
        zoom = 1f; panX = 0f; panY = 0f
        updateKeyboardPan()
        notifyTransform()
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
                    if (palm.penHover(e.eventTime)) penArrived()
                    for (h in 0 until e.historySize) sendPen(PenAction.Hover, e, 0, h, 0f)
                    sendPen(PenAction.Hover, e, 0, -1, 0f)
                    return true
                }
                MotionEvent.ACTION_HOVER_EXIT -> {
                    palm.penHoverEnd(e.eventTime)
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
        val pi = e.findPointerIndex(penPointerId)
        val idx = if (pi >= 0) pi else 0
        when (e.actionMasked) {
            MotionEvent.ACTION_MOVE -> {
                if (pi >= 0) { // only the pen's pointer; the palm's moves are never read
                    palm.penTouch(e.eventTime)
                    for (h in 0 until e.historySize) sendPen(PenAction.Move, e, pi, h)
                    sendPen(PenAction.Move, e, pi, -1)
                }
            }
            MotionEvent.ACTION_POINTER_UP -> {
                if (e.actionIndex == pi) endPen(e, pi) // the pen lifted first, the palm may stay
                else ignored = ignored and idBit(e.getPointerId(e.actionIndex)).inv() // a palm lifting
            }
            MotionEvent.ACTION_POINTER_DOWN -> ignored = ignored or idBit(e.getPointerId(e.actionIndex)) // palm lands while writing
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                endPen(e, idx)
                ignored = 0
            }
            else -> {}
        }
        return true
    }

    private fun endPen(e: MotionEvent, idx: Int) {
        sendPen(PenAction.Up, e, idx, -1)
        penActive = false
        palm.penTouchEnd(e.eventTime)
        commitFocus()
    }

    private fun startPen(e: MotionEvent, idx: Int) {
        palm.penTouch(e.eventTime)
        if (mode != Mode.Idle) cancelFinger() // a finger gesture was in progress: the PC must not keep it pressed
        penActive = true
        penPointerId = e.getPointerId(idx)
        // Every other pointer down now is a resting hand.
        for (i in 0 until e.pointerCount) if (i != idx) ignored = ignored or idBit(e.getPointerId(i))
        requestUnbufferedDispatch(e)
        noteFocus(e.getX(idx), e.getY(idx))
        sendPen(PenAction.Down, e, idx, -1)
    }

    private fun handleMouse(e: MotionEvent): Boolean {
        val n = mapper().toNorm(e.x, e.y)
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                mouseButton = buttonOf(e.buttonState)
                noteFocus(e.x, e.y)
                sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, mouseButton))
                sink(InputMsg.mouse(MouseAction.Down, n.x, n.y, mouseButton))
            }
            MotionEvent.ACTION_MOVE -> sink(InputMsg.mouse(MouseAction.Move, n.x, n.y, mouseButton))
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                sink(InputMsg.mouse(MouseAction.Up, n.x, n.y, mouseButton))
                mouseActive = false
                commitFocus()
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
        if (btn == MouseButton.Left) { noteFocusNorm(cursorY); commitFocus() }
    }

    // ---- palm rejection

    private val density = resources.displayMetrics.density
    private val palm = PalmRejector(palmMajorPx = PalmRejector.PALM_MAJOR_DP * density)

    /** The setting: while the pen is near, or the contact is huge, fingers send nothing. */
    var palmRejection: Boolean
        get() = palm.enabled
        set(v) { palm.enabled = v }

    /** Pointer ids (bit per id, ids are small) that are palms: ignored until they lift. */
    private var ignored = 0

    /** Pointer ids of the fingers that are down, so a pen that arrives can mark them all as palms. */
    private var fingersDown = 0

    private fun idBit(id: Int): Int = if (id in 0..31) 1 shl id else 0

    /** The pen came near while fingers may be down: those fingers are palms now, and what they started is undone. */
    private fun penArrived() {
        ignored = ignored or fingersDown
        if (mode != Mode.Idle) cancelFinger()
    }

    /** Ends the finger gesture without a click or any pressed state left on the PC. */
    private fun cancelFinger() {
        removeCallbacks(longPress)
        if (touchSent) sink(InputMsg.touch(TouchAction.Cancel, primaryId, 0.0, 0.0))
        if (dragging) sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, MouseButton.Left))
        pendingFocusNormY = Double.NaN
        resetFinger()
    }

    private fun flagCanceled(e: MotionEvent): Boolean =
        android.os.Build.VERSION.SDK_INT >= 33 && (e.flags and MotionEvent.FLAG_CANCELED) != 0

    private fun isPalm(e: MotionEvent, idx: Int, canceled: Boolean): Boolean =
        palm.rejectFinger(e.eventTime, e.getTouchMajor(idx), canceled)

    private fun handleFinger(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                fingersDown = idBit(e.getPointerId(0))
                if (isPalm(e, 0, flagCanceled(e))) { ignored = fingersDown; return true }
                ignored = 0
                startSingle(e, 0)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                val i = e.actionIndex
                val bit = idBit(e.getPointerId(i))
                fingersDown = fingersDown or bit
                if (isPalm(e, i, false)) { ignored = ignored or bit; return true }
                if (mode == Mode.Idle) {
                    startSingle(e, i) // a palm was down first, this is a real finger
                } else if (mode == Mode.Single && e.pointerCount == 2) {
                    startTwo(e)
                } else if (mode != Mode.Two) {
                    cancelSingleSends()
                    mode = Mode.Ignore
                }
            }
            MotionEvent.ACTION_MOVE -> {
                if (ignored != 0 && e.pointerCount == Integer.bitCount(ignored)) return true // only palms
                if (mode == Mode.Single) {
                    val pi = e.findPointerIndex(primaryId)
                    if (pi >= 0 && isPalm(e, pi, false)) { // the contact grew into a palm
                        ignored = ignored or idBit(primaryId)
                        cancelFinger()
                        return true
                    }
                }
                when (mode) {
                    Mode.Single -> moveSingle(e)
                    Mode.Two -> if (e.pointerCount >= 2) moveTwo(e)
                    else -> {}
                }
            }
            MotionEvent.ACTION_POINTER_UP -> {
                val i = e.actionIndex
                val id = e.getPointerId(i)
                fingersDown = fingersDown and idBit(id).inv()
                if (ignored and idBit(id) != 0) { // a palm lifting
                    ignored = ignored and idBit(id).inv()
                    return true
                }
                val canceled = flagCanceled(e)
                if (mode == Mode.Two) {
                    if (twoKind == Two.Undecided && e.eventTime - twoStartT < 300 && !canceled) {
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
                } else if (mode == Mode.Single && id == primaryId) {
                    endSingle(e, i, canceled) // the finger lifts while a palm is still down
                }
            }
            MotionEvent.ACTION_UP -> {
                val id = e.getPointerId(0)
                fingersDown = 0
                val wasPalm = ignored and idBit(id) != 0
                ignored = 0
                if (wasPalm && mode == Mode.Idle) return true
                endSingle(e, 0, flagCanceled(e))
            }
            MotionEvent.ACTION_CANCEL -> {
                fingersDown = 0
                ignored = 0
                cancelFinger()
            }
        }
        return true
    }

    private fun startSingle(e: MotionEvent, idx: Int) {
        val x = e.getX(idx)
        val y = e.getY(idx)
        mode = Mode.Single
        primaryId = e.getPointerId(idx)
        startX = x; startY = y; lastX = x; lastY = y
        startT = e.eventTime
        moved = false; longPressed = false; touchSent = false; dragging = false
        if (trackpad) {
            if (e.eventTime - lastTapUpT < 300 &&
                abs(x - lastTapX) < slop * 4 && abs(y - lastTapY) < slop * 4
            ) {
                dragging = true
                sink(InputMsg.mouse(MouseAction.Down, cursorX, cursorY, MouseButton.Left))
            }
        } else {
            val n = mapper().toNorm(x, y)
            noteFocusNorm(n.y)
            sink(InputMsg.touch(TouchAction.Down, primaryId, n.x, n.y))
            touchSent = true
            postDelayed(longPress, longPressMs)
        }
    }

    private fun startTwo(e: MotionEvent) {
        cancelSingleSends()
        mode = Mode.Two
        pendingFocusNormY = Double.NaN // a pinch or scroll is not a tap
        twoKind = Two.Undecided
        twoStartT = e.eventTime
        twoStartDist = Gesture.distance(e.getX(0), e.getY(0), e.getX(1), e.getY(1))
        twoStartCx = (e.getX(0) + e.getX(1)) / 2f
        twoStartCy = (e.getY(0) + e.getY(1)) / 2f
        lastDist = twoStartDist; lastCx = twoStartCx; lastCy = twoStartCy
        wheelAccX = 0f; wheelAccY = 0f
    }

    /** Undoes what the first finger already sent (a touch down, a trackpad drag) because it is no longer a single-finger gesture. */
    private fun cancelSingleSends() {
        removeCallbacks(longPress)
        if (touchSent) {
            sink(InputMsg.touch(TouchAction.Cancel, primaryId, 0.0, 0.0))
            touchSent = false
        }
        if (dragging) {
            sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, MouseButton.Left))
            dragging = false
        }
    }

    /** The single finger at [idx] lifted (or was cancelled by the system: then no tap, nothing stays pressed). */
    private fun endSingle(e: MotionEvent, idx: Int, canceled: Boolean) {
        removeCallbacks(longPress)
        if (canceled) {
            cancelSingleSends()
            pendingFocusNormY = Double.NaN
        } else if (mode == Mode.Single) {
            if (trackpad) {
                if (dragging) {
                    sink(InputMsg.mouse(MouseAction.Up, cursorX, cursorY, MouseButton.Left))
                } else if (!moved && e.eventTime - startT < tapMs) {
                    clickAtCursor(MouseButton.Left)
                    lastTapUpT = e.eventTime; lastTapX = e.getX(idx); lastTapY = e.getY(idx)
                }
            } else if (touchSent) {
                val n = mapper().toNorm(e.getX(idx), e.getY(idx))
                sink(InputMsg.touch(TouchAction.Up, primaryId, n.x, n.y))
            }
        }
        resetFinger()
    }

    private fun resetFinger() {
        commitFocus()
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
                // The picture is displayed shifted by the keyboard pan too; keep that part out of the user pan.
                panX = (fx - (fx - panX) * k) + (cx - lastCx)
                panY = (fy - (fy - (panY + keyboardPanY)) * k) + (cy - lastCy) - keyboardPanY
                clampPan()
                updateKeyboardPan()
                notifyTransform()
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
        updateKeyboardPan() // rotation or a new area size: recompute the pan for the new geometry
        notifyTransform()
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (!hasWindowFocus) palm.reset() // a hover that ended while another window had focus never reports its exit
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        val action = e.actionMasked
        if (action == MotionEvent.ACTION_DOWN) {
            // A new gesture: deliver its events as they happen, not batched to the next frame (pen, finger, mouse).
            requestUnbufferedDispatch(e)
            // Decide who owns it from the first pointer's tool.
            val tool = e.getToolType(0)
            penActive = false
            mouseActive = tool == MotionEvent.TOOL_TYPE_MOUSE
            ignored = 0
            if (isPenTool(tool)) {
                startPen(e, 0)
                return true
            }
            if (mouseActive) return handleMouse(e)
        } else if (action == MotionEvent.ACTION_POINTER_DOWN && palm.enabled && !penActive && !mouseActive &&
            isPenTool(e.getToolType(e.actionIndex))
        ) {
            // The pen lands while a palm (or a finger) is already down: the pen takes over, the hand is ignored.
            startPen(e, e.actionIndex)
            return true
        }
        return when {
            penActive -> handlePen(e)
            mouseActive -> handleMouse(e)
            else -> handleFinger(e)
        }
    }

}
