package dev.shellpp.android

import android.content.Context
import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.CalendarMonth
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.DeleteOutline
import androidx.compose.material.icons.filled.Dns
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.VerifiedUser
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** The vault's name for the headers ("Personal" when it has none). */
fun vaultName(): String = try {
    Core.array("vaults").optJSONObject(0)?.optString("name")?.ifBlank { null } ?: "Personal"
} catch (error: Exception) {
    "Personal"
}

/** What the vault home counts (reference-1 2.1). */
private data class Counts(val hosts: Int, val knownHosts: Int, val deleted: Int)

/**
 * The vault home (reference-1 2.1): the vault's name with the switcher,
 * then one grouped list of its sections with their counts. Sections not
 * built on the phone (port forwarding, snippets, logs) are not shown;
 * Recently deleted is Shell++'s own (data-model 16.6).
 */
@Composable
fun VaultHome(app: App, nav: Nav) {
    val changes by app.changes.collectAsState()
    var switcher by remember { mutableStateOf(false) }
    val name by produceState("Personal", changes) { value = withContext(Dispatchers.IO) { vaultName() } }
    val counts by produceState(Counts(0, 0, 0), changes) {
        value = withContext(Dispatchers.IO) {
            Counts(
                hosts = runCatching { records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all"))).size }.getOrDefault(0),
                knownHosts = runCatching { Core.array("known_hosts_list").length() }.getOrDefault(0),
                deleted = runCatching { Core.array("recently_deleted_list").length() }.getOrDefault(0),
            )
        }
    }
    Page(top = {
        TopBar("", titleContent = { VaultTitle(name) { switcher = true } })
    }) {
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                Spacer(Modifier.height(8.dp))
                GroupedList {
                    GroupedRow("Hosts", "vault_hosts", Icons.Filled.Dns, value = counts.hosts.toString()) { nav.push(Page.Hosts(emptyList())) }
                    GroupedRow("Keychain", "vault_keychain", Icons.Filled.Key) { nav.push(Page.Keychain) }
                    GroupedRow("Known hosts", "vault_known_hosts", Icons.Filled.VerifiedUser, value = counts.knownHosts.toString()) { nav.push(Page.KnownHosts) }
                    GroupedRow("Recently deleted", "vault_recently_deleted", Icons.Filled.DeleteOutline, value = counts.deleted.toString(), divider = false) {
                        nav.push(Page.RecentlyDeleted)
                    }
                }
            }
        }
    }
    if (switcher) VaultSwitcher(name, onDismiss = { switcher = false })
}

