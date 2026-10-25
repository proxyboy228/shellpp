package dev.shellpp.android

import android.content.Intent
import android.provider.OpenableColumns
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
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
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** The key types the Generate form offers, as on the desktop (UI 7.1). */
private val KEY_KINDS = listOf(
    "ed25519" to "Ed25519",
    "ecdsa_p256" to "ECDSA P-256",
    "ecdsa_p384" to "ECDSA P-384",
    "ecdsa_p521" to "ECDSA P-521",
    "rsa3072" to "RSA 3072",
    "rsa4096" to "RSA 4096",
)

/** `ssh-ed25519` → "Ed25519" and the like. */
fun algorithmName(algorithm: String): String = when {
    algorithm == "ssh-ed25519" -> "Ed25519"
    algorithm.startsWith("ecdsa-sha2-nistp") -> "ECDSA P-" + algorithm.removePrefix("ecdsa-sha2-nistp")
    algorithm == "ssh-rsa" || algorithm.startsWith("rsa") -> "RSA"
    algorithm.startsWith("sk-") -> "Security key"
    else -> algorithm
}

/** `SHA256:abcdef…` shortened for a list row. */
fun shortFingerprint(fingerprint: String): String =
    if (fingerprint.length > 24) fingerprint.take(20) + "…" + fingerprint.takeLast(4) else fingerprint

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Page(title: String, onBack: (() -> Unit)?, fab: @Composable () -> Unit = {}, snackbar: SnackbarHostState? = null, content: @Composable () -> Unit) {
    if (onBack != null) BackHandler(onBack = onBack)
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack, modifier = Modifier.testTag("page_back")) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Tokens.bgBase),
            )
        },
        floatingActionButton = fab,
        snackbarHost = { snackbar?.let { SnackbarHost(it) } },
        containerColor = Tokens.bgBase,
    ) { padding -> Column(Modifier.padding(padding).fillMaxSize()) { content() } }
}

private fun vault(): String = Core.array("vaults").getJSONObject(0).getString("vault_id")

/**
 * Makes a key in the vault (blocking: off the main thread) and returns its
 * record id; the passphrase crosses in bytes only (architecture 2).
 */
fun generateKey(input: JSONObject, passphrase: CharArray): String {
    input.put("vault_id", vault())
    val made = if (passphrase.isNotEmpty()) {
        Core.callWithSecrets("key_generate", input, listOf("passphrase" to passphrase)) as JSONObject
    } else {
        Core.obj("key_generate", input)
    }
    return made.getJSONObject("key").getString("record_id")
}

/**
 * The text of a picked key file as chars, never a String; the bytes are
 * wiped (architecture 6.4). Null when it cannot be read or is too big to be
 * a key. Blocking: off the main thread.
 */
fun readKeyFile(context: android.content.Context, uri: android.net.Uri): CharArray? = try {
    context.contentResolver.openInputStream(uri)?.use { stream ->
        val limit = 256 * 1024
        val buffer = ByteArray(limit + 1)
        var read = 0
        while (read < buffer.size) {
            val n = stream.read(buffer, read, buffer.size - read)
            if (n < 0) break
            read += n
        }
        // A key file is small; anything big is not one.
        val chars = if (read > limit) {
            null
        } else {
            val decoded = Charsets.UTF_8.decode(java.nio.ByteBuffer.wrap(buffer, 0, read))
            CharArray(decoded.remaining()).also { decoded.get(it); if (decoded.hasArray()) decoded.array().fill('\u0000') }
        }
        buffer.fill(0)
        chars
    }
} catch (error: Exception) {
    null
}

@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
fun GenerateKeyDialog(onDismiss: () -> Unit, onGenerate: (JSONObject, CharArray) -> Unit) {
    var label by remember { mutableStateOf("") }
    var kind by remember { mutableStateOf("ed25519") }
    var passphrase by remember { mutableStateOf("") }
    var save by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Generate a key") },
        text = {
            // A dialog is a window of its own: its tags too as ids for the emulator test.
            Column(
                Modifier.verticalScroll(rememberScrollState()).semantics { testTagsAsResourceId = true },
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedTextField(label, { label = it }, label = { Text("Label") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("key_label"))
                for ((value, name) in KEY_KINDS) {
                    Row(Modifier.fillMaxWidth().clickable { kind = value }.heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically) {
                        androidx.compose.material3.RadioButton(selected = kind == value, onClick = { kind = value })
                        Text(name)
                    }
                }
                PasswordField(passphrase, { passphrase = it }, "Passphrase (optional)", "key_passphrase")
                if (passphrase.isNotEmpty()) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(save, { save = it })
                        Text("Save the passphrase in the vault")
                    }
                }
            }
        },
        confirmButton = {
            TextButton(enabled = label.isNotBlank(), onClick = {
                val input = JSONObject().put("label", label.trim()).put("kind", kind).put("save_passphrase", save)
                val secret = passphrase.toCharArray()
                passphrase = ""
                onGenerate(input, secret)
            }) { Text("Generate") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** Imports the text of a picked key file; asks for its passphrase when it has one (UI spec 7). */
@Composable
internal fun ImportKeyDialog(text: CharArray, onDone: (String?) -> Unit) {
    var passphrase by remember { mutableStateOf("") }
    var save by remember { mutableStateOf(false) }
    var askPassphrase by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun run() {
        busy = true
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                try {
                    val input = JSONObject().put("vault_id", vault()).put("save_passphrase", save)
                    // The private key and its passphrase cross in bytes only (a
                    // copy of the key: the passphrase prompt may need it again).
                    val secrets = mutableListOf("private_key" to text.copyOf())
                    if (passphrase.isNotEmpty()) secrets += "passphrase" to passphrase.toCharArray()
                    val imported = Core.callWithSecrets("key_import", input, secrets) as JSONObject
                    if (imported.optBoolean("created")) "imported" else "exists"
                } catch (failed: CoreException) {
                    // The core's refused fields are `details.errors` (crates/core/src/error.rs).
                    val fields = (failed.details as? JSONObject)?.optJSONArray("errors")?.toString() ?: ""
                    when {
                        fields.contains("passphrase") -> "passphrase:" + (if (passphrase.isEmpty()) "" else "wrong")
                        else -> "error:" + (failed.message ?: failed.code)
                    }
                } catch (failed: Exception) {
                    "error:The key was not imported."
                }
            }
            busy = false
            when {
                result == "imported" -> onDone("Key imported")
                result == "exists" -> onDone("This key is already in the Keychain")
                result.startsWith("passphrase:") -> {
                    askPassphrase = true
                    error = if (result.endsWith("wrong")) "Wrong passphrase" else null
                    passphrase = ""
                }
                else -> onDone(
                    if (result.contains("validation")) "This file is not a key: nothing was imported." else result.removePrefix("error:"),
                )
            }
        }
    }

    if (!askPassphrase) {
        androidx.compose.runtime.LaunchedEffect(Unit) { run() }
        return
    }
    AlertDialog(
        onDismissRequest = { onDone(null) },
        title = { Text("The key's passphrase") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                PasswordField(passphrase, { passphrase = it; error = null }, "Passphrase", "import_passphrase", error = error)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(save, { save = it })
                    Text("Save the passphrase in the vault")
                }
            }
        },
        confirmButton = { TextButton(enabled = passphrase.isNotEmpty() && !busy, onClick = ::run) { Text("Import") } },
        dismissButton = { TextButton(onClick = { onDone(null) }) { Text("Cancel") } },
    )
}

