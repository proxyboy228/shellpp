package dev.shellpp.android

import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * An ssh command line or a short form typed in quick connect:
 * `ssh user@host -p 2222`, `ssh -p 2222 user@host`, `user@host:2222`.
 */
fun parseSsh(text: String): QuickTarget? {
    val words = text.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (words.isEmpty()) return null
    val rest = if (words[0] == "ssh") words.drop(1) else words
    var port: Int? = null
    val plain = ArrayList<String>()
    var index = 0
    while (index < rest.size) {
        val word = rest[index]
        when {
            word == "-p" && index + 1 < rest.size -> {
                port = rest[index + 1].toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
                index += 2
                continue
            }
            word.startsWith("-") -> return null
            else -> plain += word
        }
        index += 1
    }
    if (plain.size != 1) return null
    quickTarget(plain[0])?.let { return if (port != null) it.copy(port = port) else it }
    // `ssh host` without a user: only when it was typed as a command.
    if (words[0] == "ssh" && Regex("^[^@\\s/\\\\\\[\\]]+$").matches(plain[0])) return QuickTarget("", plain[0], port)
    return null
}

/** The equivalent command of a recent connection (reference-2 11.4). */
fun sshCommand(user: String?, address: String, port: Int): String =
    "ssh " + (if (user.isNullOrEmpty()) "" else "$user@") + address + " -p " + port

private fun quickConnect(activity: MainActivity, target: QuickTarget) {
    activity.startActivity(
        Intent(activity, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, target.address)
            .putExtra(TerminalActivity.EXTRA_PORT, target.port ?: 22)
            .putExtra(TerminalActivity.EXTRA_USERNAME, target.user),
    )
}

/**
 * The Connections tab (reference-1 3, reference-2 6): quick connect; while
 * sessions are open, one card per session with a live preview; otherwise
 * the five recent connections, newest first, and Connect via SFTP.
 */
@Composable
fun ConnectionsScreen(app: App, activity: MainActivity, nav: Nav) {
    val views by app.sessionViews.collectAsState()
    var query by rememberSaveable { mutableStateOf("") }
    var pickingSftp by remember { mutableStateOf(false) }
    val command = parseSsh(query)
    val matches by produceState(listOf<Record>(), query) {
        value = if (query.isBlank() || command != null) {
            emptyList()
        } else {
            withContext(Dispatchers.IO) {
                runCatching { records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all")).put("text", query.trim())) }.getOrDefault(emptyList())
            }
        }
    }
    Page(top = { TopBar("Connections") }) {
        QuickConnectField(query, { query = it }, enabled = command != null || matches.isNotEmpty()) {
            when {
                command != null -> {
                    quickConnect(activity, command)
                    query = ""
                }
                matches.isNotEmpty() -> connect(activity, matches[0])
            }
        }
        when {
            query.isNotBlank() && command == null -> LazyColumn(Modifier.fillMaxSize()) {
                if (matches.isEmpty()) {
                    item { Text("No saved host matches. Type an ssh command to connect: ssh user@host -p 22", color = Tokens.textSecondary, modifier = Modifier.padding(20.dp)) }
                }
                items(matches, key = { it.id }) { host ->
                    ListCard(
                        highlighted(host.title, query), host.subtitle, "match_" + host.title,
                        tile = { OsTile(OsMark.of(host.os)) },
                        onClick = { connect(activity, host) },
                    )
                }
            }
            views.isNotEmpty() -> LazyColumn(Modifier.fillMaxSize()) {
                items(views.values.sortedBy { it.optLong("id") }, key = { it.optLong("id") }) { view ->
                    SessionCard(app, activity, view)
                }
            }
            else -> Recents(activity, onSftp = { pickingSftp = true })
        }
    }
    if (pickingSftp) {
        HostChooser("Connect via SFTP", onDismiss = { pickingSftp = false }) { host ->
            pickingSftp = false
            connectSftp(activity, host)
        }
    }
}

/** The quick connect field, Connect inside it on the right (reference-1 3). */
@Composable
private fun QuickConnectField(query: String, onQuery: (String) -> Unit, enabled: Boolean, onConnect: () -> Unit) {
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().heightIn(min = 56.dp)
            .clip(RoundedCornerShape(28.dp)).background(Tokens.bgRaised).padding(start = 20.dp, end = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1f)) {
            if (query.isEmpty()) Text("Search hosts or ssh user@host -p 22", color = Tokens.textMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            BasicTextField(
                query, onQuery, singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = Tokens.text),
                cursorBrush = SolidColor(Tokens.accent),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go, autoCorrectEnabled = false),
                keyboardActions = KeyboardActions(onGo = { if (enabled) onConnect() }),
                modifier = Modifier.fillMaxWidth().testTag("quick_connect"),
            )
        }
        TextButton(onClick = onConnect, enabled = enabled, modifier = Modifier.testTag("quick_connect_go")) {
            Text("Connect", fontWeight = FontWeight.Bold, color = if (enabled) Tokens.accent else Tokens.textMuted)
        }
    }
}

/**
 * A card of an open session (reference-2 6, 11.3): the last lines of its
 * output in the terminal's colors, then its tile, name, protocol and ×.
 */
