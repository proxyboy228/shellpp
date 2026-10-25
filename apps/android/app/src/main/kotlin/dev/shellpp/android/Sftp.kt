package dev.shellpp.android

import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.text.DateFormat
import java.util.Date
import java.util.UUID

/** `/a/b` and `c` → `/a/b/c` (remote paths are POSIX). */
fun remoteJoin(dir: String, name: String): String = if (dir.endsWith("/")) dir + name else "$dir/$name"

/** The folder above `path` (`/` stays `/`). */
fun remoteParent(path: String): String {
    val trimmed = path.trimEnd('/')
    val at = trimmed.lastIndexOf('/')
    return if (at <= 0) "/" else trimmed.substring(0, at)
}

/** A size as people read it. */
fun humanSize(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val units = listOf("KB", "MB", "GB", "TB")
    var value = bytes / 1024.0
    var unit = 0
    while (value >= 1024 && unit < units.size - 1) {
        value /= 1024
        unit += 1
    }
    return String.format(java.util.Locale.ROOT, "%.1f %s", value, units[unit])
}

/** One request of the remote panel ([Native.sftpCall]); throws [CoreException] with the SFTP code. */
private fun sftp(session: Long, command: String, input: JSONObject = JSONObject()): Any? =
    Core.answer(Native.sftpCall(session, command, input.toString()))

