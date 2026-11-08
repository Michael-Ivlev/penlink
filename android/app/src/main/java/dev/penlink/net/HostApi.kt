package dev.penlink.net

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import dev.penlink.protocol.AppEntry
import dev.penlink.protocol.HostAddress
import dev.penlink.protocol.InfoResponse
import dev.penlink.protocol.PairRequest
import dev.penlink.protocol.PairResponse
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class PairException(message: String) : Exception(message)

/** HTTP side of the protocol: /pair, /info, /apps, /apps/{id}/icon. */
class HostApi(val http: OkHttpClient = defaultClient()) {
    private val json = Json { ignoreUnknownKeys = true }

    suspend fun pair(address: String, code: String, deviceName: String): PairResponse {
        val body = json.encodeToString(PairRequest.serializer(), PairRequest(code, deviceName))
            .toRequestBody("application/json".toMediaType())
        val req = Request.Builder().url(HostAddress.httpBase(address) + "/pair").post(body).build()
        return http.newCall(req).await().use { resp ->
            when {
                resp.code == 403 -> throw PairException("Wrong or expired code")
                !resp.isSuccessful -> throw PairException("Host answered ${resp.code}")
                else -> json.decodeFromString(PairResponse.serializer(), resp.body.string())
            }
        }
    }

    suspend fun info(address: String): InfoResponse {
        val req = Request.Builder().url(HostAddress.httpBase(address) + "/info").build()
        return http.newCall(req).await().use { resp ->
            if (!resp.isSuccessful) throw IOException("Host answered ${resp.code}")
            json.decodeFromString(InfoResponse.serializer(), resp.body.string())
        }
    }

    suspend fun apps(address: String, token: String): List<AppEntry> {
        val req = Request.Builder().url(HostAddress.httpBase(address) + "/apps").auth(token).build()
        return http.newCall(req).await().use { resp ->
            if (resp.code == 401) throw IOException("Unauthorised: pair again")
            if (!resp.isSuccessful) throw IOException("Host answered ${resp.code}")
            json.decodeFromString(ListSerializer(AppEntry.serializer()), resp.body.string())
        }
    }

    suspend fun icon(address: String, token: String, appId: String): Bitmap? {
        val enc = java.net.URLEncoder.encode(appId, "UTF-8").replace("+", "%20")
        val req = Request.Builder().url(HostAddress.httpBase(address) + "/apps/$enc/icon").auth(token).build()
        return runCatching {
            http.newCall(req).await().use { resp ->
                if (!resp.isSuccessful) null
                else withContext(Dispatchers.Default) {
                    resp.body.bytes().let { BitmapFactory.decodeByteArray(it, 0, it.size) }
                }
            }
        }.getOrNull()
    }

    private fun Request.Builder.auth(token: String) = header("Authorization", "Bearer $token")

    companion object {
        fun defaultClient(): OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .pingInterval(20, TimeUnit.SECONDS)
            .build()
    }
}

suspend fun Call.await(): Response = suspendCancellableCoroutine { cont ->
    enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) {
            if (cont.isActive) cont.resumeWithException(e)
        }

        override fun onResponse(call: Call, response: Response) {
            cont.resume(response)
        }
    })
    cont.invokeOnCancellation { runCatching { cancel() } }
}
