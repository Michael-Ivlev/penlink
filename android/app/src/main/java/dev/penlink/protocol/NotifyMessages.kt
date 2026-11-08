package dev.penlink.protocol

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/** Host -> phone on the `/notify` WebSocket (SPEC "Notifications"). */
sealed interface NotifyIn {
    /** First message: the ids of the notifications that exist on the PC now, and whether Windows lets the host read them. */
    data class Ready(val ids: Set<Long>, val access: String) : NotifyIn

    data class Posted(
        val id: Long,
        val appId: String,
        val appName: String,
        val title: String,
        val body: String,
        val timeMs: Long,
        /** The `/apps` id that opens this app, when the host could map it. */
        val openAppId: String?,
    ) : NotifyIn

    data class Removed(val id: Long) : NotifyIn
}

object NotifyParser {
    private val json = Json { ignoreUnknownKeys = true }

    /** Null for anything that is not a known, well-formed message. */
    fun parse(text: String): NotifyIn? = try {
        val o: JsonObject = json.parseToJsonElement(text).jsonObject
        fun str(k: String) = o[k]?.jsonPrimitive?.contentOrNull
        fun num(k: String) = o[k]?.jsonPrimitive?.longOrNull
        when (str("type")) {
            "ready" -> NotifyIn.Ready(
                o["ids"]?.jsonArray?.mapNotNull { it.jsonPrimitive.longOrNull }?.toSet() ?: emptySet(),
                str("access") ?: "allowed",
            )
            "notification" -> NotifyIn.Posted(
                id = num("id") ?: return null,
                appId = str("appId") ?: return null,
                appName = str("appName")?.takeIf { it.isNotBlank() } ?: str("appId") ?: return null,
                title = str("title") ?: "",
                body = str("body") ?: "",
                timeMs = num("time") ?: System.currentTimeMillis(),
                openAppId = str("openAppId")?.takeIf { it.isNotBlank() },
            )
            "notificationRemoved" -> NotifyIn.Removed(num("id") ?: return null)
            else -> null
        }
    } catch (_: Exception) {
        null
    }
}

object NotifyOut {
    fun dismiss(id: Long): String = buildJsonObject {
        put("type", "dismissNotification")
        put("id", id)
    }.toString()
}

fun notifyWsUrl(address: String, token: String): String = "ws://$address/notify?token=$token"
