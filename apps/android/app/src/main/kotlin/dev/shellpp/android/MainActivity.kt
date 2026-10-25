package dev.shellpp.android

import android.os.Bundle
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * The app's screens (UI spec 3-6, 11, 12) on Compose: first run, unlock,
 * and the four places of the bottom bar. The terminal is
 * [TerminalActivity], a View of its own.
 */
class MainActivity : ComponentActivity() {
    private val app get() = application as App

    @OptIn(ExperimentalComposeUiApi::class)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            ShellTheme {
                // Test tags as resource ids: the emulator test finds the
                // screens' controls with UiAutomator.
                Box(
                    Modifier
                        .fillMaxSize()
                        .background(Tokens.bgBase)
                        .semantics { testTagsAsResourceId = true },
                ) {
                    AppRoot(app, this@MainActivity)
                }
            }
        }
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) app.noteActivity()
        return super.dispatchTouchEvent(event)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        app.noteActivity()
        return super.dispatchKeyEvent(event)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        app.platform.focused = hasFocus
    }
}

@Composable
fun AppRoot(app: App, activity: MainActivity) {
    val lock by app.lock.collectAsState()
    Box(Modifier.fillMaxSize().safeDrawingPadding()) {
        when {
            lock.state == "starting" -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            lock.state == "failed" -> Text(
                "The core did not start: ${app.startError}",
                Modifier.padding(16.dp),
                color = Tokens.danger,
            )
            lock.firstRun -> FirstRunScreen(app)
            !lock.unlocked -> UnlockScreen(app, activity, lock)
            else -> MainScreen(app, activity)
        }
    }
}

/** Advice only (UI spec 4): longer and more varied is stronger. */
fun passwordStrength(password: String): Int {
    if (password.isEmpty()) return 0
    val classes = listOf(
        password.any { it.isLowerCase() },
        password.any { it.isUpperCase() },
        password.any { it.isDigit() },
        password.any { !it.isLetterOrDigit() },
    ).count { it }
    val score = password.length / 4 + classes
    return when {
        password.length < 8 -> 1
        score < 5 -> 2
        score < 7 -> 3
        else -> 4
    }
}

@Composable
fun PasswordField(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    tag: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    imeAction: ImeAction = ImeAction.Next,
    onDone: () -> Unit = {},
    error: String? = null,
) {
    var shown by remember { mutableStateOf(false) }
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = true,
        enabled = enabled,
        isError = error != null,
        supportingText = error?.let { { Text(it) } },
        visualTransformation = if (shown) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = imeAction),
        keyboardActions = KeyboardActions(onDone = { onDone() }, onGo = { onDone() }),
        trailingIcon = {
            TextButton(onClick = { shown = !shown }) { Text(if (shown) "Hide" else "Show") }
        },
        modifier = modifier.fillMaxWidth().testTag(tag),
    )
}

/** UI spec 4: the first run, a new vault ("Restore from sync" comes with sync, M3). */
@Composable
fun FirstRunScreen(app: App) {
    var creating by remember { mutableStateOf(false) }
    var password by remember { mutableStateOf("") }
    var repeat by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Spacer(Modifier.height(32.dp))
        Text("Shell++", style = MaterialTheme.typography.headlineSmall)
        Text(
            "Your hosts, keys and passwords are stored on this phone, encrypted with your master password.",
            color = Tokens.textSecondary,
        )
        if (!creating) {
            Button(
                onClick = { creating = true },
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("create_vault_start"),
            ) { Text("Create a new vault") }
            return@Column
        }
        PasswordField(password, { password = it; error = null }, "Master password", "first_password", enabled = !busy)
        val strength = passwordStrength(password)
        LinearProgressIndicator(
            progress = { strength / 4f },
            modifier = Modifier.fillMaxWidth(),
            color = if (strength <= 1) Tokens.danger else Tokens.accent,
        )
        Text(
            listOf("", "Too short", "Weak", "Fair", "Strong")[strength],
            style = MaterialTheme.typography.bodySmall,
            color = Tokens.textSecondary,
        )
        PasswordField(
            repeat, { repeat = it; error = null }, "Repeat password", "first_repeat",
            enabled = !busy, imeAction = ImeAction.Done,
            error = if (repeat.isNotEmpty() && repeat != password) "The passwords differ" else null,
        )
        Text(
            "A forgotten master password cannot be recovered: without it the data on this phone cannot be opened.",
            color = Tokens.textSecondary,
            style = MaterialTheme.typography.bodyMedium,
        )
        error?.let { Text(it, color = Tokens.danger) }
        Button(
            enabled = !busy && password.length >= 8 && password == repeat,
            onClick = {
                busy = true
                val chars = password.toCharArray()
                scope.launch {
                    val failure = withContext(Dispatchers.IO) {
                        try {
                            app.setLock(LockStatus.from(Core.withPassword("first_run_setup", "password", chars) as JSONObject))
                            // The phone locks after 15 minutes by default (UI spec 5).
                            Core.call(
                                "settings_set",
                                JSONObject().put("values", JSONObject().put("security.auto_lock_minutes", 15)),
                            )
                            app.unlockPrefs.edit().putLong(Fingerprint.PREF_LAST_PASSWORD, System.currentTimeMillis()).apply()
                            null
                        } catch (failed: CoreException) {
                            failed.message
                        }
                    }
                    password = ""
                    repeat = ""
                    busy = false
                    error = failure
                }
            },
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("create_vault"),
        ) {
            if (busy) CircularProgressIndicator(Modifier.height(20.dp), color = Tokens.onAccent) else Text("Create vault")
        }
    }
}

