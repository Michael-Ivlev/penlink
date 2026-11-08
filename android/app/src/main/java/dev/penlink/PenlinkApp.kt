package dev.penlink

import android.app.Application
import android.graphics.Bitmap
import android.util.LruCache
import dev.penlink.data.HostStore
import dev.penlink.data.SavedHost
import dev.penlink.net.HostApi
import dev.penlink.rtc.HostConnection
import dev.penlink.rtc.Rtc
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

sealed interface Screen {
    data object Hosts : Screen
    data object Pair : Screen
    data object Apps : Screen
    data class Session(val localId: Long) : Screen
}

/** Process-wide singletons: storage, HTTP client, WebRTC factory, the current host connection. */
class PenlinkApp : Application() {
    lateinit var store: HostStore
        private set
    lateinit var api: HostApi
        private set
    lateinit var rtc: Rtc
        private set

    private val _connection = MutableStateFlow<HostConnection?>(null)
    val connection: StateFlow<HostConnection?> = _connection.asStateFlow()

    private val _screen = MutableStateFlow<Screen>(Screen.Hosts)
    val screen: StateFlow<Screen> = _screen.asStateFlow()

    private val icons = object : LruCache<String, Bitmap>(256) {}

    override fun onCreate() {
        super.onCreate()
        store = HostStore(this)
        api = HostApi()
        rtc = Rtc(this)
    }

    fun navigate(s: Screen) {
        _screen.value = s
    }

    /** Make [host] the active host (keeps its open sessions if it is already active). */
    fun connect(host: SavedHost): HostConnection {
        _connection.value?.let { if (it.host.address == host.address && it.host.token == host.token) return it }
        _connection.value?.shutdown()
        val c = HostConnection(host, api.http, rtc)
        _connection.value = c
        icons.evictAll()
        return c
    }

    fun disconnect() {
        _connection.value?.shutdown()
        _connection.value = null
    }

    fun cachedIcon(appId: String): Bitmap? = icons.get(appId)
    fun cacheIcon(appId: String, bmp: Bitmap) {
        icons.put(appId, bmp)
    }
}
