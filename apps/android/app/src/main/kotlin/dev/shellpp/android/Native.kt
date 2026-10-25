package dev.shellpp.android

/**
 * The Rust core's JNI layer (apps/android/native, crate shellpp-android).
 * Sessions are addressed by handles; calls that can block (call, open,
 * putCursor, awaitWake, nextEvent, reply) never run on the main thread.
 */
object Native {
    init {
        System.loadLibrary("shellpp_android")
    }

    // The keys of the extra keys (apps/android/native/src/input.rs, Key::from_code).
    const val KEY_ESC = 1
    const val KEY_TAB = 2
    const val KEY_ENTER = 3
    const val KEY_BACKSPACE = 4
    const val KEY_UP = 5
    const val KEY_DOWN = 6
    const val KEY_LEFT = 7
    const val KEY_RIGHT = 8
    const val KEY_HOME = 9
    const val KEY_END = 10
    const val KEY_PAGE_UP = 11
    const val KEY_PAGE_DOWN = 12
    const val KEY_DELETE = 13

    /** Opens the core on the vault in [filesDir] with [platform] behind its platform services. */
    @JvmStatic external fun start(filesDir: String, deviceName: String, platform: Platform): Boolean
    /**
     * One command of the core's service API (apps/android/native/src/dispatch.rs):
     * [input] is UTF-8 JSON, filled with zeros by the call; the answer is
     * `{"ok": ...}` or `{"error": {"code", "message", "details"}}`.
     */
    @JvmStatic external fun call(command: String, input: ByteArray?): String
    @JvmStatic external fun keyDir(): String
    @JvmStatic external fun open(address: String, port: Int, username: String, cols: Int, rows: Int, dark: Boolean): Long
    @JvmStatic external fun openHost(vaultId: String, recordId: String, cols: Int, rows: Int, dark: Boolean): Long
    @JvmStatic external fun nextEvent(timeoutMs: Int): String?
    @JvmStatic external fun reply(session: Long, prompt: Long, kind: String, text: ByteArray?)
    @JvmStatic external fun key(session: Long, code: Int, ctrl: Boolean, alt: Boolean)
    @JvmStatic external fun text(session: Long, text: String, ctrl: Boolean, alt: Boolean)
    @JvmStatic external fun paste(session: Long, text: String)
    @JvmStatic external fun putCursor(session: Long, col: Int, row: Int): Boolean
    @JvmStatic external fun snapshot(session: Long): ByteArray
    @JvmStatic external fun awaitWake(session: Long): Boolean
    @JvmStatic external fun wake(session: Long)
    @JvmStatic external fun resize(session: Long, cols: Int, rows: Int)
    @JvmStatic external fun scroll(session: Long, lines: Int)
    @JvmStatic external fun close(session: Long)
    @JvmStatic external fun openSessions(): Int
}
