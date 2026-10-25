package dev.shellpp.android

import android.content.Intent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

@Composable
private fun MenuRow(label: String, tag: String, enabled: Boolean = true, detail: String? = null, onClick: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().heightIn(min = 56.dp)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp)
            .testTag(tag),
    ) {
        Text(label, color = if (enabled) Tokens.text else Tokens.textMuted)
        detail?.let { Text(it, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall) }
    }
    HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Screen(title: String, onClose: (() -> Unit)?, content: @Composable () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    if (onClose != null) {
                        IconButton(onClick = onClose, modifier = Modifier.testTag("screen_close")) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Tokens.bgBase),
            )
        },
        containerColor = Tokens.bgBase,
    ) { padding -> Column(Modifier.padding(padding).fillMaxSize()) { content() } }
}

/** More (UI spec 3): Known hosts, Recently deleted, Account and sync, Settings, Lock. */
@Composable
fun MoreScreen(app: App, onOpen: (Overlay) -> Unit) {
    val scope = rememberCoroutineScope()
    Screen("More", onClose = null) {
        MenuRow("Known hosts", "more_known_hosts", enabled = false, detail = "Comes with the keys update") {}
        MenuRow("Recently deleted", "more_recently_deleted") { onOpen(Overlay.RecentlyDeleted) }
        MenuRow("Account and sync", "more_account", enabled = false, detail = "Comes with the sync update") {}
        MenuRow("Settings", "more_settings") { onOpen(Overlay.Settings) }
        MenuRow("Lock", "more_lock") {
            scope.launch {
                withContext(Dispatchers.IO) {
                    try {
                        app.setLock(LockStatus.from(Core.obj("lock")))
                    } catch (error: CoreException) {
                        app.refreshLock()
                    }
                }
            }
        }
    }
}

/** Sessions (UI spec 3, 9.9): the open sessions; tapping one opens it. */
@Composable
fun SessionsScreen(app: App, activity: MainActivity) {
    val views by app.sessionViews.collectAsState()
    Screen("Sessions", onClose = null) {
        if (views.isEmpty()) {
            Text("No open sessions", Modifier.padding(24.dp), color = Tokens.textSecondary)
            return@Screen
        }
        TextButton(onClick = { app.closeAll() }, modifier = Modifier.padding(horizontal = 8.dp).testTag("disconnect_all")) {
            Text("Disconnect all")
        }
        LazyColumn {
            items(views.values.toList(), key = { it.optLong("id") }) { view ->
                val state = view.optString("state")
                MenuRow(
                    view.optString("title").ifBlank { "Session" },
                    "session_" + view.optLong("id"),
                    detail = when (state) {
                        "dropped" -> "Connection lost" + (view.optInt("reconnectIn", -1).takeIf { it >= 0 }?.let { " · reconnecting in $it s" } ?: "")
                        "ended" -> "Ended"
                        "failed" -> "Failed"
                        else -> state.replaceFirstChar { it.uppercase() }
                    },
                ) {
                    activity.startActivity(
                        Intent(activity, TerminalActivity::class.java).putExtra(TerminalActivity.EXTRA_SESSION, view.optLong("id")),
                    )
                }
            }
        }
    }
}

/** The Keychain place (UI spec 7) until the keys update (M4). */
@Composable
fun KeychainScreen() {
    PlaceholderScreen("Keychain", "Keys come with the next update: generate, import and share them here.")
}

