package dev.shellpp.android

import android.Manifest
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog as ComposeAlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * The session screen (reference-1 5, reference-2 7-8; D220): the open
 * sessions, terminal and SFTP, one shown at a time, the session bar with
 * its chips under it and, on a terminal session, the extra-keys row.
 * Opened from a host, a recent connection, a session card or quick
 * connect; "<" and Back go back to the lists and leave every session
 * running (Back clears a selection first).
 */
/**
 * What a tap on Send says when the password did not go ([Native.sendPassword]'s
 * answer): every answer but [Native.PASSWORD_SENT] says why (review of run 16).
 */
fun passwordNotSent(answer: Int): Int = when (answer) {
    Native.PASSWORD_NONE -> R.string.password_key_none
    Native.PASSWORD_RECORDED -> R.string.password_not_sent_recorded
    Native.PASSWORD_CONTROL -> R.string.password_not_sent_control
    Native.PASSWORD_NOT_CONNECTED -> R.string.password_not_sent_offline
    Native.PASSWORD_QUICK -> R.string.password_key_quick
    else -> R.string.password_not_sent
}

class TerminalActivity : ComponentActivity(), TerminalView.Modifiers, KeySink {
    private val app get() = application as App
    private val worker = Executors.newSingleThreadExecutor()

    /** The terminal views of the terminal sessions shown here, by session. */
    private val views = LinkedHashMap<Long, TerminalView>()

    /** A terminal whose session is opening (no id yet). */
    private var opening: TerminalView? = null
    private lateinit var content: FrameLayout
    private lateinit var sftpView: ComposeView

    /** The session shown, 0 while one opens. */
    private var active by mutableStateOf(0L)

    /** An SFTP session opening for this host (its screen connects it). */
    private var openingSftp by mutableStateOf<Record?>(null)
    private var keyboardShown by mutableStateOf(false)
    private var modifiersVersion by mutableIntStateOf(0)
    private val ctrlKey = StickyModifier()
    private val altKey = StickyModifier()
    private var cursorOn by mutableStateOf(false)
    /** What the password key can do in the session in front ([Native.passwordState]). */
    private var passwordState by mutableIntStateOf(Native.PASSWORD_QUICK)
    /** The session whose saved password the open confirmation is for. */
    private var confirmingPassword by mutableStateOf<Long?>(null)

    /** The upload's SFTP session, its home folder and the host it goes to. */
    private var attachFolder by mutableStateOf<Triple<Long, String, String>?>(null)
    private var attachUris: List<Uri> = emptyList()

    /** The terminal session the attach key was pressed on (the picker's answer goes there). */
    private var attachFor = 0L
    private var keyGroups by mutableStateOf(listOf<List<XKey>>())

    /** Hosts still to open (several selected hosts connect one after another). */
    private val queue = ArrayDeque<Target>()
    private val answered = HashMap<Long, Long>()

    /** The permission dialog was answered (or there was none to ask). */
    private var permissionsSettled = false

    /** Test-only answers (a debuggable build only): the e2e test's password and host key. */
    private var testPassword: ByteArray? = null
    private var testAcceptHostKey = false

    /** Where a terminal session goes: a saved host or a quick connect. */
    data class Target(val vaultId: String?, val recordId: String?, val address: String, val port: Int, val username: String, val title: String)

    /** The session shown (0 while one opens). */
    val session: Long get() = active

    /** The terminal shown (tests read its screen and type into it). */
    val terminal: TerminalView
        get() = views[active] ?: opening ?: views.values.lastOrNull() ?: idle

    private val idle by lazy { TerminalView(this, this) }

