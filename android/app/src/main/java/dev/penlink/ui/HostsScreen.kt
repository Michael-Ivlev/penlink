package dev.penlink.ui

import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.data.SavedHost
import dev.penlink.protocol.HostAddress
import kotlinx.coroutines.launch

@Composable
fun HostsScreen(app: PenlinkApp) {
    val hosts by app.store.hosts.collectAsState(initial = emptyList())
    val scope = rememberCoroutineScope()
    var adding by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp)) {
        Text("Penlink", style = MaterialTheme.typography.headlineMedium)
        Text("Pick your PC", style = MaterialTheme.typography.bodyMedium)
        Spacer(Modifier.height(12.dp))
        if (hosts.isEmpty()) {
            Text(
                "No PC yet. Run penlink-host on your PC, then add it here with the 6-digit code it prints. " +
                    "Emulator: 10.0.2.2",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(hosts, key = { it.address }) { h ->
                Card(
                    Modifier.fillMaxWidth().clickable {
                        app.connect(h)
                        app.navigate(Screen.Apps)
                    },
                ) {
                    Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(h.name, style = MaterialTheme.typography.titleMedium)
                            Text(h.address, style = MaterialTheme.typography.bodySmall)
                        }
                        IconButton(onClick = { scope.launch { app.store.remove(h.address) } }) {
                            Icon(Icons.Default.Delete, contentDescription = "Forget this PC")
                        }
                    }
                }
            }
        }
        ExtendedFloatingActionButton(
            onClick = { adding = true },
            icon = { Icon(Icons.Default.Add, contentDescription = null) },
            text = { Text("Add PC") },
        )
    }

    if (adding) AddHostDialog(app, onDone = { adding = false })
}

@Composable
private fun AddHostDialog(app: PenlinkApp, onDone: () -> Unit) {
    var address by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    AlertDialog(
        onDismissRequest = { if (!busy) onDone() },
        title = { Text("Add PC") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it },
                    label = { Text("Address (Tailscale IP or name)") },
                    placeholder = { Text("100.64.0.1  or  10.0.2.2:47800") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it.filter(Char::isDigit).take(6) },
                    label = { Text("6-digit code") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                )
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            Button(
                enabled = !busy && code.length == 6 && address.isNotBlank(),
                onClick = {
                    val addr = HostAddress.normalize(address) ?: return@Button
                    busy = true
                    error = null
                    scope.launch {
                        try {
                            val r = app.api.pair(addr, code, Build.MODEL)
                            val host = SavedHost(addr, r.hostName, r.token)
                            app.store.upsert(host)
                            onDone()
                        } catch (e: Exception) {
                            error = e.message ?: "Could not reach the PC"
                        } finally {
                            busy = false
                        }
                    }
                },
            ) { Text(if (busy) "Pairing..." else "Pair") }
        },
        dismissButton = { TextButton(enabled = !busy, onClick = onDone) { Text("Cancel") } },
    )
}
