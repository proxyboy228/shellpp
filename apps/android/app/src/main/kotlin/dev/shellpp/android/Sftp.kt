package dev.shellpp.android

import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.BackHandler
import android.provider.DocumentsContract
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.OutlinedButton
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import java.text.SimpleDateFormat
import java.util.Locale
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
    var deleting by remember { mutableStateOf<List<JSONObject>?>(null) }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var searching by remember { mutableStateOf(false) }
    var query by remember { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    var sortBy by remember { mutableStateOf("name") }
    var ascending by remember { mutableStateOf(true) }
    var showHidden by remember { mutableStateOf(false) }
    val prefs = remember { activity.getSharedPreferences("sftp", android.content.Context.MODE_PRIVATE) }
    var downloadFolder by remember { mutableStateOf(prefs.getString(PREF_DOWNLOAD_TREE, null)?.let(Uri::parse)) }
    val downloadFolderName = downloadFolder?.let { tree ->
        runCatching { DocumentsContract.getTreeDocumentId(tree).substringAfter(':').ifBlank { null } }.getOrNull()
    }
    LaunchedEffect(Unit) {
        showHidden = withContext(Dispatchers.IO) { coreSettings().optBoolean("sftp.show_hidden", false) }
    }
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
    // The download folder on the phone (⋮): files go there without asking.
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree == null) return@rememberLauncherForActivityResult
        runCatching {
            activity.contentResolver.takePersistableUriPermission(
                tree, android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION or android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
        }
        prefs.edit().putString(PREF_DOWNLOAD_TREE, tree.toString()).apply()
        downloadFolder = tree
    }

    fun download(entry: JSONObject) {
        val tree = downloadFolder
        if (tree == null) {
            pendingDownload = entry
            saveTo.launch(entry.optString("name"))
            return
        }
        scope.launch {
            val target = withContext(Dispatchers.IO) {
                runCatching {
                    val folder = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
                    DocumentsContract.createDocument(activity.contentResolver, folder, "application/octet-stream", entry.optString("name"))
                }.getOrNull()
            }
            if (target == null) {
                listError = "The download folder cannot be written; choose it again in ⋮."
                return@launch
            }
            val cache = File(File(activity.cacheDir, "transfers").apply { mkdirs() }, UUID.randomUUID().toString())
            app.queueTransfer(remoteJoin(path, entry.optString("name")), TransferTarget(cache, target, download = true, session = session))
            queueShown = true
        }
    }

    val pickUploads = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        app.upload(session, path, uris) { problem -> listError = problem }
        queueShown = true
    }
    val conflicts by app.conflicts.collectAsState()

    // Back leaves the session open (the session bar has it); × on its chip ends it.
    BackHandler(enabled = selected.isEmpty() && !searching, onBack = onClose)
    BackHandler(enabled = selected.isNotEmpty()) { selected = emptySet() }
    BackHandler(enabled = searching && selected.isEmpty()) { searching = false; query = "" }
    val shown = sortEntries(entries.filter { showHidden || !it.optString("name").startsWith(".") }, sortBy, ascending)
        .filter { query.isBlank() || it.optString("name").contains(query.trim(), ignoreCase = true) }
    Column(Modifier.fillMaxSize().background(Tokens.bgBase).semantics { testTagsAsResourceId = true }) {
        when {
            selected.isNotEmpty() -> SelectionBar(selected.size, canEdit = false, onClose = { selected = emptySet() }, onEdit = null) { close ->
                val chosen = entries.filter { it.optString("name") in selected }
                val one = chosen.singleOrNull()
                if (one != null && !one.optBoolean("dir")) {
                    DropdownMenuItem(text = { Text("Download") }, modifier = Modifier.testTag("sftp_download"), onClick = {
                        close(); selected = emptySet(); download(one)
                    })
                }
                if (one != null) {
                    DropdownMenuItem(text = { Text("Rename") }, modifier = Modifier.testTag("sftp_rename"), onClick = {
                        close(); selected = emptySet(); naming = "rename" to one
                    })
                    DropdownMenuItem(text = { Text("Copy path") }, modifier = Modifier.testTag("sftp_copy_path"), onClick = {
                        close(); selected = emptySet(); clipboard.setText(AnnotatedString(remoteJoin(path, one.optString("name"))))
                    })
                }
                HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                DropdownMenuItem(text = { Text("Delete", color = Tokens.danger) }, modifier = Modifier.testTag("sftp_delete"), onClick = {
                    close(); deleting = chosen
                })
            }
            searching -> SearchBar(query, { query = it }, "Search this folder", onClose = { searching = false; query = "" })
            else -> Row(Modifier.fillMaxWidth().height(64.dp).padding(start = 12.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                // The path as a breadcrumb: parents muted, the folder shown emphasised.
                val parts = path.split('/').filter { it.isNotEmpty() }
                Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
                    Text("/", color = if (parts.isEmpty()) Tokens.text else Tokens.textSecondary, modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable { load("/") }.padding(horizontal = 6.dp, vertical = 12.dp))
                    parts.forEachIndexed { index, part ->
                        Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null, tint = Tokens.textMuted, modifier = Modifier.size(18.dp))
                        val last = index == parts.lastIndex
                        Text(
                            part,
                            color = if (last) Tokens.text else Tokens.textSecondary,
                            fontWeight = if (last) FontWeight.Bold else FontWeight.Normal,
                            maxLines = 1,
                            modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable { load("/" + parts.take(index + 1).joinToString("/")) }
                                .padding(horizontal = 6.dp, vertical = 12.dp).testTag("sftp_crumb_$part"),
                        )
                    }
                }
                if (state == "connected") {
                    OutlinedButton(
                        onClick = { pickUploads.launch(arrayOf("*/*")) },
                        contentPadding = PaddingValues(horizontal = 12.dp),
                        modifier = Modifier.heightIn(min = 40.dp).testTag("sftp_add"),
                    ) {
                        Icon(Icons.Filled.Upload, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Upload")
                    }
                    Box {
                        BarIcon(Icons.Filled.MoreVert, "More", "sftp_more") { menu = true }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = Tokens.bgRaised) {
                            DropdownMenuItem(text = { Text("New folder") }, leadingIcon = { Icon(Icons.Filled.CreateNewFolder, null) }, modifier = Modifier.testTag("sftp_mkdir"), onClick = {
                                menu = false; naming = "mkdir" to null
                            })
                            HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                            for ((key, label) in listOf("name" to "Sort by name", "date" to "Sort by date", "size" to "Sort by size", "kind" to "Sort by kind")) {
                                DropdownMenuItem(
                                    text = { Text(label) },
                                    trailingIcon = { if (sortBy == key) Text(if (ascending) "↑" else "↓", color = Tokens.accent) },
                                    modifier = Modifier.testTag("sftp_sort_$key"),
                                    onClick = {
                                        menu = false
                                        if (sortBy == key) ascending = !ascending else { sortBy = key; ascending = true }
                                    },
                                )
                            }
                            HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                            DropdownMenuItem(
                                text = { Text(if (downloadFolder == null) "Download folder: ask each time" else "Download folder: ${downloadFolderName ?: "chosen"}") },
                                leadingIcon = { Icon(Icons.Filled.Download, null) },
                                modifier = Modifier.testTag("sftp_download_folder"),
                                onClick = { menu = false; pickFolder.launch(null) },
                            )
                            if (downloadFolder != null) {
                                DropdownMenuItem(text = { Text("Ask where to save each time") }, onClick = {
                                    menu = false
                                    prefs.edit().remove(PREF_DOWNLOAD_TREE).apply()
                                    downloadFolder = null
                                })
                            }
                            DropdownMenuItem(text = { Text("Copy path") }, leadingIcon = { Icon(Icons.Filled.ContentCopy, null) }, modifier = Modifier.testTag("sftp_copy_folder"), onClick = {
                                menu = false; clipboard.setText(AnnotatedString(path))
                            })
                            DropdownMenuItem(
                                text = { Text(if (showHidden) "Hide hidden files" else "Show hidden files") },
                                leadingIcon = { Icon(if (showHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility, null) },
                                modifier = Modifier.testTag("sftp_hidden"),
                                onClick = {
                                    menu = false
                                    showHidden = !showHidden
                                    scope.launch(Dispatchers.IO) { runCatching { setCoreSetting("sftp.show_hidden", showHidden) } }
                                },
                            )
                            val running = transfers.values.count { it.optLong("session") == session && it.optString("state") in setOf("queued", "running", "failed") }
                            if (transfers.values.any { it.optLong("session") == session }) {
                                HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                                DropdownMenuItem(text = { Text(if (running > 0) "Transfers ($running)" else "Transfers") }, modifier = Modifier.testTag("sftp_queue"), onClick = {
                                    menu = false; queueShown = true
                                })
                            }
                        }
                    }
                    BarIcon(Icons.Filled.Search, "Search", "sftp_search") { searching = true }
                }
            }
        }
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
        listError?.let { Text(it, color = Tokens.danger, modifier = Modifier.padding(horizontal = 16.dp).testTag("sftp_error")) }
        val running = transfers.values.count { it.optLong("session") == session && it.optString("state") in setOf("queued", "running") }
        if (running > 0) {
            Text(
                "$running transfer" + (if (running == 1) "" else "s") + " running",
                color = Tokens.accent, style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp).clickable { queueShown = true }.testTag("sftp_running"),
            )
        }
        // The listing on one large panel (reference-2 8.1).
        PullToRefreshBox(
            isRefreshing = loading, onRefresh = { load(path) },
            modifier = Modifier.fillMaxSize().padding(horizontal = 8.dp).clip(RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp)).background(Tokens.bgRaised),
        ) {
            LazyColumn(Modifier.fillMaxSize().padding(top = 8.dp)) {
                if (path != "/" && path.isNotEmpty() && !searching) {
                    item {
                        FileRow(
                            name = "..", icon = Icons.Filled.Folder, permissions = null, size = null, modified = null,
                            selected = false, tag = "sftp_up", onClick = { load(remoteParent(path)) }, onLongClick = null,
                        )
                    }
                }
                items(shown, key = { it.optString("name") }) { entry ->
                    val name = entry.optString("name")
                    val dir = entry.optBoolean("dir")
                    val modified = entry.optLong("modified", 0L).takeIf { it > 0 }?.let {
                        SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.ROOT).format(Date(it * 1000))
                    }
                    FileRow(
                        name = name,
                        icon = when {
                            name in selected -> Icons.Filled.CheckCircle
                            entry.optString("kind") == "link" -> Icons.Filled.Link
                            dir -> Icons.Filled.Folder
                            else -> Icons.AutoMirrored.Filled.InsertDriveFile
                        },
                        permissions = permissionString(entry),
                        size = if (dir) null else humanSize(entry.optLong("size")),
                        modified = modified,
                        selected = name in selected,
                        tag = "sftp_entry_$name",
                        onClick = {
                            when {
                                selected.isNotEmpty() -> selected = if (name in selected) selected - name else selected + name
                                dir -> load(remoteJoin(path, name))
                                else -> fileSheet = entry
                            }
                        },
                        onLongClick = { selected = if (name in selected) selected - name else selected + name },
                    )
                }
                item { Spacer(Modifier.height(32.dp)) }
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
                    "Download" to { download(entry) },
                    "Rename" to { naming = "rename" to entry },
                    "Delete" to { deleting = listOf(entry) },
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

    deleting?.let { chosen ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text(if (chosen.size == 1) "Delete “${chosen[0].optString("name")}”?" else "Delete ${chosen.size} items?") },
            text = { Text("They are deleted on the server; there is no undo.") },
            confirmButton = {
                TextButton(onClick = {
                    deleting = null
                    selected = emptySet()
                    scope.launch {
                        listError = withContext(Dispatchers.IO) {
                            var failure: String? = null
                            for (entry in chosen) {
                                try {
                                    sftp(session, "remove", JSONObject().put("path", remoteJoin(path, entry.optString("name"))).put("dir", entry.optBoolean("dir")))
                                } catch (error: CoreException) {
                                    failure = error.message
                                } catch (error: Exception) {
                                    failure = "Not deleted."
                                }
                            }
                            failure
                        }
                        load(path)
                    }
                }, modifier = Modifier.testTag("sftp_delete_confirm")) { Text("Delete", color = Tokens.danger) }
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

/** Where downloads go without asking (a document tree the user picked), or none. */
private const val PREF_DOWNLOAD_TREE = "download_tree"

/** An entry's permissions as `ls` shows them (`drwxr-xr-x`), or null when the server sent none. */
fun permissionString(entry: JSONObject): String? {
    if (entry.isNull("permissions") || !entry.has("permissions")) return null
    val bits = entry.optInt("permissions")
    val type = when (entry.optString("kind")) {
        "dir" -> 'd'
        "link" -> 'l'
        else -> '-'
    }
    val chars = "rwxrwxrwx"
    return type + (0 until 9).map { index -> if (bits and (1 shl (8 - index)) != 0) chars[index] else '-' }.joinToString("")
}

/** The listing in the order of ⋮: name, date, size or kind; folders first by name. */
fun sortEntries(entries: List<JSONObject>, by: String, ascending: Boolean): List<JSONObject> {
    val comparator: Comparator<JSONObject> = when (by) {
        "date" -> compareBy { it.optLong("modified") }
        "size" -> compareBy { it.optLong("size") }
        "kind" -> compareBy<JSONObject> { it.optString("name").substringAfterLast('.', "").lowercase() }.thenBy { it.optString("name").lowercase() }
        else -> compareBy { it.optString("name").lowercase() }
    }
    val ordered = entries.sortedWith(comparator).let { if (ascending) it else it.reversed() }
    return ordered.sortedByDescending { it.optBoolean("dir") }
}

/** A row of the listing: icon, name, permissions in monospace; size and date on the right. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileRow(
    name: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    permissions: String?,
    size: String?,
    modified: String?,
    selected: Boolean,
    tag: String,
    onClick: () -> Unit,
    onLongClick: (() -> Unit)?,
) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 60.dp)
            .background(if (selected) Tokens.accentSubtle else androidx.compose.ui.graphics.Color.Transparent)
            .combinedClickable(onClick = onClick, onLongClick = onLongClick)
            .padding(horizontal = 12.dp, vertical = 6.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = Tokens.accent, modifier = Modifier.size(26.dp))
        Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
            Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (permissions != null) Text(permissions, color = Tokens.textSecondary, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        }
        Column(horizontalAlignment = Alignment.End) {
            if (size != null) Text(size, color = Tokens.textSecondary, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
            if (modified != null) Text(modified, color = Tokens.textSecondary, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        }
    }
}
