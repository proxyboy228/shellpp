package dev.shellpp.android

import androidx.activity.compose.BackHandler
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

private val TAG_COLORS = listOf("graphite", "jade", "amber", "coral", "plum", "moss", "ocean", "rose")

/** Fields of the host the phone edits (UI spec 6.2); the rest is kept as it is. */
private val HOST_FIELDS = setOf("address", "label", "group_id", "tags", "color", "backspace", "charset", "ssh", "order_key")
private val HOST_SSH_FIELDS = setOf("port", "username", "key_id")
private val GROUP_FIELDS = setOf("name", "parent_id", "color", "ssh", "order_key")

/**
 * Saves [data] as it is edited (the desktop's panel saves as you go, UI
 * 3.2): a new record is made once it is valid; the answer's errors, by
 * field, are shown. Returns the record id after a save.
 */
/**
 * What a save writes (review of run 14): the record as it is in the vault
 * now, with only the editor's [fields] that changed since its last save
 * ([baseline]). A change sync brought in while the editor was open stays,
 * in the fields the editor shows but nobody touched too; a new record is
 * [original] with every field. Keys `ssh.x` go into the record's `ssh`.
 */
private fun merged(
    vaultId: String?,
    recordId: String?,
    original: JSONObject,
    fields: Map<String, Any>,
    baseline: Map<String, Any>,
): JSONObject {
    val now = if (recordId == null) {
        null
    } else {
        try {
            Core.obj("record_get", JSONObject().put("vault_id", vaultId ?: vaultId()).put("record_id", recordId)).optJSONObject("data")
        } catch (error: Exception) {
            null
        }
    }
    val data = now ?: JSONObject(original.toString())
    for ((key, value) in fields) {
        if (now != null && value.toString() == baseline[key]?.toString()) continue
        if (key.startsWith("ssh.")) {
            val ssh = data.optJSONObject("ssh") ?: JSONObject().also { data.put("ssh", it) }
            ssh.put(key.removePrefix("ssh."), value)
        } else {
            data.put(key, value)
        }
    }
    return data
}

private fun save(
    vaultId: String?,
    recordId: String?,
    type: String,
    data: JSONObject,
    secrets: JSONObject?,
): Pair<String?, Map<String, String>> = try {
    val input = JSONObject().put("vault_id", vaultId ?: vaultId()).put("type", type).put("data", data)
    if (recordId != null) input.put("record_id", recordId)
    if (secrets != null && secrets.length() > 0) input.put("secrets", secrets)
    val saved = Core.obj("record_save", input)
    saved.getString("record_id") to emptyMap()
} catch (error: CoreException) {
    val fields = HashMap<String, String>()
    // The core's refused fields are `details.errors` (crates/core/src/error.rs).
    val list = (error.details as? JSONObject)?.optJSONArray("errors")
    if (list != null) {
        for (i in 0 until list.length()) {
            val item = list.optJSONObject(i) ?: continue
            fields[item.optString("field")] = item.optString("reason").replace('_', ' ')
        }
    }
    if (fields.isEmpty()) fields[""] = error.message ?: error.code
    recordId to fields
} catch (error: Exception) {
    // Locked meanwhile, or the vault list did not answer.
    recordId to mapOf("" to (error.message ?: "not saved"))
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EditorFrame(title: String, onClose: () -> Unit, content: @Composable () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onClose, modifier = Modifier.testTag("editor_close")) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Tokens.bgBase),
            )
        },
        containerColor = Tokens.bgBase,
    ) { padding ->
        Column(
            Modifier.padding(padding).fillMaxSize().verticalScroll(rememberScrollState()).imePadding()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) { content() }
    }
}