/**
 * SFTP to a host (UI spec 10, the desktop's section 5 for one remote panel):
 * the prompts of an SSH session, then the remote home folder, a path bar,
 * the list, a file's actions, upload and download through the Storage Access
 * Framework, and the transfer queue as a bottom sheet. Back leaves the
 * session open (Sessions has it).
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class, ExperimentalComposeUiApi::class)
@Composable
fun SftpScreen(
    app: App,
    activity: android.content.Context,
    host: Record?,
    existing: Long?,
    onClose: () -> Unit,
    onOpened: (Long) -> Unit = {},
    onEnded: (Long) -> Unit = {},
) {
    val scope = rememberCoroutineScope()
    var session by remember { mutableStateOf(existing ?: 0L) }
    var view by remember { mutableStateOf<JSONObject?>(null) }
    var path by remember { mutableStateOf("") }
    var entries by remember { mutableStateOf(listOf<JSONObject>()) }
    var listError by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(false) }
    var fileSheet by remember { mutableStateOf<JSONObject?>(null) }
    var queueShown by remember { mutableStateOf(false) }
    var adding by remember { mutableStateOf(false) }
    var naming by remember { mutableStateOf<Pair<String, JSONObject?>?>(null) }
    var deleting by remember { mutableStateOf<JSONObject?>(null) }
    var answered by remember { mutableStateOf(-1L) }
    val transfers by app.transfers.collectAsState()
    val title = host?.title ?: app.sessionViews.value[session]?.optString("title") ?: "SFTP"
    val clipboard = LocalClipboardManager.current

    fun load(to: String) {
        if (session == 0L) return
        loading = true
        scope.launch {
            val listed = withContext(Dispatchers.IO) {
                try {
                    val list = sftp(session, "list", JSONObject().put("path", to)) as JSONArray
                    (0 until list.length()).map { list.getJSONObject(it) } to null
                } catch (error: CoreException) {
                    null to (error.message ?: error.code)
                } catch (error: Exception) {
                    null to "The folder could not be read."
                }
            }
            loading = false
            listed.first?.let {
                entries = it
                path = to
                listError = null
            } ?: run { listError = listed.second }
        }
    }

    // Opens the session once; its events show the steps and the prompts.
    LaunchedEffect(Unit) {
        if (session == 0L && host != null) {
            // Recorded even when the screen went meanwhile, so Sessions and
            // "Disconnect all" know the session (review of run 14).
            val opened = withContext(NonCancellable + Dispatchers.IO) {
                try {
                    Native.openSftp(host.vaultId, host.id).also { if (it != 0L) withContext(Dispatchers.Main) { app.opened(it, host.title, host.id, sftp = true) } }
                } catch (error: Exception) {
                    listError = error.message
                    0L
                }
            }
            session = opened
            if (opened != 0L) onOpened(opened)
        }
    }
    DisposableEffect(session) {
        val id = session
        if (id != 0L) app.listen(id) { event -> view = event }
        onDispose { if (id != 0L) app.unlisten(id) }
    }
    val state = view?.optString("state") ?: if (existing != null) "connected" else "connecting"
    LaunchedEffect(state, session) {
        if (state == "connected" && path.isEmpty() && session != 0L) {
            val home = withContext(Dispatchers.IO) {
                runCatching { sftp(session, "home") as String }.getOrDefault("/")
            }
            load(home)
        }
        if (state == "closed") onEnded(session)
    }

    // Download: "Save to…" every time (UI spec 10), the file goes through the cache.
    var pendingDownload by remember { mutableStateOf<JSONObject?>(null) }
    val saveTo = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("*/*")) { uri ->
        val entry = pendingDownload ?: return@rememberLauncherForActivityResult
        pendingDownload = null
        if (uri == null) return@rememberLauncherForActivityResult
        val cache = File(File(activity.cacheDir, "transfers").apply { mkdirs() }, UUID.randomUUID().toString())
        app.queueTransfer(remoteJoin(path, entry.optString("name")), TransferTarget(cache, uri, download = true, session = session))
        queueShown = true
    }
    val pickUploads = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        app.upload(session, path, uris) { problem -> listError = problem }
        queueShown = true
    }
    val conflicts by app.conflicts.collectAsState()

    // Back leaves the session open (Sessions has it); Close in the bar ends it.
    BackHandler(onBack = onClose)
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onClose, modifier = Modifier.testTag("sftp_back")) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (session != 0L) {
                        TextButton(onClick = {
                            val id = session
                            scope.launch(Dispatchers.IO) { runCatching { Native.close(id) } }
                            app.forget(id)
                            onClose()
                        }, modifier = Modifier.testTag("sftp_close")) { Text("Close") }
                    }
                    val running = transfers.values.count { it.optLong("session") == session && it.optString("state") in setOf("queued", "running", "failed") }
                    if (running > 0 || transfers.values.any { it.optLong("session") == session }) {
                        TextButton(onClick = { queueShown = true }, modifier = Modifier.testTag("sftp_queue")) {
                            Text(if (running > 0) "Transfers ($running)" else "Transfers")
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Tokens.bgBase),
            )
        },
        floatingActionButton = {
            if (state == "connected") {
                Box {
                    FloatingActionButton(onClick = { adding = true }, modifier = Modifier.testTag("sftp_add")) {
                        Icon(Icons.Filled.Add, contentDescription = "Upload or new folder")
                    }
                    DropdownMenu(expanded = adding, onDismissRequest = { adding = false }) {
                        DropdownMenuItem(text = { Text("Upload") }, onClick = { adding = false; pickUploads.launch(arrayOf("*/*")) })
                        DropdownMenuItem(text = { Text("New folder") }, onClick = { adding = false; naming = "mkdir" to null })
                    }
                }
            }
        },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize().semantics { testTagsAsResourceId = true }) {
            if (state != "connected") {
                Text(
                    when (state) {
                        "connecting" -> "Connecting…"
                        "failed", "dropped", "ended" -> view?.optString("error")?.takeIf { it.isNotBlank() && it != "null" } ?: "Not connected"
                        else -> state
                    },
                    Modifier.padding(16.dp).testTag("sftp_state"),
                )
                listError?.let { Text(it, color = Tokens.danger, modifier = Modifier.padding(horizontal = 16.dp)) }
                return@Column
            }
            // The path bar: a tap on a segment goes there.
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 4.dp)) {
                val parts = path.split('/').filter { it.isNotEmpty() }
                TextButton(onClick = { load("/") }) { Text("/") }
                parts.forEachIndexed { index, part ->
                    TextButton(onClick = { load("/" + parts.take(index + 1).joinToString("/")) }, modifier = Modifier.testTag("sftp_crumb_$part")) {
                        Text("$part /")
                    }
                }
            }
            listError?.let { Text(it, color = Tokens.danger, modifier = Modifier.padding(horizontal = 16.dp).testTag("sftp_error")) }
            PullToRefreshBox(isRefreshing = loading, onRefresh = { load(path) }, modifier = Modifier.fillMaxSize()) {
                LazyColumn(Modifier.fillMaxSize()) {
                    if (path != "/" && path.isNotEmpty()) {
                        item {
                            Text("..", Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable { load(remoteParent(path)) }.padding(16.dp).testTag("sftp_up"))
                        }
                    }
                    items(entries, key = { it.optString("name") }) { entry ->
                        val dir = entry.optBoolean("dir")
                        Column(
                            Modifier.fillMaxWidth().heightIn(min = 56.dp)
                                .clickable { if (dir) load(remoteJoin(path, entry.optString("name"))) else fileSheet = entry }
                                .padding(horizontal = 16.dp, vertical = 8.dp)
                                .testTag("sftp_entry_" + entry.optString("name")),
                        ) {
                            Text((if (dir) "▸ " else "") + entry.optString("name"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                            val modified = entry.optLong("modified", 0L).takeIf { it > 0 }?.let {
                                DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(it * 1000))
                            }
                            Text(
                                listOfNotNull(if (dir) null else humanSize(entry.optLong("size")), modified).joinToString(" · "),
                                color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                    }
                }
            }
        }
    }

    // The session's questions, as the terminal asks them.
    val prompt = view?.optJSONObject("prompt")
    if (prompt != null && prompt.optLong("id") != answered) {
        SessionPrompt(prompt, host?.address ?: title) { kind, text ->
            answered = prompt.optLong("id")
            scope.launch(Dispatchers.IO) {
                try {
                    Native.reply(session, prompt.optLong("id"), kind, text)
                    // A host key saved or replaced is a known host to sync.
                    if (kind == "accept" || kind == "replace") Core.onLocalChange?.invoke()
                } catch (error: Exception) {
                    // The next event says what happened.
                } finally {
                    text?.fill(0)
                }
            }
        }
    }

    fileSheet?.let { entry ->
        ModalBottomSheet(onDismissRequest = { fileSheet = null }, containerColor = Tokens.bgRaised) {
            Column(Modifier.padding(bottom = 24.dp)) {
                Text(entry.optString("name"), style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(16.dp))
                val actions = listOf(
                    "Download" to { pendingDownload = entry; saveTo.launch(entry.optString("name")) },
                    "Rename" to { naming = "rename" to entry },
                    "Delete" to { deleting = entry },
                    "Copy path" to { clipboard.setText(AnnotatedString(remoteJoin(path, entry.optString("name")))) },
                )
                for ((label, run) in actions) {
                    Text(
                        label,
                        color = if (label == "Delete") Tokens.danger else Tokens.text,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { fileSheet = null; run() }
                            .padding(horizontal = 16.dp, vertical = 14.dp),
                    )
                }
            }
        }
    }

    naming?.let { (what, entry) ->
        var name by remember(what, entry) { mutableStateOf(entry?.optString("name") ?: "") }
        AlertDialog(
            onDismissRequest = { naming = null },
            title = { Text(if (what == "mkdir") "New folder" else "Rename") },
            text = { OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") }) },
            confirmButton = {
                TextButton(enabled = name.isNotBlank() && !name.contains('/'), onClick = {
                    naming = null
                    scope.launch {
                        val failure = withContext(Dispatchers.IO) {
                            try {
                                if (what == "mkdir") {
                                    sftp(session, "mkdir", JSONObject().put("path", remoteJoin(path, name.trim())))
                                } else if (entry != null) {
                                    sftp(session, "rename", JSONObject().put("from", remoteJoin(path, entry.optString("name"))).put("to", remoteJoin(path, name.trim())))
                                }
                                null
                            } catch (error: CoreException) {
                                error.message
                            } catch (error: Exception) {
                                "Not done."
                            }
                        }
                        listError = failure
                        load(path)
                    }
                }) { Text("OK") }
            },
            dismissButton = { TextButton(onClick = { naming = null }) { Text("Cancel") } },
        )
    }

    deleting?.let { entry ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Delete “${entry.optString("name")}”?") },
            text = { Text("It is deleted on the server; there is no undo.") },
            confirmButton = {
                TextButton(onClick = {
                    deleting = null
                    scope.launch {
                        listError = withContext(Dispatchers.IO) {
                            try {
                                sftp(session, "remove", JSONObject().put("path", remoteJoin(path, entry.optString("name"))).put("dir", entry.optBoolean("dir")))
                                null
                            } catch (error: CoreException) {
                                error.message
                            } catch (error: Exception) {
                                "Not deleted."
                            }
                        }
                        load(path)
                    }
                }) { Text("Delete", color = Tokens.danger) }
            },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Cancel") } },
        )
    }

    conflicts.firstOrNull { it.session == session }?.let { conflict ->
        AlertDialog(
            onDismissRequest = {},
            title = { Text("“${conflict.name}” exists") },
            text = { Text("A file of this name is already in this folder on the server.") },
            confirmButton = {
                TextButton(onClick = {
                    app.conflicts.value = app.conflicts.value - conflict
                    app.queueTransfer(conflict.remote, TransferTarget(conflict.cache, null, download = false, session = session))
                }) { Text("Replace", color = Tokens.danger) }
            },
            dismissButton = {
                TextButton(onClick = {
                    app.conflicts.value = app.conflicts.value - conflict
                    conflict.cache.delete()
                }) { Text("Skip") }
            },
        )
    }

    if (queueShown) {
        ModalBottomSheet(onDismissRequest = { queueShown = false }, containerColor = Tokens.bgRaised) {
            Column(Modifier.padding(16.dp).semantics { testTagsAsResourceId = true }, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Transfers", style = MaterialTheme.typography.titleMedium)
                val mine = transfers.values.filter { it.optLong("session") == session }.sortedBy { it.optLong("id") }
                if (mine.isEmpty()) Text("No transfers", color = Tokens.textSecondary)
                for (transfer in mine) {
                    val id = transfer.optLong("id")
                    val transferState = transfer.optString("state")
                    Column(Modifier.fillMaxWidth().testTag("transfer_$transferState")) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                (if (transfer.optString("direction") == "upload") "↑ " else "↓ ") + transfer.optString("name"),
                                Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis,
                            )
                            when (transferState) {
                                "queued", "running" -> TextButton(onClick = {
                                    scope.launch(Dispatchers.IO) { runCatching { Native.transferCancel(id) } }
                                }) { Text("Cancel") }
                                "failed" -> TextButton(onClick = {
                                    // Retry: queued again from the start.
                                    app.retryTransfer(id, transfer.optString("path"))
                                }) { Text("Retry") }
                            }
                        }
                        when (transferState) {
                            "running" -> {
                                Text(humanSize(transfer.optLong("done")), color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                                LinearProgressIndicator(Modifier.fillMaxWidth())
                            }
                            "queued" -> Text("Waiting", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                            "done" -> Text("Done", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                            "cancelled" -> Text("Cancelled", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                            "failed" -> Text("Failed: " + transfer.optString("error"), color = Tokens.danger, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
            }
        }
    }
}

/** The display name of a picked document. */
fun displayName(context: android.content.Context, uri: Uri): String? =
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) cursor.getString(0) else null
    }?.replace('/', '_')

/**
 * A session's question (host key, password, passphrase, keyboard-interactive),
 * as the terminal asks it (UI spec 8): a first key offers Accept and save,
 * Connect once, Cancel; a changed key Cancel first and Replace only after a
 * second confirmation; text answers are masked unless the server echoes.
 */
@Composable
fun SessionPrompt(prompt: JSONObject, host: String, onReply: (String, ByteArray?) -> Unit) {
    when (val kind = prompt.optString("kind")) {
        "host_key_unknown" -> AlertDialog(
            onDismissRequest = {},
            title = { Text("First connection to this host") },
            text = { Text("$host\n${prompt.optString("keyType")}\n${prompt.optString("fingerprint")}") },
            confirmButton = { TextButton(onClick = { onReply("accept", null) }) { Text("Accept and save") } },
            dismissButton = {
                Row {
                    TextButton(onClick = { onReply("cancel", null) }) { Text("Cancel") }
                    TextButton(onClick = { onReply("once", null) }) { Text("Connect once") }
                }
            },
        )
        "host_key_changed" -> {
            var confirm by remember { mutableStateOf(false) }
            AlertDialog(
                onDismissRequest = {},
                title = { Text(if (confirm) "Replace the saved key?" else "The host key changed") },
                text = {
                    Text(
                        if (confirm) {
                            "Only if you know why the key of $host changed. Otherwise someone may be between you and the server."
                        } else {
                            "The key of $host is not the one saved for it.\n\nSaved key\n${prompt.optString("oldKeyType")}\n${prompt.optString("oldFingerprint")}" +
                                "\n\nKey the server shows now\n${prompt.optString("keyType")}\n${prompt.optString("fingerprint")}"
                        },
                        color = if (confirm) Tokens.text else Tokens.danger,
                    )
                },
                confirmButton = { TextButton(onClick = { onReply("cancel", null) }) { Text("Cancel") } },
                dismissButton = {
                    TextButton(onClick = { if (confirm) onReply("replace", null) else confirm = true }) {
                        Text(if (confirm) "Replace the key" else "Replace the key…", color = Tokens.danger)
                    }
                },
            )
        }
        "password", "passphrase", "keyboard_interactive", "username" -> {
            var text by remember(prompt.optLong("id")) { mutableStateOf("") }
            val label = when (kind) {
                "password" -> "Password for ${prompt.optString("username")}"
                "passphrase" -> "Passphrase of ${prompt.optString("keyLabel")}"
                "username" -> "Username"
                else -> prompt.optString("prompt")
            }
            val masked = kind == "password" || kind == "passphrase" || (kind == "keyboard_interactive" && !prompt.optBoolean("echo"))
            AlertDialog(
                onDismissRequest = {},
                title = { Text(label) },
                text = {
                    Column {
                        if (prompt.optBoolean("error")) Text("Wrong, try again.", color = Tokens.danger)
                        if (masked) {
                            PasswordField(text, { text = it }, label, "prompt_text", imeAction = androidx.compose.ui.text.input.ImeAction.Done)
                        } else {
                            OutlinedTextField(text, { text = it }, singleLine = true, modifier = Modifier.testTag("prompt_text"))
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = {
                        val chars = text.toCharArray()
                        text = ""
                        val encoded = Charsets.UTF_8.encode(java.nio.CharBuffer.wrap(chars))
                        val bytes = ByteArray(encoded.remaining()).also { encoded.get(it) }
                        chars.fill('\u0000')
                        if (encoded.hasArray()) encoded.array().fill(0)
                        onReply("text", bytes)
                    }) { Text("OK") }
                },
                dismissButton = { TextButton(onClick = { onReply("cancel", null) }) { Text("Cancel") } },
            )
        }
        else -> LaunchedEffect(prompt.optLong("id")) { onReply("cancel", null) }
    }
}
