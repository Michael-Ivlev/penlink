package dev.penlink.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.border
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.data.SavedHost
import dev.penlink.net.PairException
import dev.penlink.protocol.HostAddress
import dev.penlink.ui.theme.CodeTextStyle
import dev.penlink.ui.theme.Spacing
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import java.net.ConnectException
import java.net.NoRouteToHostException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import kotlin.math.roundToInt

const val PAIR_CODE_LENGTH = 6

/** Keeps only digits and at most six of them: what a typed, pasted ("123 456") or autofilled code reduces to. */
fun cleanPairCode(raw: String): String = raw.filter(Char::isDigit).take(PAIR_CODE_LENGTH)

/** Turns a pairing failure into a sentence a person can act on. */
fun friendlyPairError(e: Throwable, address: String): String = when {
    e is PairException && e.message?.contains("code", ignoreCase = true) == true ->
        "That code did not match. Look at the code penlink-host shows on the PC and try again; it changes after a few minutes."
    e is PairException ->
        "The PC refused to pair (${e.message}). Make sure penlink-host is up to date and try again."
    e is UnknownHostException ->
        "Can't find \"$address\". Check the spelling, or use the PC's Tailscale IP address."
    e is SocketTimeoutException || e is ConnectException || e is NoRouteToHostException || e is java.io.IOException ->
        "Can't reach $address. Check that penlink-host is running and that Tailscale is connected on both the phone and the PC."
    else -> e.message ?: "Something went wrong while pairing. Try again."
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairScreen(app: PenlinkApp) {
    val scope = rememberCoroutineScope()
    val haptics = rememberHaptics()
    val context = LocalContext.current
    var address by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var addressTouched by remember { mutableStateOf(false) }
    var failTick by remember { mutableIntStateOf(0) }
    val addressFocus = remember { FocusRequester() }
    val codeFocus = remember { FocusRequester() }

    val normalized = HostAddress.normalize(address)
    val canSubmit = !busy && normalized != null && code.length == PAIR_CODE_LENGTH
    val back = { if (!busy) app.navigate(Screen.Hosts) }
    BackHandler { back() }

    fun submit() {
        val addr = normalized ?: return
        if (busy || code.length != PAIR_CODE_LENGTH) return
        busy = true
        error = null
        scope.launch {
            try {
                val r = app.api.pair(addr, code, Build.MODEL)
                app.store.upsert(SavedHost(addr, r.hostName, r.token))
                haptics.confirm()
                app.navigate(Screen.Hosts)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                haptics.reject()
                error = friendlyPairError(e, addr)
                if (e is PairException) code = ""
                failTick++
                if (e is PairException) codeFocus.requestFocus() else addressFocus.requestFocus()
            } finally {
                busy = false
            }
        }
    }

    // Typing the sixth digit pairs straight away (once the address is valid).
    LaunchedEffect(code) { if (code.length == PAIR_CODE_LENGTH && normalized != null && !busy) submit() }
    LaunchedEffect(Unit) { if (address.isEmpty()) addressFocus.requestFocus() else codeFocus.requestFocus() }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text("Add a PC") },
                navigationIcon = {
                    IconButton(onClick = back, enabled = !busy) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to PCs")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
            )
        },
    ) { padding ->
        Column(
            Modifier.padding(padding).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = Spacing.xl),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Box(
                Modifier.size(72.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Default.Computer, contentDescription = null, Modifier.size(36.dp), tint = MaterialTheme.colorScheme.onPrimaryContainer)
            }
            Spacer(Modifier.height(Spacing.lg))
            Text("Connect to your PC", style = MaterialTheme.typography.headlineSmall, textAlign = TextAlign.Center)
            Text(
                "Start penlink-host on the PC. It prints an address and a 6-digit code.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(top = Spacing.xs),
            )
            Spacer(Modifier.height(Spacing.xl))

            Column(Modifier.widthIn(max = 420.dp), verticalArrangement = Arrangement.spacedBy(Spacing.lg)) {
                val addressInvalid = addressTouched && normalized == null
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it; error = null },
                    label = { Text("PC address") },
                    placeholder = { Text("100.64.0.1") },
                    supportingText = {
                        Text(if (addressInvalid) "Enter the PC's Tailscale IP or name." else "Tailscale IP or name. Emulator: 10.0.2.2")
                    },
                    isError = addressInvalid,
                    leadingIcon = { Icon(Icons.Default.Computer, contentDescription = null) },
                    singleLine = true,
                    enabled = !busy,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
                    keyboardActions = KeyboardActions(onNext = { addressTouched = true; codeFocus.requestFocus() }),
                    shape = MaterialTheme.shapes.medium,
                    modifier = Modifier.fillMaxWidth().focusRequester(addressFocus),
                )

                Column(verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Pairing code", style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
                        TextButton(
                            enabled = !busy,
                            onClick = {
                                haptics.tap()
                                val pasted = readClipboardText(context)?.let(::cleanPairCode).orEmpty()
                                if (pasted.isNotEmpty()) { code = pasted; error = null }
                            },
                        ) {
                            Icon(Icons.Default.ContentPaste, contentDescription = null, Modifier.size(18.dp))
                            Text("Paste", Modifier.padding(start = Spacing.xs))
                        }
                    }
                    CodeEntry(
                        value = code,
                        onValueChange = { code = cleanPairCode(it); error = null },
                        enabled = !busy,
                        isError = error != null,
                        shakeTrigger = failTick,
                        focusRequester = codeFocus,
                        onDone = { addressTouched = true; submit() },
                    )
                }

                AnimatedVisibility(
                    visible = error != null,
                    enter = fadeIn() + expandVertically(),
                    exit = fadeOut() + shrinkVertically(),
                ) {
                    val msg = error ?: ""
                    Surface(shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.errorContainer) {
                        Row(Modifier.padding(Spacing.md), horizontalArrangement = Arrangement.spacedBy(Spacing.md)) {
                            Icon(Icons.Default.ErrorOutline, contentDescription = null, tint = MaterialTheme.colorScheme.onErrorContainer)
                            Text(msg, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer)
                        }
                    }
                }

                Button(
                    onClick = { addressTouched = true; haptics.tap(); submit() },
                    enabled = canSubmit,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
                ) {
                    if (busy) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
                        Text("Pairing...", Modifier.padding(start = Spacing.md))
                    } else {
                        Text("Pair")
                    }
                }
            }
            Spacer(Modifier.height(Spacing.xl))
        }
    }
}

