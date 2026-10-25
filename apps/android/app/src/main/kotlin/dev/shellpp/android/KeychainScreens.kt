package dev.shellpp.android

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Badge
import androidx.compose.material.icons.filled.FileOpen
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.VerifiedUser
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * Compares addresses with their numbers as numbers (D220): an IPv4 address
 * by its octets, "13.x" before "109.x"; letters case-insensitively.
 */
val NaturalOrder: Comparator<String> = Comparator { a, b ->
    val pattern = Regex("\\d+|\\D+")
    val left = pattern.findAll(a.lowercase()).map { it.value }.toList()
    val right = pattern.findAll(b.lowercase()).map { it.value }.toList()
    for (index in 0 until minOf(left.size, right.size)) {
        val x = left[index]
        val y = right[index]
        val compared = if (x[0].isDigit() && y[0].isDigit()) {
            val trimmedX = x.trimStart('0')
            val trimmedY = y.trimStart('0')
            if (trimmedX.length != trimmedY.length) trimmedX.length - trimmedY.length else trimmedX.compareTo(trimmedY)
        } else {
            x.compareTo(y)
        }
        if (compared != 0) return@Comparator compared
    }
    left.size - right.size
}

/** A known host's address as it shows: `[host]:port` when the port is not 22. */
fun knownHostName(entry: JSONObject): String {
    if (entry.optBoolean("hashed")) return "hashed host"
    entry.text("display_address")?.let { return it }
    val host = entry.optString("host")
    val port = entry.optInt("port", 22)
    return if (port == 22 || entry.isNull("port")) host else "[$host]:$port"
}

/** The vault lists' bar: back to the vault, the title, search and sort (reference-2 4, 5). */
@Composable
private fun ListBar(title: String, nav: Nav, order: SortOrder, onOrder: (SortOrder) -> Unit, onSearch: () -> Unit) {
    val vault by produceState("Personal") { value = withContext(Dispatchers.IO) { vaultName() } }
    TopBar(
        title,
        left = { BackButton({ nav.pop() }, label = vault) },
        actions = {
            BarIcon(Icons.Filled.Search, "Search", "search_open", onClick = onSearch)
            SortButton(order, onOrder)
        },
    )
}