/** The vault's icon, name and a drop-down arrow: opens the switcher. */
@Composable
private fun VaultTitle(name: String, onClick: () -> Unit) {
    Row(
        Modifier.clip(RoundedCornerShape(20.dp)).clickable(onClick = onClick).padding(horizontal = 12.dp, vertical = 8.dp).testTag("vault_switcher"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Filled.Storage, contentDescription = null, tint = Tokens.accent, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(8.dp))
        Text(name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Icon(Icons.Filled.ArrowDropDown, contentDescription = "Switch vault")
    }
}

/** The vault switcher (reference-2 9.1): one personal vault in 1.0, with its check mark. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun VaultSwitcher(name: String, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = Tokens.bgRaised) {
        Column(Modifier.padding(bottom = 32.dp)) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(onClick = onDismiss).padding(horizontal = 24.dp).testTag("vault_personal"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Filled.Storage, contentDescription = null, tint = Tokens.accent)
                Spacer(Modifier.width(16.dp))
                Text(name, Modifier.weight(1f))
                Icon(Icons.Filled.Check, contentDescription = "Current vault", tint = Tokens.accent)
            }
            Text(
                "Team vaults come in a later version.",
                color = Tokens.textSecondary,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
        }
    }
}

/** The four orders of the sort menu (reference-1 2.2), kept on this device. */
enum class SortOrder(val badge: String, val label: String) {
    AZ("Az", "A–Z"),
    ZA("Za", "Z–A"),
    NEWEST("", "Newest to oldest"),
    OLDEST("", "Oldest to newest");

    fun sort(list: List<Record>): List<Record> = when (this) {
        AZ -> list.sortedBy { it.title.lowercase() }
        ZA -> list.sortedByDescending { it.title.lowercase() }
        NEWEST -> list.sortedByDescending { it.createdAt }
        OLDEST -> list.sortedBy { it.createdAt }
    }

    companion object {
        fun load(context: Context, key: String): SortOrder =
            runCatching { valueOf(context.getSharedPreferences("ui", Context.MODE_PRIVATE).getString(key, AZ.name)!!) }.getOrDefault(AZ)

        fun save(context: Context, key: String, order: SortOrder) =
            context.getSharedPreferences("ui", Context.MODE_PRIVATE).edit().putString(key, order.name).apply()
    }
}

/** The sort button with its badge, and its menu. */
@Composable
fun SortButton(order: SortOrder, onOrder: (SortOrder) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Box(
            Modifier.size(48.dp).clip(RoundedCornerShape(24.dp)).clickable { open = true }.testTag("sort"),
            contentAlignment = Alignment.Center,
        ) {
            if (order.badge.isNotEmpty()) {
                Text(
                    order.badge,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.clip(RoundedCornerShape(6.dp)).background(Tokens.accentSubtle).padding(horizontal = 5.dp, vertical = 1.dp),
                )
            } else {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Filled.CalendarMonth, contentDescription = null, modifier = Modifier.size(20.dp))
                    Text(if (order == SortOrder.NEWEST) "↓" else "↑", fontSize = 12.sp)
                }
            }
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = Tokens.bgRaised) {
            for (choice in SortOrder.entries) {
                if (choice == SortOrder.NEWEST) HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                DropdownMenuItem(
                    text = { Text(choice.label) },
                    trailingIcon = { if (choice == order) Icon(Icons.Filled.Check, contentDescription = "Current", tint = Tokens.accent) },
                    onClick = { open = false; onOrder(choice) },
                    modifier = Modifier.testTag("sort_" + choice.name.lowercase()),
                )
            }
        }
    }
}

/** A search field across the top bar: × on the left, the keyboard open (reference-1 2.2). */
@Composable
fun SearchBar(query: String, onQuery: (String) -> Unit, placeholder: String, onClose: () -> Unit) {
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Row(
        Modifier.fillMaxWidth().height(64.dp).padding(horizontal = 12.dp, vertical = 8.dp)
            .clip(RoundedCornerShape(24.dp)).background(Tokens.bgInput),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BarIcon(Icons.Filled.Close, "Close search", "search_close", onClick = onClose)
        Box(Modifier.weight(1f).padding(end = 16.dp)) {
            if (query.isEmpty()) Text(placeholder, color = Tokens.textMuted)
            BasicTextField(
                query,
                onQuery,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = Tokens.text),
                cursorBrush = SolidColor(Tokens.accent),
                modifier = Modifier.fillMaxWidth().focusRequester(focus).testTag("search"),
            )
        }
    }
}

/** Open sessions of a saved host, by id (their views carry `recordId`). */
fun sessionsOf(views: Map<Long, JSONObject>, recordId: String): List<JSONObject> =
    views.values.filter { it.optString("recordId") == recordId }.sortedBy { it.optLong("id") }

/** Opens an open session: SFTP in its screen, a terminal in the terminal. */
fun openSession(app: App, activity: MainActivity, nav: Nav, id: Long) {
    if (id in app.sftpSessions) {
        nav.push(Page.Sftp(null, id))
    } else {
        activity.startActivity(Intent(activity, TerminalActivity::class.java).putExtra(TerminalActivity.EXTRA_SESSION, id))
    }
}

