package dev.shellpp.android

import android.content.Context
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
private fun SettingsRow(title: String, value: String?, tag: String, toggle: Boolean? = null, onClick: () -> Unit) {
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
        } else {
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
            val theme = terminalTheme(values.optJSONObject("terminal.default_theme")?.text("builtin"))?.name ?: "Graphite"
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
            SettingsRow("Shell++", "Version $version", "settings_about") {}
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

/** Settings → Customize keyboard (reference-2 9.2): built in 3.4 of run 15. */
@Composable
fun CustomizeKeyboardScreen(app: App, onClose: () -> Unit) {
    Page(top = { TopBar("Customize keyboard", left = { BackButton(onClose) }) }) {}
}

/** Settings → Font & colours (reference-2 9.3): built in 3.4 of run 15. */
@Composable
fun FontColoursScreen(app: App, onClose: () -> Unit) {
    Page(top = { TopBar("Font & colours", left = { BackButton(onClose) }) }) {}
}
