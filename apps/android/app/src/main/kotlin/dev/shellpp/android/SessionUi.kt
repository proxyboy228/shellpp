package dev.shellpp.android

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Apps
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.OpenWith
import androidx.compose.material.icons.filled.Password
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** The bar's colors: the terminal theme's on a terminal session, the app's on SFTP (reference-2 11.2). */
data class BarColors(val background: Color, val surface: Color, val content: Color, val accent: Color, val muted: Color)

/** A session as the bar shows it. */
data class BarSession(val id: Long, val title: String, val state: String, val sftp: Boolean, val mark: OsMark)

/**
 * The session bar (reference-1 5, reference-2 7.5-7.6): "<", the strip of
 * session chips (the active one with its ×, the others a tap away), "+",
 * and the keyboard button on terminal sessions. The × stands apart from the
 * buttons beside it and no pop-up covers it (D220).
 */
@Composable
fun SessionBar(
    sessions: List<BarSession>,
    active: Long,
    colors: BarColors,
    keyboardShown: Boolean,
    showKeyboardButton: Boolean,
    onBack: () -> Unit,
    onSwitch: (Long) -> Unit,
    onClose: (Long) -> Unit,
    onConnect: () -> Unit,
    onSftp: () -> Unit,
    onKeyboard: () -> Unit,
) {
    var plus by remember { mutableStateOf(false) }
    val strip = rememberLazyListState()
    val index = sessions.indexOfFirst { it.id == active }
    LaunchedEffect(active, sessions.size) { if (index >= 0) strip.animateScrollToItem(index) }
    Row(
        Modifier.fillMaxWidth().background(colors.background).padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SquareButton(Icons.AutoMirrored.Filled.ArrowBack, "Back to the lists", "session_back", colors, onBack)
        Spacer(Modifier.width(8.dp))
        LazyRow(Modifier.weight(1f), state = strip, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            items(sessions, key = { it.id }) { session ->
                SessionChip(session, session.id == active, colors, onSwitch = { onSwitch(session.id) }, onClose = { onClose(session.id) })
            }
        }
        // Set apart from the chip's ×, so that a miss does not close a session.
        Spacer(Modifier.width(12.dp))
        Box {
            SquareButton(Icons.Filled.Add, "Connect again", "session_plus", colors) { plus = true }
            // Opens above the bar, anchored to "+": never over the chip's × (D220).
            DropdownMenu(expanded = plus, onDismissRequest = { plus = false }, containerColor = Tokens.bgRaised) {
                DropdownMenuItem(text = { Text("Connect") }, onClick = { plus = false; onConnect() }, modifier = Modifier.testTag("plus_connect"))
                DropdownMenuItem(text = { Text("Connect via SFTP") }, onClick = { plus = false; onSftp() }, modifier = Modifier.testTag("plus_sftp"))
            }
        }
        if (showKeyboardButton) {
            Spacer(Modifier.width(6.dp))
            SquareButton(
                if (keyboardShown) Icons.Filled.Apps else Icons.Filled.Keyboard,
                if (keyboardShown) "Hide the keyboard" else "Show the keyboard",
                "session_keyboard", colors, onKeyboard,
            )
        }
    }
}

@Composable
private fun SquareButton(icon: ImageVector, description: String, tag: String, colors: BarColors, onClick: () -> Unit) {
    Box(
        Modifier.size(48.dp).clip(RoundedCornerShape(12.dp)).background(colors.surface).clickable(onClick = onClick).testTag(tag),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, contentDescription = description, tint = colors.accent) }
}

/** A chip of the strip: the active one in full with its ×, the others faded, a tap switches. */
@Composable
private fun SessionChip(session: BarSession, active: Boolean, colors: BarColors, onSwitch: () -> Unit, onClose: () -> Unit) {
    Row(
        Modifier.height(48.dp).clip(RoundedCornerShape(12.dp)).background(colors.surface)
            .alpha(if (active) 1f else 0.55f)
            .clickable(enabled = !active, onClick = onSwitch)
            .padding(start = 6.dp, end = if (active) 0.dp else 10.dp)
            .widthIn(max = if (active) 260.dp else 140.dp)
            .testTag((if (active) "chip_active_" else "chip_") + session.title),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OsTile(session.mark, 32.dp, sftp = session.sftp)
        Spacer(Modifier.width(8.dp))
        Text(
            session.title + if (session.state.isNotEmpty() && session.state != "connected") " · ${stateText(session.state)}" else "",
            color = colors.content, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        if (active) {
            Box(
                Modifier.size(48.dp).clickable(onClick = onClose).testTag("chip_close"),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.Close, contentDescription = "Close the session", tint = colors.muted, modifier = Modifier.size(20.dp)) }
        }
    }
}