/**
 * The hosts of the vault (reference-1 2.2), at the top level or inside the
 * group at the end of [path]: groups and hosts as cards, a tap connects a
 * host or opens a group, a long press selects, the session counter opens
 * its menu, "+" opens the sheet of new things.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostsScreen(app: App, activity: MainActivity, nav: Nav, path: List<Record>) {
    val context = LocalContext.current
    val changes by app.changes.collectAsState()
    val views by app.sessionViews.collectAsState()
    val group = path.lastOrNull()
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var order by remember { mutableStateOf(SortOrder.load(context, "hosts_sort")) }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var adding by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<List<Record>?>(null) }
    var moving by remember { mutableStateOf<List<Record>?>(null) }
    var refresh by remember { mutableStateOf(0) }
    var syncing by remember { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()

    val listed by produceState(Pair(listOf<Record>(), listOf<Record>()), changes, group?.id, searching, query, refresh) {
        value = withContext(Dispatchers.IO) {
            try {
                if (searching) {
                    // The whole vault, flat; typing filters it (reference-2 11.5).
                    val input = JSONObject().put("types", JSONArray().put("group").put("host")).put("scope", JSONObject().put("kind", "all"))
                    if (query.isNotBlank()) input.put("text", query.trim())
                    val found = records(input)
                    Pair(found.filter { it.isGroup }, found.filter { !it.isGroup })
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
    val groups = order.sort(listed.first)
    val hosts = order.sort(listed.second)
    val all = groups + hosts
    val selecting = selected.isNotEmpty()

    BackHandler(enabled = selecting) { selected = emptySet() }
    BackHandler(enabled = searching && !selecting) { searching = false; query = "" }

    fun toggle(record: Record) {
        selected = if (record.id in selected) selected - record.id else selected + record.id
    }

    Scaffold(
        topBar = {
            when {
                selecting -> SelectionBar(
                    count = selected.size,
                    canEdit = selected.size == 1,
                    onClose = { selected = emptySet() },
                    onEdit = {
                        val record = all.first { it.id in selected }
                        selected = emptySet()
                        nav.push(if (record.isGroup) Page.EditGroup(record, null) else Page.EditHost(record, null))
                    },
                    menu = { close ->
                        val chosen = all.filter { it.id in selected }
                        if (chosen.size == 1 && !chosen[0].isGroup) {
                            DropdownMenuItem(text = { Text("Connect via SFTP") }, modifier = Modifier.testTag("selection_sftp"), onClick = {
                                close(); selected = emptySet(); nav.push(Page.Sftp(chosen[0], null))
                            })
                        }
                        if (chosen.all { !it.isGroup }) {
                            DropdownMenuItem(text = { Text("Duplicate") }, modifier = Modifier.testTag("selection_duplicate"), onClick = {
                                close()
                                scope.launch {
                                    withContext(Dispatchers.IO) {
                                        for (record in chosen) {
                                            runCatching { Core.call("record_duplicate", JSONObject().put("vault_id", record.vaultId).put("record_id", record.id)) }
                                        }
                                    }
                                    selected = emptySet()
                                    refresh += 1
                                }
                            })
                        }
                        DropdownMenuItem(text = { Text("Move to group") }, modifier = Modifier.testTag("selection_move"), onClick = { close(); moving = chosen })
                        HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
                        DropdownMenuItem(
                            text = { Text("Delete", color = Tokens.danger) },
                            modifier = Modifier.testTag("selection_delete"),
                            onClick = { close(); deleting = chosen },
                        )
                    },
                )
                searching -> SearchBar(query, { query = it }, "Search hosts and groups", onClose = { searching = false; query = "" })
                group == null -> TopBar(
                    "Hosts",
                    left = { BackButton({ nav.pop() }, label = vaultNameCached(changes)) },
                    actions = {
                        BarIcon(Icons.Filled.Search, "Search", "search_open") { searching = true }
                        SortButton(order) { order = it; SortOrder.save(context, "hosts_sort", it) }
                    },
                )
                else -> TopBar(
                    group.title,
                    left = { BarIcon(Icons.AutoMirrored.Filled.ArrowBack, "Back", "up") { nav.pop() } },
                    titleContent = { GroupTitle(group, path, nav) },
                    actions = {
                        BarIcon(Icons.Filled.Search, "Search", "search_open") { searching = true }
                        SortButton(order) { order = it; SortOrder.save(context, "hosts_sort", it) }
                    },
                )
            }
        },
        floatingActionButton = {
            if (selecting) {
                val chosenHosts = hosts.filter { it.id in selected }
                if (chosenHosts.isNotEmpty()) {
                    ExtendedFloatingActionButton(
                        onClick = {
                            selected = emptySet()
                            for (host in chosenHosts) connect(activity, host)
                        },
                        containerColor = Tokens.accent,
                        contentColor = Tokens.onAccent,
                        shape = RoundedCornerShape(16.dp),
                        modifier = Modifier.testTag("connect_selected"),
                    ) { Text("Connect") }
                }
            } else {
                FloatingActionButton(
                    onClick = { adding = true },
                    containerColor = Tokens.accent,
                    contentColor = Tokens.onAccent,
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.testTag("add"),
                ) { Icon(Icons.Filled.Add, contentDescription = "New") }
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            if (all.isEmpty()) {
                EmptyHosts(app, nav, group, searching)
                return@Box
            }
            // Pull to refresh runs a sync when an account is connected (UI spec 6.1).
            PullToRefreshBox(
                isRefreshing = syncing,
                onRefresh = {
                    syncing = true
                    scope.launch {
                        val failure = withContext(Dispatchers.IO) {
                            try {
                                if (Core.obj("sync_status").optBoolean("bound")) Core.call("sync_now")
                                null
                            } catch (error: CoreException) {
                                syncErrorText(error)
                            } catch (error: Exception) {
                                error.message ?: "No sync."
                            }
                        }
                        syncing = false
                        refresh += 1
                        failure?.let { snackbar.showSnackbar(it) }
                    }
                },
                modifier = Modifier.fillMaxSize(),
            ) {
                LazyColumn(Modifier.fillMaxSize(), state = listState) {
                    if (groups.isNotEmpty()) {
                        item(key = "groups") { SectionHeader("Groups") }
                        items(groups, key = { it.id }) { row ->
                            ListCard(
                                title = highlighted(row.title, if (searching) query else ""),
                                subtitle = hostCount(row.hostCount),
                                tag = "group_" + row.title,
                                tile = { if (row.id in selected) CheckTile() else IconTile(Icons.Filled.Folder) },
                                selected = row.id in selected,
                                onClick = {
                                    if (selecting) {
                                        toggle(row)
                                    } else {
                                        // From the search, the group's ancestors come first.
                                        val target = if (searching) pathTo(row, path) else path + row
                                        searching = false
                                        query = ""
                                        nav.push(Page.Hosts(target))
                                    }
                                },
                                onLongClick = { toggle(row) },
                            )
                        }
                    }
                    if (hosts.isNotEmpty()) {
                        item(key = "hosts") { SectionHeader("Hosts") }
                        items(hosts, key = { it.id }) { row ->
                            val open = sessionsOf(views, row.id)
                            ListCard(
                                title = highlighted(row.title, if (searching) query else ""),
                                subtitle = if (open.isNotEmpty()) "Active" else row.subtitle,
                                tag = "host_" + row.title,
                                tile = { if (row.id in selected) CheckTile() else OsTile(OsMark.of(row.os), active = open.isNotEmpty()) },
                                selected = row.id in selected,
                                onClick = { if (selecting) toggle(row) else connect(activity, row) },
                                onLongClick = { toggle(row) },
                                trailing = if (open.isEmpty() || selecting) null else {
                                    { SessionCounter(app, activity, nav, row, open) }
                                },
                            )
                        }
                    }
                    item { Spacer(Modifier.height(96.dp)) }
                }
            }
        }
    }

    if (adding) {
        NewSheet(
            onDismiss = { adding = false },
            onHost = { adding = false; nav.push(Page.EditHost(null, group?.id)) },
            onGroup = { adding = false; nav.push(Page.EditGroup(null, group?.id)) },
        )
    }

    deleting?.let { chosen ->
        DeleteDialog(chosen, onDismiss = { deleting = null }) {
            deleting = null
            selected = emptySet()
            scope.launch { deleteWithUndo(chosen, snackbar) { refresh += 1 } }
        }
    }

    moving?.let { chosen ->
        GroupPicker(
            title = if (chosen.size == 1) "Move “${chosen[0].title}” to" else "Move ${chosen.size} items to",
            exclude = chosen.singleOrNull { it.isGroup }?.id,
            onDismiss = { moving = null },
            onPick = { target ->
                moving = null
                selected = emptySet()
                scope.launch {
                    val failure = withContext(Dispatchers.IO) {
                        try {
                            val ids = JSONArray()
                            chosen.forEach { ids.put(it.id) }
                            val input = JSONObject().put("vault_id", chosen[0].vaultId).put("record_ids", ids)
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

@Composable
private fun vaultNameCached(changes: Long): String {
    val name by produceState("Personal", changes) { value = withContext(Dispatchers.IO) { vaultName() } }
    return name
}

/** The path to [group] for its page: from the search, its ancestors are read from the vault. */
private fun pathTo(group: Record, current: List<Record>): List<Record> {
    val ancestors = group.json.optJSONArray("path")
    if (ancestors == null || ancestors.length() == 0) return listOf(group)
    val known = current.associateBy { it.id }
    val list = ArrayList<Record>()
    for (index in 0 until ancestors.length()) {
        val id = ancestors.optString(index)
        known[id]?.let { list += it; continue }
        val vault = group.vaultId
        runCatching {
            list += Record(Core.obj("record_get", JSONObject().put("vault_id", vault).put("record_id", id)).put("type", "group"))
        }
    }
    return list + group
}

