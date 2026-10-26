package dev.shellpp.android

/**
 * The character a hardware keyboard's key types (review of run 16): what
 * AltGr (right Alt) makes on the keyboard's layout (`@` on a German Q) goes
 * as text without Alt, as on the desktop; otherwise Alt puts Esc before the
 * character the key makes without it.
 */
data class HardwareChar(val char: Int, val alt: Boolean)

/**
 * [plain]: the key's character without Ctrl and Alt; [withAlt]: with Alt
 * kept (`KeyEvent.getUnicodeChar`, 0 when the layout has none). A dead key
 * (an accent waiting for its letter, the sign bit set) types nothing.
 */
fun hardwareChar(plain: Int, withAlt: Int, rightAlt: Boolean, alt: Boolean): HardwareChar? {
    if (rightAlt && withAlt > 0 && withAlt != plain) return HardwareChar(withAlt, false)
    if (plain <= 0) return null
    return HardwareChar(plain, alt)
}
