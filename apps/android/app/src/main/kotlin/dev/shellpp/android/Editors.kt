package dev.shellpp.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Badge
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Dns
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Lan
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/*
 * The host and group editors (reference-2 1-3, D220 item 7): cards of
 * fields with floating labels, round buttons that open pickers, check
 * boxes and value rows, a check mark that saves. Only the fields of the
 * desktop's data model (data-model 7) are shown, in the reference's order;
 * the rest of a record is kept as it is. A save writes only what the
 * editor changed (`record_save` with `base`, D221).
 */

private fun vaultId(): String = Core.array("vaults").getJSONObject(0).getString("vault_id")

// -- The form's data -------------------------------------------------------------

/** The member at a dotted [path] ("ssh.port"), or null. */
fun JSONObject.at(path: String): Any? {
    var level: JSONObject = this
    val parts = path.split('.')
    for (part in parts.dropLast(1)) level = level.optJSONObject(part) ?: return null
    val value = level.opt(parts.last())
    return if (value == JSONObject.NULL) null else value
}

/** A copy of this object with [path] set to [value] (null or "" removes it). */
fun JSONObject.with(path: String, value: Any?): JSONObject {
    val copy = JSONObject(toString())
    var level = copy
    val parts = path.split('.')
    for (part in parts.dropLast(1)) {
        level = level.optJSONObject(part) ?: JSONObject().also { level.put(part, it) }
    }
    if (value == null || value == "" || value == JSONObject.NULL) level.remove(parts.last()) else level.put(parts.last(), value)
    return copy
}

/** Equal data, a missing member the same as null or "" (the core's own rule, D221). */
fun sameData(a: Any?, b: Any?): Boolean {
    fun empty(value: Any?): Boolean = value == null || value == JSONObject.NULL || value == "" || value == false ||
        (value is JSONObject && value.keys().asSequence().all { empty(value.opt(it)) })
    if (empty(a) && empty(b)) return true
    if (a is JSONObject && b is JSONObject) {
        val keys = a.keys().asSequence().toSet() + b.keys().asSequence().toSet()
        return keys.all { sameData(a.opt(it), b.opt(it)) }
    }
    if (a is JSONArray && b is JSONArray) {
        return a.length() == b.length() && (0 until a.length()).all { sameData(a.opt(it), b.opt(it)) }
    }
    return a?.toString() == b?.toString()
}

private fun JSONArray?.strings(): List<String> = if (this == null) emptyList() else (0 until length()).mapNotNull { optString(it).ifBlank { null } }

/** The editor's errors by field, from the core's `validation_failed` (crates/core/src/error.rs). */
private fun fieldErrors(error: CoreException): Map<String, String> {
    val fields = HashMap<String, String>()
    val list = (error.details as? JSONObject)?.optJSONArray("errors")
    if (list != null) {
        for (i in 0 until list.length()) {
            val item = list.optJSONObject(i) ?: continue
            fields[item.optString("field")] = item.optString("reason").replace('_', ' ')
        }
    }
    if (fields.isEmpty()) fields[""] = error.message ?: error.code
    return fields
}

// -- Building blocks ---------------------------------------------------------------

/** The editor's top bar: back arrow, title, and the check mark that saves. */
@Composable
private fun EditorBar(title: String, onBack: () -> Unit, onSave: (() -> Unit)?, saving: Boolean = false) {
    TopBar(
        title,
        left = {
            BarIcon(Icons.AutoMirrored.Filled.ArrowBack, "Back", "editor_close", onClick = onBack)
            Text(title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(start = 4.dp))
        },
        titleContent = {},
        actions = {
            if (onSave != null) BarIcon(Icons.Filled.Check, "Save", "editor_save", enabled = !saving, tint = Tokens.accent, onClick = onSave)
        },
    )
}

@Composable
private fun FormCard(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.padding(horizontal = 16.dp, vertical = 6.dp).fillMaxWidth().clip(CardShape).background(Tokens.bgRaised)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        content = content,
    )
}

@Composable
private fun FormHeader(text: String) {
    Text(text, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
}

/** The round button beside a field, in the accent (reference-2 1.2). */
@Composable
fun RoundAction(icon: ImageVector, description: String, tag: String, onClick: () -> Unit) {
    Box(Modifier.size(48.dp).clip(CircleShape).clickable(onClick = onClick).testTag(tag), contentAlignment = Alignment.Center) {
        Box(Modifier.size(40.dp).clip(CircleShape).background(Tokens.accent), contentAlignment = Alignment.Center) {
            Icon(icon, contentDescription = description, tint = Tokens.onAccent, modifier = Modifier.size(22.dp))
        }
    }
}

/**
 * An underlined field with a floating label (reference-2 1.2) and an
 * optional round [action]. [readOnly] fields open a picker on a tap.
 */
@Composable
private fun FormField(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    tag: String,
    placeholder: String? = null,
    error: String? = null,
    note: String? = null,
    number: Boolean = false,
    secret: Boolean = false,
    shown: Boolean = false,
    readOnly: Boolean = false,
    onTap: (() -> Unit)? = null,
    action: (@Composable () -> Unit)? = null,
    multiLine: Boolean = false,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) {
            TextField(
                value = value,
                onValueChange = onChange,
                label = { Text(label) },
                placeholder = placeholder?.let { { Text(it, color = Tokens.textMuted, maxLines = 1, overflow = TextOverflow.Ellipsis) } },
                isError = error != null,
                supportingText = (error ?: note)?.let { { Text(it) } },
                singleLine = !multiLine,
                minLines = if (multiLine) 3 else 1,
                readOnly = readOnly,
                visualTransformation = if (secret && !shown) PasswordVisualTransformation() else VisualTransformation.None,
                keyboardOptions = KeyboardOptions(
                    keyboardType = when {
                        secret -> KeyboardType.Password
                        number -> KeyboardType.Number
                        else -> KeyboardType.Text
                    },
                ),
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = Color.Transparent,
                    unfocusedContainerColor = Color.Transparent,
                    disabledContainerColor = Color.Transparent,
                    errorContainerColor = Color.Transparent,
                    focusedIndicatorColor = Tokens.accent,
                    focusedLabelColor = Tokens.accent,
                    cursorColor = Tokens.accent,
                ),
                modifier = Modifier.fillMaxWidth().testTag(tag),
            )
            // A read-only field opens its picker on a tap anywhere on it.
            if (readOnly && onTap != null) {
                Box(Modifier.matchParentSize().clickable(onClickLabel = "Choose $label", onClick = onTap).testTag(tag + "_tap"))
            }
        }
        if (action != null) {
            Spacer(Modifier.width(12.dp))
            action()
        }
    }
}

/** A label with a square check box on the right (reference-2 1.2: no switches). */
@Composable
private fun CheckRow(label: String, checked: Boolean, tag: String, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable { onChange(!checked) }.testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f))
        Checkbox(checked, onCheckedChange = onChange, colors = CheckboxDefaults.colors(checkedColor = Tokens.accent, checkmarkColor = Tokens.onAccent))
    }
}

