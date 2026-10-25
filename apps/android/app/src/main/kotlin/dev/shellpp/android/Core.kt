package dev.shellpp.android

import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener

/** An error of a core command: the core's code (`invalid_password`, `locked`, ...). */
class CoreException(val code: String, message: String, val details: Any?) : Exception(message)

/** The lock state (`lock_status`). */
data class LockStatus(
    val state: String,
    val rememberAvailable: Boolean,
    val rememberEnabled: Boolean,
    val rememberLost: Boolean,
) {
    val unlocked get() = state == "unlocked"
    val firstRun get() = state == "first_run"

    companion object {
        val STARTING = LockStatus("starting", false, false, false)

        fun from(json: JSONObject) = LockStatus(
            state = json.optString("state"),
            rememberAvailable = json.optBoolean("remember_device_available"),
            rememberEnabled = json.optBoolean("remember_device_enabled"),
            rememberLost = json.optBoolean("remember_device_lost"),
        )
    }
}

/**
 * The core's service API through [Native.call] (architecture 2). Every call
 * blocks: never on the main thread.
 */
object Core {
    /**
     * The commands that change records here: a sync follows them
     * (architecture 3), keys and known hosts included (review of run 14).
     */
    private val LOCAL_CHANGES = setOf(
        "record_save", "record_delete", "record_restore", "record_move", "record_duplicate", "recently_deleted_restore",
        "key_generate", "key_import", "key_label_set", "known_host_remove", "known_host_restore",
    )

    /** Called after a local change ([App.kickSync]). */
    @Volatile
    var onLocalChange: (() -> Unit)? = null

    /** Parses an answer of [Native.call]: the `ok` value, or throws [CoreException]. */
    fun answer(text: String): Any? {
        val json = JSONObject(text)
        if (json.has("error")) {
            val error = json.getJSONObject("error")
            throw CoreException(error.optString("code"), error.optString("message"), error.opt("details"))
        }
        val ok = json.opt("ok")
        return if (ok == JSONObject.NULL) null else ok
    }

    /** Runs [command] with [input] (a JSON object, or null for none). */
    fun call(command: String, input: JSONObject? = null): Any? {
        val bytes = input?.toString()?.toByteArray(Charsets.UTF_8)
        try {
            val result = answer(Native.call(command, bytes))
            if (command in LOCAL_CHANGES) onLocalChange?.invoke()
            return result
        } finally {
            bytes?.fill(0)
        }
    }

    fun obj(command: String, input: JSONObject? = null): JSONObject = call(command, input) as JSONObject

    fun array(command: String, input: JSONObject? = null): JSONArray = call(command, input) as JSONArray

    fun lockStatus(): LockStatus = LockStatus.from(obj("lock_status"))

    /** Commands with a secret: the password goes in UTF-8 bytes that are wiped after the call. */
    fun withPassword(command: String, field: String, password: CharArray): Any? {
        val bytes = secretJson(field, password)
        try {
            return answer(Native.call(command, bytes))
        } finally {
            bytes.fill(0)
            password.fill('\u0000')
        }
    }

    /**
     * [command] with the fields of [input] (no secrets) and [field] set to
     * [secret], which crosses in bytes only and is wiped after the call.
     */
    fun callWithSecret(command: String, input: JSONObject, field: String, secret: CharArray): Any? =
        callWithSecrets(command, input, listOf(field to secret))

    /** [callWithSecret] with several secret fields. */
    fun callWithSecrets(command: String, input: JSONObject, secrets: List<Pair<String, CharArray>>): Any? {
        var bytes = input.toString().removeSuffix("}").toByteArray(Charsets.UTF_8)
        var first = bytes.size <= 1
        for ((field, secret) in secrets) {
            val part = secretJson(field, secret)
            secret.fill('\u0000')
            // `{"f":"s"}` without its braces, after a comma unless it is the first member.
            val member = part.copyOfRange(1, part.size - 1)
            part.fill(0)
            val joined = bytes + (if (first) ByteArray(0) else byteArrayOf(','.code.toByte())) + member
            bytes.fill(0)
            member.fill(0)
            bytes = joined
            first = false
        }
        val closed = bytes + byteArrayOf('}'.code.toByte())
        bytes.fill(0)
        try {
            val result = answer(Native.call(command, closed))
            if (command in LOCAL_CHANGES) onLocalChange?.invoke()
            return result
        } finally {
            closed.fill(0)
        }
    }

    /**
     * `{"<field>":"<secret>"}` as UTF-8 without a String of the secret
     * (architecture 2): escaped by hand into a char array, encoded, and
     * every buffer between filled with zeros.
     */
    fun secretJson(field: String, secret: CharArray): ByteArray {
        val head = "{\"$field\":\""
        var length = head.length + 2
        for (c in secret) {
            length += when {
                c == '"' || c == '\\' -> 2
                c < ' ' -> 6
                else -> 1
            }
        }
        val chars = CharArray(length)
        var at = 0
        for (c in head) chars[at++] = c
        for (c in secret) {
            when {
                c == '"' || c == '\\' -> {
                    chars[at++] = '\\'
                    chars[at++] = c
                }
                c < ' ' -> {
                    val hex = "0123456789abcdef"
                    chars[at++] = '\\'
                    chars[at++] = 'u'
                    chars[at++] = '0'
                    chars[at++] = '0'
                    chars[at++] = hex[c.code shr 4]
                    chars[at++] = hex[c.code and 15]
                }
                else -> chars[at++] = c
            }
        }
        chars[at++] = '"'
        chars[at] = '}'
        val encoded = Charsets.UTF_8.newEncoder().encode(java.nio.CharBuffer.wrap(chars))
        val bytes = ByteArray(encoded.remaining())
        encoded.get(bytes)
        chars.fill('\u0000')
        if (encoded.hasArray()) encoded.array().fill(0)
        return bytes
    }

    /** A JSON value from its text (tests and events). */
    fun parse(text: String): Any? = JSONTokener(text).nextValue()
}
