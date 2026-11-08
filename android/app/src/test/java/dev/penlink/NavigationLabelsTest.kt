package dev.penlink

import dev.penlink.rtc.SessionStatus
import dev.penlink.ui.sessionStateLabel
import org.junit.Assert.assertEquals
import org.junit.Test

class NavigationLabelsTest {
    @Test
    fun openNowShowsLiveState() {
        assertEquals("Connected", sessionStateLabel(SessionStatus.Live))
        assertEquals("Reconnecting...", sessionStateLabel(SessionStatus.Reconnecting))
        assertEquals("Opening...", sessionStateLabel(SessionStatus.Opening))
        assertEquals("Closed", sessionStateLabel(SessionStatus.Closed("closed")))
    }
}
