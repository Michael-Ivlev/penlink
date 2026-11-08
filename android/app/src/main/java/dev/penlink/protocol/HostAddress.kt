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

    fun httpBase(address: String): String = "http://$address"
    fun wsUrl(address: String, token: String): String = "ws://$address/ws?token=$token"
}
