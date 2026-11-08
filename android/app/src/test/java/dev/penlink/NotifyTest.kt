package dev.penlink

import dev.penlink.data.NotifPrefs
import dev.penlink.notify.NotifyBackoff
import dev.penlink.protocol.NotifyIn
import dev.penlink.protocol.NotifyOut
import dev.penlink.protocol.NotifyParser
import dev.penlink.protocol.notifyWsUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class NotifyTest {
    @Test
    fun parsesReady() {
        val m = NotifyParser.parse("""{"type":"ready","ids":[1,4000000000],"access":"denied"}""")
        assertEquals(NotifyIn.Ready(setOf(1L, 4_000_000_000L), "denied"), m)
    }

    @Test
    fun parsesNotificationWithAndWithoutOpenAppId() {
        val m = NotifyParser.parse(
            """{"type":"notification","id":7,"appId":"Claude_x!App","appName":"Claude","title":"Done","body":"Task finished","time":1700000000000,"openAppId":"ab12"}""",
        )
        assertEquals(NotifyIn.Posted(7, "Claude_x!App", "Claude", "Done", "Task finished", 1_700_000_000_000, "ab12"), m)
        val n = NotifyParser.parse("""{"type":"notification","id":8,"appId":"a","appName":"","title":"t","body":"","time":5}""") as NotifyIn.Posted
        assertNull(n.openAppId)
        assertEquals("a", n.appName) // a blank name falls back to the app id
    }

    @Test
    fun parsesRemovedAndIgnoresJunk() {
        assertEquals(NotifyIn.Removed(9), NotifyParser.parse("""{"type":"notificationRemoved","id":9}"""))
        assertNull(NotifyParser.parse("""{"type":"nope"}"""))
        assertNull(NotifyParser.parse("not json"))
        assertNull(NotifyParser.parse("""{"type":"notification","title":"x"}"""))
    }

    @Test
    fun dismissAndUrl() {
        assertEquals("""{"type":"dismissNotification","id":42}""", NotifyOut.dismiss(42))
        assertEquals("ws://10.0.0.2:47800/notify?token=t", notifyWsUrl("10.0.0.2:47800", "t"))
    }

    @Test
    fun prefsFilterByAppAndMasterSwitch() {
        val p = NotifPrefs()
        assertFalse(p.allows("a")) // off until the master switch is on
        val on = p.withEnabled(true)
        assertTrue(on.allows("a"))
        val muted = on.withApp("a", false)
        assertFalse(muted.allows("a"))
        assertTrue(muted.allows("b"))
        assertTrue(muted.withApp("a", true).allows("a"))
        assertFalse(muted.withEnabled(false).allows("b"))
    }

    @Test
    fun prefsRememberAppsNewestFirstWithoutDuplicates() {
        var p = NotifPrefs().withSeen("a", "Alpha").withSeen("b", "Beta")
        assertEquals(listOf("b", "a"), p.seen.map { it.appId })
        assertSame(p, p.withSeen("b", "Beta")) // unchanged: same instance, no needless write
        p = p.withSeen("a", "Alpha 2")
        assertEquals(listOf("a", "b"), p.seen.map { it.appId })
        assertEquals("Alpha 2", p.seen.first().name)
        var many = NotifPrefs()
        for (i in 0 until 150) many = many.withSeen("app$i", "App $i")
        assertEquals(NotifPrefs.MAX_SEEN, many.seen.size)
    }

    @Test
    fun prefsRoundTripAndTolerateGarbage() {
        val p = NotifPrefs(true, setOf("x"), emptyList()).withSeen("x", "X")
        assertEquals(p, NotifPrefs.decode(p.encode()))
        assertEquals(NotifPrefs(), NotifPrefs.decode(null))
        assertEquals(NotifPrefs(), NotifPrefs.decode("{broken"))
    }

    @Test
    fun backoffGrowsAndCaps() {
        assertEquals(1_000, NotifyBackoff.delayMs(0))
        assertEquals(5_000, NotifyBackoff.delayMs(2))
        assertEquals(30_000, NotifyBackoff.delayMs(4))
        assertEquals(30_000, NotifyBackoff.delayMs(99))
        assertEquals(1_000, NotifyBackoff.delayMs(-3))
    }
}