@Composable
private fun Field(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    tag: String,
    placeholder: String? = null,
    error: String? = null,
    number: Boolean = false,
    inherited: String? = null,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        placeholder = placeholder?.let { { Text(it, color = Tokens.textMuted) } },
        isError = error != null,
        // An inherited value shows under the empty field, focused or not.
        supportingText = (error ?: inherited)?.let { { Text(it) } },
        singleLine = true,
        keyboardOptions = if (number) KeyboardOptions(keyboardType = KeyboardType.Number) else KeyboardOptions.Default,
        modifier = Modifier.fillMaxWidth().testTag(tag),
    )
}

@Composable
private fun Section(title: String) {
    Text(title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 8.dp))
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun ColorPicker(value: String?, onPick: (String) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (name in TAG_COLORS) {
            val selected = (value ?: "graphite") == name
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(tagColor(name) ?: Color.Gray)
                    .border(if (selected) 3.dp else 0.dp, Tokens.text, CircleShape)
                    .clickable { onPick(name) }
                    .testTag("color_$name"),
            )
        }
    }
}

/** A row that opens a picker; shows the current choice. */
@Composable
private fun PickerRow(label: String, value: String, tag: String, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 8.dp).testTag(tag)) {
        Text(label, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
        Text(value)
    }
}

private fun vaultId(): String = Core.array("vaults").getJSONObject(0).getString("vault_id")

/** "from group Servers" for a field the record leaves empty (UI spec 6.2). */
private fun inheritedHint(record: JSONObject?, field: String, groups: Map<String, String>): Pair<String, String>? {
    val inherited = record?.optJSONObject("inherited")?.optJSONObject(field) ?: return null
    val value = inherited.opt("value")?.toString() ?: return null
    val from = groups[inherited.optString("from_group_id")] ?: "a group"
    return value to "from group $from"
}

/** The errors no field of the editor shows, as one line at its top. */
private fun unshown(errors: Map<String, String>, shown: Set<String>): String? =
    errors.filterKeys { it !in shown }.entries.joinToString("; ") { (field, reason) ->
        if (field.isEmpty()) reason else "${field.replace('_', ' ')}: $reason"
    }.ifEmpty { null }

