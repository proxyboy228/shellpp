package dev.shellpp.android

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.DeleteOutline
import androidx.compose.material.icons.filled.DragHandle
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material.icons.filled.RestartAlt
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * The phone's own terminal settings (device settings, not synced, [decision]):
 * the extra-keys row, pinch to zoom, the cursor speed, the screen kept on.
 */
object TerminalPrefs {
    const val NAME = "terminal"
    const val PINCH = "pinch_zoom"
    const val CURSOR_SPEED = "cursor_speed"
    const val KEEP_ON = "keep_screen_on"
    const val KEYS = "extra_keys"
    const val FONT = "font_size"
    const val THEME = "theme"

    /** The theme of new terminals (a built-in slug), or Graphite / Paper with the system's mode. */
    fun theme(context: Context, dark: Boolean): TerminalTheme =
        terminalTheme(of(context).getString(THEME, null)) ?: terminalTheme(if (dark) "graphite" else "paper") ?: TERMINAL_THEMES.first()

    fun of(context: Context) = context.getSharedPreferences(NAME, Context.MODE_PRIVATE)
}

/** The core's settings (UI 12) as JSON, empty when they cannot be read. */
fun coreSettings(): JSONObject = runCatching { Core.obj("settings_get") }.getOrDefault(JSONObject())

fun setCoreSetting(key: String, value: Any?) {
    Core.call("settings_set", JSONObject().put("values", JSONObject().put(key, value ?: JSONObject.NULL)))
}

/** A section header of Settings: large and bold, a full-width line above it (reference-1 4). */
@Composable
private fun SettingsHeader(text: String, first: Boolean = false) {
    if (!first) HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
    Text(text, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, modifier = Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp))
}

/** A two-line row: the title, the current value muted under it, a toggle or a chevron. */
@Composable
private fun SettingsRow(title: String, value: String?, tag: String, toggle: Boolean? = null, chevron: Boolean = true, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(onClick = onClick).padding(horizontal = 20.dp, vertical = 8.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title)
            if (!value.isNullOrEmpty()) Text(value, color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium)
        }
        if (toggle != null) {
            Switch(toggle, onCheckedChange = { onClick() }, colors = SwitchDefaults.colors(checkedTrackColor = Tokens.accent))
        } else if (chevron) {
            Text("›", color = Tokens.textMuted, fontSize = 22.sp)
        }
    }
}

/**
 * Settings (reference-1 4, D220 item 8): one long page of sections with
 * only the rows whose setting works on the phone.
 */
