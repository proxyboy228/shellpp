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
            return answer(Native.call(command, bytes))
        } finally {
            bytes?.fill(0)
        }
    }

    fun obj(command: String, input: JSONObject? = null): JSONObject = call(command, input) as JSONObject

    fun array(command: String, input: JSONObject? = null): JSONArray = call(command, input) as JSONArray

    fun lockStatus(): LockStatus = LockStatus.from(obj("lock_status"))

    /** Commands with a secret: the password goes in UTF-8 bytes that are wiped after the call. */
    fun withPassword(command: String, field: String, password: CharArray): Any? {
        val prefix = "{\"" + field + "\":"
        val quoted = JSONObject.quote(String(password))
        val bytes = (prefix + quoted + "}").toByteArray(Charsets.UTF_8)
        try {
            return answer(Native.call(command, bytes))
        } finally {
            bytes.fill(0)
            password.fill('\u0000')
        }
    }

    /** A JSON value from its text (tests and events). */
    fun parse(text: String): Any? = JSONTokener(text).nextValue()
}