/** A label with the current value on the right; a tap opens a picker. */
@Composable
private fun ValueRow(label: String, value: String, tag: String, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable(onClick = onClick).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f))
        Text(value, color = Tokens.accent, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** The vault the record is saved in (reference-2 1.2): one personal vault in 1.0. */
@Composable
private fun VaultSelector() {
    val name by produceState("Personal") { value = withContext(Dispatchers.IO) { vaultName() } }
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 6.dp).fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(Tokens.bgRaised)
            .heightIn(min = 48.dp).padding(horizontal = 16.dp).testTag("editor_vault"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Filled.Storage, contentDescription = null, tint = Tokens.accent, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(12.dp))
        Text(name, Modifier.weight(1f))
        Icon(Icons.Filled.ArrowDropDown, contentDescription = null, tint = Tokens.textMuted)
    }
}

/** "from group X" placeholders for fields the record leaves empty. */
private fun inherited(view: JSONObject?, field: String, groups: Map<String, String>): Pair<String, String>? {
    val value = view?.optJSONObject("inherited")?.optJSONObject(field) ?: return null
    val text = value.opt("value")?.takeIf { it != JSONObject.NULL }?.toString() ?: return null
    return text to (groups[value.optString("from_group_id")] ?: "a group")
}

/**
 * What a record placed in [group] (its view) inherits for [field]: the
 * group's own value, else what the group inherits, with the group it comes
 * from (data-model 8). It follows the group chosen in the form.
 */
private fun inheritedFrom(group: JSONObject?, field: String, groups: Map<String, String>): Pair<String, String>? {
    group ?: return null
    val own = group.optJSONObject("data")?.optJSONObject("ssh")?.opt(field)
    if (own != null && own != JSONObject.NULL && own.toString().isNotEmpty() && own.toString() != "[]") {
        return own.toString() to (group.optJSONObject("data")?.text("name") ?: "a group")
    }
    return inherited(group, field, groups)
}

/** The view of the group a form puts its record in, read again when it changes. */
@Composable
private fun groupView(groupId: String?): JSONObject? {
    val view by produceState<JSONObject?>(null, groupId) {
        value = if (groupId == null) {
            null
        } else {
            withContext(Dispatchers.IO) {
                runCatching { Core.obj("record_get", JSONObject().put("vault_id", vaultId()).put("record_id", groupId)) }.getOrNull()
            }
        }
    }
    return view
}

// -- The editors ---------------------------------------------------------------------

/** Pickers and sub-editors pushed over an editor (reference-2 3). */
private enum class Picker { Group, Tags, Identity, Key, Chain, Proxy, Env, Theme, Charset }

/**
 * What the editors share: the form's data, the version it started from,
 * saving with that base (D221), the discard prompt.
 */
private class EditorState(val record: Record?, initial: JSONObject) {
    val baseRevision = record?.revision
    val baseData: JSONObject = JSONObject(initial.toString())
    var data by mutableStateOf(JSONObject(initial.toString()))
    var errors by mutableStateOf(mapOf<String, String>())
    var saving by mutableStateOf(false)
    var discarding by mutableStateOf(false)
    var picker by mutableStateOf<Picker?>(null)

    fun set(path: String, value: Any?) {
        data = data.with(path, value)
        errors = errors - path
    }

    fun str(path: String): String = data.at(path)?.toString() ?: ""

    val changed: Boolean get() = !sameData(data, baseData)

    /** Saves; true when the record was written. */
    fun save(type: String, secrets: JSONObject?): Boolean = try {
        val input = JSONObject().put("vault_id", record?.vaultId ?: vaultId()).put("type", type).put("data", data)
        if (record != null) {
            input.put("record_id", record.id)
            input.put("base", JSONObject().put("revision", baseRevision).put("data", baseData))
        }
        if (secrets != null && secrets.length() > 0) input.put("secrets", secrets)
        Core.obj("record_save", input)
        true
    } catch (error: CoreException) {
        errors = fieldErrors(error)
        false
    } catch (error: Exception) {
        errors = mapOf("" to (error.message ?: "Not saved."))
        false
    }
}

/** The members whose errors show under their own fields. */
private val SHOWN_ERRORS = setOf("label", "address", "tags", "ssh.port", "ssh.username", "ssh.password", "name", "parent_id", "telnet.port", "notes")

/** The editor's frame: the bar, the scrolling cards, the discard prompt. */
@Composable
private fun EditorFrame(
    title: String,
    state: EditorState,
    scroll: ScrollState,
    onSave: () -> Unit,
    onClose: () -> Unit,
    extraChanges: Boolean,
    content: @Composable ColumnScope.() -> Unit,
) {
    // Leaving without changes closes at once; with changes it asks (D220, [decision]).
    fun back() {
        if (state.changed || extraChanges) state.discarding = true else onClose()
    }
    BackHandler(onBack = ::back)
    Page(top = { EditorBar(title, ::back, onSave, state.saving) }) {
        Column(Modifier.fillMaxSize().verticalScroll(scroll).imePadding().padding(bottom = 32.dp)) {
            // Errors of members no field shows (a Telnet host's protocol, a
            // reference) go here, with the general ones.
            val unshown = state.errors.filterKeys { it !in SHOWN_ERRORS }.entries.joinToString("; ") { (field, reason) ->
                if (field.isEmpty()) reason else "${field.replace('_', ' ')}: $reason"
            }
            if (unshown.isNotEmpty()) Text(unshown, color = Tokens.danger, modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp).testTag("editor_error"))
            content()
        }
    }
    if (state.discarding) {
        AlertDialog(
            onDismissRequest = { state.discarding = false },
            title = { Text("Discard changes?") },
            text = { Text("What you changed in this editor is not saved.") },
            confirmButton = { TextButton(onClick = { state.discarding = false; onClose() }, modifier = Modifier.testTag("discard")) { Text("Discard", color = Tokens.danger) } },
            dismissButton = { TextButton(onClick = { state.discarding = false }, modifier = Modifier.testTag("keep_editing")) { Text("Keep editing") } },
        )
    }
}