@Composable
private fun SessionCard(app: App, activity: MainActivity, view: JSONObject) {
    val id = view.optLong("id")
    val sftp = id in app.sftpSessions
    val theme = TerminalPrefs.theme(androidx.compose.ui.platform.LocalContext.current, androidx.compose.foundation.isSystemInDarkTheme())
    val recordId = view.optString("recordId").ifBlank { null }
    val mark by produceState(OsMark(null, null), recordId) { value = withContext(Dispatchers.IO) { hostMark(recordId) } }
    // A live preview: read again every second while the card shows.
    val lines by produceState(listOf<String>(), id) {
        if (sftp) return@produceState
        while (true) {
            value = withContext(Dispatchers.IO) {
                runCatching { Screen.parse(Native.snapshot(id)).logicalLines().dropLastWhile { it.isBlank() }.takeLast(6) }.getOrDefault(value)
            }
            delay(1_000)
        }
    }
    Column(
        Modifier.padding(horizontal = 16.dp, vertical = 6.dp).fillMaxWidth().clip(CardShape)
            .background(if (sftp) Tokens.bgRaised else theme.bg).testTag("session_card_" + view.optString("title")),
    ) {
        Column(
            Modifier.fillMaxWidth().height(120.dp).clickable { openSession(activity, id) }.padding(12.dp).testTag("session_preview_$id"),
        ) {
            if (sftp) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Filled.Folder, contentDescription = null, tint = Tokens.accent)
                    Spacer(Modifier.width(8.dp))
                    Text("Files of ${view.optString("title")}", color = Tokens.textSecondary)
                }
            } else {
                for (line in lines) {
                    Text(line, color = theme.fg, fontFamily = FontFamily.Monospace, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Clip)
                }
            }
        }
        Row(
            Modifier.fillMaxWidth().background(if (sftp) Tokens.bgRaised else Color.Black.copy(alpha = 0.18f)).padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OsTile(mark, 40.dp, sftp = sftp)
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(view.optString("title"), color = if (sftp) Tokens.text else Tokens.accent, maxLines = 1, overflow = TextOverflow.Ellipsis)
                val state = view.optString("state")
                Text(
                    (if (sftp) "sftp" else "ssh") + if (state.isNotEmpty() && state != "connected") " · ${stateText(state)}" else "",
                    color = if (sftp) Tokens.textSecondary else Tokens.accent.copy(alpha = 0.7f), style = MaterialTheme.typography.bodyMedium,
                )
            }
            Box(
                Modifier.size(48.dp).clip(RoundedCornerShape(10.dp)).clickable {
                    runCatching { Native.close(id) }
                    app.forget(id)
                }.testTag("session_close_$id"),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.Close, contentDescription = "Close the session", tint = if (sftp) Tokens.textSecondary else theme.fg) }
        }
    }
}

/** Recent connections (five, newest first; a tap connects again) and the other way: SFTP. */
@Composable
private fun Recents(activity: MainActivity, onSftp: () -> Unit) {
    val recents by produceState(listOf<JSONObject>()) {
        value = withContext(Dispatchers.IO) {
            runCatching { Core.array("recent_connections").let { list -> (0 until minOf(5, list.length())).map { list.getJSONObject(it) } } }.getOrDefault(emptyList())
        }
    }
    LazyColumn(Modifier.fillMaxSize()) {
        item {
            Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(CardShape).background(Tokens.bgRaised).padding(vertical = 8.dp)) {
                Text("Recent connections", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
                Text("A tap connects again.", color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 16.dp))
                if (recents.isEmpty()) {
                    Text("Nothing yet: the sessions you open show here.", color = Tokens.textMuted, modifier = Modifier.padding(16.dp))
                }
                for (recent in recents) RecentRow(activity, recent)
            }
        }
        item {
            Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(CardShape).background(Tokens.bgRaised).padding(vertical = 8.dp)) {
                Text("Other ways to connect", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
                Text("Files of a saved host, by SFTP.", color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 16.dp))
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(onClick = onSftp).padding(horizontal = 16.dp).testTag("connections_sftp"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconTile(Icons.Filled.Folder, 40.dp)
                    Text("Connect via SFTP", Modifier.padding(start = 14.dp))
                }
            }
        }
    }
}

@Composable
private fun RecentRow(activity: MainActivity, recent: JSONObject) {
    val host = recent.optJSONObject("host")
    val address = recent.optString("address")
    val port = recent.optInt("port", 22)
    val user = recent.text("username")
    val os = recent.optJSONObject("os")
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable {
            if (host != null) {
                activity.startActivity(
                    Intent(activity, TerminalActivity::class.java)
                        .putExtra(TerminalActivity.EXTRA_VAULT_ID, host.optString("vault_id"))
                        .putExtra(TerminalActivity.EXTRA_RECORD_ID, host.optString("record_id"))
                        .putExtra(TerminalActivity.EXTRA_ADDRESS, address)
                        .putExtra(TerminalActivity.EXTRA_TITLE, recent.optString("title")),
                )
            } else {
                quickConnect(activity, QuickTarget(user ?: "", address, port))
            }
        }.padding(horizontal = 16.dp, vertical = 6.dp).testTag("recent_" + recent.optString("title")),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OsTile(OsMark.of(os), 40.dp)
        Column(Modifier.weight(1f).padding(start = 14.dp)) {
            Text(recent.optString("title").ifBlank { address }, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(sshCommand(user, address, port), color = Tokens.textSecondary, fontFamily = FontFamily.Monospace, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Picks a saved host (Connect via SFTP from Connections). */
@Composable
private fun HostChooser(title: String, onDismiss: () -> Unit, onPick: (Record) -> Unit) {
    val hosts by produceState(listOf<Record>()) {
        value = withContext(Dispatchers.IO) {
            runCatching { records(JSONObject().put("types", JSONArray().put("host")).put("scope", JSONObject().put("kind", "all"))) }.getOrDefault(emptyList()).sortedBy { it.title.lowercase() }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                items(hosts, key = { it.id }) { host ->
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable { onPick(host) }.testTag("choose_" + host.title),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        OsTile(OsMark.of(host.os), 32.dp)
                        Text(host.title, Modifier.padding(start = 12.dp))
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