/** Recently deleted (UI spec 6.3): restore only; deleting forever stays on the desktop. */
@Composable
fun RecentlyDeletedScreen(app: App, onClose: () -> Unit) {
    var refresh by remember { mutableStateOf(0) }
    var search by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val items by produceState(listOf<JSONObject>(), refresh) {
        value = withContext(Dispatchers.IO) {
            try {
                val list = Core.array("recently_deleted_list")
                (0 until list.length()).map { list.getJSONObject(it) }
            } catch (error: CoreException) {
                emptyList()
            }
        }
    }
    Screen("Recently deleted", onClose) {
        androidx.compose.material3.OutlinedTextField(
            value = search, onValueChange = { search = it }, placeholder = { Text("Search") }, singleLine = true,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        )
        val shown = items.filter { search.isBlank() || it.optString("title").contains(search, ignoreCase = true) }
        if (shown.isEmpty()) {
            Text("Nothing deleted in the last 90 days", Modifier.padding(24.dp), color = Tokens.textSecondary)
            return@Screen
        }
        LazyColumn {
            items(shown, key = { it.optString("record_id") }) { item ->
                Row(Modifier.fillMaxWidth().heightIn(min = 72.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(item.optString("title"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            item.optString("type_name") + " · " + item.optString("deleted_at").take(16).replace('T', ' '),
                            color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    TextButton(onClick = {
                        scope.launch {
                            withContext(Dispatchers.IO) {
                                try {
                                    Core.call(
                                        "recently_deleted_restore",
                                        JSONObject().put("record_ids", JSONArray().put(item.optString("record_id"))),
                                    )
                                } catch (error: CoreException) {
                                    // The list says what is left.
                                }
                            }
                            refresh += 1
                        }
                    }) { Text("Restore") }
                }
                HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
            }
        }
    }
}

private val AUTO_LOCK = listOf<Pair<Int?, String>>(null to "Never", 5 to "5 minutes", 15 to "15 minutes", 30 to "30 minutes", 60 to "60 minutes")

/** Settings (UI spec 12) as far as M1 goes: Security (auto-lock, fingerprint unlock). */
@Composable
fun SettingsScreen(app: App, activity: MainActivity, onClose: () -> Unit) {
    val lock by app.lock.collectAsState()
    var refresh by remember { mutableStateOf(0) }
    var picking by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<String?>(null) }
    var askPassword by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val autoLock by produceState<Int?>(null, refresh) {
        value = withContext(Dispatchers.IO) {
            try {
                val values = Core.obj("settings_get")
                if (values.isNull("security.auto_lock_minutes")) null else values.optInt("security.auto_lock_minutes")
            } catch (error: CoreException) {
                null
            }
        }
    }
    val prefs = app.unlockPrefs
    var fingerprintOn by remember { mutableStateOf(prefs.getBoolean(Fingerprint.PREF_ON, false) && lock.rememberEnabled) }
    val available = remember(refresh) { app.platform.deviceKeys.available() }
    val offReason = prefs.getString(Fingerprint.PREF_OFF_REASON, null)

    fun setFingerprint(on: Boolean) {
        busy = true
        message = null
        scope.launch {
            if (!on) {
                // The item and the core's row go first; when that fails the
                // option stays on and the switch can try again (architecture 4).
                val off = withContext(Dispatchers.IO) {
                    try {
                        app.setLock(LockStatus.from(Core.obj("remember_device_set", JSONObject().put("enabled", false))))
                        true
                    } catch (error: CoreException) {
                        message = "Fingerprint unlock could not be turned off: ${error.message}"
                        false
                    }
                }
                if (off) {
                    prefs.edit().putBoolean(Fingerprint.PREF_ON, false).remove(Fingerprint.PREF_OFF_REASON).apply()
                    fingerprintOn = false
                }
                busy = false
                return@launch
            }
            // On: a new device key wrapped with the Keystore key, then the
            // prompt reads it back once, so a key the phone cannot use is
            // found now rather than at the next unlock.
            val turnedOn = withContext(Dispatchers.IO) {
                try {
                    app.setLock(LockStatus.from(Core.obj("remember_device_set", JSONObject().put("enabled", true))))
                    true
                } catch (error: CoreException) {
                    message = "Fingerprint unlock could not be turned on: ${error.message}"
                    false
                }
            }
            if (turnedOn) {
                val (result, read) = Fingerprint.withDeviceKey(activity, app, "Turn on fingerprint unlock") {
                    val keys = app.platform.deviceKeys
                    val account = keys.storedAccount()
                    val out = ByteArray(32)
                    val status = if (account != null) keys.read(account, out) else 0
                    out.fill(0)
                    status == 1
                }
                if (result is PromptResult.Passed && read) {
                    prefs.edit().putBoolean(Fingerprint.PREF_ON, true).remove(Fingerprint.PREF_OFF_REASON).apply()
                    fingerprintOn = true
                } else {
                    withContext(Dispatchers.IO) {
                        try {
                            app.setLock(LockStatus.from(Core.obj("remember_device_set", JSONObject().put("enabled", false))))
                        } catch (error: CoreException) {
                            // The option stays off in the app either way.
                        }
                    }
                    if (result is PromptResult.Failed) message = result.message
                }
            }
            busy = false
        }
    }

    Screen("Settings", onClose) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Security", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            MenuRow("Auto-lock", "settings_auto_lock", detail = AUTO_LOCK.firstOrNull { it.first == autoLock }?.second ?: "Never") {
                picking = true
            }
            Row(
                Modifier.fillMaxWidth().heightIn(min = 72.dp).padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Fingerprint unlock", color = if (available) Tokens.text else Tokens.textMuted)
                    val detail = when {
                        offReason == Fingerprint.REASON_KEY_GONE -> "Turned off: the phone's fingerprint key is gone (the screen lock changed). Turn it on again."
                        !available && !fingerprintOn -> "No fingerprint is enrolled on this phone (Android 11 or newer)."
                        fingerprintOn -> "Unlocks at a start and after a lock; the master password is asked once a week."
                        offReason == Fingerprint.REASON_NEW_FINGERPRINT -> "Turned off: a new fingerprint was added to the phone. Turn it on again."
                        offReason == Fingerprint.REASON_NONE_ENROLLED -> "Turned off: no fingerprint is enrolled any more."
                        offReason == Fingerprint.REASON_FAILED -> "Turned off: the fingerprint key could not be used."
                        else -> "Off. Unlock with the fingerprint instead of the master password."
                    }
                    Text(detail, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                }
                Switch(
                    checked = fingerprintOn,
                    enabled = (available || fingerprintOn) && !busy,
                    // Turning it on asks for the master password once first (UI spec 12).
                    onCheckedChange = { on -> if (on) askPassword = true else setFingerprint(false) },
                    modifier = Modifier.testTag("settings_fingerprint"),
                )
            }
            message?.let { Text(it, color = Tokens.danger, modifier = Modifier.padding(horizontal = 16.dp)) }
            Text(
                "A copied password is cleared from the clipboard after 30 seconds, or when the app next has focus.",
                color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(16.dp),
            )
        }
    }
    if (askPassword) {
        var password by remember { mutableStateOf("") }
        var wrong by remember { mutableStateOf<String?>(null) }
        var checking by remember { mutableStateOf(false) }
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { if (!checking) askPassword = false },
            title = { Text("Master password") },
            text = {
                // A dialog is a window of its own: its tags too as ids for the emulator test.
                @OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
                Column(
                    Modifier.semantics { testTagsAsResourceId = true },
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text("Enter the master password to turn on fingerprint unlock.")
                    PasswordField(password, { password = it; wrong = null }, "Master password", "fingerprint_password", error = wrong)
                }
            },
            confirmButton = {
                TextButton(
                    enabled = password.isNotEmpty() && !checking,
                    onClick = {
                        checking = true
                        val chars = password.toCharArray()
                        scope.launch {
                            val failure = withContext(Dispatchers.IO) {
                                try {
                                    Core.withPassword("password_check", "password", chars)
                                    null
                                } catch (error: CoreException) {
                                    when (error.code) {
                                        "invalid_password" -> "Wrong password"
                                        "too_many_attempts" -> "Too many wrong passwords: try again later."
                                        else -> error.message
                                    }
                                }
                            }
                            password = ""
                            checking = false
                            if (failure != null) {
                                wrong = failure
                            } else {
                                askPassword = false
                                setFingerprint(true)
                            }
                        }
                    },
                    modifier = Modifier.testTag("fingerprint_password_ok"),
                ) { Text("Continue") }
            },
            dismissButton = { TextButton(onClick = { askPassword = false }) { Text("Cancel") } },
        )
    }
    if (picking) {
        ChoiceDialog("Auto-lock", AUTO_LOCK.map { it.first?.toString() to it.second }, onDismiss = { picking = false }) { picked ->
            picking = false
            scope.launch {
                withContext(Dispatchers.IO) {
                    try {
                        Core.call(
                            "settings_set",
                            JSONObject().put("values", JSONObject().put("security.auto_lock_minutes", picked?.toInt() ?: JSONObject.NULL)),
                        )
                    } catch (error: CoreException) {
                        // The row shows the value in force.
                    }
                }
                refresh += 1
            }
        }
    }
}