/** The host editor (reference-2 1): the fields of data-model 7.2-7.5 in the reference's order. */
@Composable
fun HostEditor(app: App, nav: Nav, record: Record?, groupId: String?, onClose: () -> Unit) {
    val state = remember {
        EditorState(record, record?.data ?: JSONObject().also { if (groupId != null) it.put("group_id", groupId) })
    }
    val scroll = rememberScrollState()
    val scope = rememberCoroutineScope()
    val passwordSaved = record?.json?.optJSONArray("secrets_set").strings().contains("ssh.password")
    val proxyPasswordSaved = record?.json?.optJSONArray("secrets_set").strings().contains("ssh.proxy.password")
    var password by remember { mutableStateOf("") }
    var clearPassword by remember { mutableStateOf(false) }
    var showPassword by remember { mutableStateOf(false) }
    var proxyPassword by remember { mutableStateOf<String?>(null) }
    val groups by produceState(mapOf<String, String>(), state.picker) {
        value = withContext(Dispatchers.IO) { runCatching { records(JSONObject().put("types", JSONArray().put("group")).put("scope", JSONObject().put("kind", "all"))).associate { it.id to it.title } }.getOrDefault(emptyMap()) }
    }
    val names = rememberNames(state.picker)
    val group = groupView(state.data.at("group_id") as? String)
    // The tags as typed (commas and spaces stay until the save reads them).
    var tagsText by remember { mutableStateOf(state.data.optJSONArray("tags").strings().joinToString(", ")) }

    fun secrets(): JSONObject {
        val secrets = JSONObject()
        if (clearPassword) secrets.put("ssh.password", JSONObject.NULL) else if (password.isNotEmpty()) secrets.put("ssh.password", password)
        proxyPassword?.let { if (it.isEmpty()) secrets.put("ssh.proxy.password", JSONObject.NULL) else secrets.put("ssh.proxy.password", it) }
        return secrets
    }

    fun save() {
        if (state.saving) return
        state.saving = true
        scope.launch {
            val ok = withContext(Dispatchers.IO) { state.save("host", secrets()) }
            state.saving = false
            if (ok) {
                password = ""
                onClose()
            }
        }
    }

    when (state.picker) {
        null -> {}
        Picker.Group -> {
            GroupPickerPage(state.data.at("group_id") as? String, exclude = null, onBack = { state.picker = null }) { picked ->
                state.set("group_id", picked)
                state.picker = null
            }
            return
        }
        Picker.Tags -> {
            TagPickerPage(state.data.optJSONArray("tags").strings(), onBack = { state.picker = null }) { tags ->
                state.set("tags", if (tags.isEmpty()) null else JSONArray(tags))
                tagsText = tags.joinToString(", ")
            }
            return
        }
        Picker.Identity -> {
            IdentityPickerPage(state.data.at("ssh.identity_id") as? String, onBack = { state.picker = null }) { picked ->
                state.set("ssh.identity_id", picked)
                state.picker = null
            }
            return
        }
        Picker.Key -> {
            KeyPickerPage(state.data.at("ssh.key_id") as? String, onBack = { state.picker = null }) { picked ->
                state.set("ssh.key_id", picked)
                state.picker = null
            }
            return
        }
        Picker.Chain -> {
            ChainEditorPage(
                state.data.optJSONObject("ssh")?.optJSONArray("jump_chain")?.strings(),
                selfId = record?.id,
                selfTitle = state.str("label").ifBlank { state.str("address") },
                inheritedFrom = inheritedFrom(group, "jump_chain", groups)?.second,
                onBack = { state.picker = null },
            ) { chain ->
                state.set("ssh.jump_chain", chain?.let { JSONArray(it) })
                state.picker = null
            }
            return
        }
        Picker.Proxy -> {
            ProxyEditorPage(state.data.optJSONObject("ssh")?.optJSONObject("proxy"), proxyPasswordSaved && proxyPassword == null, onBack = { state.picker = null }) { proxy, newPassword ->
                state.set("ssh.proxy", proxy)
                if (newPassword != null) proxyPassword = newPassword
                if (proxy == null) proxyPassword = null
                state.picker = null
            }
            return
        }
        Picker.Theme -> {
            ThemePickerPage(state.data.optJSONObject("terminal_theme")?.text("builtin"), onBack = { state.picker = null }) { picked ->
                // Confirmed as it was: a theme imported on the desktop stays.
                if (picked != state.data.optJSONObject("terminal_theme")?.text("builtin")) {
                    state.set("terminal_theme", picked?.let { JSONObject().put("builtin", it) })
                }
                state.picker = null
            }
            return
        }
        Picker.Charset -> {
            CharsetPickerPage(state.str("charset").ifBlank { "utf-8" }, onBack = { state.picker = null }) { picked ->
                state.set("charset", picked)
                state.picker = null
            }
            return
        }
        Picker.Env -> {}
    }

    val extra = password.isNotEmpty() || clearPassword || proxyPassword != null
    EditorFrame(if (record == null) "New host" else "Edit host", state, scroll, ::save, onClose, extra) {
        VaultSelector()
        FormCard {
            FormField(state.str("label"), { state.set("label", it) }, "Label", "host_label", error = state.errors["label"])
            FormField(state.str("address"), { state.set("address", it.trim()) }, "Hostname or IP address", "host_address", error = state.errors["address"])
            val group = state.data.at("group_id") as? String
            FormField(
                group?.let { groups[it] ?: "A group" } ?: "", {}, "Group", "host_group",
                readOnly = true, onTap = { state.picker = Picker.Group },
                action = {
                    if (group != null) {
                        RoundAction(Icons.Filled.Close, "Remove from the group", "host_group_clear") { state.set("group_id", null) }
                    } else {
                        RoundAction(Icons.Filled.Folder, "Pick a group", "host_group_pick") { state.picker = Picker.Group }
                    }
                },
            )
            FormField(
                tagsText,
                { text ->
                    tagsText = text
                    state.set("tags", text.split(',').map { it.trim() }.filter { it.isNotEmpty() }.let { if (it.isEmpty()) null else JSONArray(it) })
                },
                "Tags", "host_tags", error = state.errors["tags"],
                action = { RoundAction(Icons.Filled.Sell, "Pick tags", "host_tags_pick") { state.picker = Picker.Tags } },
            )
            CheckRow("Backspace sends Ctrl+H", state.str("backspace") == "ctrl_h", "host_backspace") {
                state.set("backspace", if (it) "ctrl_h" else null)
            }
        }
        FormCard {
            val port = inheritedFrom(group, "port", groups)
            FormField(
                state.str("ssh.port"), { text -> state.set("ssh.port", text.filter(Char::isDigit).take(5).toIntOrNull()) }, "Port", "host_port",
                placeholder = "22", note = port?.takeIf { state.str("ssh.port").isEmpty() }?.let { "${it.first}, from group ${it.second}" },
                error = state.errors["ssh.port"], number = true,
            )
        }
        FormHeader("Credentials")
        FormCard {
            val user = inheritedFrom(group, "username", groups)
            FormField(
                state.str("ssh.username"), { state.set("ssh.username", it.trim()) }, "Username", "host_username",
                note = user?.takeIf { state.str("ssh.username").isEmpty() }?.let { "${it.first}, from group ${it.second}" }, error = state.errors["ssh.username"],
                action = { RoundAction(Icons.Filled.Person, "Pick an identity", "host_identity_pick") { state.picker = Picker.Identity } },
            )
            val identity = state.data.at("ssh.identity_id") as? String
            if (identity != null) {
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Filled.Badge, contentDescription = null, tint = Tokens.accent, modifier = Modifier.size(18.dp))
                    Text("Identity: ${names[identity] ?: "an identity"}", Modifier.weight(1f).padding(start = 8.dp).testTag("host_identity"), color = Tokens.textSecondary)
                    TextButton(onClick = { state.set("ssh.identity_id", null) }, modifier = Modifier.testTag("host_identity_clear")) { Text("Remove") }
                }
            } else {
                inheritedFrom(group, "identity_id", groups)?.let { (id, from) ->
                    Text("Identity ${names[id] ?: ""}, from group $from", color = Tokens.textSecondary, modifier = Modifier.padding(start = 16.dp, bottom = 4.dp).testTag("host_identity"))
                }
            }
            FormField(
                password, { password = it; clearPassword = false }, "Password", "host_password",
                placeholder = if (passwordSaved && !clearPassword) "Saved" else null,
                error = state.errors["ssh.password"], secret = true, shown = showPassword,
                action = {
                    RoundAction(if (showPassword) Icons.Filled.LockOpen else Icons.Filled.Lock, if (showPassword) "Hide the password" else "Show the password", "host_password_show") {
                        showPassword = !showPassword
                    }
                },
            )
            if (passwordSaved && !clearPassword && password.isEmpty()) {
                TextButton(onClick = { clearPassword = true }, modifier = Modifier.testTag("host_password_clear")) { Text("Remove the saved password") }
            }
            val key = state.data.at("ssh.key_id") as? String
            val keyFrom = inheritedFrom(group, "key_id", groups)
            FormField(
                key?.let { names[it] ?: "A key" } ?: "", {}, "Key", "host_key",
                note = keyFrom?.takeIf { key == null }?.let { "${names[it.first] ?: "A key"}, from group ${it.second}" },
                readOnly = true, onTap = { state.picker = Picker.Key },
                action = { RoundAction(Icons.Filled.Key, "Pick a key", "host_key_pick") { state.picker = Picker.Key } },
            )
        }
        FormCard {
            CheckRow("Agent forwarding", state.data.at("ssh.agent_forwarding") == true, "host_agent_forwarding") {
                state.set("ssh.agent_forwarding", if (it) true else null)
            }
            val chainJson = state.data.optJSONObject("ssh")?.optJSONArray("jump_chain")
            val chain = chainJson.strings()
            FormField(
                if (chainJson != null && chain.isEmpty()) "Directly, no jump hosts" else chain.joinToString(" → ") { names[it] ?: "a host" }, {}, "Host chaining", "host_chain",
                note = inheritedFrom(group, "jump_chain", groups)?.takeIf { chainJson == null }?.let { "Through the chain of group ${it.second}" },
                readOnly = true, onTap = { state.picker = Picker.Chain },
                action = { RoundAction(Icons.Filled.Lan, "Edit the chain", "host_chain_edit") { state.picker = Picker.Chain } },
            )
            val proxy = state.data.optJSONObject("ssh")?.optJSONObject("proxy")
            FormField(
                proxy?.let { "${it.optString("type").uppercase()} ${it.optString("address")}:${it.optInt("port")}" } ?: "", {}, "Proxy", "host_proxy",
                readOnly = true, onTap = { state.picker = Picker.Proxy }, error = state.errors.entries.firstOrNull { it.key.startsWith("ssh.proxy") }?.value,
                action = { RoundAction(Icons.Filled.Dns, "Edit the proxy", "host_proxy_edit") { state.picker = Picker.Proxy } },
            )
            EnvVars(state)
            ValueRow(
                "Colour scheme",
                state.data.optJSONObject("terminal_theme").let { theme ->
                    when {
                        theme == null -> "Default"
                        theme.has("record_id") -> "Custom (desktop)"
                        else -> terminalTheme(theme.text("builtin"))?.name ?: "Default"
                    }
                },
                "host_theme",
            ) { state.picker = Picker.Theme }
            ValueRow("Charset", state.str("charset").ifBlank { "utf-8" }.uppercase(), "host_charset") { state.picker = Picker.Charset }
        }
        FormCard {
            val telnet = state.data.optJSONObject("telnet")
            CheckRow("Telnet", telnet != null, "host_telnet") { on ->
                state.set("telnet", if (on) JSONObject().put("port", 23) else null)
            }
            if (telnet != null) {
                FormField(
                    telnet.opt("port")?.takeIf { it != JSONObject.NULL }?.toString() ?: "", { text -> state.set("telnet.port", text.filter(Char::isDigit).take(5).toIntOrNull()) },
                    "Telnet port", "host_telnet_port", number = true, error = state.errors["telnet.port"],
                )
            }
        }
        // The one hidden field the first version shows (D223 item 4).
        FormCard {
            FormField(state.str("notes"), { state.set("notes", it) }, "Notes", "host_notes", error = state.errors["notes"], multiLine = true)
        }
    }
}

