package dev.shellpp.android

import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
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
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** A string member, or null when it is missing, JSON null or empty. */
fun JSONObject.text(key: String): String? = if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }

/** A host or a group of a records query. */
data class Record(val json: JSONObject) {
    val id: String get() = json.getString("record_id")
    val vaultId: String get() = json.getString("vault_id")
    val isGroup: Boolean get() = json.optString("type") == "group"
    val data: JSONObject get() = json.optJSONObject("data") ?: JSONObject()
    val inherited: JSONObject get() = json.optJSONObject("inherited") ?: JSONObject()

    val address: String get() = data.text("address") ?: ""
    val title: String
        get() = if (isGroup) data.text("name") ?: "" else data.text("label")?.takeIf { it.isNotBlank() } ?: address
    val color: String? get() = data.text("color")

    /** The username the host signs in with: its own, else the group's. */
    val username: String?
        get() = data.optJSONObject("ssh")?.text("username")?.takeIf { it.isNotBlank() }
            ?: inherited.optJSONObject("username")?.text("value")

    val hostCount: Int get() = json.optInt("host_count")
}

/** The design system's tag colors (`TagColor`), for color labels. */
fun tagColor(name: String?): Color? = when (name) {
    "graphite" -> Color(0xFF6B6B66)
    "jade" -> Color(0xFF13856E)
    "amber" -> Color(0xFFC98A12)
    "coral" -> Color(0xFFD5603F)
    "plum" -> Color(0xFF8E5BA8)
    "moss" -> Color(0xFF6E8B3D)
    "ocean" -> Color(0xFF2F78B7)
    "rose" -> Color(0xFFC94F7C)
    else -> null
}

fun records(input: JSONObject): List<Record> {
    val answer = Core.array("records_query", input)
    return (0 until answer.length()).map { Record(answer.getJSONObject(it)) }
}

fun scopeOf(groupId: String?): JSONObject =
    if (groupId == null) JSONObject().put("kind", "top_level") else JSONObject().put("kind", "group").put("group_id", groupId)

/** `user@host[:port]` typed in the search: a quick connect (UI spec 6.1). */
data class QuickTarget(val user: String, val address: String, val port: Int?)

fun quickTarget(text: String): QuickTarget? {
    val match = Regex("^([^@\\s]+)@([^:@\\s]+)(?::(\\d{1,5}))?$").find(text.trim()) ?: return null
    val port = match.groupValues[3].toIntOrNull()
    if (port != null && port !in 1..65535) return null
    return QuickTarget(match.groupValues[1], match.groupValues[2], port)
}

/** The four places of the bottom bar (UI spec 3) and the screens over them. */
@Composable
fun MainScreen(app: App, activity: MainActivity) {
    var tab by remember { mutableStateOf(0) }
    var overlay by remember { mutableStateOf<Overlay?>(null) }
    // Kept here: an editor over the list must not lose the group it was opened in.
    var path by remember { mutableStateOf(listOf<Record>()) }
    val current = overlay
    if (current != null) {
        BackHandler { overlay = null }
        when (current) {
            is Overlay.EditHost -> HostEditor(app, current.record, current.groupId, onClose = { overlay = null })
            is Overlay.EditGroup -> GroupEditor(app, current.record, current.parentId, onClose = { overlay = null })
            Overlay.Settings -> SettingsScreen(app, activity, onClose = { overlay = null })
            Overlay.RecentlyDeleted -> RecentlyDeletedScreen(app, onClose = { overlay = null })
            Overlay.Account -> AccountScreen(app, activity, onClose = { overlay = null })
        }
        return
    }
    Scaffold(
        bottomBar = {
            NavigationBar(containerColor = Tokens.bgBase) {
                val places = listOf("Hosts" to Icons.Filled.Home, "Sessions" to Icons.Filled.PlayArrow, "Keychain" to Icons.Filled.Lock, "More" to Icons.Filled.Menu)
                places.forEachIndexed { index, (label, icon) ->
                    NavigationBarItem(
                        selected = tab == index,
                        onClick = { tab = index },
                        icon = { Icon(icon, contentDescription = null) },
                        label = { Text(label) },
                        modifier = Modifier.testTag("tab_" + label.lowercase()),
                    )
                }
            }
        },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Box(Modifier.padding(padding)) {
            when (tab) {
                0 -> HostsScreen(app, activity, path, onPath = { path = it }, onEdit = { overlay = it })
                1 -> SessionsScreen(app, activity)
                2 -> KeychainScreen()
                else -> MoreScreen(app, onOpen = { overlay = it })
            }
        }
    }
}