fun hostCount(count: Int): String = when (count) {
    0 -> "No hosts"
    1 -> "1 host"
    else -> "$count hosts"
}

/** A group's header: its icon, name and a drop-down to jump up the tree. */
@Composable
private fun GroupTitle(group: Record, path: List<Record>, nav: Nav) {
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier.clip(RoundedCornerShape(20.dp)).clickable { open = true }.padding(horizontal = 8.dp, vertical = 6.dp).testTag("group_title"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Filled.Folder, contentDescription = null, tint = Tokens.accent, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(6.dp))
            Text(group.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Icon(Icons.Filled.ArrowDropDown, contentDescription = "Go up")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = Tokens.bgRaised) {
            DropdownMenuItem(text = { Text("All hosts") }, onClick = {
                open = false
                nav.popTo { it is Page.Hosts && it.path.isEmpty() }
            })
            path.dropLast(1).forEachIndexed { index, ancestor ->
                DropdownMenuItem(text = { Text(ancestor.title) }, onClick = {
                    open = false
                    nav.popTo { it is Page.Hosts && it.path.size == index + 1 }
                })
            }
        }
    }
}

/** The bar of selection mode: ×, "N selected", the pencil and ⋮ (reference-1 2.2). */
@Composable
fun SelectionBar(
    count: Int,
    canEdit: Boolean,
    onClose: () -> Unit,
    onEdit: (() -> Unit)?,
    menu: @Composable (close: () -> Unit) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    TopBar(
        "$count selected",
        left = { BarIcon(Icons.Filled.Close, "Leave selection", "selection_close", onClick = onClose) },
        titleContent = { Text("$count selected", style = MaterialTheme.typography.titleMedium, modifier = Modifier.testTag("selection_count")) },
        actions = {
            if (onEdit != null) BarIcon(Icons.Filled.Edit, "Edit", "selection_edit", enabled = canEdit, onClick = onEdit)
            Box {
                BarIcon(Icons.Filled.MoreVert, "More", "selection_more") { open = true }
                DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = Tokens.bgRaised) {
                    menu { open = false }
                }
            }
        },
    )
}

