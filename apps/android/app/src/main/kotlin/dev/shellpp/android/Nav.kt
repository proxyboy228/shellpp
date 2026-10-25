package dev.shellpp.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** The three tabs of the bottom bar (reference-1 1). */
enum class Tab(val label: String, val tag: String) {
    Vaults("Vaults", "tab_vaults"),
    Connections("Connections", "tab_connections"),
    Settings("Settings", "tab_settings"),
}

/** A screen of a tab's back stack. */
sealed interface Page {
    /** Pages that show the bottom bar (the lists); editors and pickers cover it. */
    val withBar: Boolean get() = false

    data object VaultHome : Page {
        override val withBar get() = true
    }

    /** The hosts of the vault at [path] (empty: the top level), one page per group level. */
    data class Hosts(val path: List<Record>) : Page {
        override val withBar get() = true
    }

    data object Keychain : Page {
        override val withBar get() = true
    }

    data object KnownHosts : Page {
        override val withBar get() = true
    }

    data object RecentlyDeleted : Page {
        override val withBar get() = true
    }

    data class EditHost(val record: Record?, val groupId: String?) : Page
    data class EditGroup(val record: Record?, val parentId: String?) : Page
    data object Account : Page
    data class Sftp(val host: Record?, val session: Long?) : Page

    data object Connections : Page {
        override val withBar get() = true
    }

    data object SettingsHome : Page {
        override val withBar get() = true
    }

    data object CustomizeKeyboard : Page
    data object FontColours : Page
    data object Security : Page
}

/**
 * The tabs and their back stacks (reference-1 1): each tab keeps its own
 * stack, so another tab and back returns to where the user was; Back pops
 * the current tab's stack.
 */
class Nav {
    var tab by mutableStateOf(Tab.Vaults)
    private val stacks = mutableStateMapOf(
        Tab.Vaults to listOf<Page>(Page.VaultHome),
        Tab.Connections to listOf<Page>(Page.Connections),
        Tab.Settings to listOf<Page>(Page.SettingsHome),
    )

    val stack: List<Page> get() = stacks.getValue(tab)
    val top: Page get() = stack.last()

    fun push(page: Page) {
        stacks[tab] = stack + page
    }

    /** Back: the previous page of this tab; false at its first page. */
    fun pop(): Boolean {
        if (stack.size <= 1) return false
        stacks[tab] = stack.dropLast(1)
        return true
    }

    /** Pops pages until [keep] holds for the top one. */
    fun popTo(keep: (Page) -> Boolean) {
        val list = stack
        val index = list.indexOfLast(keep)
        if (index >= 0) stacks[tab] = list.take(index + 1)
    }

    /** Shows [tab], and [page] on top of its stack when given. */
    fun open(tab: Tab, page: Page? = null) {
        this.tab = tab
        if (page != null) push(page)
    }
}

/** The app after unlock: the current page of the current tab and the bottom bar. */
@Composable
fun MainScreen(app: App, activity: MainActivity) {
    val nav = remember { Nav() }
    val holder = rememberSaveableStateHolder()
    val top = nav.top
    BackHandler(enabled = nav.stack.size > 1) { nav.pop() }
    Column(Modifier.fillMaxSize()) {
        Box(Modifier.weight(1f)) {
            // Each page keeps its scroll position and fields while others
            // cover it (reference-2 10: the Vaults tab reopens where it was).
            holder.SaveableStateProvider("${nav.tab}/${nav.stack.size}/${top.hashCode()}") {
                PageContent(app, activity, nav, top)
            }
        }
        if (top.withBar) BottomBar(app, nav)
    }
}

@Composable
private fun PageContent(app: App, activity: MainActivity, nav: Nav, page: Page) {
    when (page) {
        Page.VaultHome -> VaultHome(app, nav)
        is Page.Hosts -> HostsScreen(app, activity, nav, page.path)
        Page.Keychain -> KeychainScreen(app, activity, nav)
        Page.KnownHosts -> KnownHostsScreen(app, nav)
        Page.RecentlyDeleted -> RecentlyDeletedScreen(app, onClose = { nav.pop() })
        is Page.EditHost -> HostEditor(app, nav, page.record, page.groupId, onClose = { nav.pop() })
        is Page.EditGroup -> GroupEditor(app, nav, page.record, page.parentId, onClose = { nav.pop() })
        Page.Account -> AccountScreen(app, activity, onClose = { nav.pop() })
        is Page.Sftp -> SftpScreen(app, activity, page.host, page.session, onClose = { nav.pop() })
        Page.Connections -> ConnectionsScreen(app, activity, nav)
        Page.SettingsHome -> SettingsScreen(app, activity, nav)
        Page.CustomizeKeyboard -> CustomizeKeyboardScreen(app, onClose = { nav.pop() })
        Page.FontColours -> FontColoursScreen(app, onClose = { nav.pop() })
        Page.Security -> SecuritySettingsScreen(app, activity, onClose = { nav.pop() })
    }
}

/**
 * The bottom bar: Vaults, Connections, Settings. While a session is open
 * the Connections tab shows the OS mark of the last session's host, in the
 * accent (reference-1 2.2), so a running session is seen from any screen.
 */
@Composable
private fun BottomBar(app: App, nav: Nav) {
    val views by app.sessionViews.collectAsState()
    val active = app.activeSession(views)
    val recordId = active?.optString("recordId")?.ifBlank { null }
    val mark by produceState<OsMark?>(null, recordId, views.size) {
        value = if (active == null) null else withContext(Dispatchers.IO) { hostMark(recordId) }
    }
    NavigationBar(containerColor = Tokens.bgBase) {
        for (tab in Tab.entries) {
            val selected = nav.tab == tab
            NavigationBarItem(
                selected = selected,
                onClick = {
                    // A second tap on the current tab goes back to its first page.
                    if (selected) nav.popTo { it == nav.stack.first() } else nav.tab = tab
                },
                icon = {
                    when (tab) {
                        Tab.Vaults -> Icon(Icons.Filled.Storage, contentDescription = null)
                        Tab.Connections -> if (mark != null) {
                            OsTile(mark!!, 26.dp)
                        } else {
                            Icon(TerminalGlyph, contentDescription = null)
                        }
                        Tab.Settings -> Icon(Icons.Filled.Settings, contentDescription = null)
                    }
                },
                label = { Text(tab.label, fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal) },
                colors = NavigationBarItemDefaults.colors(indicatorColor = Tokens.accentSubtle),
                modifier = Modifier.testTag(tab.tag),
            )
        }
    }
}

/** The OS mark of a saved host (the generic one for a host not saved or not detected). */
fun hostMark(recordId: String?): OsMark {
    if (recordId == null) return OsMark(null, null)
    return try {
        val vault = Core.array("vaults").getJSONObject(0).getString("vault_id")
        val view = Core.obj("record_get", JSONObject().put("vault_id", vault).put("record_id", recordId))
        OsMark.of(view.optJSONObject("data")?.optJSONObject("os"))
    } catch (error: Exception) {
        OsMark(null, null)
    }
}
