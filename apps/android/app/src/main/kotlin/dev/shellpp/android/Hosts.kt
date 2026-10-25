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

    /**
     * The effective user name (D220, data-model 8.3): the host's own, then
     * its own identity's, then the groups' username or identity.
     */
    fun effectiveUser(identityUsers: Map<String, String>): String? {
        val ssh = data.optJSONObject("ssh")
        ssh?.text("username")?.takeIf { it.isNotBlank() }?.let { return it }
        ssh?.text("identity_id")?.let { identityUsers[it] }?.let { return it }
        inherited.optJSONObject("username")?.text("value")?.let { return it }
        return inherited.optJSONObject("identity_id")?.text("value")?.let { identityUsers[it] }
    }

    val hostCount: Int get() = json.optInt("host_count")
    val revision: Long get() = json.optLong("revision")
    val createdAt: String get() = json.optString("created_at")
    val os: JSONObject? get() = data.optJSONObject("os")

    /** The protocol a tap connects with (data-model 7.2 `primary_protocol`). */
    val protocol: String get() = data.text("primary_protocol") ?: "ssh"

    /** The second line of a host card (D220): the protocol and the effective user name. */
    val subtitle: String get() = listOfNotNull(protocol, username).joinToString(", ")

    fun subtitle(identityUsers: Map<String, String>): String = listOfNotNull(protocol, effectiveUser(identityUsers)).joinToString(", ")
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

/** Terminal sessions to several saved hosts, one after another in the session screen. */
fun connectAll(activity: android.app.Activity, hosts: List<Record>) {
    if (hosts.isEmpty()) return
    activity.startActivity(
        Intent(activity, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_VAULT_ID, hosts[0].vaultId)
            .putStringArrayListExtra(TerminalActivity.EXTRA_RECORD_IDS, ArrayList(hosts.map { it.id }))
            .putStringArrayListExtra(TerminalActivity.EXTRA_TITLES, ArrayList(hosts.map { it.title })),
    )
}

/** An SFTP session to a saved host in the session screen (reference-2 8). */
fun connectSftp(activity: android.app.Activity, host: Record) {
    activity.startActivity(
        Intent(activity, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_SFTP, true)
            .putExtra(TerminalActivity.EXTRA_VAULT_ID, host.vaultId)
            .putExtra(TerminalActivity.EXTRA_RECORD_ID, host.id)
            .putExtra(TerminalActivity.EXTRA_TITLE, host.title),
    )
}

/** Opens a terminal on a saved host (UI spec 6.1: tap a host to connect). */
fun connect(activity: android.app.Activity, host: Record) {
    activity.startActivity(
        Intent(activity, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_VAULT_ID, host.vaultId)
            .putExtra(TerminalActivity.EXTRA_RECORD_ID, host.id)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, host.address)
            .putExtra(TerminalActivity.EXTRA_TITLE, host.title),
    )
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