/**
 * The Keychain (reference-2 4): keys and identities as cards, an empty
 * state, "+" with a sheet of what the core makes on the phone: a new
 * identity, a generated or imported SSH key (FIDO2 and biometric keys come
 * after the first version).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeychainScreen(app: App, activity: MainActivity, nav: Nav) {
    val context = LocalContext.current
    val changes by app.changes.collectAsState()
    var refresh by remember { mutableStateOf(0) }
    var adding by remember { mutableStateOf(false) }
    var generating by remember { mutableStateOf(false) }
    var newIdentity by remember { mutableStateOf(false) }
    var editingIdentity by remember { mutableStateOf<JSONObject?>(null) }
    var importing by remember { mutableStateOf<CharArray?>(null) }
    var opened by remember { mutableStateOf<JSONObject?>(null) }
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var order by remember { mutableStateOf(SortOrder.load(context, "keychain_sort")) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val keys by produceState(listOf<JSONObject>(), changes, refresh) {
        value = withContext(Dispatchers.IO) {
            runCatching { Core.array("keys_list").let { list -> (0 until list.length()).map { list.getJSONObject(it) } } }.getOrDefault(emptyList())
        }
    }
    val identities by produceState(listOf<JSONObject>(), changes, refresh) {
        value = withContext(Dispatchers.IO) {
            runCatching { Core.array("identities_list").let { list -> (0 until list.length()).map { list.getJSONObject(it) } } }.getOrDefault(emptyList())
        }
    }
    val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            val text = withContext(Dispatchers.IO) { readKeyFile(activity, uri) }
            if (text == null) snackbar.showSnackbar("The file could not be read as a key.") else importing = text
        }
    }
    // The private key text of an import in progress goes when the screen goes.
    DisposableEffect(Unit) { onDispose { importing?.fill('\u0000') } }
    BackHandler(enabled = searching) { searching = false; query = "" }

    opened?.let { key ->
        KeyScreen(app, activity, key, onBack = { opened = null; refresh += 1 }, onDeleted = { refs, label ->
            // Here, not in the key's screen, which is gone by then: the Undo
            // must outlive it (review of run 14).
            opened = null
            scope.launch {
                val done = withContext(Dispatchers.IO) { runCatching { Core.call("record_delete", refs) }.isSuccess }
                refresh += 1
                if (done) {
                    val answer = snackbar.showSnackbar("Deleted “$label”", "Undo", duration = SnackbarDuration.Long)
                    if (answer == SnackbarResult.ActionPerformed) {
                        withContext(Dispatchers.IO) { runCatching { Core.call("record_restore", refs) } }
                        refresh += 1
                    }
                }
            }
        })
        return
    }

    fun sorted(list: List<JSONObject>): List<JSONObject> {
        val shown = list.filter { query.isBlank() || it.optString("label").contains(query.trim(), ignoreCase = true) }
        return when (order) {
            SortOrder.AZ -> shown.sortedWith(compareBy(NaturalOrder) { it.optString("label") })
            SortOrder.ZA -> shown.sortedWith(compareBy(NaturalOrder) { it.optString("label") }).reversed()
            SortOrder.NEWEST -> shown.sortedByDescending { it.optString("created_at") }
            SortOrder.OLDEST -> shown.sortedBy { it.optString("created_at") }
        }
    }

    Scaffold(
        topBar = {
            if (searching) {
                SearchBar(query, { query = it }, "Search keys and identities", onClose = { searching = false; query = "" })
            } else {
                ListBar("Keychain", nav, order, { order = it; SortOrder.save(context, "keychain_sort", it) }) { searching = true }
            }
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = { adding = true }, containerColor = Tokens.accent, contentColor = Tokens.onAccent,
                shape = RoundedCornerShape(16.dp), modifier = Modifier.testTag("key_add"),
            ) { Icon(Icons.Filled.Add, contentDescription = "New") }
        },
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            val shownKeys = sorted(keys)
            val shownIdentities = sorted(identities)
            if (shownKeys.isEmpty() && shownIdentities.isEmpty()) {
                if (query.isBlank()) {
                    EmptyState(Icons.Filled.Key, "No keys yet", "Keys and identities sign you in to your hosts. Make one with +, or import a key file.")
                } else {
                    EmptyState(Icons.Filled.Search, "Nothing found", "No key or identity matches the search.")
                }
                return@Box
            }
            LazyColumn(Modifier.fillMaxSize()) {
                if (shownKeys.isNotEmpty()) {
                    item { SectionHeader("Keys") }
                    items(shownKeys, key = { it.optString("record_id") }) { key ->
                        ListCard(
                            highlighted(key.optString("label"), query),
                            algorithmName(key.optString("algorithm")) + " · " + shortFingerprint(key.optString("fingerprint")),
                            "key_" + key.optString("label"),
                            tile = { IconTile(Icons.Filled.Key) },
                            onClick = { opened = key },
                        )
                    }
                }
                if (shownIdentities.isNotEmpty()) {
                    item { SectionHeader("Identities") }
                    items(shownIdentities, key = { it.optString("record_id") }) { identity ->
                        ListCard(
                            highlighted(identity.optString("label"), query),
                            listOfNotNull(identity.text("username"), if (identity.optBoolean("password_set")) "password saved" else null).joinToString(" · "),
                            "identity_" + identity.optString("label"),
                            tile = { IconTile(Icons.Filled.Badge) },
                            onClick = { editingIdentity = identity },
                        )
                    }
                }
                item { Spacer(Modifier.height(96.dp)) }
            }
        }
    }

    if (adding) {
        ModalBottomSheet(onDismissRequest = { adding = false }, containerColor = Tokens.bgRaised) {
            Column(Modifier.padding(bottom = 32.dp)) {
                SheetRow(Icons.Filled.Badge, "New identity", "keychain_new_identity") { adding = false; newIdentity = true }
                HorizontalDivider(Modifier.padding(vertical = 4.dp), color = Tokens.border.copy(alpha = 0.4f))
                SheetRow(Icons.Filled.Key, "Generate a new SSH key", "keychain_generate") { adding = false; generating = true }
                SheetRow(Icons.Filled.FileOpen, "Import a key from a file", "keychain_import") { adding = false; pickFile.launch(arrayOf("*/*")) }
            }
        }
    }
    if (newIdentity) {
        IdentityDialog(onDismiss = { newIdentity = false }) { made ->
            newIdentity = false
            refresh += 1
            if (made != null) scope.launch { snackbar.showSnackbar("Identity saved") }
        }
    }
    editingIdentity?.let { identity ->
        IdentityEditDialog(identity, onDismiss = { editingIdentity = null }) {
            editingIdentity = null
            refresh += 1
        }
    }
    if (generating) {
        GenerateKeyDialog(onDismiss = { generating = false }) { input, passphrase ->
            generating = false
            scope.launch {
                val failure = withContext(Dispatchers.IO) {
                    try {
                        generateKey(input, passphrase)
                        null
                    } catch (error: CoreException) {
                        error.message
                    } catch (error: Exception) {
                        "The key was not made."
                    } finally {
                        passphrase.fill('\u0000')
                    }
                }
                refresh += 1
                failure?.let { snackbar.showSnackbar(it) }
            }
        }
    }
    importing?.let { text ->
        ImportKeyDialog(text, onDone = { message ->
            text.fill('\u0000')
            importing = null
            refresh += 1
            message?.let { scope.launch { snackbar.showSnackbar(it) } }
        })
    }
}

