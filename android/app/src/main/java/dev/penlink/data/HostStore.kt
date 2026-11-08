package dev.penlink.data

import android.content.Context
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json

/** A paired PC. [address] is "host:port". */
@Serializable
data class SavedHost(val address: String, val name: String, val token: String)

private val Context.dataStore by preferencesDataStore(name = "penlink")
private val HOSTS_KEY = stringPreferencesKey("hosts")

/** Stream settings are remembered per PC (key `stream:<address>`). */
private fun streamKey(address: String) = stringPreferencesKey("stream:$address")

/** Notification forwarding settings are remembered per PC (key `notif:<address>`). */
private fun notifKey(address: String) = stringPreferencesKey("notif:$address")

class HostStore(private val context: Context) {
    private val json = Json { ignoreUnknownKeys = true }
    private val ser = ListSerializer(SavedHost.serializer())

    val hosts: Flow<List<SavedHost>> = context.dataStore.data.map { prefs ->
        prefs[HOSTS_KEY]?.let { runCatching { json.decodeFromString(ser, it) }.getOrNull() } ?: emptyList()
    }

    fun streamPrefs(address: String): Flow<StreamPrefs> =
        context.dataStore.data.map { StreamPrefs.decode(it[streamKey(address)]) }

    suspend fun upsert(host: SavedHost) {
        context.dataStore.edit { prefs ->
            val cur = prefs[HOSTS_KEY]?.let { runCatching { json.decodeFromString(ser, it) }.getOrNull() } ?: emptyList()
            val next = cur.filterNot { it.address == host.address } + host
            prefs[HOSTS_KEY] = json.encodeToString(ser, next)
        }
    }

    suspend fun remove(address: String) {
        context.dataStore.edit { prefs ->
            val cur = prefs[HOSTS_KEY]?.let { runCatching { json.decodeFromString(ser, it) }.getOrNull() } ?: emptyList()
            prefs[HOSTS_KEY] = json.encodeToString(ser, cur.filterNot { it.address == address })
        }
    }

    suspend fun setStreamPrefs(address: String, prefs: StreamPrefs) {
        context.dataStore.edit { it[streamKey(address)] = prefs.encode() }
    }

    fun notifPrefs(address: String): Flow<NotifPrefs> =
        context.dataStore.data.map { NotifPrefs.decode(it[notifKey(address)]) }

    /** Read-modify-write inside one DataStore transaction. */
    suspend fun updateNotifPrefs(address: String, change: (NotifPrefs) -> NotifPrefs) {
        context.dataStore.edit { prefs ->
            val cur = NotifPrefs.decode(prefs[notifKey(address)])
            val next = change(cur)
            if (next != cur) prefs[notifKey(address)] = next.encode()
        }
    }

    /** The saved PCs whose notification switch is on (changes as PCs or switches change). */
    val notifHosts: Flow<List<SavedHost>> = context.dataStore.data.map { prefs ->
        val all = prefs[HOSTS_KEY]?.let { runCatching { json.decodeFromString(ser, it) }.getOrNull() } ?: emptyList()
        all.filter { NotifPrefs.decode(prefs[notifKey(it.address)]).enabled }
    }
}