@Composable
fun SettingsScreen(app: App, activity: MainActivity, nav: Nav) {
    val context = LocalContext.current
    val prefs = remember { TerminalPrefs.of(context) }
    val changes by app.changes.collectAsState()
    var refresh by remember { mutableStateOf(0) }
    var editing by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val values by produceState(JSONObject(), changes, refresh) { value = withContext(Dispatchers.IO) { coreSettings() } }
    val account by produceState<JSONObject?>(null, changes) { value = withContext(Dispatchers.IO) { runCatching { Core.obj("sync_status") }.getOrNull() } }
    var pinch by remember { mutableStateOf(prefs.getBoolean(TerminalPrefs.PINCH, true)) }
    var keepOn by remember { mutableStateOf(prefs.getBoolean(TerminalPrefs.KEEP_ON, false)) }
    var cursorSpeed by remember { mutableStateOf(prefs.getInt(TerminalPrefs.CURSOR_SPEED, 2)) }

    fun set(key: String, value: Any?) {
        scope.launch {
            withContext(Dispatchers.IO) { runCatching { setCoreSetting(key, value) } }
            refresh += 1
        }
    }

    Page(top = { TopBar("Settings") }) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            SettingsHeader("Account", first = true)
            val signedIn = account?.optBoolean("signed_in") == true
            SettingsRow(
                "Account and sync",
                if (signedIn) account?.text("email") ?: "Signed in" else "Local only: sign in to sync with your server",
                "settings_account",
            ) { nav.push(Page.Account) }

            SettingsHeader("Terminal")
            val theme = terminalTheme(prefs.getString(TerminalPrefs.THEME, null))?.name ?: "Graphite / Paper"
            SettingsRow("Font and colours", "$theme / ${prefs.getInt(TerminalPrefs.FONT, 14)} / JetBrains Mono", "settings_font") { nav.push(Page.FontColours) }
            SettingsRow("Terminal type", values.text("terminal.term") ?: "xterm-256color", "settings_term") { editing = "terminal.term" }
            SettingsRow("Customize keyboard", "The extra-keys row of the terminal", "settings_keyboard") { nav.push(Page.CustomizeKeyboard) }
            SettingsRow("Pinch to zoom", "Two fingers change the font size", "settings_pinch", toggle = pinch) {
                pinch = !pinch
                prefs.edit().putBoolean(TerminalPrefs.PINCH, pinch).apply()
            }
            SettingsRow("Cursor speed", listOf("Slow", "Normal", "Fast")[cursorSpeed - 1], "settings_cursor_speed") {
                cursorSpeed = cursorSpeed % 3 + 1
                prefs.edit().putInt(TerminalPrefs.CURSOR_SPEED, cursorSpeed).apply()
            }
            SettingsRow("Keep the screen on", "While a terminal is shown", "settings_keep_on", toggle = keepOn) {
                keepOn = !keepOn
                prefs.edit().putBoolean(TerminalPrefs.KEEP_ON, keepOn).apply()
            }

            SettingsHeader("Session")
            val keepalive = if (values.isNull("terminal.keepalive_s")) null else values.optInt("terminal.keepalive_s", -1).takeIf { it >= 0 }
            SettingsRow("Keep-alive interval", keepalive?.let { if (it == 0) "Off" else "$it s" } ?: "Default", "settings_keepalive") { editing = "terminal.keepalive_s" }
            val detect = values.optBoolean("terminal.os_detection_enabled", true)
            SettingsRow("Detect OS", "Shows the system of each host on its tile", "settings_detect_os", toggle = detect) { set("terminal.os_detection_enabled", !detect) }
            val reconnect = values.optBoolean("terminal.autoreconnect", true)
            SettingsRow("Reconnect", "A dropped session connects again by itself", "settings_reconnect", toggle = reconnect) { set("terminal.autoreconnect", !reconnect) }

            SettingsHeader("Security & Privacy")
            SettingsRow("Lock and fingerprint", "Auto-lock, fingerprint unlock", "settings_security") { nav.push(Page.Security) }
            SettingsRow("Lock now", "The master password or the fingerprint opens it again", "more_lock") {
                scope.launch {
                    withContext(Dispatchers.IO) {
                        runCatching { app.setLock(LockStatus.from(Core.obj("lock"))) }
                        app.refreshLock()
                    }
                }
            }

            SettingsHeader("About")
            val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "" }
            SettingsRow("Shell++", "Version $version", "settings_about", chevron = false) {}
        }
    }

    editing?.let { key ->
        var text by remember(key) { mutableStateOf(if (key == "terminal.term") values.text(key) ?: "xterm-256color" else values.opt(key)?.takeIf { it != JSONObject.NULL }?.toString() ?: "") }
        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text(if (key == "terminal.term") "Terminal type" else "Keep-alive interval, seconds (0: off)") },
            text = { OutlinedTextField(text, { text = it.trim() }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("settings_value")) },
            confirmButton = {
                TextButton(onClick = {
                    editing = null
                    if (key == "terminal.term") set(key, text.ifBlank { null }) else set(key, text.toIntOrNull()?.coerceIn(0, 3600))
                }, modifier = Modifier.testTag("settings_value_ok")) { Text("OK") }
            },
            dismissButton = { TextButton(onClick = { editing = null }) { Text("Cancel") } },
        )
    }
}