    private val pickFiles = registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) startAttach(uris)
    }

    @OptIn(ExperimentalComposeUiApi::class)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val debuggable = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        if (debuggable) {
            testPassword = intent.getByteArrayExtra(EXTRA_TEST_PASSWORD)
            testAcceptHostKey = intent.getBooleanExtra(EXTRA_TEST_ACCEPT_HOST_KEY, false)
        }
        keyGroups = XKey.groups(this)
        content = FrameLayout(this)
        sftpView = ComposeView(this).apply {
            setContent {
                ShellTheme {
                    // Text in the theme's color, as inside a Scaffold.
                    androidx.compose.runtime.CompositionLocalProvider(androidx.compose.material3.LocalContentColor provides Tokens.text) {
                        Box(Modifier.semantics { testTagsAsResourceId = true }) { SftpContent() }
                    }
                }
            }
        }
        val bottom = ComposeView(this).apply {
            setContent {
                ShellTheme {
                    Box(Modifier.semantics { testTagsAsResourceId = true }) { Bottom() }
                }
            }
        }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(color(R.color.bg_base))
            addView(content, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
            addView(bottom, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        }
        // The bars sit right above the keyboard; the keyboard button follows it.
        root.setOnApplyWindowInsetsListener { view, insets ->
            if (Build.VERSION.SDK_INT >= 30) {
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.ime())
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
                keyboardShown = insets.isVisible(WindowInsets.Type.ime())
            } else {
                @Suppress("DEPRECATION")
                view.setPadding(insets.systemWindowInsetLeft, insets.systemWindowInsetTop, insets.systemWindowInsetRight, insets.systemWindowInsetBottom)
            }
            insets
        }
        setContentView(root)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                // A selection goes first; then the lists (D220).
                if (views[active]?.clearSelectionIfAny() == true) return
                finish()
            }
        })
        val kept = savedInstanceState?.getLong(STATE_SESSION, 0L)?.takeIf { it != 0L }
        if (kept != null && kept in app.sessions) {
            permissionsSettled = true
            show(kept)
            return
        }
        // Recreated after the intent was handled (an SFTP or a terminal still
        // opening then): the sessions are in App; nothing opens twice.
        if (savedInstanceState?.getBoolean(STATE_HANDLED) == true) {
            permissionsSettled = true
            app.sessions.maxOrNull()?.let(::show) ?: finish()
            return
        }
        // The notification of the foreground service (Android 13+) and, from
        // Android 17, the local network (a server at home, 10.x or 192.168.x):
        // asked before a session opens, which waits for the answer.
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Build.VERSION.SDK_INT >= 37) add(ACCESS_LOCAL_NETWORK)
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty() && intent.getLongExtra(EXTRA_SESSION, 0L) == 0L) {
            requestPermissions(wanted.toTypedArray(), PERMISSIONS)
        } else {
            permissionsSettled = true
        }
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handle(intent)
    }

    /** What an intent asks for: an open session, an SFTP session, or new terminal sessions. */
    private fun handle(intent: Intent) {
        val existing = intent.getLongExtra(EXTRA_SESSION, 0L)
        if (existing != 0L) {
            if (existing in app.sessions) show(existing) else if (app.sessions.isEmpty()) finish()
            return
        }
        val vaultId = intent.getStringExtra(EXTRA_VAULT_ID)
        if (intent.getBooleanExtra(EXTRA_SFTP, false)) {
            val recordId = intent.getStringExtra(EXTRA_RECORD_ID) ?: return finish()
            openSftp(vaultId ?: return finish(), recordId, intent.getStringExtra(EXTRA_TITLE) ?: "SFTP")
            return
        }
        val many = intent.getStringArrayListExtra(EXTRA_RECORD_IDS)
        if (many != null && vaultId != null) {
            val titles = intent.getStringArrayListExtra(EXTRA_TITLES) ?: arrayListOf()
            many.forEachIndexed { index, id -> queue.addLast(Target(vaultId, id, "", 22, "", titles.getOrElse(index) { "" })) }
        } else {
            val address = intent.getStringExtra(EXTRA_ADDRESS) ?: return finish()
            val username = intent.getStringExtra(EXTRA_USERNAME) ?: ""
            queue.addLast(
                Target(
                    vaultId, intent.getStringExtra(EXTRA_RECORD_ID), address, intent.getIntExtra(EXTRA_PORT, 22), username,
                    intent.getStringExtra(EXTRA_TITLE) ?: if (username.isEmpty()) address else "$username@$address",
                ),
            )
        }
        openNext()
    }

    @Deprecated("The result API is for Compose screens; this one asks once at start")
    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != PERMISSIONS) return
        val localDenied = permissions.indices.any {
            permissions[it] == ACCESS_LOCAL_NETWORK && grantResults.getOrNull(it) != PackageManager.PERMISSION_GRANTED
        }
        if (localDenied) Toast.makeText(this, R.string.local_network_denied, Toast.LENGTH_LONG).show()
        permissionsSettled = true
        openNext()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        if (active != 0L) outState.putLong(STATE_SESSION, active)
        outState.putBoolean(STATE_HANDLED, true)
    }

    // ----- sessions --------------------------------------------------------

    /** Opens the next queued terminal session once the screen has a size. */
    private fun openNext() {
        if (!permissionsSettled || opening != null) return
        val target = queue.removeFirstOrNull() ?: return
        val view = TerminalView(this, this)
        opening = view
        active = 0L
        openingSftp = null
        showView(view)
        var started = false
        view.onSize = { cols, rows ->
            if (!started) {
                started = true
                open(view, target, cols, rows)
            }
        }
    }

    private fun open(view: TerminalView, target: Target, cols: Int, rows: Int) {
        worker.execute {
            val id = try {
                if (target.vaultId != null && target.recordId != null) {
                    Native.openHost(target.vaultId, target.recordId, cols, rows, isDark)
                } else {
                    Native.open(target.address, target.port, target.username, cols, rows, isDark)
                }
            } catch (error: RuntimeException) {
                runOnUiThread {
                    Toast.makeText(this, error.message ?: getString(R.string.connecting), Toast.LENGTH_LONG).show()
                    if (opening == view) opening = null
                    view.release()
                    afterOpen()
                }
                return@execute
            }
            runOnUiThread {
                // Gone (Back) while it opened: the session goes on in the
                // list, as one left with "<" (the bar's sessions have it).
                val title = target.title.ifEmpty { target.address }
                app.opened(id, title, target.recordId)
                if (isDestroyed || isFinishing) {
                    view.release()
                    return@runOnUiThread
                }
                if (opening == view) opening = null
                views[id] = view
                app.listen(id) { event -> onEvent(id, event) }
                view.attach(id)
                show(id)
                view.showKeyboard()
                afterOpen()
            }
        }
    }

    private fun afterOpen() {
        if (queue.isNotEmpty()) openNext() else if (views.isEmpty() && app.sessions.isEmpty() && openingSftp == null) finish()
    }

    /** Shows session [id]: its terminal, made here when another screen opened it, or its files. */
    private fun show(id: Long) {
        if (id in app.sftpSessions) {
            active = id
            openingSftp = null
            app.activeSessionId.value = id
            showView(sftpView)
            hideKeyboard()
            passwordState = Native.PASSWORD_QUICK
            return
        }
        val view = views.getOrPut(id) {
            TerminalView(this, this).also { made ->
                app.listen(id) { event -> onEvent(id, event) }
                made.attach(id)
            }
        }
        active = id
        openingSftp = null
        app.activeSessionId.value = id
        showView(view)
        view.cursorPad = cursorOn
        view.requestFocus()
        // The key does not show the session left behind while it asks
        // (review of run 16).
        passwordState = Native.PASSWORD_NOT_CONNECTED
        refreshPassword()
    }

    private fun showView(view: View) {
        if (content.childCount == 1 && content.getChildAt(0) == view) return
        content.removeAllViews()
        content.addView(view, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
    }

    private fun hideKeyboard() {
        getSystemService(android.view.inputmethod.InputMethodManager::class.java)
            ?.hideSoftInputFromWindow(content.windowToken, 0)
    }

    /** × on the active chip: the session closes at once (D220). */
    private fun close(id: Long) {
        worker.execute { runCatching { Native.close(id) } }
        app.forget(id)
        gone(id)
    }

    /** Session [id] ended: its view goes; the bar shows another session, or the lists come back. */
    private fun gone(id: Long) {
        views.remove(id)?.let { view ->
            app.unlisten(id)
            view.release()
        }
        if (active != id) return
        val next = app.sessions.filter { it != id }.maxOrNull()
        if (next != null) show(next) else if (opening == null && openingSftp == null && queue.isEmpty()) finish()
    }

    private fun onEvent(id: Long, event: JSONObject) {
        val state = event.optString("state")
        if (state == "closed") {
            gone(id)
            return
        }
        val prompt = event.optJSONObject("prompt") ?: return
        val promptId = prompt.getLong("id")
        if (answered[id] == promptId) return
        answered[id] = promptId
        // A question of another session: that session comes to the front first.
        if (id != active) show(id)
        when (prompt.getString("kind")) {
            "host_key_unknown" -> if (testAcceptHostKey) {
                reply(id, promptId, "accept", null)
            } else {
                hostKeyDialog(id, promptId, prompt, changed = false)
            }
            "host_key_changed" -> hostKeyDialog(id, promptId, prompt, changed = true)
            "password" -> {
                val ready = if (prompt.optBoolean("error")) null else testPassword
                testPassword = null
                if (ready != null) {
                    reply(id, promptId, "text", ready)
                } else {
                    textDialog(id, promptId, getString(R.string.password_for, prompt.optString("username")), true, prompt.optBoolean("error"))
                }
            }
            "passphrase" -> textDialog(id, promptId, getString(R.string.passphrase_for, prompt.optString("keyLabel")), true, prompt.optBoolean("error"))
            "keyboard_interactive" -> textDialog(id, promptId, prompt.optString("prompt"), !prompt.optBoolean("echo"), prompt.optBoolean("error"))
            "username" -> textDialog(id, promptId, getString(R.string.username), false, false)
            else -> reply(id, promptId, "cancel", null)
        }
    }

    private fun reply(id: Long, prompt: Long, kind: String, text: ByteArray?) {
        worker.execute {
            try {
                Native.reply(id, prompt, kind, text)
                // A host key saved or replaced is a known host to sync.
                if (kind == "accept" || kind == "replace") Core.onLocalChange?.invoke()
            } catch (_: RuntimeException) {
            } finally {
                text?.fill(0)
            }
        }
    }

    /**
     * The host key questions of UI spec 8 (the desktop's 10.2), which a tap
     * outside does not dismiss: a first key offers Accept and save, Connect
     * once, Cancel; a changed key warns with the old and new fingerprints,
     * offers Cancel first, and replaces the key only after a second
     * confirmation.
     */
    private fun hostKeyDialog(id: Long, prompt: Long, info: JSONObject, changed: Boolean) {
        val host = app.sessionViews.value[id]?.optString("title")?.ifBlank { null } ?: getString(R.string.this_host)
        if (!changed) {
            AlertDialog.Builder(this)
                .setTitle(R.string.host_key_unknown)
                .setMessage("$host\n${info.optString("keyType")}\n${info.optString("fingerprint")}")
                .setCancelable(false)
                .setPositiveButton(R.string.accept_and_save) { _, _ -> reply(id, prompt, "accept", null) }
                .setNeutralButton(R.string.connect_once) { _, _ -> reply(id, prompt, "once", null) }
                .setNegativeButton(R.string.cancel) { _, _ -> reply(id, prompt, "cancel", null) }
                .show()
            return
        }
        val warning = getString(R.string.host_key_changed_warning, host) + "\n\n" +
            getString(R.string.host_key_old) + "\n${info.optString("oldKeyType")}\n${info.optString("oldFingerprint")}\n\n" +
            getString(R.string.host_key_new) + "\n${info.optString("keyType")}\n${info.optString("fingerprint")}"
        val dialog = AlertDialog.Builder(this)
            .setTitle(R.string.host_key_changed)
            .setMessage(warning)
            .setCancelable(false)
            .setPositiveButton(R.string.cancel) { _, _ -> reply(id, prompt, "cancel", null) }
            .setNegativeButton(R.string.replace_key_ellipsis) { _, _ ->
                AlertDialog.Builder(this)
                    .setTitle(R.string.replace_key_confirm)
                    .setMessage(getString(R.string.replace_key_confirm_text, host))
                    .setCancelable(false)
                    .setPositiveButton(R.string.cancel) { _, _ -> reply(id, prompt, "cancel", null) }
                    .setNegativeButton(R.string.replace_key) { _, _ -> reply(id, prompt, "replace", null) }
                    .show()
                    .getButton(AlertDialog.BUTTON_NEGATIVE)?.setTextColor(color(R.color.danger))
            }
            .show()
        dialog.getButton(AlertDialog.BUTTON_NEGATIVE)?.setTextColor(color(R.color.danger))
    }

    private fun textDialog(id: Long, prompt: Long, title: String, masked: Boolean, wrong: Boolean) {
        val field = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or
                if (masked) InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        }
        AlertDialog.Builder(this)
            .setTitle(title)
            .setMessage(if (wrong) getString(R.string.wrong_password) else null)
            .setView(field)
            .setCancelable(false)
            .setPositiveButton(R.string.ok) { _, _ ->
                val editable = field.text
                val chars = CharArray(editable.length).also { android.text.TextUtils.getChars(editable, 0, editable.length, it, 0) }
                val encoded = Charsets.UTF_8.encode(java.nio.CharBuffer.wrap(chars))
                val bytes = ByteArray(encoded.remaining()).also { encoded.get(it) }
                chars.fill('\u0000')
                if (encoded.hasArray()) encoded.array().fill(0)
                editable.clear()
                reply(id, prompt, "text", bytes)
            }
            .setNegativeButton(R.string.cancel) { _, _ -> reply(id, prompt, "cancel", null) }
            .show()
    }

    // ----- SFTP --------------------------------------------------------------

    /** A new SFTP session to a saved host, shown here (reference-2 8; D220: the terminal stays). */
    private fun openSftp(vaultId: String, recordId: String, title: String) {
        openingSftp = Record(
            JSONObject().put("record_id", recordId).put("vault_id", vaultId).put("type", "host")
                .put("data", JSONObject().put("label", title)),
        )
        active = 0L
        showView(sftpView)
        hideKeyboard()
    }

    @Composable
    private fun SftpContent() {
        val pending = openingSftp
        val shown = active.takeIf { it in app.sftpSessions }
        if (pending == null && shown == null) return
        androidx.compose.runtime.key(pending?.id ?: shown) {
            SftpScreen(
                app, this, host = if (shown == null) pending else null, existing = shown,
                onClose = { finish() },
                onOpened = { id ->
                    openingSftp = null
                    show(id)
                },
                onEnded = { id -> gone(id) },
            )
        }
    }

    // ----- the bars ----------------------------------------------------------

    @Composable
    private fun Bottom() {
        val sessionViews by app.sessionViews.collectAsState()
        val changes by app.changes.collectAsState()
        val records = sessionViews.values.mapNotNull { it.optString("recordId").ifBlank { null } }.toSet()
        // Again after a change: OS detection writes the host after connecting.
        val marks by produceState(mapOf<String, OsMark>(), records, changes) {
            value = withContext(Dispatchers.IO) { records.associateWith { hostMark(it) } }
        }
        val sessions = sessionViews.values.sortedBy { it.optLong("id") }.map { view ->
            val id = view.optLong("id")
            BarSession(
                id, view.optString("title").ifBlank { "Session" }, view.optString("state"), id in app.sftpSessions,
                marks[view.optString("recordId")] ?: OsMark(null, null),
            )
        }
        val onTerminal = openingSftp == null && active !in app.sftpSessions
        val theme = TerminalPrefs.theme(this, isDark)
        val shown = TerminalView.colors[active]
        val colors = if (onTerminal) {
            val bg = shown?.let { Color(it.first) } ?: theme.bg
            val fg = shown?.let { Color(it.second) } ?: theme.fg
            BarColors(bg, fg.copy(alpha = 0.10f), fg, Tokens.accent, fg.copy(alpha = 0.6f))
        } else {
            BarColors(Tokens.bgBase, Tokens.bgRaised, Tokens.text, Tokens.accent, Tokens.textSecondary)
        }
        Column {
            SessionBar(
                sessions, active, colors, keyboardShown, showKeyboardButton = onTerminal,
                onBack = { finish() },
                onSwitch = { show(it) },
                onClose = { close(it) },
                onConnect = { connectAgain() },
                onSftp = { sftpAgain() },
                onKeyboard = { terminal.toggleKeyboard() },
            )
            if (onTerminal) {
                val saved = sessionViews[active]?.optString("recordId")?.isNotBlank() == true
                // A session that connects, drops or reconnects may start or
                // stop its recording: the key asks again (D223 item 13).
                val state = sessionViews[active]?.optString("state")
                val recording = sessionViews[active]?.optBoolean("recording")
                LaunchedEffect(active, state, recording) { refreshPassword() }
                ExtraKeysRow(
                    keyGroups, colors, this@TerminalActivity, modifiersVersion,
                    password = passwordState == Native.PASSWORD_SENT, passwordWhy = passwordReason(passwordState), attach = saved,
                )
            }
        }
        confirmingPassword?.let { session ->
            // Another session came to the front: the confirmation goes, it
            // never sends to a session it did not name (review of 3.3).
            if (session != active) confirmingPassword = null else PasswordConfirm(session)
        }
        attachFolder?.let { (session, home, host) -> AttachDialog(session, home, host) }
    }

    /** "+" → Connect: a new session to the host of the active one (reference-2 7.6). */
    private fun connectAgain() {
        val view = app.sessionViews.value[active] ?: return
        val recordId = view.optString("recordId").ifBlank { null }
        if (recordId == null) {
            Toast.makeText(this, R.string.connect_again_saved_only, Toast.LENGTH_LONG).show()
            return
        }
        withVault { vaultId ->
            queue.addLast(Target(vaultId, recordId, "", 22, "", view.optString("title")))
            openNext()
        }
    }

    /** The vault's id, read off the main thread, then [then] on it. */
    private fun withVault(then: (String) -> Unit) {
        Thread({
            val vault = recordVault()
            runOnUiThread { if (vault != null && !isFinishing) then(vault) }
        }, "vault-id").start()
    }

    /** "+" → Connect via SFTP: a new SFTP session; the terminal session stays (D220). */
    private fun sftpAgain() {
        val view = app.sessionViews.value[active] ?: return
        val recordId = view.optString("recordId").ifBlank { null }
        if (recordId == null) {
            Toast.makeText(this, R.string.connect_again_saved_only, Toast.LENGTH_LONG).show()
            return
        }
        withVault { vaultId -> openSftp(vaultId, recordId, view.optString("title")) }
    }

    private fun recordVault(): String? = runCatching { Core.array("vaults").getJSONObject(0).getString("vault_id") }.getOrNull()

    // ----- keys --------------------------------------------------------------

    override val ctrl: Boolean get() = ctrlKey.active
    override val alt: Boolean get() = altKey.active

    override fun consume() {
        ctrlKey.consume()
        altKey.consume()
        modifiersVersion += 1
    }

    override val ctrlState: StickyModifier.State get() = ctrlKey.state
    override val altState: StickyModifier.State get() = altKey.state
    override val cursorMode: Boolean get() = cursorOn

    override fun target(): Long = active

    override fun key(key: XKey, session: Long) {
        // The session switched since the press: nothing goes to another one.
        if (session != active) return
        val view = views[session] ?: return
        when (key) {
            // A key that cannot send says why (D223 item 13); Send checks
            // again in Rust, whatever the key showed.
            XKey.Password -> if (passwordState == Native.PASSWORD_SENT) {
                confirmingPassword = session
            } else {
                passwordReason(passwordState)?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                refreshPassword()
            }
            XKey.Cursor -> {
                cursorOn = !cursorOn
                view.cursorPad = cursorOn
            }
            XKey.Attach -> {
                attachFor = session
                pickFiles.launch(arrayOf("*/*"))
            }
            XKey.Ctrl -> {
                ctrlKey.tap(System.currentTimeMillis())
                modifiersVersion += 1
            }
            XKey.Alt -> {
                altKey.tap(System.currentTimeMillis())
                modifiersVersion += 1
            }
            is XKey.Code -> view.sendKey(key.code)
            is XKey.Text -> repeat(key.times) { view.sendText(key.text, ctrl = key.ctrl) }
            // A tmux prefix sequence as it is, without the armed Ctrl or Alt.
            is XKey.Sequence -> key.steps.forEach { view.sendText(it.text, ctrl = it.ctrl, raw = true) }
        }
        modifiersVersion += 1
    }

    /**
     * What the password key can do (D220, D223 item 13): send when the
     * session is connected to its saved host, does not record its output
     * (the session layer says) and the host has an effective password (its
     * own or its identity's).
     */
    private fun refreshPassword() {
        val session = active
        val recordId = app.sessionViews.value[session]?.optString("recordId")?.ifBlank { null }
        if (recordId == null) {
            passwordState = Native.PASSWORD_QUICK
            return
        }
        worker.execute {
            val state = runCatching {
                val vault = recordVault() ?: return@runCatching Native.PASSWORD_FAILED
                Native.passwordState(session, vault, recordId)
            }.getOrDefault(Native.PASSWORD_FAILED)
            // An answer for a session no longer in front changes nothing.
            runOnUiThread { if (active == session) passwordState = state }
        }
    }

    /** Why the password key cannot send, or null when it can. */
    private fun passwordReason(state: Int): String? = when (state) {
        Native.PASSWORD_SENT -> null
        Native.PASSWORD_QUICK -> getString(R.string.password_key_quick)
        Native.PASSWORD_NONE -> getString(R.string.password_key_none)
        Native.PASSWORD_RECORDED -> getString(R.string.password_not_sent_recorded)
        Native.PASSWORD_NOT_CONNECTED -> getString(R.string.password_not_sent_offline)
        else -> getString(R.string.password_not_sent)
    }

    /**
     * The password key's confirmation (D220): it names the host; the
     * password goes only after Send, once and without Enter, from the vault
     * straight to the session ([Native.sendPassword]).
     */
    @Composable
    private fun PasswordConfirm(session: Long) {
        val view = app.sessionViews.value[session]
        val title = view?.optString("title") ?: ""
        ComposeAlertDialog(
            onDismissRequest = { confirmingPassword = null },
            title = { Text(getString(R.string.send_password_title)) },
            text = { Text(getString(R.string.send_password_text, title)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmingPassword = null
                    val notSent = { answer: Int ->
                        runOnUiThread { Toast.makeText(this, passwordNotSent(answer), Toast.LENGTH_LONG).show() }
                    }
                    val recordId = view?.optString("recordId")?.ifBlank { null }
                    if (recordId == null) {
                        notSent(Native.PASSWORD_FAILED)
                        return@TextButton
                    }
                    // A thread of its own: never queued behind other work, so
                    // it goes right after the tap or not at all.
                    Thread({
                        val vault = recordVault()
                        val answer = if (vault == null) {
                            Native.PASSWORD_FAILED
                        } else {
                            runCatching { Native.sendPassword(session, vault, recordId) }.getOrDefault(Native.PASSWORD_FAILED)
                        }
                        if (answer != Native.PASSWORD_SENT) notSent(answer)
                    }, "password-key").start()
                }, modifier = Modifier.testTag("password_send")) { Text(getString(R.string.send)) }
            },
            dismissButton = { TextButton(onClick = { confirmingPassword = null }) { Text(getString(R.string.cancel)) } },
        )
    }

    /**
     * The attach key (D220 item 6): files from the system picker go by SFTP
     * to the user's home folder on this server, or the folder typed here.
     * An SFTP session to the host opens beside the terminal for it (its chip
     * shows the transfers); the shell's current folder is not guessed.
     */
    private fun startAttach(uris: List<Uri>) {
        // The session the key was pressed on, not the one in front now
        // (another may have come up while the picker was open).
        val terminalSession = attachFor
        attachFor = 0L
        val view = app.sessionViews.value[terminalSession]
        val recordId = view?.optString("recordId")?.ifBlank { null }
        if (view == null || recordId == null || terminalSession !in app.sessions) {
            Toast.makeText(this, R.string.attach_session_gone, Toast.LENGTH_LONG).show()
            return
        }
        attachUris = uris
        val host = view.optString("title")
        Toast.makeText(this, R.string.attach_connecting, Toast.LENGTH_SHORT).show()
        // A thread of its own: the wait below never holds up other work.
        Thread({
            val vault = recordVault() ?: return@Thread
            // An SFTP session to this host that is open already, else a new one.
            val existing = app.sessionViews.value.values.firstOrNull {
                it.optLong("id") in app.sftpSessions && it.optString("recordId") == recordId && it.optString("state") == "connected"
            }?.optLong("id")
            val sftp = existing ?: try {
                Native.openSftp(vault, recordId).also { if (it != 0L) runOnUiThread { app.opened(it, view.optString("title"), recordId, sftp = true) } }
            } catch (error: RuntimeException) {
                0L
            }
            if (sftp == 0L) {
                runOnUiThread { Toast.makeText(this, R.string.attach_failed, Toast.LENGTH_LONG).show() }
                return@Thread
            }
            val deadline = System.currentTimeMillis() + 30_000
            var connected = false
            while (System.currentTimeMillis() < deadline) {
                val state = app.sessionViews.value[sftp]
                if (state?.optString("state") == "connected") {
                    connected = true
                    break
                }
                if (state?.optJSONObject("prompt") != null || state?.optString("state") == "closed") {
                    // It asks something (or failed): its screen answers, and
                    // the files wait for the attach key again.
                    runOnUiThread {
                        show(sftp)
                        Toast.makeText(this, R.string.attach_answer_first, Toast.LENGTH_LONG).show()
                    }
                    return@Thread
                }
                Thread.sleep(200)
            }
            if (!connected) {
                runOnUiThread { Toast.makeText(this, R.string.attach_failed, Toast.LENGTH_LONG).show() }
                return@Thread
            }
            val home = runCatching { Core.answer(Native.sftpCall(sftp, "home", "{}")) as String }.getOrNull()
            runOnUiThread {
                if (home == null) Toast.makeText(this, R.string.attach_failed, Toast.LENGTH_LONG).show() else attachFolder = Triple(sftp, home, host)
            }
        }, "attach").start()
    }

    @Composable
    private fun AttachDialog(session: Long, home: String, host: String) {
        var folder by remember { mutableStateOf(home) }
        val count = attachUris.size
        ComposeAlertDialog(
            onDismissRequest = { attachFolder = null },
            // The host is named: the files go to its server (review of 3.3).
            title = { Text(resources.getQuantityString(R.plurals.attach_title, count, count, host)) },
            text = {
                OutlinedTextField(folder, { folder = it }, label = { Text(getString(R.string.attach_folder)) }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("attach_folder"))
            },
            confirmButton = {
                TextButton(enabled = folder.isNotBlank(), onClick = {
                    attachFolder = null
                    app.upload(session, folder.trim(), attachUris) { problem -> Toast.makeText(this, problem, Toast.LENGTH_LONG).show() }
                    Toast.makeText(this, getString(R.string.attach_started, folder.trim()), Toast.LENGTH_LONG).show()
                }, modifier = Modifier.testTag("attach_upload")) { Text(getString(R.string.upload)) }
            },
            dismissButton = { TextButton(onClick = { attachFolder = null }) { Text(getString(R.string.cancel)) } },
        )
    }

    // ----- lifecycle ---------------------------------------------------------

    /** Locked while shown (auto-lock, Lock): the terminal goes and the unlock screen comes (UI spec 5). */
    private val onLock: (LockStatus) -> Unit = { status ->
        if (!status.unlocked && !isFinishing) leaveForUnlock()
    }

    private fun leaveForUnlock() {
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP))
        finish()
    }

    override fun onStart() {
        super.onStart()
        // Locked: a live terminal asks for the password first (UI spec 5);
        // the session keeps running meanwhile.
        app.onLockChange(onLock)
        if (app.ready && !app.lock.value.unlocked) leaveForUnlock()
        // Snapshots of the screen only while it is seen (M2).
        for (view in views.values) view.paused = false
        keyGroups = XKey.groups(this)
        if (TerminalPrefs.of(this).getBoolean(TerminalPrefs.KEEP_ON, false)) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    override fun onStop() {
        super.onStop()
        app.removeLockListener(onLock)
        for (view in views.values) view.paused = true
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) app.noteActivity()
        return super.dispatchTouchEvent(event)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        app.noteActivity()
        return super.dispatchKeyEvent(event)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        app.platform.focused = hasFocus
    }

    override fun onDestroy() {
        super.onDestroy()
        // Back and "<" leave the sessions running (the bar and Connections have them).
        for ((id, view) in views) {
            app.unlisten(id)
            view.release()
        }
        opening?.release()
        worker.shutdown()
    }

    companion object {
        const val EXTRA_ADDRESS = "address"
        const val EXTRA_PORT = "port"
        const val EXTRA_USERNAME = "username"
        const val EXTRA_VAULT_ID = "vaultId"
        const val EXTRA_RECORD_ID = "recordId"
        const val EXTRA_RECORD_IDS = "recordIds"
        const val EXTRA_TITLES = "titles"
        const val EXTRA_TITLE = "title"
        const val EXTRA_SESSION = "session"
        const val EXTRA_SFTP = "sftp"
        const val EXTRA_TEST_PASSWORD = "test.password"
        private const val PERMISSIONS = 1
        private const val STATE_SESSION = "session"
        private const val STATE_HANDLED = "handled"
        const val EXTRA_TEST_ACCEPT_HOST_KEY = "test.acceptHostKey"
        const val ACCESS_LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
    }
}