/**
 * The session counter of a host card ("1 ▾") and its menu (reference-1
 * 2.2): Connect, Connect via SFTP, the host's open sessions, Close — which
 * closes only the menu (D220).
 */
@Composable
private fun SessionCounter(app: App, activity: MainActivity, nav: Nav, host: Record, open: List<JSONObject>) {
    var shown by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier.clip(RoundedCornerShape(16.dp)).background(Tokens.accentSubtle).clickable { shown = true }
                .heightIn(min = 40.dp).padding(horizontal = 12.dp).testTag("sessions_" + host.title),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(open.size.toString(), fontWeight = FontWeight.Bold)
            Icon(Icons.Filled.ArrowDropDown, contentDescription = "Sessions")
        }
        DropdownMenu(expanded = shown, onDismissRequest = { shown = false }, containerColor = Tokens.bgRaised) {
            DropdownMenuItem(text = { Text("Connect") }, onClick = { shown = false; connect(activity, host) }, modifier = Modifier.testTag("counter_connect"))
            DropdownMenuItem(text = { Text("Connect via SFTP") }, onClick = { shown = false; nav.push(Page.Sftp(host, null)) }, modifier = Modifier.testTag("counter_sftp"))
            HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
            for (view in open) {
                val id = view.optLong("id")
                DropdownMenuItem(
                    text = { Text((if (id in app.sftpSessions) "SFTP · " else "") + view.optString("title").ifBlank { host.title }) },
                    onClick = { shown = false; openSession(app, activity, nav, id) },
                    modifier = Modifier.testTag("counter_session_$id"),
                )
            }
            HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
            DropdownMenuItem(text = { Text("Close") }, onClick = { shown = false }, modifier = Modifier.testTag("counter_close"))
        }
    }
}

