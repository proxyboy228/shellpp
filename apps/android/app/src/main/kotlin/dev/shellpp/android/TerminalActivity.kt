package dev.shellpp.android

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.content.Intent
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * An SSH session in the terminal: a saved host's or a quick connect's
 * (UI spec 6.1), or an open one from Sessions. The session's state and
 * Close in a thin top bar, the terminal, the extra keys; the connection's
 * questions (host key, password) as dialogs, as the desktop asks them.
 * Back leaves the session running (UI spec 3).
 */
class TerminalActivity : Activity(), ExtraKeys.Sink {
    lateinit var terminal: TerminalView
        private set
    private lateinit var keys: ExtraKeys
    private lateinit var status: TextView
    private val worker = Executors.newSingleThreadExecutor()
    /** The session shown here (0 until it opened). */
    var session = 0L
        private set
    private var openedOnce = false
    private var answeredPrompt = -1L

    /** The size laid out, once known; the session opens with it. */
    private var size: Pair<Int, Int>? = null

    /** The permission dialog was answered (or there was none to ask). */
    private var permissionsSettled = false

    /** Test-only answers (a debuggable build only): the e2e test's password and host key. */
    private var testPassword: ByteArray? = null
    private var testAcceptHostKey = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val debuggable = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        if (debuggable) {
            testPassword = intent.getByteArrayExtra(EXTRA_TEST_PASSWORD)
            testAcceptHostKey = intent.getBooleanExtra(EXTRA_TEST_ACCEPT_HOST_KEY, false)
        }
        keys = ExtraKeys(this, this)
        terminal = TerminalView(this, keys)
        status = TextView(this).apply {
            textSize = 14f
            setTextColor(color(R.color.text_secondary))
            setPadding(dp(12f), dp(8f), dp(12f), dp(8f))
            text = getString(R.string.connecting)
            setSingleLine()
        }
        val close = Button(this).apply {
            text = getString(R.string.close)
            isAllCaps = false
            textSize = 14f
            background = null
            setTextColor(color(R.color.accent))
            setOnClickListener { closeSession() }
        }
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(status, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
            addView(close, LinearLayout.LayoutParams(WRAP_CONTENT, dp(48f)))
        }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(color(R.color.bg_base))
            addView(bar, LinearLayout.LayoutParams(MATCH_PARENT, dp(48f)))
            addView(terminal, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
            addView(keys, LinearLayout.LayoutParams(MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
            padForInsets()
        }
        setContentView(root)
        terminal.onSize = { cols, rows ->
            size = Pair(cols, rows)
            openWhenReady()
        }
        terminal.requestFocus()
        // Recreated (split screen, a fold, a font or language change): the
        // session this screen had goes on, it is not opened again (review
        // of run 13, architecture 3).
        // An open session from Sessions is shown again, not opened anew.
        val kept = savedInstanceState?.getLong(STATE_SESSION, 0L)?.takeIf { it != 0L }
            ?: intent.getLongExtra(EXTRA_SESSION, 0L)
        if (kept != 0L) {
            openedOnce = true
            permissionsSettled = true
            if (kept in (application as App).sessions) adopt(kept) else finish()
            return
        }
        // The notification of the foreground service (Android 13+) and, from
        // Android 17, the local network (a server at home, 10.x or 192.168.x):
        // asked before the session opens, which waits for the answer.
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Build.VERSION.SDK_INT >= 37) add(ACCESS_LOCAL_NETWORK)
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) {
            requestPermissions(wanted.toTypedArray(), PERMISSIONS)
        } else {
            permissionsSettled = true
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != PERMISSIONS) return
        val localDenied = permissions.indices.any {
            permissions[it] == ACCESS_LOCAL_NETWORK && grantResults.getOrNull(it) != PackageManager.PERMISSION_GRANTED
        }
        if (localDenied) status.text = getString(R.string.local_network_denied)
        permissionsSettled = true
        openWhenReady()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        if (session != 0L) outState.putLong(STATE_SESSION, session)
    }

    private fun openWhenReady() {
        val (cols, rows) = size ?: return
        if (openedOnce || !permissionsSettled) return
        open(cols, rows)
    }

    private fun open(cols: Int, rows: Int) {
        openedOnce = true
        val vaultId = intent.getStringExtra(EXTRA_VAULT_ID)
        val recordId = intent.getStringExtra(EXTRA_RECORD_ID)
        val address = intent.getStringExtra(EXTRA_ADDRESS) ?: return finish()
        val port = intent.getIntExtra(EXTRA_PORT, 22)
        val username = intent.getStringExtra(EXTRA_USERNAME) ?: ""
        val title = intent.getStringExtra(EXTRA_TITLE) ?: if (username.isEmpty()) address else "$username@$address"
        worker.execute {
            val id = try {
                if (vaultId != null && recordId != null) {
                    Native.openHost(vaultId, recordId, cols, rows, isDark)
                } else {
                    Native.open(address, port, username, cols, rows, isDark)
                }
            } catch (error: RuntimeException) {
                runOnUiThread { status.text = error.message }
                return@execute
            }
            runOnUiThread {
                // Closed (Back) while it opened: the session goes too.
                if (isDestroyed || isFinishing) {
                    runCatching { Native.close(id) }
                    return@runOnUiThread
                }
                (application as App).opened(id, title, recordId)
                adopt(id)
            }
        }
    }

    /** Shows session [id] here: its events and its terminal. */
    private fun adopt(id: Long) {
        session = id
        (application as App).listen(id, ::onEvent)
        terminal.attach(id)
    }

    private fun onEvent(event: JSONObject) {
        val state = event.optString("state")
        val error = event.optString("error").takeIf { !event.isNull("error") && it.isNotEmpty() }
        status.text = when (state) {
            "connecting" -> getString(R.string.connecting)
            "connected" -> event.optString("title")
            "dropped" -> listOfNotNull(error, event.optInt("reconnectIn", -1).takeIf { it >= 0 }?.let { "↻ $it s" })
                .joinToString(" · ")
            else -> listOfNotNull(event.optString("title"), error).joinToString(" · ")
        }
        if (state == "closed") {
            finish()
            return
        }
        val prompt = event.optJSONObject("prompt") ?: return
        val promptId = prompt.getLong("id")
        if (promptId == answeredPrompt) return
        answeredPrompt = promptId
        when (prompt.getString("kind")) {
            "host_key_unknown" -> if (testAcceptHostKey) {
                reply(promptId, "accept", null)
            } else {
                hostKeyDialog(promptId, prompt, changed = false)
            }
            "host_key_changed" -> hostKeyDialog(promptId, prompt, changed = true)
            "password" -> {
                val ready = if (prompt.optBoolean("error")) null else testPassword
                testPassword = null
                if (ready != null) {
                    reply(promptId, "text", ready)
                } else {
                    textDialog(promptId, getString(R.string.password_for, prompt.optString("username")), true, prompt.optBoolean("error"))
                }
            }
            "passphrase" -> textDialog(promptId, getString(R.string.passphrase_for, prompt.optString("keyLabel")), true, prompt.optBoolean("error"))
            "keyboard_interactive" -> textDialog(promptId, prompt.optString("prompt"), !prompt.optBoolean("echo"), prompt.optBoolean("error"))
            "username" -> textDialog(promptId, getString(R.string.username), false, false)
            else -> reply(promptId, "cancel", null)
        }
    }

    private fun reply(prompt: Long, kind: String, text: ByteArray?) {
        val id = session
        worker.execute {
            try {
                Native.reply(id, prompt, kind, text)
            } catch (_: RuntimeException) {
            } finally {
                text?.fill(0)
            }
        }
    }

    private fun hostKeyDialog(prompt: Long, info: JSONObject, changed: Boolean) {
        val message = buildString {
            append(intent.getStringExtra(EXTRA_ADDRESS)).append('\n').append(info.optString("keyType")).append('\n')
            append(info.optString("fingerprint"))
            if (changed) append("\n\n").append(info.optString("oldKeyType")).append('\n').append(info.optString("oldFingerprint"))
        }
        val builder = AlertDialog.Builder(this)
            .setTitle(if (changed) R.string.host_key_changed else R.string.host_key_unknown)
            .setMessage(message)
            .setCancelable(false)
            .setNegativeButton(R.string.cancel) { _, _ -> reply(prompt, "cancel", null) }
        if (changed) {
            builder.setPositiveButton(R.string.replace_key) { _, _ -> reply(prompt, "replace", null) }
        } else {
            builder.setPositiveButton(R.string.accept_and_save) { _, _ -> reply(prompt, "accept", null) }
            builder.setNeutralButton(R.string.connect_once) { _, _ -> reply(prompt, "once", null) }
        }
        builder.show()
    }

    private fun textDialog(prompt: Long, title: String, masked: Boolean, wrong: Boolean) {
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
                reply(prompt, "text", bytes)
            }
            .setNegativeButton(R.string.cancel) { _, _ -> reply(prompt, "cancel", null) }
            .show()
    }

    /** Close in the bar: the session ends and the screen goes. */
    private fun closeSession() {
        val id = session
        if (id != 0L) {
            runCatching { Native.close(id) }
            (application as App).forget(id)
        }
        finish()
    }

    override fun onStart() {
        super.onStart()
        // Locked: a live terminal asks for the password first (UI spec 5);
        // the session keeps running meanwhile.
        val app = application as App
        if (app.ready && !app.lock.value.unlocked) {
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP))
            finish()
        }
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) (application as App).noteActivity()
        return super.dispatchTouchEvent(event)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        (application as App).noteActivity()
        return super.dispatchKeyEvent(event)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        (application as App).platform.focused = hasFocus
    }

    // ExtraKeys.Sink
    override fun key(code: Int) = terminal.sendKey(code)
    override fun text(text: String) = terminal.sendText(text)
    override fun toggleKeyboard() = terminal.toggleKeyboard()

    override fun onDestroy() {
        super.onDestroy()
        val app = application as App
        // Back leaves the session running (Sessions has it); Close ends it.
        if (session != 0L) app.unlisten(session)
        terminal.release()
        worker.shutdown()
    }

    companion object {
        const val EXTRA_ADDRESS = "address"
        const val EXTRA_PORT = "port"
        const val EXTRA_USERNAME = "username"
        const val EXTRA_VAULT_ID = "vaultId"
        const val EXTRA_RECORD_ID = "recordId"
        const val EXTRA_TITLE = "title"
        const val EXTRA_SESSION = "session"
        const val EXTRA_TEST_PASSWORD = "test.password"
        private const val PERMISSIONS = 1
        private const val STATE_SESSION = "session"
        const val EXTRA_TEST_ACCEPT_HOST_KEY = "test.acceptHostKey"
        const val ACCESS_LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
    }
}