/** The host editor (UI spec 6.2): the desktop's host panel for the fields the phone edits. */
@Composable
fun HostEditor(app: App, record: Record?, groupId: String?, onClose: () -> Unit) {
    val original = remember { record?.data ?: JSONObject().put("group_id", groupId ?: JSONObject.NULL) }
    val ssh0 = original.optJSONObject("ssh") ?: JSONObject()
    var recordId by remember { mutableStateOf(record?.id) }
    var view by remember { mutableStateOf(record?.json) }
    var address by remember { mutableStateOf(original.optString("address")) }
    var label by remember { mutableStateOf(original.optString("label").takeIf { it != "null" } ?: "") }
    var group by remember { mutableStateOf(original.optString("group_id").takeIf { it.isNotBlank() && it != "null" }) }
    var tags by remember {
        mutableStateOf(original.optJSONArray("tags")?.let { list -> (0 until list.length()).joinToString(", ") { list.getString(it) } } ?: "")
    }
    var color by remember { mutableStateOf(original.optString("color").takeIf { it.isNotBlank() && it != "null" }) }
    var port by remember { mutableStateOf(if (ssh0.has("port") && !ssh0.isNull("port")) ssh0.optInt("port").toString() else "") }
    var username by remember { mutableStateOf(ssh0.optString("username").takeIf { it != "null" } ?: "") }
    var keyId by remember { mutableStateOf(ssh0.optString("key_id").takeIf { it.isNotBlank() && it != "null" }) }
    var backspace by remember { mutableStateOf(original.optString("backspace").ifBlank { "default" }) }
    var charset by remember { mutableStateOf(original.optString("charset").ifBlank { "utf-8" }) }
    // A host that signs in with an identity, its own or its group's, shows
    // the identity's name, read-only (UI spec 6.2).
    val inheritedIdentity = record?.inherited?.optJSONObject("identity_id")
    val identityId = ssh0.optString("identity_id").takeIf { it.isNotBlank() && it != "null" }
        ?: inheritedIdentity?.optString("value")?.takeIf { it.isNotBlank() && it != "null" }
    val identityGroup = inheritedIdentity?.optString("from_group_id")?.takeIf { !ssh0.has("identity_id") || ssh0.isNull("identity_id") }
    val identityName by produceState<String?>(null, identityId) {
        if (identityId == null) return@produceState
        value = withContext(Dispatchers.IO) {
            try {
                Core.obj("record_get", JSONObject().put("vault_id", record?.vaultId ?: vaultId()).put("record_id", identityId))
                    .optJSONObject("data")?.optString("label")?.takeIf { it.isNotBlank() }
            } catch (error: Exception) {
                null
            }
        }
    }
    var passwordSaved by remember {
        mutableStateOf(record?.json?.optJSONArray("secrets_set")?.let { list -> (0 until list.length()).any { list.getString(it) == "ssh.password" } } == true)
    }
    var newPassword by remember { mutableStateOf("") }
    var clearPassword by remember { mutableStateOf(false) }
    var errors by remember { mutableStateOf(mapOf<String, String>()) }
    var picking by remember { mutableStateOf<String?>(null) }
    var edits by remember { mutableStateOf(0) }
    var saved by remember { mutableStateOf(0) }
    var triedClose by remember { mutableStateOf(false) }
    val saving = remember { Mutex() }
    val scope = rememberCoroutineScope()

    val groups by produceState(mapOf<String, String>(), edits) {
        value = withContext(Dispatchers.IO) {
            try {
                records(JSONObject().put("types", JSONArray().put("group"))).associate { it.id to it.title }
            } catch (error: Exception) {
                emptyMap()
            }
        }
    }
    val keys by produceState(listOf<Pair<String, String>>()) {
        value = withContext(Dispatchers.IO) {
            try {
                val list = Core.array("keys_list")
                (0 until list.length()).map { list.getJSONObject(it) }.map { it.getString("record_id") to it.optString("label") }
            } catch (error: Exception) {
                emptyList()
            }
        }
    }

    fun changed() {
        edits += 1
        app.noteActivity()
    }

    /** The fields the host editor writes, as it writes them. */
    fun fields(): Map<String, Any> = mapOf(
        "address" to address.trim(),
        "label" to label.trim().ifBlank { JSONObject.NULL },
        "group_id" to (group ?: JSONObject.NULL),
        "tags" to JSONArray(tags.split(',').map { it.trim() }.filter { it.isNotEmpty() }),
        "color" to (color ?: JSONObject.NULL),
        "backspace" to backspace,
        "charset" to charset.trim().lowercase().ifBlank { "utf-8" },
        "ssh.port" to (port.trim().toIntOrNull() ?: JSONObject.NULL),
        "ssh.username" to username.trim().ifBlank { JSONObject.NULL },
        "ssh.key_id" to (keyId ?: JSONObject.NULL),
    )
    val baseline = remember { HashMap(fields()) }

    /**
     * Saves what the editor shows; the password only with [withPassword]
     * (when the editor closes), so a pause while typing it never stores a
     * piece of it. One save at a time, never cancelled half way: a new
     * host gets its record id once (review of run 14).
     */
    suspend fun persist(withPassword: Boolean) = withContext(NonCancellable) {
        saving.withLock {
            if (address.isBlank()) return@withLock
            val passwordChange = withPassword && (newPassword.isNotEmpty() || clearPassword)
            if (saved == edits && !passwordChange) return@withLock
            val upTo = edits
            val current = fields()
            val vault = record?.vaultId
            val id0 = recordId
            val data = withContext(Dispatchers.IO) { merged(vault, id0, original, current, baseline) }
            val secrets = JSONObject()
            if (passwordChange) {
                if (clearPassword) secrets.put("ssh.password", JSONObject.NULL) else secrets.put("ssh.password", newPassword)
            }
            val (id, problems) = withContext(Dispatchers.IO) {
                save(record?.vaultId, recordId, "host", data, secrets)
            }
            errors = problems
            if (problems.isEmpty()) {
                recordId = id
                saved = upTo
                baseline.putAll(current)
                if (passwordChange) {
                    passwordSaved = !clearPassword
                    newPassword = ""
                    clearPassword = false
                }
                view = withContext(Dispatchers.IO) {
                    try {
                        Core.obj("record_get", JSONObject().put("vault_id", record?.vaultId ?: vaultId()).put("record_id", id))
                    } catch (error: Exception) {
                        view
                    }
                }
            }
        }
    }

    // Saves a moment after the last change (the desktop saves as you go).
    LaunchedEffect(edits) {
        if (edits == 0) return@LaunchedEffect
        delay(500)
        persist(withPassword = false)
    }

    // Closing saves what is left, the password included (back arrow or gesture).
    // With errors the first Back shows them; a second one leaves without them.
    fun close() {
        scope.launch {
            persist(withPassword = true)
            if (errors.isEmpty() || triedClose) onClose() else triedClose = true
        }
    }
    BackHandler(onBack = ::close)

    EditorFrame(if (record == null) "New host" else "Edit host", ::close) {
        unshown(errors, setOf("address", "label", "tags", "ssh.port", "ssh.username", "ssh.password", "charset"))?.let {
            Text(it, color = Tokens.danger, modifier = Modifier.testTag("editor_error"))
        }
        if (triedClose && errors.isNotEmpty()) {
            Text("Not saved: correct it, or go back again to leave without this change.", color = Tokens.danger)
        }
        Field(address, { address = it; changed() }, "Address", "host_address", error = errors["address"])
        Field(label, { label = it; changed() }, "Label", "host_label", error = errors["label"])
        PickerRow("Group", group?.let { groups[it] } ?: "Top level", "host_group") { picking = "group" }
        Field(tags, { tags = it; changed() }, "Tags (comma-separated)", "host_tags", error = errors["tags"])
        ColorPicker(color) { color = it; changed() }

        Section("SSH")
        val inheritedPort = inheritedHint(view, "port", groups)
        Field(
            port, { port = it.filter(Char::isDigit).take(5); changed() }, "Port", "host_port",
            placeholder = inheritedPort?.first ?: "22",
            error = errors["ssh.port"], number = true,
            inherited = inheritedPort?.takeIf { port.isBlank() }?.let { "${it.first}, ${it.second}" },
        )
        val inheritedUser = inheritedHint(view, "username", groups)
        Field(
            username, { username = it; changed() }, "Username", "host_username",
            placeholder = inheritedUser?.first,
            error = errors["ssh.username"],
            inherited = inheritedUser?.takeIf { username.isBlank() }?.let { "${it.first}, ${it.second}" },
        )
        PasswordField(
            newPassword, { newPassword = it; clearPassword = false; app.noteActivity() },
            if (passwordSaved) "Password (saved; type to replace)" else "Password", "host_password",
            error = errors["ssh.password"] ?: if (newPassword.isNotEmpty()) "Saved when you leave the editor" else null,
        )
        if (passwordSaved) {
            TextButton(onClick = { clearPassword = true; newPassword = ""; changed() }) { Text("Remove the saved password") }
        }
        val keyLabel = keyId?.let { id -> keys.firstOrNull { it.first == id }?.second ?: "A key" }
            ?: inheritedHint(view, "key_id", groups)?.let { "From the group" } ?: "None"
        PickerRow("Key", keyLabel, "host_key") { picking = "key" }

        Section("Terminal")
        PickerRow(
            "Backspace sends",
            mapOf("default" to "Default", "del" to "^? (DEL)", "ctrl_h" to "^H")[backspace] ?: backspace,
            "host_backspace",
        ) { picking = "backspace" }
        Field(charset, { charset = it; changed() }, "Charset", "host_charset", error = errors["charset"])

        val others = original.keys().asSequence().filter { it !in HOST_FIELDS && !original.isNull(it) }.toList() +
            ssh0.keys().asSequence().filter { it !in HOST_SSH_FIELDS && it != "password" && !ssh0.isNull(it) }.map { "ssh.$it" }.toList()
        if (others.isNotEmpty() || identityId != null) {
            Section("More settings (edit on the desktop)")
            if (identityId != null) {
                val from = identityGroup?.let { ", from group ${groups[it] ?: "a group"}" } ?: ""
                Text(
                    "Identity: ${identityName ?: "an identity"}$from",
                    color = Tokens.textSecondary,
                    modifier = Modifier.testTag("host_identity"),
                )
            }
            val rest = others.filter { it != "ssh.identity_id" }
            if (rest.isNotEmpty()) Text(rest.joinToString(", ") { it.replace('_', ' ') }, color = Tokens.textSecondary)
        }
    }

    when (picking) {
        "group" -> GroupPicker("Group", exclude = null, onDismiss = { picking = null }) { picked ->
            group = picked
            picking = null
            changed()
        }
        "key" -> ChoiceDialog(
            "Key",
            listOf<Pair<String?, String>>(null to "None") + keys.map { it.first to it.second },
            onDismiss = { picking = null },
        ) { picked ->
            keyId = picked
            picking = null
            changed()
        }
        "backspace" -> ChoiceDialog(
            "Backspace sends",
            listOf<Pair<String?, String>>("default" to "Default", "del" to "^? (DEL)", "ctrl_h" to "^H"),
            onDismiss = { picking = null },
        ) { picked ->
            backspace = picked ?: "default"
            picking = null
            changed()
        }
    }
}