/** The sheet of the floating "+" (reference-1 2.2): new host, new group. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NewSheet(onDismiss: () -> Unit, onHost: () -> Unit, onGroup: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = Tokens.bgRaised) {
        Column(Modifier.padding(bottom = 32.dp)) {
            SheetRow(Icons.Filled.Dns, "New host", "new_host", onHost)
            SheetRow(Icons.Filled.CreateNewFolder, "New group", "new_group", onGroup)
        }
    }
}

@Composable
fun SheetRow(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, tag: String, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(onClick = onClick).padding(horizontal = 24.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = Tokens.accent)
        Spacer(Modifier.width(20.dp))
        Text(label)
    }
}

/** No groups or hosts here: New host, and "Restore from sync" while no account is set up (D219). */
@Composable
private fun EmptyHosts(app: App, nav: Nav, group: Record?, searching: Boolean) {
    if (searching) {
        EmptyState(Icons.Filled.Search, "Nothing found", "No host or group matches the search.")
        return
    }
    val bound by produceState<Boolean?>(null) {
        value = withContext(Dispatchers.IO) { runCatching { Core.obj("sync_status").optBoolean("bound") }.getOrNull() }
    }
    EmptyState(
        Icons.Filled.Dns,
        if (group == null) "No hosts yet" else "This group is empty",
        "Add the servers you connect to; a tap on a host connects.",
    ) {
        TextButton(onClick = { nav.push(Page.EditHost(null, group?.id)) }, modifier = Modifier.testTag("empty_new_host")) { Text("New host") }
        // With no account yet, the hosts may be on the server (UI spec 6.1).
        if (group == null && bound == false) {
            TextButton(onClick = { nav.open(Tab.Settings, Page.Account) }, modifier = Modifier.testTag("empty_restore")) { Text("Restore from sync") }
        }
    }
}

@Composable
private fun DeleteDialog(chosen: List<Record>, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    val one = chosen.singleOrNull()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                when {
                    one == null -> "Delete ${chosen.size} items?"
                    one.isGroup -> "Delete group?"
                    else -> "Delete host?"
                },
            )
        },
        text = {
            Text(
                when {
                    one == null -> "They go to Recently deleted; the hosts and groups of a deleted group move one level up."
                    one.isGroup -> "“${one.title}” goes to Recently deleted; its hosts and groups move one level up."
                    else -> "“${one.title}” goes to Recently deleted."
                },
            )
        },
        confirmButton = { TextButton(onClick = onConfirm, modifier = Modifier.testTag("confirm_delete")) { Text("Delete", color = Tokens.danger) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** Deletes [chosen] and offers Undo (UI spec 6.1): a group's moved children go back into it. */
private suspend fun deleteWithUndo(chosen: List<Record>, snackbar: SnackbarHostState, refreshed: () -> Unit) {
    val ids = JSONArray()
    chosen.forEach { ids.put(it.id) }
    val ref = JSONObject().put("vault_id", chosen[0].vaultId).put("record_ids", ids)
    val deleted = withContext(Dispatchers.IO) {
        try {
            Core.obj("record_delete", ref)
        } catch (error: Exception) {
            null
        }
    }
    refreshed()
    if (deleted == null) return
    val label = chosen.singleOrNull()?.let { "Deleted “${it.title}”" } ?: "Deleted ${chosen.size} items"
    val answer = snackbar.showSnackbar(label, "Undo", duration = SnackbarDuration.Long)
    if (answer != SnackbarResult.ActionPerformed) return
    val undone = withContext(Dispatchers.IO) {
        try {
            Core.call("record_restore", ref)
            // A group's hosts and groups moved one level up when it went:
            // they go back into it, as on the desktop (review of run 14).
            val moved = deleted.optJSONArray("moved")
            val groupsGone = chosen.filter { it.isGroup }
            if (groupsGone.size == 1 && moved != null && moved.length() > 0) {
                Core.call("record_move", JSONObject().put("vault_id", chosen[0].vaultId).put("record_ids", moved).put("target_group_id", groupsGone[0].id))
            }
            null
        } catch (error: CoreException) {
            "Not undone: ${error.message}. Recently deleted has it."
        } catch (error: Exception) {
            "Not undone. Recently deleted has it."
        }
    }
    refreshed()
    undone?.let { snackbar.showSnackbar(it) }
}