/** The environment variables of a host and the "+" link to add one (reference-2 3.8). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EnvVars(state: EditorState) {
    val list = state.data.optJSONObject("ssh")?.optJSONArray("env")
    val vars = if (list == null) emptyList() else (0 until list.length()).mapNotNull { list.optJSONObject(it) }
    for ((index, item) in vars.withIndex()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("${item.optString("name")} = ${item.optString("value")}", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
            BarIcon(Icons.Filled.Close, "Remove", "host_env_remove_$index") {
                val copy = JSONArray()
                vars.forEachIndexed { at, value -> if (at != index) copy.put(value) }
                state.set("ssh.env", if (copy.length() == 0) null else copy)
            }
        }
    }
    state.errors.entries.firstOrNull { it.key.startsWith("ssh.env") }?.let { Text("${it.key.removePrefix("ssh.")}: ${it.value}", color = Tokens.danger) }
    Text(
        "+ Add environment variable",
        color = Tokens.accent,
        modifier = Modifier.fillMaxWidth().clickable { state.picker = Picker.Env }.padding(vertical = 14.dp).testTag("host_env_add"),
    )
    if (state.picker == Picker.Env) {
        var name by remember { mutableStateOf("") }
        var value by remember { mutableStateOf("") }
        ModalBottomSheet(onDismissRequest = { state.picker = null }, containerColor = Tokens.bgRaised) {
            Column(Modifier.padding(horizontal = 24.dp).padding(bottom = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("New environment variable", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                OutlinedTextField(name, { name = it.trim() }, label = { Text("Variable") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("env_name"))
                OutlinedTextField(value, { value = it }, label = { Text("Value") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("env_value"))
                Button(
                    enabled = name.isNotEmpty() && value.isNotEmpty(),
                    onClick = {
                        val copy = JSONArray()
                        vars.forEach { copy.put(it) }
                        copy.put(JSONObject().put("name", name).put("value", value))
                        state.set("ssh.env", copy)
                        state.picker = null
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = Tokens.accent, contentColor = Tokens.onAccent),
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("env_save"),
                ) { Text("Save") }
            }
        }
    }
}

/** The labels of keys, identities and hosts by record id, for the editors' fields. */
@Composable
private fun rememberNames(trigger: Any?): Map<String, String> {
    val names by produceState(mapOf<String, String>(), trigger) {
        value = withContext(Dispatchers.IO) {
            val map = HashMap<String, String>()
            runCatching {
                val keys = Core.array("keys_list")
                for (i in 0 until keys.length()) keys.getJSONObject(i).let { map[it.getString("record_id")] = it.optString("label") }
            }
            runCatching {
                val identities = Core.array("identities_list")
                for (i in 0 until identities.length()) identities.getJSONObject(i).let { map[it.getString("record_id")] = it.optString("label") }
            }
            runCatching {
                for (host in records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all")))) map[host.id] = host.title
            }
            map
        }
    }
    return names
}

/** The group editor (reference-2 2.2): the host editor without the host-only rows. */
@Composable
fun GroupEditor(app: App, nav: Nav, record: Record?, parentId: String?, onClose: () -> Unit) {
    val state = remember {
        EditorState(record, record?.data ?: JSONObject().also { if (parentId != null) it.put("parent_id", parentId) })
    }
    val scroll = rememberScrollState()
    val scope = rememberCoroutineScope()
    val groups by produceState(mapOf<String, String>(), state.picker) {
        value = withContext(Dispatchers.IO) { runCatching { records(JSONObject().put("types", JSONArray().put("group")).put("scope", JSONObject().put("kind", "all"))).associate { it.id to it.title } }.getOrDefault(emptyMap()) }
    }
    val names = rememberNames(state.picker)
    val parentView = groupView(state.data.at("parent_id") as? String)

    fun save() {
        if (state.saving) return
        state.saving = true
        scope.launch {
            val ok = withContext(Dispatchers.IO) { state.save("group", null) }
            state.saving = false
            if (ok) onClose()
        }
    }

    when (state.picker) {
        Picker.Group -> {
            GroupPickerPage(state.data.at("parent_id") as? String, exclude = record?.id, onBack = { state.picker = null }) { picked ->
                state.set("parent_id", picked)
                state.picker = null
            }
            return
        }
        Picker.Identity -> {
            IdentityPickerPage(state.data.at("ssh.identity_id") as? String, onBack = { state.picker = null }) { picked ->
                state.set("ssh.identity_id", picked)
                state.picker = null
            }
            return
        }
        Picker.Key -> {
            KeyPickerPage(state.data.at("ssh.key_id") as? String, onBack = { state.picker = null }) { picked ->
                state.set("ssh.key_id", picked)
                state.picker = null
            }
            return
        }
        Picker.Chain -> {
            ChainEditorPage(
                state.data.optJSONObject("ssh")?.optJSONArray("jump_chain")?.strings(),
                selfId = null,
                selfTitle = "Hosts of ${state.str("name").ifBlank { "this group" }}",
                inheritedFrom = inheritedFrom(parentView, "jump_chain", groups)?.second,
                onBack = { state.picker = null },
            ) { chain ->
                state.set("ssh.jump_chain", chain?.let { JSONArray(it) })
                state.picker = null
            }
            return
        }
        else -> {}
    }

    EditorFrame(if (record == null) "New group" else "Edit group", state, scroll, ::save, onClose, extraChanges = false) {
        VaultSelector()
        FormCard {
            FormField(state.str("name"), { state.set("name", it) }, "Name", "group_name", error = state.errors["name"])
            val parent = state.data.at("parent_id") as? String
            FormField(
                parent?.let { groups[it] ?: "A group" } ?: "", {}, "Parent group", "group_parent",
                readOnly = true, onTap = { state.picker = Picker.Group }, error = state.errors["parent_id"],
                action = {
                    if (parent != null) {
                        RoundAction(Icons.Filled.Close, "Move to the top level", "group_parent_clear") { state.set("parent_id", null) }
                    } else {
                        RoundAction(Icons.Filled.Folder, "Pick the parent group", "group_parent_pick") { state.picker = Picker.Group }
                    }
                },
            )
        }
        FormCard {
            val port = inheritedFrom(parentView, "port", groups)
            FormField(
                state.str("ssh.port"), { text -> state.set("ssh.port", text.filter(Char::isDigit).take(5).toIntOrNull()) }, "Port", "group_port",
                placeholder = "22", note = port?.takeIf { state.str("ssh.port").isEmpty() }?.let { "${it.first}, from group ${it.second}" }, error = state.errors["ssh.port"], number = true,
            )
        }
        FormHeader("Credentials")
        FormCard {
            val user = inheritedFrom(parentView, "username", groups)
            FormField(
                state.str("ssh.username"), { state.set("ssh.username", it.trim()) }, "Username", "group_username",
                note = user?.takeIf { state.str("ssh.username").isEmpty() }?.let { "${it.first}, from group ${it.second}" }, error = state.errors["ssh.username"],
                action = { RoundAction(Icons.Filled.Person, "Pick an identity", "group_identity_pick") { state.picker = Picker.Identity } },
            )
            (state.data.at("ssh.identity_id") as? String)?.let { identity ->
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("Identity: ${names[identity] ?: "an identity"}", Modifier.weight(1f), color = Tokens.textSecondary)
                    TextButton(onClick = { state.set("ssh.identity_id", null) }) { Text("Remove") }
                }
            }
            val key = state.data.at("ssh.key_id") as? String
            FormField(
                key?.let { names[it] ?: "A key" } ?: "", {}, "Key", "group_key",
                readOnly = true, onTap = { state.picker = Picker.Key },
                action = { RoundAction(Icons.Filled.Key, "Pick a key", "group_key_pick") { state.picker = Picker.Key } },
            )
        }
        FormCard {
            val chainJson = state.data.optJSONObject("ssh")?.optJSONArray("jump_chain")
            val chain = chainJson.strings()
            FormField(
                if (chainJson != null && chain.isEmpty()) "Directly, no jump hosts" else chain.joinToString(" → ") { names[it] ?: "a host" }, {}, "Host chaining", "group_chain",
                readOnly = true, onTap = { state.picker = Picker.Chain },
                action = { RoundAction(Icons.Filled.Lan, "Edit the chain", "group_chain_edit") { state.picker = Picker.Chain } },
            )
        }
        // The one hidden field the first version shows (D223 item 4).
        FormCard {
            FormField(state.str("notes"), { state.set("notes", it) }, "Notes", "group_notes", error = state.errors["notes"], multiLine = true)
        }
    }
}

