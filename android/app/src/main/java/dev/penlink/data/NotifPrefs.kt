package dev.penlink.data

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** An app that has sent a notification from the PC (shown in the per-app switches). */
@Serializable
data class SeenApp(val appId: String, val name: String)

/**
 * Notification forwarding settings of one PC. Off until the user turns the master switch on; then every app
 * is forwarded except those in [off]. [seen] lists the apps met so far (newest first).
 */
@Serializable
data class NotifPrefs(
    val enabled: Boolean = false,
    val off: Set<String> = emptySet(),
    val seen: List<SeenApp> = emptyList(),
) {
    fun allows(appId: String): Boolean = enabled && appId !in off

    fun withEnabled(on: Boolean) = copy(enabled = on)

    fun withApp(appId: String, on: Boolean) = copy(off = if (on) off - appId else off + appId)

    /** Remember an app (or its new display name); returns the same instance when nothing changed. */
    fun withSeen(appId: String, name: String): NotifPrefs {
        val cur = seen.firstOrNull { it.appId == appId }
        if (cur != null && cur.name == name) return this
        return copy(seen = (listOf(SeenApp(appId, name)) + seen.filterNot { it.appId == appId }).take(MAX_SEEN))
    }

    fun encode(): String = json.encodeToString(serializer(), this)

    companion object {
        const val MAX_SEEN = 100
        private val json = Json { ignoreUnknownKeys = true }

        fun decode(s: String?): NotifPrefs =
            s?.let { runCatching { json.decodeFromString(serializer(), it) }.getOrNull() } ?: NotifPrefs()
    }
}