fun stateText(state: String): String = when (state) {
    "connecting" -> "connecting"
    "dropped" -> "reconnecting"
    "ended" -> "ended"
    "failed" -> "failed"
    else -> state
}

/** What the extra-keys row does with a key. */
interface KeySink {
    fun key(key: XKey)
    val ctrlState: StickyModifier.State
    val altState: StickyModifier.State
    val cursorMode: Boolean
}

/**
 * The extra-keys row (reference-2 7.3-7.4): the groups of Customize
 * keyboard one after another, scrolling sideways, keys without borders in
 * the accent; held arrows and Delete repeat.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ExtraKeysRow(groups: List<List<XKey>>, colors: BarColors, sink: KeySink, version: Int, password: Boolean, attach: Boolean) {
    LazyRow(
        Modifier.fillMaxWidth().background(colors.background).heightIn(min = 48.dp).testTag("extra_keys"),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        items(groups.size) { index ->
            Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                for (key in groups[index]) KeyChip(key, colors, sink, version, password, attach)
            }
        }
    }
}

@Composable
private fun KeyChip(key: XKey, colors: BarColors, sink: KeySink, @Suppress("UNUSED_PARAMETER") version: Int, password: Boolean, attach: Boolean) {
    val scope = rememberCoroutineScope()
    val enabled = when (key) {
        XKey.Password -> password
        XKey.Attach -> attach
        else -> true
    }
    val lit = when (key) {
        XKey.Ctrl -> sink.ctrlState != StickyModifier.State.OFF
        XKey.Alt -> sink.altState != StickyModifier.State.OFF
        XKey.Cursor -> sink.cursorMode
        else -> false
    }
    val locked = (key == XKey.Ctrl && sink.ctrlState == StickyModifier.State.LOCKED) ||
        (key == XKey.Alt && sink.altState == StickyModifier.State.LOCKED)
    val repeats = key is XKey.Code && key.repeats
    Box(
        Modifier.heightIn(min = 48.dp).widthIn(min = 44.dp).clip(RoundedCornerShape(10.dp))
            .background(if (locked) colors.accent else if (lit) colors.accent.copy(alpha = 0.25f) else Color.Transparent)
            .pointerInput(key, enabled) {
                if (!enabled) return@pointerInput
                awaitEachGesture {
                    awaitFirstDown()
                    sink.key(key)
                    if (!repeats) {
                        waitForUpOrCancellation()
                        return@awaitEachGesture
                    }
                    // A held arrow or Delete repeats until it is let go.
                    val repeat = scope.launch {
                        delay(400)
                        while (true) {
                            sink.key(key)
                            delay(60)
                        }
                    }
                    waitForUpOrCancellation()
                    repeat.cancel()
                }
            }
            .padding(horizontal = 10.dp)
            .testTag("key_" + key.id),
        contentAlignment = Alignment.Center,
    ) {
        val tint = when {
            !enabled -> colors.muted.copy(alpha = 0.4f)
            locked -> colors.background
            else -> colors.accent
        }
        when (key) {
            XKey.Password -> Icon(Icons.Filled.Password, contentDescription = "Send the saved password", tint = tint)
            XKey.Cursor -> Icon(Icons.Filled.OpenWith, contentDescription = "Move the cursor by dragging", tint = tint)
            XKey.Attach -> Icon(Icons.Filled.AttachFile, contentDescription = "Send a file to the server", tint = tint)
            else -> Text(key.label, color = tint, fontFamily = FontFamily.Monospace, fontSize = 15.sp, fontWeight = FontWeight.Medium)
        }
    }
}