private fun readClipboardText(context: Context): String? = runCatching {
    val cm = context.getSystemService(ClipboardManager::class.java)
    cm?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
}.getOrNull()

/**
 * Six boxes for the pairing code. One invisible text field underneath takes the typing, so the keyboard, paste and
 * autofill all work; the boxes only draw it. The active box follows the cursor ("auto-advance").
 */
@Composable
private fun CodeEntry(
    value: String,
    onValueChange: (String) -> Unit,
    enabled: Boolean,
    isError: Boolean,
    shakeTrigger: Int,
    focusRequester: FocusRequester,
    onDone: () -> Unit,
) {
    var focused by remember { mutableStateOf(false) }
    val keyboard = LocalSoftwareKeyboardController.current
    val shake = remember { Animatable(0f) }
    LaunchedEffect(shakeTrigger) {
        if (shakeTrigger == 0) return@LaunchedEffect
        for (target in listOf(14f, -12f, 8f, -4f, 0f)) shake.animateTo(target, tween(50))
    }
    val description = "6 digit pairing code, ${value.length} of $PAIR_CODE_LENGTH entered"

    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        enabled = enabled,
        singleLine = true,
        textStyle = CodeTextStyle.copy(color = Color.Transparent),
        cursorBrush = SolidColor(Color.Transparent),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { onDone() }),
        modifier = Modifier
            .fillMaxWidth()
            .focusRequester(focusRequester)
            .onFocusChanged { focused = it.isFocused }
            .offset { IntOffset(shake.value.roundToInt(), 0) }
            .semantics { contentDescription = description },
        decorationBox = { inner ->
            Box(
                Modifier.clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                    enabled = enabled,
                ) {
                    focusRequester.requestFocus()
                    keyboard?.show()
                },
            ) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                    val active = value.length.coerceAtMost(PAIR_CODE_LENGTH - 1)
                    repeat(PAIR_CODE_LENGTH) { i ->
                        CodeBox(
                            char = value.getOrNull(i),
                            isActive = focused && i == active,
                            isError = isError,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
                // The real field stays in the tree (focus, IME, paste) but draws nothing.
                Box(Modifier.size(1.dp).alpha(0f)) { inner() }
            }
        },
    )
}

@Composable
private fun CodeBox(char: Char?, isActive: Boolean, isError: Boolean, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    val border by animateColorAsState(
        when {
            isError -> cs.error
            isActive -> cs.primary
            char != null -> cs.outline
            else -> cs.outlineVariant
        },
        label = "codeBorder",
    )
    val fill by animateColorAsState(if (char != null) cs.surfaceContainerHigh else cs.surfaceContainerLow, label = "codeFill")
    Box(
        modifier
            .heightIn(min = 56.dp)
            .height(64.dp)
            .clip(MaterialTheme.shapes.medium)
            .background(fill)
            .border(if (isActive || isError) 2.dp else 1.dp, border, MaterialTheme.shapes.medium),
        contentAlignment = Alignment.Center,
    ) {
        if (char != null) {
            Text(char.toString(), style = CodeTextStyle, color = cs.onSurface)
        } else if (isActive) {
            val t = rememberInfiniteTransition(label = "caret")
            val a = t.animateFloat(1f, 0f, infiniteRepeatable(tween(550, easing = LinearEasing), RepeatMode.Reverse), label = "caretAlpha")
            Box(
                Modifier.size(width = 2.dp, height = 28.dp).graphicsLayer { alpha = a.value }.background(cs.primary),
            )
        }
    }
}