// -- Pickers -------------------------------------------------------------------------

/** A picker page: back arrow, title, an optional check mark (reference-2 3). */
@Composable
private fun PickerPage(title: String, onBack: () -> Unit, onConfirm: (() -> Unit)? = null, fab: (@Composable () -> Unit)? = null, content: @Composable ColumnScope.() -> Unit) {
    BackHandler(onBack = onBack)
    Box(Modifier.fillMaxSize()) {
        Page(top = {
            TopBar(
                title,
                left = { BarIcon(Icons.AutoMirrored.Filled.ArrowBack, "Back", "picker_back", onClick = onBack) },
                actions = { if (onConfirm != null) BarIcon(Icons.Filled.Check, "Done", "picker_done", tint = Tokens.accent, onClick = onConfirm) },
            )
        }, content = content)
        if (fab != null) Box(Modifier.align(Alignment.BottomEnd).padding(24.dp)) { fab() }
    }
}

@Composable
private fun AddFab(tag: String, onClick: () -> Unit) {
    FloatingActionButton(onClick = onClick, containerColor = Tokens.accent, contentColor = Tokens.onAccent, shape = RoundedCornerShape(16.dp), modifier = Modifier.testTag(tag)) {
        Icon(Icons.Filled.Add, contentDescription = "New")
    }
}

