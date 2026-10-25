package dev.shellpp.android

import android.util.Base64
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** How the wizard was opened (UI spec 4, 11.1). */
enum class WizardMode { SignIn, Register, Restore }

/** The sync error's reason (`details.reason`), else its code. */
fun reasonOf(error: CoreException): String =
    (error.details as? JSONObject)?.optString("reason")?.takeIf { it.isNotEmpty() } ?: error.code

/** The server's texts for the codes the wizard meets (UI 12.2, in short). */
fun syncErrorText(error: CoreException): String {
    return when (val reason = reasonOf(error)) {
        "network" -> "No connection to the server."
        "timeout" -> "The server did not answer in time."
        "invalid_origin" -> "Not a server address: use https://, or http:// for this phone itself."
        "certificate_refused" -> "The server's certificate was refused."
        "certificate_changed" -> "The server shows another certificate than the one trusted."
        "invalid_credentials" -> "Wrong email or password."
        "account_exists" -> "An account with this email exists: sign in instead."
        "registration_closed" -> "The server does not take new accounts."
        "invalid_registration_code" -> "Wrong invite code."
        "totp_invalid", "invalid_totp" -> "Wrong code."
        "rate_limited", "too_many_attempts" -> "Too many attempts: wait a little and try again."
        "kdf_too_expensive", "unsafe_kdf" -> "This account's key settings are more than a phone can run."
        "previous_account" -> "This phone's vault belongs to another account: it cannot sign in to this one."
        "weaker_kdf" -> "The server offers weaker key settings than this phone holds."
        else -> error.message ?: reason
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WizardFrame(title: String, onBack: () -> Unit, content: @Composable () -> Unit) {
    BackHandler(onBack = onBack)
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onBack, modifier = Modifier.testTag("wizard_back")) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Tokens.bgBase),
            )
        },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Column(
            Modifier.padding(padding).fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) { content() }
    }
}

@Composable
private fun Busy(busy: Boolean, label: String, tag: String, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        enabled = enabled && !busy,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag),
    ) {
        if (busy) CircularProgressIndicator(Modifier.heightIn(max = 20.dp), color = Tokens.onAccent) else Text(label)
    }
}

/** The certificate the server shows, as the trust dialog lists it. */
@Composable
private fun CertDetails(cert: JSONObject) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Issued to: " + cert.optString("subject"), style = MaterialTheme.typography.bodyMedium)
        Text("Issued by: " + cert.optString("issuer"), style = MaterialTheme.typography.bodyMedium)
        Text("Valid: " + cert.optString("not_before").take(10) + " – " + cert.optString("not_after").take(10), style = MaterialTheme.typography.bodyMedium)
        Text("SHA-256", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
        Text(cert.optString("fingerprint"), style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("cert_fingerprint"))
    }
}

/**
 * The connect wizard (UI spec 11.1, the desktop's 13.5), one step per
 * screen: the server and its certificate, the account (sign in or
 * register), the TOTP code, the recovery key after registering. [onDone]
 * runs when the device is bound.
 */