/** An identity's label, username and password (UI spec 7; the core's `identity_save`). */
@Composable
private fun IdentityEditDialog(identity: JSONObject, onDismiss: () -> Unit, onDone: () -> Unit) {
    var label by remember { mutableStateOf(identity.optString("label")) }
    var username by remember { mutableStateOf(identity.text("username") ?: "") }
    var password by remember { mutableStateOf("") }
    var clear by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val saved = identity.optBoolean("password_set")
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Identity") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(label, { label = it }, label = { Text("Label") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("identity_label"))
                OutlinedTextField(username, { username = it.trim() }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("identity_username"))
                PasswordField(password, { password = it; clear = false }, if (saved && !clear) "Password (saved; type to replace)" else "Password", "identity_password")
                if (saved && !clear && password.isEmpty()) TextButton(onClick = { clear = true }) { Text("Remove the saved password") }
                error?.let { Text(it, color = Tokens.danger) }
            }
        },
        confirmButton = {
            TextButton(enabled = label.isNotBlank() && !busy, onClick = {
                busy = true
                val chars = password.toCharArray()
                scope.launch {
                    val failure = withContext(Dispatchers.IO) {
                        try {
                            // The identity as it is now (a sync may have
                            // changed it): what this dialog did not change
                            // stays as it is there (review of 3.4).
                            val id = identity.optString("record_id")
                            val current = runCatching {
                                Core.array("identities_list").let { list -> (0 until list.length()).map { list.getJSONObject(it) } }.firstOrNull { it.optString("record_id") == id }
                            }.getOrNull() ?: identity
                            val newLabel = if (label.trim() == identity.optString("label")) current.optString("label") else label.trim()
                            val newUser = if (username == (identity.text("username") ?: "")) current.text("username") ?: "" else username
                            val input = JSONObject().put("record_id", id).put("label", newLabel)
                                .put("username", newUser).put("key_id", current.opt("key_id") ?: JSONObject.NULL).put("clear_password", clear)
                            if (chars.isNotEmpty()) Core.callWithSecrets("identity_save", input, listOf("password" to chars)) else Core.call("identity_save", input)
                            null
                        } catch (failed: CoreException) {
                            failed.message
                        } finally {
                            chars.fill('\u0000')
                        }
                    }
                    busy = false
                    // A failed save keeps what was typed, so Save again sends it.
                    if (failure == null) {
                        password = ""
                        onDone()
                    } else {
                        error = failure
                    }
                }
            }, modifier = Modifier.testTag("identity_save")) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * Known hosts (reference-2 5): one card per entry with the labels of the
 * saved hosts that use its address; no "+" (entries come from connecting);
 * sorted with numbers as numbers (D220); a tap shows the key, a long press
 * selects for removal (with Undo).
 */
@Composable
fun KnownHostsScreen(app: App, nav: Nav) {
    val context = LocalContext.current
    val changes by app.changes.collectAsState()
    var refresh by remember { mutableStateOf(0) }
    var opened by remember { mutableStateOf<JSONObject?>(null) }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var removing by remember { mutableStateOf<List<JSONObject>?>(null) }
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var order by remember { mutableStateOf(SortOrder.load(context, "known_hosts_sort")) }
    val snackbar = remember { SnackbarHostState() }
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    val entries by produceState(listOf<JSONObject>(), changes, refresh) {
        value = withContext(Dispatchers.IO) {
            runCatching { Core.array("known_hosts_list").let { list -> (0 until list.length()).map { list.getJSONObject(it) } } }.getOrDefault(emptyList())
        }
    }
    // The labels of the saved hosts each entry is for: the core matches
    // them (`host_ids`: the normalised address and the effective port).
    val titles by produceState(mapOf<String, String>(), changes) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all"))).associate { it.id to it.title }
            }.getOrDefault(emptyMap())
        }
    }
    fun usersOf(entry: JSONObject): List<String> {
        val ids = entry.optJSONArray("host_ids") ?: return emptyList()
        return (0 until ids.length()).mapNotNull { titles[ids.optString(it)] }
    }
    // A sync that removed an entry takes it out of the selection too.
    LaunchedEffect(entries) {
        val present = entries.map { it.optString("record_id") }.toSet()
        if (!present.containsAll(selected)) selected = selected intersect present
    }

    BackHandler(enabled = selected.isNotEmpty()) { selected = emptySet() }
    BackHandler(enabled = searching && selected.isEmpty()) { searching = false; query = "" }

    opened?.let { entry ->
        BackHandler { opened = null }
        Page(top = { TopBar(knownHostName(entry), left = { BackButton({ opened = null }) }) }) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Key type: " + entry.optString("key_type"))
                Text("SHA-256 fingerprint", color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                Text(entry.optString("fingerprint"), modifier = Modifier.testTag("known_fingerprint"))
                OutlinedButton(onClick = { clipboard.setText(AnnotatedString(entry.optString("fingerprint"))) }) { Text("Copy") }
                Text("First seen " + entry.optString("created_at").take(16).replace('T', ' '), color = Tokens.textSecondary)
                val names = usersOf(entry)
                if (names.isNotEmpty()) Text("Used by " + names.joinToString(", "), color = Tokens.textSecondary)
                if (entry.optBoolean("revoked")) Text("Revoked: connections to this host are refused.", color = Tokens.danger)
                TextButton(onClick = { removing = listOf(entry) }, modifier = Modifier.testTag("known_remove")) { Text("Remove", color = Tokens.danger) }
            }
        }
    }
    if (opened == null) {
        val shown = entries
            .filter { query.isBlank() || knownHostName(it).contains(query.trim(), ignoreCase = true) || usersOf(it).any { name -> name.contains(query.trim(), ignoreCase = true) } }
            .let { list ->
                when (order) {
                    SortOrder.AZ -> list.sortedWith(compareBy(NaturalOrder) { knownHostName(it) })
                    SortOrder.ZA -> list.sortedWith(compareBy(NaturalOrder) { knownHostName(it) }).reversed()
                    SortOrder.NEWEST -> list.sortedByDescending { it.optString("created_at") }
                    SortOrder.OLDEST -> list.sortedBy { it.optString("created_at") }
                }
            }
        Scaffold(
            topBar = {
                when {
                    selected.isNotEmpty() -> SelectionBar(selected.size, canEdit = false, onClose = { selected = emptySet() }, onEdit = null) { close ->
                        DropdownMenuItem(
                            text = { Text("Remove", color = Tokens.danger) },
                            onClick = { close(); removing = entries.filter { it.optString("record_id") in selected }.ifEmpty { null } },
                            modifier = Modifier.testTag("known_remove_selected"),
                        )
                    }
                    searching -> SearchBar(query, { query = it }, "Search known hosts", onClose = { searching = false; query = "" })
                    else -> ListBar("Known hosts", nav, order, { order = it; SortOrder.save(context, "known_hosts_sort", it) }) { searching = true }
                }
            },
            snackbarHost = { SnackbarHost(snackbar) },
            containerColor = Tokens.bgBase,
        ) { padding ->
            Box(Modifier.padding(padding).fillMaxSize()) {
                if (shown.isEmpty()) {
                    EmptyState(Icons.Filled.VerifiedUser, "No known hosts yet", "A server's key is kept here the first time you accept it, and checked at every connection after.")
                    return@Box
                }
                LazyColumn(Modifier.fillMaxSize()) {
                    item { Spacer(Modifier.height(8.dp)) }
                    items(shown, key = { it.optString("record_id") }) { entry ->
                        val id = entry.optString("record_id")
                        val names = usersOf(entry)
                        ListCard(
                            highlighted(knownHostName(entry), query),
                            names.joinToString(", ").ifEmpty { null },
                            "known_" + knownHostName(entry),
                            tile = { if (id in selected) CheckTile() else IconTile(Icons.Filled.VerifiedUser) },
                            selected = id in selected,
                            onClick = { if (selected.isNotEmpty()) selected = if (id in selected) selected - id else selected + id else opened = entry },
                            onLongClick = { selected = if (id in selected) selected - id else selected + id },
                        )
                    }
                }
            }
        }
    }
    removing?.takeIf { it.isNotEmpty() }?.let { chosen ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text(if (chosen.size == 1) "Remove this known host?" else "Remove ${chosen.size} known hosts?") },
            text = { Text("The next connection to " + (if (chosen.size == 1) knownHostName(chosen[0]) else "them") + " asks about its key again.") },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    opened = null
                    selected = emptySet()
                    scope.launch {
                        val ids = JSONArray()
                        chosen.forEach { ids.put(it.optString("record_id")) }
                        val refs = JSONObject().put("vault_id", chosen[0].optString("vault_id")).put("record_ids", ids)
                        val done = withContext(Dispatchers.IO) { runCatching { Core.call("known_host_remove", refs) }.isSuccess }
                        refresh += 1
                        if (done) {
                            val label = if (chosen.size == 1) "Removed ${knownHostName(chosen[0])}" else "Removed ${chosen.size} known hosts"
                            val answer = snackbar.showSnackbar(label, "Undo", duration = SnackbarDuration.Long)
                            if (answer == SnackbarResult.ActionPerformed) {
                                withContext(Dispatchers.IO) { runCatching { Core.call("known_host_restore", refs) } }
                                refresh += 1
                            }
                        }
                    }
                }, modifier = Modifier.testTag("known_remove_confirm")) { Text("Remove", color = Tokens.danger) }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Cancel") } },
        )
    }
}