/**
 * Settings → Customize keyboard (reference-2 9.2): the extra-keys row's
 * groups as cards, each with a delete button, its keys and a drag handle;
 * "+" adds a group, the circular arrow puts back the default set. Changes
 * apply at once and stay on this device (D220 [decision]).
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun CustomizeKeyboardScreen(app: App, onClose: () -> Unit) {
    val context = LocalContext.current
    var groups by remember { mutableStateOf(XKey.groups(context).map { group -> group.map { it.id } }) }
    var adding by remember { mutableStateOf(false) }
    var resetting by remember { mutableStateOf(false) }
    fun store(next: List<List<String>>) {
        groups = next
        XKey.save(context, next)
    }
    BackHandler(onBack = onClose)
    Box(Modifier.fillMaxSize()) {
        Page(top = {
            TopBar(
                "Customize keyboard",
                left = { BackButton(onClose) },
                actions = { BarIcon(Icons.Filled.RestartAlt, "Back to the default keys", "keys_reset") { resetting = true } },
            )
        }) {
            Text(
                "The groups of the extra-keys row, in its order. Drag a group by its handle to move it.",
                color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
            )
            LazyColumn(Modifier.fillMaxSize()) {
                itemsIndexed(groups, key = { index, group -> "$index:" + group.joinToString(",") }) { index, group ->
                    KeyGroupCard(
                        group,
                        tag = "keys_group_$index",
                        onDelete = { store(groups.filterIndexed { at, _ -> at != index }) },
                        onMove = { step ->
                            val target = (index + step).coerceIn(0, groups.size - 1)
                            if (target != index) {
                                val list = groups.toMutableList()
                                list.add(target, list.removeAt(index))
                                store(list)
                            }
                        },
                    )
                }
                item { Spacer(Modifier.height(96.dp)) }
            }
        }
        FloatingActionButton(
            onClick = { adding = true }, containerColor = Tokens.accent, contentColor = Tokens.onAccent,
            shape = RoundedCornerShape(16.dp), modifier = Modifier.align(Alignment.BottomEnd).padding(24.dp).testTag("keys_add"),
        ) { Icon(Icons.Filled.Add, contentDescription = "Add a group") }
    }
    if (adding) {
        var chosen by remember { mutableStateOf(listOf<String>()) }
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text("New group (up to four keys)") },
            text = {
                FlowRow(Modifier.verticalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    for (key in XKey.ALL.values) {
                        val on = key.id in chosen
                        KeyLabel(key, selected = on, modifier = Modifier.clickable {
                            chosen = if (on) chosen - key.id else if (chosen.size < 4) chosen + key.id else chosen
                        }.testTag("keys_pick_" + key.id))
                    }
                }
            },
            confirmButton = {
                TextButton(enabled = chosen.isNotEmpty(), onClick = { adding = false; store(groups + listOf(chosen)) }, modifier = Modifier.testTag("keys_add_ok")) { Text("Add") }
            },
            dismissButton = { TextButton(onClick = { adding = false }) { Text("Cancel") } },
        )
    }
    if (resetting) {
        AlertDialog(
            onDismissRequest = { resetting = false },
            title = { Text("Back to the default keys?") },
            text = { Text("The row gets its default groups again; your changes go.") },
            confirmButton = {
                TextButton(onClick = {
                    resetting = false
                    XKey.save(context, null)
                    groups = XKey.DEFAULT
                }, modifier = Modifier.testTag("keys_reset_ok")) { Text("Reset") }
            },
            dismissButton = { TextButton(onClick = { resetting = false }) { Text("Cancel") } },
        )
    }
}

/** A key as its chip shows it: its label, or the icon keys' names. */
@Composable
private fun KeyLabel(key: XKey, selected: Boolean = false, modifier: Modifier = Modifier) {
    val label = when (key) {
        XKey.Password -> "password"
        XKey.Cursor -> "cursor"
        XKey.Attach -> "file"
        else -> key.label
    }
    Box(
        modifier.heightIn(min = 40.dp).widthIn(min = 40.dp).clip(RoundedCornerShape(10.dp))
            .background(if (selected) Tokens.accent else Tokens.bgInput).padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = if (selected) Tokens.onAccent else Tokens.accent, fontFamily = FontFamily.Monospace, fontSize = 14.sp)
    }
}