/** The group picker (reference-2 3.1): group cards; a tap marks one, the check mark takes it. */
@Composable
fun GroupPickerPage(current: String?, exclude: String?, onBack: () -> Unit, onPick: (String?) -> Unit) {
    var chosen by remember { mutableStateOf(current) }
    var version by remember { mutableStateOf(0) }
    var creating by remember { mutableStateOf(false) }
    val groups by produceState(listOf<Record>(), version) {
        value = withContext(Dispatchers.IO) {
            runCatching { records(JSONObject().put("types", JSONArray().put("group")).put("scope", JSONObject().put("kind", "all"))) }.getOrDefault(emptyList())
                .filter { it.id != exclude && exclude !in it.json.optJSONArray("path").strings() }
                .sortedBy { it.title.lowercase() }
        }
    }
    PickerPage("Group", onBack, onConfirm = { onPick(chosen) }, fab = { AddFab("picker_new_group") { creating = true } }) {
        LazyColumn(Modifier.fillMaxSize()) {
            item { SectionHeader("Groups") }
            items(groups, key = { it.id }) { group ->
                ListCard(
                    AnnotatedString(group.title), hostCount(group.hostCount), "pick_" + group.title,
                    tile = { if (group.id == chosen) CheckTile() else IconTile(Icons.Filled.Folder) },
                    selected = group.id == chosen,
                    onClick = { chosen = if (chosen == group.id) null else group.id },
                )
            }
        }
    }
    if (creating) {
        NameDialog("New group", "Name", onDismiss = { creating = false }) { name ->
            creating = false
            val made = runCatching {
                Core.obj("record_save", JSONObject().put("vault_id", vaultId()).put("type", "group").put("data", JSONObject().put("name", name))).getString("record_id")
            }.getOrNull()
            if (made != null) chosen = made
            version += 1
        }
    }
}

/** A small dialog asking for one name (a new group or tag from a picker). */
@Composable
private fun NameDialog(title: String, label: String, onDismiss: () -> Unit, onDone: suspend (String) -> Unit) {
    var name by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = { OutlinedTextField(name, { name = it }, label = { Text(label) }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("name_field")) },
        confirmButton = {
            TextButton(enabled = name.isNotBlank(), onClick = { scope.launch { withContext(Dispatchers.IO) { onDone(name.trim()) } } }, modifier = Modifier.testTag("name_ok")) { Text("Create") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** The tag picker (reference-2 3.2): every tag of the vault, several at once; a tap applies. */
@Composable
private fun TagPickerPage(current: List<String>, onBack: () -> Unit, onChange: (List<String>) -> Unit) {
    var chosen by remember { mutableStateOf(current) }
    var added by remember { mutableStateOf(listOf<String>()) }
    var creating by remember { mutableStateOf(false) }
    val all by produceState(listOf<String>()) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all")))
                    .flatMap { it.data.optJSONArray("tags").strings() }
            }.getOrDefault(emptyList())
        }
    }
    val tags = (all + current + added).distinctBy { it.lowercase() }.sortedBy { it.lowercase() }
    PickerPage("Tags", onBack, fab = { AddFab("picker_new_tag") { creating = true } }) {
        if (tags.isEmpty()) {
            EmptyState(Icons.Filled.Sell, "No tags yet", "Tags group hosts across groups; add one with +.")
        }
        LazyColumn(Modifier.fillMaxSize().padding(top = 8.dp)) {
            items(tags, key = { it }) { tag ->
                val on = chosen.any { it.equals(tag, ignoreCase = true) }
                Row(
                    Modifier.padding(horizontal = 16.dp, vertical = 1.dp).fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(Tokens.bgRaised)
                        .clickable {
                            chosen = if (on) chosen.filterNot { it.equals(tag, ignoreCase = true) } else chosen + tag
                            onChange(chosen)
                        }
                        .heightIn(min = 52.dp).padding(horizontal = 16.dp).testTag("tag_$tag"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(tag, Modifier.weight(1f))
                    Icon(Icons.Filled.Check, contentDescription = if (on) "Selected" else null, tint = if (on) Tokens.accent else Tokens.textMuted.copy(alpha = 0.4f))
                }
            }
        }
    }
    if (creating) {
        NameDialog("New tag", "Tag", onDismiss = { creating = false }) { name ->
            withContext(Dispatchers.Main) {
                added = added + name
                chosen = chosen + name
                onChange(chosen)
                creating = false
            }
        }
    }
}

/** The identity picker (reference-2 3.3): the vault's identities, None, and "+" for a new one. */
@Composable
private fun IdentityPickerPage(current: String?, onBack: () -> Unit, onPick: (String?) -> Unit) {
    var version by remember { mutableStateOf(0) }
    var creating by remember { mutableStateOf(false) }
    val identities by produceState(listOf<JSONObject>(), version) {
        value = withContext(Dispatchers.IO) {
            runCatching { Core.array("identities_list").let { list -> (0 until list.length()).map { list.getJSONObject(it) } } }.getOrDefault(emptyList())
        }
    }
    PickerPage("Identities", onBack, fab = { AddFab("picker_new_identity") { creating = true } }) {
        if (identities.isEmpty()) {
            EmptyState(Icons.Filled.Badge, "No identities", "An identity is a username with its password or key, shared by the hosts that sign in with it.")
        } else {
            LazyColumn(Modifier.fillMaxSize()) {
                item {
                    ListCard(AnnotatedString("None"), "Sign in with the host's own username", "pick_identity_none", tile = { IconTile(Icons.Filled.Close) }, onClick = { onPick(null) }, selected = current == null)
                }
                items(identities, key = { it.getString("record_id") }) { identity ->
                    val id = identity.getString("record_id")
                    ListCard(
                        AnnotatedString(identity.optString("label")),
                        identity.text("username"),
                        "pick_identity_" + identity.optString("label"),
                        tile = { if (id == current) CheckTile() else IconTile(Icons.Filled.Badge) },
                        selected = id == current,
                        onClick = { onPick(id) },
                    )
                }
            }
        }
    }
    if (creating) {
        IdentityDialog(onDismiss = { creating = false }) { made ->
            creating = false
            version += 1
            if (made != null) onPick(made)
        }
    }
}

/** A new identity: label, username, password (UI spec 7; the core's `identity_save`). */
@Composable
fun IdentityDialog(onDismiss: () -> Unit, onDone: (String?) -> Unit) {
    var label by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New identity") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(label, { label = it }, label = { Text("Label") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("identity_label"))
                OutlinedTextField(username, { username = it.trim() }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("identity_username"))
                PasswordField(password, { password = it }, "Password (optional)", "identity_password")
                error?.let { Text(it, color = Tokens.danger) }
            }
        },
        confirmButton = {
            TextButton(enabled = label.isNotBlank() && !busy, onClick = {
                busy = true
                val chars = password.toCharArray()
                scope.launch {
                    val made = withContext(Dispatchers.IO) {
                        try {
                            val input = JSONObject().put("label", label.trim()).put("username", username)
                            val answer = if (chars.isNotEmpty()) Core.callWithSecrets("identity_save", input, listOf("password" to chars)) else Core.call("identity_save", input)
                            (answer as JSONObject).getString("record_id")
                        } catch (failed: CoreException) {
                            error = failed.message
                            null
                        } finally {
                            chars.fill('\u0000')
                        }
                    }
                    busy = false
                    // A failed save keeps what was typed (review of 3.4).
                    if (made != null) {
                        password = ""
                        onDone(made)
                    }
                }
            }, modifier = Modifier.testTag("identity_save")) { Text("Create") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** The key picker (reference-2 3.4): None, the vault's keys, and making a new one. */
@Composable
private fun KeyPickerPage(current: String?, onBack: () -> Unit, onPick: (String?) -> Unit) {
    var generating by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val keys by produceState(listOf<JSONObject>()) {
        value = withContext(Dispatchers.IO) { runCatching { Core.array("keys_list").let { list -> (0 until list.length()).map { list.getJSONObject(it) } } }.getOrDefault(emptyList()) }
    }
    PickerPage("Keys", onBack) {
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                Spacer(Modifier.height(8.dp))
                GroupedList {
                    GroupedRow("New SSH key…", "pick_key_new", Icons.Filled.Key, divider = false) { generating = true }
                }
                SectionHeader("Keys")
                ListCard(AnnotatedString("None"), "No key: the password or the agent", "pick_key_none", tile = { IconTile(Icons.Filled.Close) }, selected = current == null, onClick = { onPick(null) })
            }
            items(keys, key = { it.getString("record_id") }) { key ->
                val id = key.getString("record_id")
                ListCard(
                    AnnotatedString(key.optString("label")),
                    algorithmName(key.optString("algorithm")),
                    "pick_key_" + key.optString("label"),
                    tile = { if (id == current) CheckTile() else IconTile(Icons.Filled.Key) },
                    selected = id == current,
                    onClick = { onPick(id) },
                )
            }
        }
    }
    if (generating) {
        GenerateKeyDialog(onDismiss = { generating = false }) { input, passphrase ->
            generating = false
            scope.launch {
                val made = withContext(Dispatchers.IO) {
                    try {
                        generateKey(input, passphrase)
                    } catch (error: Exception) {
                        failure = error.message ?: "The key was not made."
                        null
                    } finally {
                        passphrase.fill('\u0000')
                    }
                }
                if (made != null) onPick(made)
            }
        }
    }
    failure?.let { message ->
        AlertDialog(
            onDismissRequest = { failure = null },
            title = { Text("The key was not made") },
            text = { Text(message) },
            confirmButton = { TextButton(onClick = { failure = null }) { Text("OK") } },
        )
    }
}