sealed interface Overlay {
    data class EditHost(val record: Record?, val groupId: String?) : Overlay
    data class EditGroup(val record: Record?, val parentId: String?) : Overlay
    data object Settings : Overlay
    data object RecentlyDeleted : Overlay
    data object Account : Overlay
}

/** Opens a terminal on a saved host (UI spec 6.1: tap a host to connect). */
fun connect(activity: MainActivity, host: Record) {
    activity.startActivity(
        Intent(activity, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_VAULT_ID, host.vaultId)
            .putExtra(TerminalActivity.EXTRA_RECORD_ID, host.id)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, host.address)
            .putExtra(TerminalActivity.EXTRA_TITLE, host.title),
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun HostsScreen(
    app: App,
    activity: MainActivity,
    path: List<Record>,
    onPath: (List<Record>) -> Unit,
    onEdit: (Overlay) -> Unit,
) {
    val changes by app.changes.collectAsState()
    val sessionViews by app.sessionViews.collectAsState()
    var search by remember { mutableStateOf("") }
    var sheet by remember { mutableStateOf<Record?>(null) }
    var deleting by remember { mutableStateOf<Record?>(null) }
    var moving by remember { mutableStateOf<Record?>(null) }
    var adding by remember { mutableStateOf(false) }
    var refresh by remember { mutableStateOf(0) }
    var syncing by remember { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val group = path.lastOrNull()

    if (path.isNotEmpty()) BackHandler { onPath(path.dropLast(1)) }

    val vaultName by produceState("Hosts", changes) {
        value = withContext(Dispatchers.IO) {
            try {
                Core.array("vaults").optJSONObject(0)?.optString("name")?.ifBlank { null } ?: "Hosts"
            } catch (error: CoreException) {
                "Hosts"
            }
        }
    }
    val listed by produceState(Pair(listOf<Record>(), listOf<Record>()), changes, group?.id, search, refresh) {
        value = withContext(Dispatchers.IO) {
            try {
                if (search.isNotBlank()) {
                    Pair(emptyList(), records(JSONObject().put("types", JSONArray().put("host")).put("text", search.trim())))
                } else {
                    val scopeJson = scopeOf(group?.id)
                    Pair(
                        records(JSONObject().put("types", JSONArray().put("group")).put("scope", scopeJson)),
                        records(JSONObject().put("types", JSONArray().put("host")).put("scope", scopeJson)),
                    )
                }
            } catch (error: CoreException) {
                Pair(emptyList(), emptyList())
            }
        }
    }
    val (groups, hosts) = listed
    val liveHosts = sessionViews.values.mapNotNull { it.optString("recordId").ifBlank { null } }.toSet()

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        if (path.isEmpty()) vaultName else path.joinToString(" › ") { it.title },
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    if (path.isNotEmpty()) {
                        IconButton(onClick = { onPath(path.dropLast(1)) }, modifier = Modifier.testTag("up")) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Tokens.bgBase),
            )
        },
        floatingActionButton = {
            Box {
                FloatingActionButton(onClick = { adding = true }, modifier = Modifier.testTag("add")) {
                    Icon(Icons.Filled.Add, contentDescription = "New")
                }
                DropdownMenu(expanded = adding, onDismissRequest = { adding = false }) {
                    DropdownMenuItem(
                        text = { Text("New host") },
                        onClick = { adding = false; onEdit(Overlay.EditHost(null, group?.id)) },
                        modifier = Modifier.testTag("new_host"),
                    )
                    DropdownMenuItem(
                        text = { Text("New group") },
                        onClick = { adding = false; onEdit(Overlay.EditGroup(null, group?.id)) },
                        modifier = Modifier.testTag("new_group"),
                    )
                }
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            OutlinedTextField(
                value = search,
                onValueChange = { search = it },
                placeholder = { Text("Search or user@host") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("search"),
            )
            val quick = if (search.isNotBlank() && hosts.isEmpty()) quickTarget(search) else null
            if (groups.isEmpty() && hosts.isEmpty() && quick == null) {
                Column(
                    Modifier.fillMaxSize().padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(if (search.isBlank()) "No hosts yet" else "Nothing found", style = MaterialTheme.typography.titleMedium)
                    if (search.isBlank()) {
                        TextButton(onClick = { onEdit(Overlay.EditHost(null, group?.id)) }) { Text("New host") }
                    }
                }
                return@Column
            }
            // Pull to refresh runs a sync when an account is connected (UI spec 6.1).
            PullToRefreshBox(
                isRefreshing = syncing,
                onRefresh = {
                    syncing = true
                    scope.launch {
                        val failure = withContext(Dispatchers.IO) {
                            try {
                                val status = Core.obj("sync_status")
                                if (status.optBoolean("bound")) Core.call("sync_now")
                                null
                            } catch (error: CoreException) {
                                syncErrorText(error)
                            }
                        }
                        syncing = false
                        refresh += 1
                        failure?.let { snackbar.showSnackbar(it) }
                    }
                },
                modifier = Modifier.fillMaxSize(),
            ) {
                LazyColumn(Modifier.fillMaxSize()) {
                    if (quick != null) {
                        item {
                            Row(
                                Modifier.fillMaxWidth().heightIn(min = 56.dp)
                                    .combinedClickable(onClick = {
                                        activity.startActivity(
                                            Intent(activity, TerminalActivity::class.java)
                                                .putExtra(TerminalActivity.EXTRA_ADDRESS, quick.address)
                                                .putExtra(TerminalActivity.EXTRA_PORT, quick.port ?: 22)
                                                .putExtra(TerminalActivity.EXTRA_USERNAME, quick.user),
                                        )
                                    })
                                    .padding(horizontal = 16.dp)
                                    .testTag("quick_connect"),
                                verticalAlignment = Alignment.CenterVertically,
                            ) { Text("Connect to ${quick.user}@${quick.address}", color = Tokens.accent) }
                        }
                    }
                    items(groups, key = { it.id }) { row ->
                        RecordRow(
                            row, live = false,
                            onClick = { search = ""; onPath(path + row) },
                            onLongClick = { sheet = row },
                        )
                    }
                    items(hosts, key = { it.id }) { row ->
                        RecordRow(
                            row, live = row.id in liveHosts,
                            onClick = { connect(activity, row) },
                            onLongClick = { sheet = row },
                        )
                    }
                }
            }
        }
    }

    sheet?.let { record ->
        ModalBottomSheet(onDismissRequest = { sheet = null }, containerColor = Tokens.bgRaised) {
            Column(Modifier.padding(bottom = 24.dp)) {
                Text(record.title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(16.dp))
                val actions = buildList {
                    if (!record.isGroup) add(Triple("Connect", "sheet_connect") { connect(activity, record) })
                    add(Triple("Edit", "sheet_edit") {
                        onEdit(if (record.isGroup) Overlay.EditGroup(record, null) else Overlay.EditHost(record, null))
                    })
                    if (!record.isGroup) {
                        add(Triple("Duplicate", "sheet_duplicate") {
                            scope.launch {
                                withContext(Dispatchers.IO) {
                                    try {
                                        Core.call("record_duplicate", JSONObject().put("vault_id", record.vaultId).put("record_id", record.id))
                                    } catch (error: CoreException) {
                                        // The list stays as it is.
                                    }
                                }
                                refresh += 1
                            }
                        })
                    }
                    add(Triple(if (record.isGroup) "Move" else "Move to group", "sheet_move") { moving = record })
                    add(Triple("Delete", "sheet_delete") { deleting = record })
                }
                for ((label, tag, run) in actions) {
                    Text(
                        label,
                        color = if (tag == "sheet_delete") Tokens.danger else Tokens.text,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                            .combinedClickable(onClick = { sheet = null; run() })
                            .padding(horizontal = 16.dp, vertical = 14.dp)
                            .testTag(tag),
                    )
                }
            }
        }
    }

    deleting?.let { record ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Delete ${if (record.isGroup) "group" else "host"}?") },
            text = {
                Text(
                    if (record.isGroup) {
                        "“${record.title}” goes to Recently deleted; its hosts and groups move one level up."
                    } else {
                        "“${record.title}” goes to Recently deleted."
                    },
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        deleting = null
                        scope.launch {
                            val ref = JSONObject().put("vault_id", record.vaultId).put("record_ids", JSONArray().put(record.id))
                            val deleted = withContext(Dispatchers.IO) {
                                try {
                                    Core.obj("record_delete", ref)
                                } catch (error: Exception) {
                                    null
                                }
                            }
                            refresh += 1
                            if (deleted == null) return@launch
                            val answer = snackbar.showSnackbar("Deleted “${record.title}”", "Undo", duration = SnackbarDuration.Long)
                            if (answer == SnackbarResult.ActionPerformed) {
                                val undone = withContext(Dispatchers.IO) {
                                    try {
                                        Core.call("record_restore", ref)
                                        // A group's hosts and groups moved one level up
                                        // when it went: they go back into it, as on the
                                        // desktop (review of run 14).
                                        val moved = deleted.optJSONArray("moved")
                                        if (record.isGroup && moved != null && moved.length() > 0) {
                                            Core.call(
                                                "record_move",
                                                JSONObject().put("vault_id", record.vaultId).put("record_ids", moved)
                                                    .put("target_group_id", record.id),
                                            )
                                        }
                                        null
                                    } catch (error: CoreException) {
                                        "Not undone: ${error.message}. Recently deleted has it."
                                    } catch (error: Exception) {
                                        "Not undone. Recently deleted has it."
                                    }
                                }
                                refresh += 1
                                undone?.let { snackbar.showSnackbar(it) }
                            }
                        }
                    },
                    modifier = Modifier.testTag("confirm_delete"),
                ) { Text("Delete", color = Tokens.danger) }
            },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Cancel") } },
        )
    }

    moving?.let { record ->
        GroupPicker(
            title = "Move “${record.title}” to",
            exclude = if (record.isGroup) record.id else null,
            onDismiss = { moving = null },
            onPick = { target ->
                moving = null
                scope.launch {
                    val failure = withContext(Dispatchers.IO) {
                        try {
                            val input = JSONObject().put("vault_id", record.vaultId).put("record_ids", JSONArray().put(record.id))
                            if (target != null) input.put("target_group_id", target)
                            Core.call("record_move", input)
                            null
                        } catch (error: Exception) {
                            // A move under itself is refused by the core.
                            "Not moved: ${error.message}"
                        }
                    }
                    refresh += 1
                    failure?.let { snackbar.showSnackbar(it) }
                }
            },
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun RecordRow(record: Record, live: Boolean, onClick: () -> Unit, onLongClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = if (record.isGroup) 56.dp else 72.dp)
            .combinedClickable(onClick = onClick, onLongClick = onLongClick)
            .testTag((if (record.isGroup) "group_" else "host_") + record.title),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.width(4.dp).height(if (record.isGroup) 56.dp else 72.dp)
                .background(tagColor(record.color) ?: Color.Transparent),
        )
        Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
            Text(record.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val second = if (record.isGroup) {
                if (record.hostCount == 1) "1 host" else "${record.hostCount} hosts"
            } else {
                listOfNotNull(record.username, record.address).joinToString("@")
            }
            Text(second, color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (live) {
            Box(Modifier.padding(end = 16.dp).size(8.dp).clip(CircleShape).background(Tokens.accent))
        }
    }
    HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
}

/** Picks a group (or the top level, null) from every group of the vault. */
@Composable
fun GroupPicker(title: String, exclude: String?, onDismiss: () -> Unit, onPick: (String?) -> Unit) {
    val groups by produceState(listOf<Record>()) {
        value = withContext(Dispatchers.IO) {
            try {
                records(JSONObject().put("types", JSONArray().put("group")).put("sort", "name"))
            } catch (error: CoreException) {
                emptyList()
            }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            LazyColumn {
                item {
                    Text(
                        "Top level",
                        Modifier.fillMaxWidth().heightIn(min = 48.dp).combinedClickableCompat { onPick(null) }
                            .padding(vertical = 12.dp).testTag("pick_top"),
                    )
                }
                items(groups.filter { it.id != exclude }, key = { it.id }) { group ->
                    Text(
                        group.title,
                        Modifier.fillMaxWidth().heightIn(min = 48.dp).combinedClickableCompat { onPick(group.id) }
                            .padding(vertical = 12.dp).testTag("pick_" + group.title),
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@OptIn(ExperimentalFoundationApi::class)
fun Modifier.combinedClickableCompat(onClick: () -> Unit): Modifier = this.combinedClickable(onClick = onClick)

@Composable
fun PlaceholderScreen(title: String, text: String) {
    Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Spacer(Modifier.height(16.dp))
        Text(title, style = MaterialTheme.typography.titleLarge)
        Text(text, color = Tokens.textSecondary)
    }
}