@Composable
fun ChoiceDialog(title: String, choices: List<Pair<String?, String>>, onDismiss: () -> Unit, onPick: (String?) -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                for ((value, label) in choices) {
                    Text(
                        label,
                        Modifier.fillMaxWidth().clickable { onPick(value) }.padding(vertical = 12.dp)
                            .testTag("choice_$label"),
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** The group editor (UI spec 6.2): name, color label, parent group and the group's SSH defaults. */
@Composable
fun GroupEditor(app: App, record: Record?, parentId: String?, onClose: () -> Unit) {
    val original = remember { record?.data ?: JSONObject().put("parent_id", parentId ?: JSONObject.NULL) }
    val ssh0 = original.optJSONObject("ssh") ?: JSONObject()
    var recordId by remember { mutableStateOf(record?.id) }
    var name by remember { mutableStateOf(original.optString("name").takeIf { it != "null" } ?: "") }
    var parent by remember { mutableStateOf(original.optString("parent_id").takeIf { it.isNotBlank() && it != "null" }) }
    var color by remember { mutableStateOf(original.optString("color").takeIf { it.isNotBlank() && it != "null" }) }
    var port by remember { mutableStateOf(if (ssh0.has("port") && !ssh0.isNull("port")) ssh0.optInt("port").toString() else "") }
    var username by remember { mutableStateOf(ssh0.optString("username").takeIf { it != "null" } ?: "") }
    var keyId by remember { mutableStateOf(ssh0.optString("key_id").takeIf { it.isNotBlank() && it != "null" }) }
    var errors by remember { mutableStateOf(mapOf<String, String>()) }
    var picking by remember { mutableStateOf<String?>(null) }
    var edits by remember { mutableStateOf(0) }
    var saved by remember { mutableStateOf(0) }
    var triedClose by remember { mutableStateOf(false) }
    val saving = remember { Mutex() }
    val scope = rememberCoroutineScope()
    val groups by produceState(mapOf<String, String>()) {
        value = withContext(Dispatchers.IO) {
            try {
                records(JSONObject().put("types", JSONArray().put("group"))).associate { it.id to it.title }
            } catch (error: Exception) {
                emptyMap()
            }
        }
    }
    val keys by produceState(listOf<Pair<String, String>>()) {
        value = withContext(Dispatchers.IO) {
            try {
                val list = Core.array("keys_list")
                (0 until list.length()).map { list.getJSONObject(it) }.map { it.getString("record_id") to it.optString("label") }
            } catch (error: Exception) {
                emptyList()
            }
        }
    }

    fun changed() {
        edits += 1
        app.noteActivity()
    }

    /** The fields the group editor writes, as it writes them. */
    fun fields(): Map<String, Any> = mapOf(
        "name" to name.trim(),
        "parent_id" to (parent ?: JSONObject.NULL),
        "color" to (color ?: JSONObject.NULL),
        "ssh.port" to (port.trim().toIntOrNull() ?: JSONObject.NULL),
        "ssh.username" to username.trim().ifBlank { JSONObject.NULL },
        "ssh.key_id" to (keyId ?: JSONObject.NULL),
    )
    val baseline = remember { HashMap(fields()) }

    // One save at a time, onto the group as it is now (see merged).
    suspend fun persist() = withContext(NonCancellable) {
        saving.withLock {
            if (name.isBlank() || saved == edits) return@withLock
            val upTo = edits
            val current = fields()
            val vault = record?.vaultId
            val id0 = recordId
            val (id, problems) = withContext(Dispatchers.IO) {
                save(vault, id0, "group", merged(vault, id0, original, current, baseline), null)
            }
            errors = problems
            if (problems.isEmpty()) {
                recordId = id
                saved = upTo
                baseline.putAll(current)
            }
        }
    }

    LaunchedEffect(edits) {
        if (edits == 0) return@LaunchedEffect
        delay(500)
        persist()
    }

    // With errors the first Back shows them; a second one leaves without them.
    fun close() {
        scope.launch {
            persist()
            if (errors.isEmpty() || triedClose) onClose() else triedClose = true
        }
    }
    BackHandler(onBack = ::close)

    EditorFrame(if (record == null) "New group" else "Edit group", ::close) {
        unshown(errors, setOf("name", "ssh.port", "ssh.username"))?.let {
            Text(it, color = Tokens.danger, modifier = Modifier.testTag("editor_error"))
        }
        if (triedClose && errors.isNotEmpty()) {
            Text("Not saved: correct it, or go back again to leave without this change.", color = Tokens.danger)
        }
        Field(name, { name = it; changed() }, "Name", "group_name", error = errors["name"])
        ColorPicker(color) { color = it; changed() }
        PickerRow("Parent group", parent?.let { groups[it] } ?: "Top level", "group_parent") { picking = "parent" }
        Section("SSH defaults for its hosts")
        Field(port, { port = it.filter(Char::isDigit).take(5); changed() }, "Port", "group_port", error = errors["ssh.port"], number = true)
        Field(username, { username = it; changed() }, "Username", "group_username", error = errors["ssh.username"])
        PickerRow("Key", keyId?.let { id -> keys.firstOrNull { it.first == id }?.second ?: "A key" } ?: "None", "group_key") { picking = "key" }
        val others = original.keys().asSequence().filter { it !in GROUP_FIELDS && !original.isNull(it) }.toList() +
            ssh0.keys().asSequence().filter { it !in setOf("port", "username", "key_id") && !ssh0.isNull(it) }.map { "ssh.$it" }.toList()
        if (others.isNotEmpty()) {
            Section("More settings (edit on the desktop)")
            Text(others.joinToString(", ") { it.replace('_', ' ') }, color = Tokens.textSecondary)
        }
    }
    when (picking) {
        "parent" -> GroupPicker("Parent group", exclude = recordId, onDismiss = { picking = null }) { picked ->
            parent = picked
            picking = null
            changed()
        }
        "key" -> ChoiceDialog(
            "Key",
            listOf<Pair<String?, String>>(null to "None") + keys.map { it.first to it.second },
            onDismiss = { picking = null },
        ) { picked ->
            keyId = picked
            picking = null
            changed()
        }
    }
}