/**
 * The chain editor (reference-2 3.6): the jump hosts top to bottom, then
 * the host being edited; "Add a host" adds one before it, Clear empties it.
 */
@Composable
private fun ChainEditorPage(current: List<String>?, selfId: String?, selfTitle: String, inheritedFrom: String?, onBack: () -> Unit, onDone: (List<String>?) -> Unit) {
    // `null` inherits the group's chain; an empty list connects directly and
    // stops the inheritance (data-model 7.1, 8.3).
    var inherit by remember { mutableStateOf(current == null) }
    var chain by remember { mutableStateOf(current ?: emptyList()) }
    var adding by remember { mutableStateOf(false) }
    val hosts by produceState(listOf<Record>()) {
        value = withContext(Dispatchers.IO) { runCatching { records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all"))) }.getOrDefault(emptyList()) }
    }
    val byId = hosts.associateBy { it.id }
    PickerPage("Host chaining", onBack, onConfirm = { onDone(if (inherit) null else chain) }) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Column(Modifier.fillMaxWidth().clip(CardShape).background(Tokens.bgRaised).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Connect through these hosts first, top to bottom.", color = Tokens.textSecondary)
                if (inherit) {
                    Text(
                        if (inheritedFrom != null) "Now: the chain of group $inheritedFrom." else "Now: no jump hosts (none set in the groups).",
                        color = Tokens.textSecondary, modifier = Modifier.testTag("chain_inherits"),
                    )
                } else if (chain.isEmpty()) {
                    Text("Now: directly, without jump hosts.", color = Tokens.textSecondary, modifier = Modifier.testTag("chain_direct"))
                }
                for ((index, id) in chain.withIndex()) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OsTile(OsMark.of(byId[id]?.os), 36.dp)
                        Text(byId[id]?.title ?: "A host that is gone", Modifier.weight(1f).padding(start = 12.dp))
                        BarIcon(Icons.Filled.Close, "Remove", "chain_remove_$index") { chain = chain.filterIndexed { at, _ -> at != index } }
                    }
                }
                OutlinedButton(onClick = { adding = true }, enabled = chain.size < 8, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("chain_add")) {
                    Text("Add a host")
                }
            }
            Icon(Icons.Filled.ArrowDownward, contentDescription = null, tint = Tokens.textMuted, modifier = Modifier.padding(8.dp))
            Row(Modifier.fillMaxWidth().clip(CardShape).background(Tokens.bgRaised).padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                OsTile(OsMark(null, null), 36.dp)
                Text(selfTitle.ifBlank { "This host" }, Modifier.padding(start = 12.dp))
            }
            Spacer(Modifier.height(16.dp))
            TextButton(onClick = { chain = emptyList(); inherit = false }, modifier = Modifier.align(Alignment.Start).testTag("chain_clear")) { Text("Clear: connect directly", color = Tokens.danger) }
            if (!inherit) {
                TextButton(onClick = { chain = emptyList(); inherit = true }, modifier = Modifier.align(Alignment.Start).testTag("chain_inherit")) { Text("Use the group's chain") }
            }
        }
    }
    if (adding) {
        val choices = hosts.filter { it.id != selfId && it.id !in chain }.sortedBy { it.title.lowercase() }
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text("Add a jump host") },
            text = {
                LazyColumn {
                    items(choices, key = { it.id }) { host ->
                        Text(host.title, Modifier.fillMaxWidth().clickable { chain = chain + host.id; inherit = false; adding = false }.padding(vertical = 14.dp).testTag("chain_pick_" + host.title))
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { adding = false }) { Text("Cancel") } },
        )
    }
}