/** A group's card: delete, its keys, and a handle that moves it one place per drag step. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun KeyGroupCard(group: List<String>, tag: String, onDelete: () -> Unit, onMove: (Int) -> Unit) {
    val density = LocalDensity.current
    val step = with(density) { 64.dp.toPx() }
    var dragged by remember { mutableStateOf(0f) }
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 4.dp).fillMaxWidth().clip(CardShape).background(Tokens.bgRaised)
            .heightIn(min = 64.dp).padding(horizontal = 4.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BarIcon(Icons.Filled.DeleteOutline, "Delete the group", tag + "_delete", onClick = onDelete)
        FlowRow(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally)) {
            for (id in group) XKey.ALL[id]?.let { KeyLabel(it) }
        }
        Box(
            Modifier.size(48.dp).pointerInput(Unit) {
                detectVerticalDragGestures(onDragEnd = { dragged = 0f }) { _, amount ->
                    dragged += amount
                    if (dragged > step) {
                        dragged = 0f
                        onMove(1)
                    } else if (dragged < -step) {
                        dragged = 0f
                        onMove(-1)
                    }
                }
            }.testTag(tag + "_handle"),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Filled.DragHandle, contentDescription = "Move the group", tint = Tokens.textSecondary) }
    }
}

/** The font sizes of the stepper (sp). */
private val FONT_SIZES = listOf(8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28)

/**
 * Settings → Font & colours (reference-2 9.3): the font size stepper and
 * the font above a grid of the terminal themes; a tap applies at once.
 * The phone ships JetBrains Mono only.
 */
@Composable
fun FontColoursScreen(app: App, onClose: () -> Unit) {
    val context = LocalContext.current
    val prefs = remember { TerminalPrefs.of(context) }
    var size by remember { mutableStateOf(prefs.getInt(TerminalPrefs.FONT, 14)) }
    var theme by remember { mutableStateOf(prefs.getString(TerminalPrefs.THEME, "") ?: "") }
    fun setSize(next: Int) {
        size = next
        prefs.edit().putInt(TerminalPrefs.FONT, next).apply()
    }
    BackHandler(onBack = onClose)
    Page(top = { TopBar("Font & colours", left = { BackButton(onClose) }) }) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            BarIcon(Icons.Filled.Remove, "Smaller", "font_smaller", enabled = size > FONT_SIZES.first()) {
                setSize(FONT_SIZES.lastOrNull { it < size } ?: size)
            }
            Box(
                Modifier.widthIn(min = 48.dp).clip(RoundedCornerShape(8.dp)).background(Tokens.bgInput).padding(horizontal = 12.dp, vertical = 8.dp).testTag("font_size"),
                contentAlignment = Alignment.Center,
            ) { Text(size.toString()) }
            BarIcon(Icons.Filled.Add, "Larger", "font_larger", enabled = size < FONT_SIZES.last()) {
                setSize(FONT_SIZES.firstOrNull { it > size } ?: size)
            }
            Spacer(Modifier.width(12.dp))
            Row(
                Modifier.weight(1f).clip(RoundedCornerShape(8.dp)).background(Tokens.bgInput).heightIn(min = 48.dp).padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("JetBrains Mono", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                Icon(Icons.Filled.ArrowDropDown, contentDescription = null, tint = Tokens.textMuted)
            }
        }
        LazyVerticalGrid(GridCells.Fixed(2), Modifier.fillMaxSize().padding(horizontal = 10.dp)) {
            item {
                // Graphite in dark mode, Paper in light mode: the app's own default.
                ThemeTile(terminalTheme(if (isSystemInDarkTheme()) "graphite" else "paper"), "Graphite / Paper (system)", theme.isEmpty(), "theme_system") {
                    theme = ""
                    prefs.edit().remove(TerminalPrefs.THEME).apply()
                    runCatching { Native.setTheme("") }
                }
            }
            items(TERMINAL_THEMES.size) { index ->
                val shown = TERMINAL_THEMES[index]
                ThemeTile(shown, shown.name, theme == shown.id, "theme_" + shown.id) {
                    theme = shown.id
                    prefs.edit().putString(TerminalPrefs.THEME, shown.id).apply()
                    runCatching { Native.setTheme(shown.id) }
                }
            }
        }
    }
}
