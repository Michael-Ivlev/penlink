package dev.penlink

import dev.penlink.input.ImeMapping
import dev.penlink.input.ImePiece
import dev.penlink.input.KeyAction
import dev.penlink.input.KeyMod
import dev.penlink.input.KeyboardLogic
import dev.penlink.input.Vk
import dev.penlink.protocol.WsIn
import dev.penlink.protocol.WsOut
import dev.penlink.protocol.WsParser
import dev.penlink.rtc.LinkState
import dev.penlink.rtc.ReconnectMachine
import dev.penlink.ui.fittedHint
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ReconnectMachineTest {
    @Test
    fun backsOffAndCaps() {
        val m = ReconnectMachine()
        assertEquals(0, m.begin(0))
        assertEquals(LinkState.Reconnecting(0), m.state)
        val delays = (1..7).map { m.onAttemptFailed(it * 100L) }
        assertEquals(listOf(1000L, 2000L, 4000L, 5000L, 5000L, 5000L, 5000L), delays)
        assertEquals(LinkState.Reconnecting(7), m.state)
    }

    @Test
    fun droppedLinkRetriesQuicklyThenGrows() {
        val m = ReconnectMachine()
        m.begin(0)
        m.onConnected()
        assertEquals(LinkState.Connected, m.state)
        assertEquals(500, m.onDropped(10_000))
        assertEquals(LinkState.Reconnecting(0), m.state)
        assertEquals(1000, m.onAttemptFailed(10_600))
    }

    @Test
    fun unreachableAfterThirtySecondsOfFailing() {
        val m = ReconnectMachine()
        m.onDropped(1_000)
        m.onAttemptFailed(10_000)
        assertEquals(LinkState.Reconnecting(1), m.state)
        m.onAttemptFailed(30_999)
        assertTrue(m.state is LinkState.Reconnecting)
        val slow = m.onAttemptFailed(31_001)
        assertEquals(LinkState.Unreachable, m.state)
        assertEquals(10_000, slow) // keeps trying, slowly
        m.onAttemptFailed(60_000)
        assertEquals(LinkState.Unreachable, m.state)
    }

    @Test
    fun retryButtonRestartsTheClockAndConnectedClearsEverything() {
        val m = ReconnectMachine()
        m.onDropped(0)
        m.onAttemptFailed(40_000)
        assertEquals(LinkState.Unreachable, m.state)
        assertEquals(0, m.retryNow(40_500))
        assertEquals(LinkState.Reconnecting(0), m.state)
        m.onAttemptFailed(41_000)
        assertEquals(LinkState.Reconnecting(1), m.state) // not unreachable again right away
        m.onConnected()
        assertEquals(LinkState.Connected, m.state)
        // a later outage starts from the short delay again
        assertEquals(500, m.onDropped(100_000))
    }

    @Test
    fun stopGoesIdle() {
        val m = ReconnectMachine()
        m.begin(0)
        m.stop()
        assertEquals(LinkState.Idle, m.state)
    }
}

class ReopenProtocolTest {
    @Test
    fun attachOnlyWhenAsked() {
        assertFalse(WsOut.open("a", 1, 2, 1.0).contains("attach"))
        assertTrue(WsOut.open("a", 1, 2, 1.0, attach = true).contains("\"attach\":true"))
    }

    @Test
    fun parsesWindowMessage() {
        val m = WsParser.parse("""{"type":"window","sessionId":"s","width":812,"height":1760,"fitted":true}""")
        assertEquals(WsIn.Window("s", 812, 1760, true), m)
        assertNull(WsParser.parse("""{"type":"window","sessionId":"s"}"""))
    }

    @Test
    fun hintMentionsUpscale() {
        assertTrue(fittedHint(1440, 720).contains("2.0x"))
    }
}

class ImeMappingTest {
    private fun keys(s: String) = ImeMapping.plan(s, true).map { (it as ImePiece.Key).key }

    @Test
    fun plainTextStaysTextAndNewlineIsEnter() {
        val p = ImeMapping.plan("héllo\nworld", false)
        assertEquals(
            listOf(ImePiece.Text("héllo"), ImePiece.Key(dev.penlink.input.KeyLayout.enter), ImePiece.Text("world")),
            p,
        )
    }

    @Test
    fun latinWithModifierIsRealKeys() {
        val k = keys("cV;")
        assertEquals(listOf(0x43, 0x56, Vk.OEM_1), k.map { it.vk })
        assertEquals(listOf(false, true, false), k.map { it.shift })
    }

    @Test
    fun hebrewMapsToPhysicalKeyOfStandardLayout() {
        // ש=A נ=B ב=C ד=S ק=E ר=R א=T ט=Y ו=U פ=P ז=Z ס=X ה=V מ=N צ=M
        assertEquals("ABCSERTYUPZXVNM".map { it.code }, keys("שנבדקראטופזסהמצ").map { it.vk })
    }

    @Test
    fun hebrewFinalFormsAndPunctuation() {
        assertEquals(listOf(0x49, 0x4F, 0x4C, Vk.OEM_1, Vk.OEM_PERIOD), keys("ןםךףץ").map { it.vk })
        assertEquals(listOf(Vk.OEM_COMMA), keys("ת").map { it.vk })
    }

    @Test
    fun unmappableCharFallsBackToText() {
        val p = ImeMapping.plan("a😀", true)
        assertEquals(2, p.size)
        assertTrue(p[0] is ImePiece.Key)
        assertEquals(ImePiece.Text("😀"), p[1])
    }

    @Test
    fun stickyCtrlThenHebrewBetCopies() {
        val logic = KeyboardLogic()
        val out = ArrayList<KeyAction>()
        out += logic.tapModifier(KeyMod.Ctrl)
        assertTrue(logic.anyActive())
        for (p in ImeMapping.plan("ב", logic.anyActive())) out += logic.tapKey((p as ImePiece.Key).key)
        assertEquals(
            listOf(KeyAction(Vk.CONTROL, true), KeyAction(0x43, true), KeyAction(0x43, false), KeyAction(Vk.CONTROL, false)),
            out,
        )
        assertFalse(logic.anyActive())
    }
}