/** The proxy editor (reference-2 3.7, data-model 7.5): a flat page; the check mark takes it. */
@Composable
private fun ProxyEditorPage(current: JSONObject?, passwordSaved: Boolean, onBack: () -> Unit, onDone: (JSONObject?, String?) -> Unit) {
    var type by remember { mutableStateOf(current?.text("type") ?: "http") }
    var address by remember { mutableStateOf(current?.text("address") ?: "") }
    var port by remember { mutableStateOf(current?.opt("port")?.takeIf { it != JSONObject.NULL }?.toString() ?: "") }
    var username by remember { mutableStateOf(current?.text("username") ?: "") }
    var password by remember { mutableStateOf("") }
    var typeMenu by remember { mutableStateOf(false) }
    val valid = address.isNotBlank() && port.toIntOrNull() in 1..65535
    PickerPage(if (current == null) "New proxy" else "Proxy", onBack, onConfirm = {
        if (valid) {
            val proxy = JSONObject().put("type", type).put("address", address.trim()).put("port", port.toInt())
            if (username.isNotBlank()) proxy.put("username", username.trim())
            onDone(proxy, password.ifEmpty { null })
        }
    }) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            Box {
                ValueRow("Type", type.uppercase(), "proxy_type") { typeMenu = true }
                DropdownMenu(expanded = typeMenu, onDismissRequest = { typeMenu = false }, containerColor = Tokens.bgRaised) {
                    for (choice in listOf("http", "socks5")) {
                        DropdownMenuItem(text = { Text(choice.uppercase()) }, onClick = { type = choice; typeMenu = false }, modifier = Modifier.testTag("proxy_type_$choice"))
                    }
                }
            }
            FormField(address, { address = it.trim() }, "Address", "proxy_address")
            FormField(port, { port = it.filter(Char::isDigit).take(5) }, "Port", "proxy_port", number = true)
            FormHeader("Credentials")
            FormField(username, { username = it }, "Username", "proxy_username")
            FormField(password, { password = it }, "Password", "proxy_password", secret = true, placeholder = if (passwordSaved) "Saved" else null)
            if (!valid && (address.isNotEmpty() || port.isNotEmpty())) Text("An address and a port from 1 to 65535.", color = Tokens.danger, modifier = Modifier.padding(top = 8.dp))
            if (current != null) {
                TextButton(onClick = { onDone(null, null) }, modifier = Modifier.padding(top = 16.dp).testTag("proxy_remove")) { Text("Remove the proxy", color = Tokens.danger) }
            }
        }
    }
}

/** A theme as a miniature terminal: a few bars of its colors on its background (reference-2 3.9). */
@Composable
fun ThemeTile(theme: TerminalTheme?, name: String, selected: Boolean, tag: String, onClick: () -> Unit) {
    val shown = theme ?: TERMINAL_THEMES.first()
    Column(Modifier.padding(6.dp).clickable(onClick = onClick).testTag(tag), horizontalAlignment = Alignment.CenterHorizontally) {
        Column(
            Modifier.fillMaxWidth().height(72.dp).clip(RoundedCornerShape(10.dp)).background(shown.bg)
                .border(if (selected) BorderStroke(2.dp, Tokens.accent) else BorderStroke(1.dp, Tokens.border.copy(alpha = 0.4f)), RoundedCornerShape(10.dp))
                .padding(8.dp),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            for ((width, color) in listOf(0.7f to shown.color(2), 0.45f to shown.color(4), 0.85f to shown.fg, 0.3f to shown.color(1))) {
                Box(Modifier.fillMaxWidth(width).height(6.dp).clip(RoundedCornerShape(3.dp)).background(color))
            }
        }
        Text(
            name, color = if (selected) Tokens.accent else Tokens.text, fontSize = 13.sp, textAlign = TextAlign.Center,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 4.dp),
        )
    }
}

/** The per-host theme picker (reference-2 3.9): Default first (the app's theme), then the built-in themes. */
@Composable
private fun ThemePickerPage(current: String?, onBack: () -> Unit, onPick: (String?) -> Unit) {
    var chosen by remember { mutableStateOf(current) }
    PickerPage("Colour scheme", onBack, onConfirm = { onPick(chosen) }) {
        LazyVerticalGrid(GridCells.Fixed(3), Modifier.fillMaxSize().padding(horizontal = 10.dp)) {
            item { ThemeTile(null, "Default", chosen == null, "theme_default") { chosen = null } }
            items(TERMINAL_THEMES, key = { it.id }) { theme ->
                ThemeTile(theme, theme.name, chosen == theme.id, "theme_" + theme.id) { chosen = theme.id }
            }
        }
    }
}

/** WHATWG encoding labels a host can name (data-model 7.2 `charset`), the common ones. */
private val CHARSETS = listOf(
    "utf-8", "iso-8859-1", "iso-8859-2", "iso-8859-5", "iso-8859-7", "iso-8859-15", "windows-1250", "windows-1251",
    "windows-1252", "windows-1253", "windows-1254", "windows-1256", "koi8-r", "koi8-u", "ibm866", "macintosh",
    "shift_jis", "euc-jp", "iso-2022-jp", "euc-kr", "gbk", "gb18030", "big5",
)

/** The charset picker (reference-2 3.10): Current and Use default, then the list; a tap chooses. */
@Composable
private fun CharsetPickerPage(current: String, onBack: () -> Unit, onPick: (String?) -> Unit) {
    PickerPage("Charset", onBack) {
        LazyColumn(Modifier.fillMaxSize().padding(horizontal = 20.dp)) {
            item {
                Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable(onClick = onBack), verticalAlignment = Alignment.CenterVertically) {
                    Text("Current", color = Tokens.accent, modifier = Modifier.weight(1f))
                    Text(current.uppercase())
                }
                Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable { onPick(null) }.testTag("charset_default"), verticalAlignment = Alignment.CenterVertically) {
                    Text("Use default", color = Tokens.accent, modifier = Modifier.weight(1f))
                    Text("UTF-8")
                }
                HorizontalDivider(color = Tokens.border.copy(alpha = 0.4f))
            }
            items(CHARSETS, key = { it }) { label ->
                Text(
                    label.uppercase(),
                    Modifier.fillMaxWidth().clickable { onPick(if (label == "utf-8") null else label) }.padding(vertical = 14.dp).testTag("charset_$label"),
                    color = if (label == current) Tokens.accent else Tokens.text,
                )
            }
        }
    }
}

/** A short list to pick one value from (the auto-lock delay and the like). */
@Composable
fun ChoiceDialog(title: String, choices: List<Pair<String?, String>>, onDismiss: () -> Unit, onPick: (String?) -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                for ((value, label) in choices) {
                    Text(label, Modifier.fillMaxWidth().clickable { onPick(value) }.padding(vertical = 12.dp).testTag("choice_$label"))
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
