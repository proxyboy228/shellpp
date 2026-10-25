package dev.shellpp.android

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import java.io.File

/**
 * The prototype's quick connect (plan 3.7): address, port, user, and a
 * password or a key file; no vault and no sync yet.
 */
class MainActivity : Activity() {
    private lateinit var address: EditText
    private lateinit var port: EditText
    private lateinit var username: EditText
    private lateinit var password: EditText
    private lateinit var keyLabel: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val app = application as App
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16f), dp(24f), dp(16f), dp(16f))
        }
        column.addView(text(getString(R.string.quick_connect), 22f, R.color.text_primary))
        column.addView(text(getString(R.string.prototype_note), 14f, R.color.text_secondary).apply {
            setPadding(0, dp(4f), 0, dp(16f))
        })
        address = field(R.string.address, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        port = field(R.string.port, InputType.TYPE_CLASS_NUMBER).apply { setText("22") }
        username = field(R.string.username, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD)
        password = field(R.string.password, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        for (view in listOf(address, port, username, password)) column.addView(view)
        keyLabel = text("", 14f, R.color.text_secondary)
        column.addView(button(getString(R.string.key_file), primary = false) { pickKey() })
        column.addView(keyLabel)
        column.addView(button(getString(R.string.connect), primary = true) { connect() })
        val scroll = ScrollView(this).apply {
            setBackgroundColor(color(R.color.bg_base))
            addView(column)
            padForInsets()
        }
        setContentView(scroll)
    }

    private fun text(value: String, size: Float, colorId: Int) = TextView(this).apply {
        text = value
        textSize = size
        setTextColor(color(colorId))
    }

    private fun field(hint: Int, type: Int) = EditText(this).apply {
        setHint(hint)
        inputType = type
        setSingleLine()
        textSize = 16f
        setTextColor(color(R.color.text_primary))
        setHintTextColor(color(R.color.text_muted))
        background = rounded(color(R.color.bg_input), stroke = color(R.color.border_strong))
        setPadding(dp(12f), dp(12f), dp(12f), dp(12f))
        layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(8f) }
    }

    private fun button(label: String, primary: Boolean, onClick: () -> Unit) = Button(this).apply {
        text = label
        isAllCaps = false
        textSize = 16f
        minHeight = dp(48f)
        gravity = Gravity.CENTER
        if (primary) {
            setTextColor(color(R.color.text_on_accent))
            background = rounded(color(R.color.accent))
        } else {
            setTextColor(color(R.color.text_primary))
            background = rounded(color(R.color.bg_raised), stroke = color(R.color.border_strong))
        }
        layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(12f) }
        setOnClickListener { onClick() }
    }

    /** The engine runs; otherwise says why not (it failed, or it still starts). */
    private fun engineReady(): Boolean {
        val app = application as App
        if (app.ready) return true
        val error = app.startError
        val message = if (error != null) getString(R.string.engine_failed, error) else getString(R.string.starting)
        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
        return false
    }

    private fun pickKey() {
        if (!engineReady()) return
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, PICK_KEY)
    }

    @Deprecated("Activity's own result API; the prototype has no AndroidX")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri: Uri = data?.data ?: return
        if (requestCode != PICK_KEY || resultCode != RESULT_OK) return
        // The session layer offers the key files of its key folder (as
        // `~/.ssh` on the desktop); the picked file goes there, private to the app.
        // Off the main thread; the copy lasts this run of the app (the
        // engine empties the folder at its start).
        val label = uri.lastPathSegment ?: "key"
        Thread {
            val bytes = runCatching { contentResolver.openInputStream(uri)?.use { it.readBytes() } }.getOrNull()
                ?: return@Thread
            runCatching { File(Native.keyDir(), "id_picked").writeBytes(bytes) }
            bytes.fill(0)
            runOnUiThread { keyLabel.text = getString(R.string.key_file_set, label) }
        }.start()
    }

    private fun connect() {
        val host = address.text.toString().trim()
        if (host.isEmpty()) {
            address.error = getString(R.string.address)
            return
        }
        val portNumber = port.text.toString().toIntOrNull()
        if (portNumber == null || portNumber !in 1..65535) {
            port.error = getString(R.string.port)
            return
        }
        if (!engineReady()) return
        // The password waits in memory for the session's password prompt and
        // is wiped there (architecture 2: never a String handed around).
        val typed = password.text
        val chars = CharArray(typed.length).also { android.text.TextUtils.getChars(typed, 0, typed.length, it, 0) }
        val encoded = Charsets.UTF_8.encode(java.nio.CharBuffer.wrap(chars))
        val secret = ByteArray(encoded.remaining()).also { encoded.get(it) }
        chars.fill('\u0000')
        if (encoded.hasArray()) encoded.array().fill(0)
        password.text.clear()
        val token = PendingPassword.put(if (secret.isEmpty()) null else secret)
        startActivity(
            Intent(this, TerminalActivity::class.java)
                .putExtra(TerminalActivity.EXTRA_PASSWORD_TOKEN, token)
                .putExtra(TerminalActivity.EXTRA_ADDRESS, host)
                .putExtra(TerminalActivity.EXTRA_PORT, portNumber)
                .putExtra(TerminalActivity.EXTRA_USERNAME, username.text.toString().trim()),
        )
    }

    companion object {
        private const val PICK_KEY = 1
    }
}

/**
 * The quick connect's password between the form and its terminal screen:
 * one slot per Connect, taken once by the screen whose intent carries its
 * token (review of run 13: never by another session's prompt). A slot not
 * taken is wiped by the next Connect.
 */
object PendingPassword {
    private val slots = HashMap<Long, ByteArray>()
    private var next = 1L

    @Synchronized
    fun put(value: ByteArray?): Long {
        for (old in slots.values) old.fill(0)
        slots.clear()
        val token = next++
        if (value != null) slots[token] = value
        return token
    }

    @Synchronized
    fun take(token: Long): ByteArray? = slots.remove(token)
}
