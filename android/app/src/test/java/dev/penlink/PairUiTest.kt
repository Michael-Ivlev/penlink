package dev.penlink

import dev.penlink.net.PairException
import dev.penlink.ui.cleanPairCode
import dev.penlink.ui.friendlyPairError
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.ConnectException
import java.net.UnknownHostException

class PairUiTest {
    @Test
    fun pastedCodeKeepsOnlySixDigits() {
        assertEquals("123456", cleanPairCode("123 456"))
        assertEquals("123456", cleanPairCode("Code: 123-456-789"))
        assertEquals("", cleanPairCode("abc"))
        assertEquals("12", cleanPairCode("12"))
    }

    @Test
    fun errorsAreFriendly() {
        assertTrue(friendlyPairError(PairException("Wrong or expired code"), "pc:47800").contains("did not match"))
        assertTrue(friendlyPairError(ConnectException("refused"), "100.64.0.1:47800").contains("Can't reach 100.64.0.1:47800"))
        assertTrue(friendlyPairError(UnknownHostException("x"), "nope:47800").contains("Can't find"))
        assertTrue(friendlyPairError(PairException("Host answered 500"), "pc").contains("refused to pair"))
    }
}

class HostAddressValidTest {
    @Test
    fun acceptsUsableAddresses() {
        for (a in listOf("100.64.0.1", "pc:47800", "http://10.0.2.2", "my-pc.tailnet.ts.net", "[fd7a::1]:47800", " 10.0.2.2 ")) {
            assertTrue(a, dev.penlink.protocol.HostAddress.isValid(a))
        }
    }

    @Test
    fun rejectsUnusableAddresses() {
        for (a in listOf("", "   ", "hello world", "10.0.2.2:abc", "10.0.2.2:99999", "10.0.2.2:0", ":47800", "a b:47800")) {
            assertTrue(a, !dev.penlink.protocol.HostAddress.isValid(a))
        }
    }
}