@Composable
fun SyncWizard(app: App, activity: MainActivity, mode: WizardMode, onClose: () -> Unit, onDone: () -> Unit) {
    var step by remember { mutableStateOf("server") }
    var origin by remember { mutableStateOf("https://") }
    var checkedOrigin by remember { mutableStateOf("") }
    var inviteOnly by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<JSONObject?>(null) }
    var register by remember { mutableStateOf(mode == WizardMode.Register) }
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var invite by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var recoveryKey by remember { mutableStateOf<String?>(null) }
    var totpSecret by remember { mutableStateOf<String?>(null) }
    var weaker by remember { mutableStateOf<JSONObject?>(null) }
    var changed by remember { mutableStateOf(false) }
    // What the weaker-settings and changed-settings notices accepted: kept
    // together for the next attempt (review of run 14).
    var acceptedSettings by remember { mutableStateOf<JSONObject?>(null) }
    var afterKdfChange by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var caCandidate by remember { mutableStateOf<JSONObject?>(null) }
    val scope = rememberCoroutineScope()

    fun run(block: suspend () -> Unit) {
        busy = true
        error = null
        scope.launch {
            try {
                block()
            } catch (failed: CoreException) {
                error = syncErrorText(failed)
            } catch (failed: Exception) {
                error = failed.message ?: "Something went wrong."
            } finally {
                busy = false
            }
        }
    }

    fun afterCheck(check: JSONObject) {
        checkedOrigin = check.optString("origin")
        if (check.optString("kind") == "ready") {
            inviteOnly = check.optBoolean("invite_only")
            problem = null
            step = "account"
        } else {
            problem = check.optJSONObject("problem")
            step = "trust"
        }
    }

    fun trust(choice: JSONObject) = run {
        val check = withContext(Dispatchers.IO) {
            Core.obj("sync_server_trust_verified", JSONObject().put("origin", checkedOrigin).put("choice", choice))
        }
        if (step != "trust") return@run
        if (check.optString("kind") != "ready") {
            error = if (choice.optString("kind") == "ca") "This CA did not issue the server's certificate." else "The trust was not kept: the server shows another certificate now."
            problem = check.optJSONObject("problem") ?: problem
        } else {
            afterCheck(check)
        }
    }

    val pickCa = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        run {
            val bytes = withContext(Dispatchers.IO) {
                activity.contentResolver.openInputStream(uri)?.use { stream ->
                    val buffer = ByteArray(64 * 1024 + 1)
                    var read = 0
                    while (read < buffer.size) {
                        val n = stream.read(buffer, read, buffer.size - read)
                        if (n < 0) break
                        read += n
                    }
                    buffer.copyOf(read)
                }
            } ?: return@run
            if (bytes.size > 64 * 1024) {
                error = "Not a certificate file."
                return@run
            }
            caCandidate = withContext(Dispatchers.IO) {
                Core.obj("ca_file_info", JSONObject().put("file", Base64.encodeToString(bytes, Base64.NO_WRAP)))
            }
        }
    }

    /**
     * A step that may read the device key (a sign-in again after a rotation,
     * its TOTP step): the fingerprint first when fingerprint unlock is on
     * (D218).
     */
    suspend fun <T> withKey(title: String, step: () -> T): T =
        if (!register && app.unlockPrefs.getBoolean(Fingerprint.PREF_ON, false) && app.lock.value.unlocked) {
            Fingerprint.withDeviceKey(activity, app, title, step).second
        } else {
            withContext(Dispatchers.IO) { step() }
        }

    fun signIn(acceptedKdf: JSONObject? = acceptedSettings, afterChange: Boolean = afterKdfChange) = run {
        acceptedSettings = acceptedKdf
        afterKdfChange = afterChange
        val input = JSONObject().put("origin", checkedOrigin).put("email", email.trim())
        if (register && invite.isNotBlank()) input.put("registration_code", invite.trim())
        if (acceptedKdf != null) input.put("accepted_kdf", acceptedKdf)
        if (afterChange) input.put("after_kdf_change", true)
        val command = if (register) "sync_register" else "sync_sign_in"
        val result = try {
            withKey("Confirm the sign-in") {
                // The password crosses in bytes, never as a String (architecture 2).
                Core.callWithSecret(command, input, "password", password.toCharArray()) as JSONObject
            }
        } catch (failed: CoreException) {
            when (reasonOf(failed)) {
                // D189: the server's settings are weaker than this device's.
                "weaker_kdf" -> if (acceptedKdf == null) {
                    weaker = withContext(Dispatchers.IO) { Core.call("sync_weaker_kdf") as? JSONObject }
                    if (weaker != null) return@run
                }
                // D200: the password or the email changed on another device.
                "kdf_changed" -> if (!afterChange) {
                    changed = true
                    return@run
                }
            }
            throw failed
        }
        when (result.optString("kind")) {
            "totp_required" -> step = "totp"
            else -> {
                if (register) {
                    recoveryKey = withContext(Dispatchers.IO) {
                        try {
                            Core.obj("recovery_key_granted").optString("recovery_key")
                        } catch (failed: CoreException) {
                            null
                        }
                    }
                    // The offer to turn on TOTP, which can be skipped (UI spec 11.1, step 3).
                    step = "totp_offer"
                } else {
                    step = "done"
                }
                withContext(Dispatchers.IO) { app.refreshLock() }
            }
        }
    }

    LaunchedEffect(step) {
        if (step == "done") {
            password = ""
            onDone()
        }
    }

    val title = when (mode) {
        WizardMode.Restore -> "Restore from sync"
        else -> if (register) "Register" else "Sign in"
    }
    WizardFrame(title, onBack = {
        if (busy) return@WizardFrame
        when (step) {
            "trust", "account" -> step = "server"
            "totp" -> {
                scope.launch { withContext(Dispatchers.IO) { runCatching { Core.call("sync_sign_in_cancel") } } }
                step = "account"
            }
            "recovery", "totp_offer" -> {}
            else -> onClose()
        }
    }) {
        error?.let { Text(it, color = Tokens.danger, modifier = Modifier.testTag("wizard_error")) }
        when (step) {
            "server" -> {
                Text("Your own Shell++ sync server.", color = Tokens.textSecondary)
                OutlinedTextField(
                    value = origin, onValueChange = { origin = it; error = null },
                    label = { Text("Server address") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    modifier = Modifier.fillMaxWidth().testTag("sync_origin"),
                )
                Busy(busy, "Continue", "sync_continue", enabled = origin.trim().length > "https://".length) {
                    run {
                        val check = withContext(Dispatchers.IO) {
                            Core.obj("sync_server_check", JSONObject().put("origin", origin.trim()))
                        }
                        afterCheck(check)
                    }
                }
            }
            "trust" -> {
                val current = problem
                val kind = current?.optString("kind")
                val cert = current?.optJSONObject("cert")
                Text(
                    when (kind) {
                        "unknown_issuer" -> "The phone does not trust this server's certificate. Check its fingerprint with the server, then trust it here or add the CA that issued it."
                        "expired" -> "The server's certificate is out of its dates: the connection is refused."
                        "wrong_name" -> "The server's certificate is not for this address: the connection is refused."
                        "pin_changed" -> "The server shows another certificate than the one trusted."
                        else -> "The server's certificate was refused: ${current?.optString("reason")}"
                    },
                )
                Text(checkedOrigin, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                if (kind == "pin_changed") {
                    Text("Trusted until now (SHA-256)", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                    Text(current.optString("pinned"), style = MaterialTheme.typography.bodySmall)
                    Text("Shown now", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                }
                cert?.let { CertDetails(it) }
                if (kind == "unknown_issuer" || kind == "pin_changed") {
                    Busy(busy, "Trust this certificate", "trust_pin") {
                        trust(JSONObject().put("kind", "pin").put("sha256", cert?.optString("sha256")))
                    }
                }
                if (kind == "unknown_issuer") {
                    OutlinedButton(
                        onClick = { pickCa.launch(arrayOf("*/*")) },
                        enabled = !busy,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("trust_ca"),
                    ) { Text("Add its CA…") }
                }
                if (kind == "pin_changed" && current.optBoolean("system_trusts")) {
                    OutlinedButton(
                        onClick = { trust(JSONObject().put("kind", "system")) },
                        enabled = !busy,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("trust_system"),
                    ) { Text("Use the system's trust") }
                }
                TextButton(onClick = { step = "server" }, enabled = !busy) { Text("Cancel") }
            }
            "account" -> {
                if (mode == WizardMode.SignIn) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = register, onCheckedChange = { register = it }, enabled = !busy, modifier = Modifier.testTag("sync_register_toggle"))
                        Text("Register a new account")
                    }
                }
                Text(checkedOrigin, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(
                    value = email, onValueChange = { email = it; error = null }, label = { Text("Email") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
                    modifier = Modifier.fillMaxWidth().testTag("sync_email"),
                )
                PasswordField(password, { password = it; error = null }, if (register) "Master password" else "Password", "sync_password")
                if (mode == WizardMode.Restore) {
                    Text("The account's password becomes this phone's master password.", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                }
                if (register && inviteOnly) {
                    OutlinedTextField(
                        value = invite, onValueChange = { invite = it }, label = { Text("Invite code") }, singleLine = true,
                        modifier = Modifier.fillMaxWidth().testTag("sync_invite"),
                    )
                }
                Busy(busy, if (register) "Register" else "Sign in", "sync_sign_in", enabled = email.contains('@') && password.length >= 8) {
                    signIn()
                }
            }
            "totp" -> {
                Text("The one-time code of your authenticator app.")
                OutlinedTextField(
                    value = code, onValueChange = { code = it.filter(Char::isDigit).take(8); error = null },
                    label = { Text("Code") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                    modifier = Modifier.fillMaxWidth().testTag("sync_totp"),
                )
                Busy(busy, "Continue", "sync_totp_continue", enabled = code.length >= 6) {
                    run {
                        val result = withKey("Confirm the sign-in") { Core.obj("sync_sign_in_totp", JSONObject().put("code", code)) }
                        code = ""
                        if (result.optString("kind") == "done") {
                            withContext(Dispatchers.IO) { app.refreshLock() }
                            step = "done"
                        }
                    }
                }
            }
            "totp_offer" -> {
                fun next() {
                    totpSecret = null
                    code = ""
                    step = if (recoveryKey != null) "recovery" else "done"
                }
                Text("Two-factor sign-in", style = MaterialTheme.typography.titleMedium)
                val secret = totpSecret
                if (secret == null) {
                    Text(
                        "Ask for a one-time code from an authenticator app at every sign-in to this account, besides the password.",
                        color = Tokens.textSecondary,
                    )
                    Busy(busy, "Turn it on", "totp_set_up") {
                        run {
                            val setup = withContext(Dispatchers.IO) {
                                Core.withPassword("sync_totp_setup", "password", password.toCharArray()) as JSONObject
                            }
                            totpSecret = setup.optString("secret")
                        }
                    }
                    TextButton(onClick = ::next, modifier = Modifier.testTag("totp_skip")) { Text("Skip") }
                } else {
                    Text(
                        "Add this key to your authenticator app (the QR code is on the desktop), then type the code it shows.",
                        color = Tokens.textSecondary,
                    )
                    Text(secret.chunked(4).joinToString(" "), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.testTag("totp_secret"))
                    OutlinedTextField(
                        value = code, onValueChange = { code = it.filter(Char::isDigit).take(8); error = null },
                        label = { Text("Code") }, singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                        modifier = Modifier.fillMaxWidth().testTag("totp_enable_code"),
                    )
                    Busy(busy, "Turn on", "totp_enable", enabled = code.length >= 6) {
                        run {
                            withContext(Dispatchers.IO) { Core.call("sync_totp_enable", JSONObject().put("code", code)) }
                            next()
                        }
                    }
                    TextButton(onClick = ::next) { Text("Skip") }
                }
            }
            "recovery" -> {
                Text("Your recovery key", style = MaterialTheme.typography.titleMedium)
                Text(
                    "It opens your data when the password is forgotten. Keep it somewhere safe, away from this phone; it is not shown again.",
                    color = Tokens.textSecondary,
                )
                Text(recoveryKey ?: "", style = MaterialTheme.typography.bodyLarge, modifier = Modifier.testTag("recovery_key"))
                OutlinedButton(onClick = {
                    scope.launch {
                        val copied = withContext(Dispatchers.IO) {
                            runCatching { Core.call("secret_copy", JSONObject().put("kind", "recovery_key")) }.isSuccess
                        }
                        error = if (copied) null else "Not copied any more (5 minutes have passed): write it down from the screen."
                    }
                }) { Text("Copy") }
                Busy(busy, "I saved it", "recovery_saved") {
                    recoveryKey = null
                    step = "done"
                }
            }
        }
    }

    caCandidate?.let { candidate ->
        val cert = candidate.optJSONObject("cert") ?: JSONObject()
        AlertDialog(
            onDismissRequest = { caCandidate = null },
            title = { Text("Add this CA for this server?") },
            text = { CertDetails(cert) },
            confirmButton = {
                TextButton(onClick = {
                    caCandidate = null
                    trust(JSONObject().put("kind", "ca").put("der", candidate.optString("der")))
                }) { Text("Add this CA") }
            },
            dismissButton = { TextButton(onClick = { caCandidate = null }) { Text("Cancel") } },
        )
    }

    if (changed) {
        AlertDialog(
            onDismissRequest = { changed = false },
            title = { Text("The account's settings changed") },
            text = {
                Text(
                    "The password or the email of this account was changed on another device. " +
                        "Sign in with the account's current password.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    changed = false
                    signIn(afterChange = true)
                }) { Text("Sign in") }
            },
            dismissButton = { TextButton(onClick = { changed = false }) { Text("Cancel") } },
        )
    }

    weaker?.let { found ->
        AlertDialog(
            onDismissRequest = { weaker = null },
            title = { Text("Weaker key derivation") },
            text = {
                Text(
                    "The server offers weaker key derivation settings for this account than this phone holds. " +
                        "Sign in only if you changed them yourself on another device.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    weaker = null
                    signIn(found.optJSONObject("server"))
                }) { Text("Sign in anyway", color = Tokens.danger) }
            },
            dismissButton = { TextButton(onClick = { weaker = null }) { Text("Cancel") } },
        )
    }
}

/**
 * A changed pinned certificate of a bound phone (UI spec 11.2, the
 * desktop's 12.2): the pinned and the new fingerprints; "Trust the new
 * certificate", "Use the system's trust" when the system trusts it, or
 * Cancel (sync stays stopped).
 */
@Composable
fun PinChangedDialog(origin: String, onDone: (String?) -> Unit) {
    var problem by remember { mutableStateOf<JSONObject?>(null) }
    var busy by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(origin) {
        val check = withContext(Dispatchers.IO) {
            try {
                Core.obj("sync_server_check", JSONObject().put("origin", origin))
            } catch (failed: Exception) {
                null
            }
        }
        busy = false
        when {
            check == null -> onDone("The server did not answer.")
            check.optString("kind") == "ready" -> onDone(null)
            else -> problem = check.optJSONObject("problem")
        }
    }
    fun choose(choice: JSONObject) {
        busy = true
        scope.launch {
            val check = withContext(Dispatchers.IO) {
                try {
                    Core.obj("sync_server_trust_verified", JSONObject().put("origin", origin).put("choice", choice))
                } catch (failed: CoreException) {
                    error = syncErrorText(failed)
                    null
                } catch (failed: Exception) {
                    error = failed.message
                    null
                }
            }
            busy = false
            if (check?.optString("kind") == "ready") onDone(null) else if (check != null) {
                problem = check.optJSONObject("problem")
                error = "The trust was not kept: the server shows another certificate now."
            }
        }
    }
    val current = problem ?: return
    val cert = current.optJSONObject("cert")
    AlertDialog(
        onDismissRequest = {},
        title = { Text(if (current.optString("kind") == "pin_changed") "The server's certificate changed" else "The server's certificate") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(origin, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                if (current.optString("kind") == "pin_changed") {
                    Text("Sync stopped: the server shows another certificate than the one trusted. Check the new one with the server first.")
                    Text("Trusted until now (SHA-256)", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                    Text(current.optString("pinned"), style = MaterialTheme.typography.bodySmall)
                    Text("Shown now", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                }
                cert?.let { CertDetails(it) }
                error?.let { Text(it, color = Tokens.danger) }
            }
        },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                if (current.optString("kind") in setOf("pin_changed", "unknown_issuer") && cert != null) {
                    TextButton(enabled = !busy, onClick = {
                        choose(JSONObject().put("kind", "pin").put("sha256", cert.optString("sha256")))
                    }) { Text("Trust the new certificate") }
                }
                if (current.optBoolean("system_trusts")) {
                    TextButton(enabled = !busy, onClick = { choose(JSONObject().put("kind", "system")) }) { Text("Use the system's trust") }
                }
                TextButton(enabled = !busy, onClick = { onDone("Sync stays stopped until the certificate is trusted.") }) { Text("Cancel") }
            }
        },
    )
}

/** Account and sync once connected (UI spec 11.2). */
@Composable
fun AccountScreen(app: App, activity: MainActivity, onClose: () -> Unit) {
    var refresh by remember { mutableStateOf(0) }
    var busy by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<String?>(null) }
    var signingOut by remember { mutableStateOf(false) }
    var wizard by remember { mutableStateOf(false) }
    var pinCheck by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val changes by app.changes.collectAsState()
    val status by produceState<JSONObject?>(null, refresh, changes) {
        value = withContext(Dispatchers.IO) { runCatching { Core.obj("sync_status") }.getOrNull() }
    }
    val devices by produceState(listOf<JSONObject>(), refresh) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val list = Core.array("sync_devices")
                (0 until list.length()).map { list.getJSONObject(it) }
            }.getOrDefault(emptyList())
        }
    }
    if (wizard) {
        SyncWizard(app, activity, WizardMode.SignIn, onClose = { wizard = false }, onDone = { wizard = false; refresh += 1 })
        return
    }
    val current = status
    WizardFrame("Account and sync", onBack = onClose) {
        if (current == null) {
            CircularProgressIndicator()
            return@WizardFrame
        }
        if (!current.optBoolean("bound")) {
            Text("Sync keeps your hosts, groups and keys the same on your devices, through your own server.", color = Tokens.textSecondary)
            Busy(false, "Sign in or register", "account_connect") { wizard = true }
            return@WizardFrame
        }
        @Composable
        fun row(label: String, value: String, tag: String) {
            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp).testTag(tag)) {
                Text(label, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                Text(value)
            }
        }
        row("Server", current.text("server_url") ?: "", "account_server")
        row("Account", current.text("email") ?: "", "account_email")
        row(
            "Last sync",
            (current.text("last_sync_at")?.take(19)?.replace('T', ' ') ?: "Not yet") +
                (current.text("last_error")?.let { " · $it" } ?: ""),
            "account_last_sync",
        )
        if (current.text("last_error")?.contains("certificate") == true) {
            Busy(false, "Check the server's certificate", "account_check_certificate") {
                pinCheck = current.text("server_url")
            }
        }
        if (!current.optBoolean("signed_in")) {
            Text("Signed out of the server: sign in again to go on syncing.", color = Tokens.danger)
            Busy(false, "Sign in again", "account_sign_in_again") { wizard = true }
        }
        val stopped = listOfNotNull(
            "the server rolled back".takeIf { current.optBoolean("newer_epoch") },
            "the recovery key does not match".takeIf { current.optBoolean("recovery_mismatch") },
        )
        if (stopped.isNotEmpty()) {
            Text("Sync stopped: ${stopped.joinToString()}. Handle it on the desktop.", color = Tokens.danger)
        }
        if (current.optInt("conflicts") > 0) {
            Text("${current.optInt("conflicts")} records changed on two devices at once: both versions are kept; resolve them on the desktop.", color = Tokens.textSecondary)
        }
        if (current.optInt("duplicates") > 0) {
            Text("${current.optInt("duplicates")} possible duplicates: look at them on the desktop.", color = Tokens.textSecondary)
        }
        message?.let { Text(it, color = Tokens.textSecondary, modifier = Modifier.testTag("account_message")) }
        Busy(busy, "Sync now", "account_sync_now") {
            busy = true
            scope.launch {
                message = withContext(Dispatchers.IO) {
                    try {
                        val report = Core.obj("sync_now")
                        "Synced: ${report.optInt("pulled")} in, ${report.optInt("pushed")} out."
                    } catch (failed: CoreException) {
                        // A changed pinned certificate: its dialog (UI 12.2).
                        if (reasonOf(failed) == "certificate_changed") pinCheck = current.text("server_url")
                        syncErrorText(failed)
                    } catch (failed: Exception) {
                        failed.message
                    }
                }
                busy = false
                refresh += 1
            }
        }
        Text("Devices", style = MaterialTheme.typography.titleMedium)
        for (device in devices) {
            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Text(
                    (if (device.optBoolean("current")) "This phone" else "Device " + device.optString("device_id").take(8)) +
                        (if (device.optBoolean("revoked")) " (revoked)" else ""),
                )
                Text("Last seen " + device.optString("last_seen_at").take(16).replace('T', ' '), color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
            }
            HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
        }
        Text(
            "Changing the password, the recovery key and \"Lost or stolen\" are on the desktop.",
            color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall,
        )
        TextButton(onClick = { signingOut = true }, modifier = Modifier.testTag("account_sign_out")) {
            Text("Sign out", color = Tokens.danger)
        }
    }
    pinCheck?.let { origin ->
        PinChangedDialog(origin) { result ->
            pinCheck = null
            message = result ?: "The new certificate is trusted: sync goes on."
            refresh += 1
        }
    }
    if (signingOut) {
        var deleteLocal by remember { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { signingOut = false },
            title = { Text("Sign out?") },
            text = {
                Column {
                    Text("This phone stops syncing; its data stays here unless you delete it.")
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = deleteLocal, onCheckedChange = { deleteLocal = it })
                        Text("Also delete the local copy")
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    signingOut = false
                    scope.launch {
                        withContext(Dispatchers.IO) {
                            try {
                                Core.call("sync_sign_out", JSONObject().put("delete_local", deleteLocal))
                            } catch (failed: CoreException) {
                                message = syncErrorText(failed)
                            }
                            app.refreshLock()
                        }
                        refresh += 1
                    }
                }) { Text("Sign out", color = Tokens.danger) }
            },
            dismissButton = { TextButton(onClick = { signingOut = false }) { Text("Cancel") } },
        )
    }
}