/** A key's screen (UI spec 7): label, type, fingerprint, public key (Copy, Share), its hosts, Delete. */
@Composable
internal fun KeyScreen(
    app: App,
    activity: MainActivity,
    key: JSONObject,
    onBack: () -> Unit,
    onDeleted: (JSONObject, String) -> Unit,
) {
    val ref = JSONObject().put("vault_id", key.optString("vault_id")).put("record_id", key.optString("record_id"))
    var label by remember { mutableStateOf(key.optString("label")) }
    var deleting by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    val public by produceState<String?>(null) {
        value = withContext(Dispatchers.IO) { runCatching { Core.call("key_public_key", ref) as String }.getOrNull() }
    }
    val users by produceState<JSONObject?>(null) {
        value = withContext(Dispatchers.IO) { runCatching { Core.obj("key_users", ref) }.getOrNull() }
    }
    fun names(kind: String) = users?.optJSONArray(kind)?.let { list -> (0 until list.length()).map { list.getJSONObject(it).optString("name") } } ?: emptyList()
    val hostNames = names("hosts")
    // The delete dialog names every user of the key: hosts, groups, identities.
    val allUsers = hostNames + names("groups").map { "group $it" } + names("identities").map { "identity $it" }
    Page(key.optString("label"), onBack = {
        // The label saves when the screen closes.
        val trimmed = label.trim()
        if (trimmed.isNotEmpty() && trimmed != key.optString("label")) {
            scope.launch {
                withContext(Dispatchers.IO) {
                    runCatching { Core.call("key_label_set", JSONObject().put("key", ref).put("label", trimmed)) }
                }
                onBack()
            }
        } else {
            onBack()
        }
    }) {
        Column(Modifier.verticalScroll(rememberScrollState()).imePadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(label, { label = it }, label = { Text("Label") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("key_screen_label"))
            Text("Type: " + algorithmName(key.optString("algorithm")))
            Text("SHA-256 fingerprint", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
            Text(key.optString("fingerprint"), modifier = Modifier.clickable {
                clipboard.setText(AnnotatedString(key.optString("fingerprint")))
            })
            Text("Public key", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
            Text(public ?: "…", style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("key_public"))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(enabled = public != null, onClick = { clipboard.setText(AnnotatedString(public ?: "")) }) { Text("Copy") }
                OutlinedButton(enabled = public != null, onClick = {
                    // The public key only: the private key is never shown or shared (UI spec 7).
                    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, public)
                    activity.startActivity(Intent.createChooser(send, "Share the public key"))
                }, modifier = Modifier.testTag("key_share")) { Text("Share") }
            }
            Text(
                if (hostNames.isEmpty()) "No host uses it." else "Used by: " + hostNames.joinToString(),
                color = Tokens.textSecondary,
            )
            TextButton(onClick = { deleting = true }, modifier = Modifier.testTag("key_delete")) { Text("Delete", color = Tokens.danger) }
        }
    }
    if (deleting) {
        AlertDialog(
            onDismissRequest = { deleting = false },
            title = { Text("Delete the key?") },
            text = {
                Text(
                    if (allUsers.isEmpty()) "“${key.optString("label")}” goes to Recently deleted."
                    else "“${key.optString("label")}” goes to Recently deleted, and these stop using it: ${allUsers.joinToString()}.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    deleting = false
                    val refs = JSONObject().put("vault_id", key.optString("vault_id")).put("record_ids", JSONArray().put(key.optString("record_id")))
                    onDeleted(refs, key.optString("label"))
                }) { Text("Delete", color = Tokens.danger) }
            },
            dismissButton = { TextButton(onClick = { deleting = false }) { Text("Cancel") } },
        )
    }
}