/** UI spec 5: the master password, or the fingerprint first when it is on (D218). */
@Composable
fun UnlockScreen(app: App, activity: MainActivity, lock: LockStatus) {
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<String?>(null) }
    var forgot by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }
    val blocker = remember(lock) { Fingerprint.blocker(app, lock) }

    fun useFingerprint() {
        scope.launch {
            busy = true
            val (result, unlocked) = Fingerprint.withDeviceKey(activity, app, "Unlock Shell++") {
                try {
                    Core.call("try_device_unlock") as Boolean
                } catch (failed: CoreException) {
                    false
                }
            }
            busy = false
            when {
                unlocked -> withContext(Dispatchers.IO) { app.refreshLock() }
                result is PromptResult.LockedOut -> message = "Too many attempts with the fingerprint. Enter the master password."
                result is PromptResult.Failed -> message = result.message
                else -> {}
            }
        }
    }

    LaunchedEffect(Unit) {
        if (blocker == null) useFingerprint() else focus.requestFocus()
    }

    fun unlock() {
        if (password.isEmpty() || busy) return
        busy = true
        val chars = password.toCharArray()
        scope.launch {
            val failure = withContext(Dispatchers.IO) {
                try {
                    app.setLock(LockStatus.from(Core.withPassword("unlock", "password", chars) as JSONObject))
                    Fingerprint.afterPasswordUnlock(app)
                    app.refreshLock()
                    null
                } catch (failed: CoreException) {
                    when (failed.code) {
                        "invalid_password" -> "Wrong password"
                        "too_many_attempts" -> {
                            val seconds = (failed.details as? JSONObject)?.optLong("retry_after_secs") ?: 0
                            "Too many wrong passwords. Try again in $seconds s."
                        }
                        else -> failed.message
                    }
                }
            }
            password = ""
            busy = false
            message = failure
        }
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Spacer(Modifier.height(32.dp))
        Text("Shell++", style = MaterialTheme.typography.headlineSmall)
        if (blocker == "weekly") {
            Text("Enter the master password: it is asked once a week so that it is not forgotten.", color = Tokens.textSecondary)
        }
        PasswordField(
            password, { password = it; message = null }, "Master password", "unlock_password",
            modifier = Modifier.focusRequester(focus), enabled = !busy, imeAction = ImeAction.Go,
            onDone = ::unlock, error = message,
        )
        Button(
            onClick = ::unlock,
            enabled = !busy && password.isNotEmpty(),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("unlock"),
        ) { Text("Unlock") }
        if (blocker == null) {
            OutlinedButton(
                onClick = ::useFingerprint,
                enabled = !busy,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("unlock_fingerprint"),
            ) { Text("Use fingerprint") }
        }
        TextButton(onClick = { forgot = true }, modifier = Modifier.testTag("forgot")) { Text("Forgot password?") }
    }
    if (forgot) {
        ForgotDialog(app, onDismiss = { forgot = false })
    }
}

/** "Forgot password?" (UI spec 5): Start with a new vault; Restore from sync comes with sync. */
@Composable
fun ForgotDialog(app: App, onDismiss: () -> Unit) {
    var confirm by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (confirm) "Start with a new vault?" else "Forgot password?") },
        text = {
            Text(
                if (confirm) {
                    "The data on this phone stays in a renamed file that the forgotten password still opens; " +
                        "the new vault starts empty."
                } else {
                    "Without the master password the data on this phone cannot be opened. " +
                        "Restore from sync comes with the sync update."
                },
            )
        },
        confirmButton = {
            TextButton(
                onClick = {
                    if (!confirm) {
                        confirm = true
                    } else {
                        scope.launch {
                            withContext(Dispatchers.IO) {
                                try {
                                    Core.call("start_new_vault")
                                    // Fingerprint unlock belonged to the set-aside vault.
                                    val keys = app.platform.deviceKeys
                                    keys.storedAccount()?.let(keys::delete)
                                    app.unlockPrefs.edit().putBoolean(Fingerprint.PREF_ON, false).apply()
                                } catch (failed: CoreException) {
                                    // The lock state says what happened.
                                }
                                app.refreshLock()
                            }
                            onDismiss()
                        }
                    }
                },
            ) { Text(if (confirm) "Start with a new vault" else "Start with a new vault…", color = Tokens.danger) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
