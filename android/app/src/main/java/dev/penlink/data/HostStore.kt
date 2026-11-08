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
private val SCALE_KEY = stringPreferencesKey("default_scale")

class HostStore(private val context: Context) {
    private val json = Json { ignoreUnknownKeys = true }
    private val ser = ListSerializer(SavedHost.serializer())

    val hosts: Flow<List<SavedHost>> = context.dataStore.data.map { prefs ->
        prefs[HOSTS_KEY]?.let { runCatching { json.decodeFromString(ser, it) }.getOrNull() } ?: emptyList()
    }

    val defaultScale: Flow<Double> = context.dataStore.data.map { it[SCALE_KEY]?.toDoubleOrNull() ?: 2.0 }

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

    suspend fun setDefaultScale(scale: Double) {
        context.dataStore.edit { it[SCALE_KEY] = scale.toString() }
    }
}
