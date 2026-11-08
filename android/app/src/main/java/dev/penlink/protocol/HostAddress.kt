package dev.penlink.protocol

/** Turns what the user types ("100.64.1.2", "pc:47800", "http://10.0.2.2") into URLs. */
object HostAddress {
    /** Normalised "host:port" (port defaults to 47800). Returns null for blank input. */
    fun normalize(input: String): String? {
        var s = input.trim()
        s = s.removePrefix("http://").removePrefix("https://").removePrefix("ws://").removePrefix("wss://")
        s = s.trimEnd('/')
        val slash = s.indexOf('/')
        if (slash >= 0) s = s.substring(0, slash)
        if (s.isEmpty()) return null
        val hasPort = if (s.startsWith("[")) s.contains("]:") else s.count { it == ':' } == 1
        return if (hasPort) s else "$s:$DEFAULT_PORT"
    }

    /** Whether [input] normalises to something an HTTP client can open (no spaces, a numeric port, a host). */
    fun isValid(input: String): Boolean {
        val n = normalize(input) ?: return false
        if (n.any { it.isWhitespace() }) return false
        val bracket = n.startsWith("[")
        val port = if (bracket) n.substringAfter("]:", "") else n.substringAfterLast(':', "")
        val host = if (bracket) n.substringBefore("]:") else n.substringBeforeLast(':')
        val p = port.toIntOrNull() ?: return false
        return p in 1..65535 && host.isNotEmpty() && host != "["
    }

    fun httpBase(address: String): String = "http://$address"
    fun wsUrl(address: String, token: String): String = "ws://$address/ws?token=$token"
}
