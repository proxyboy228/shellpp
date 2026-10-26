package dev.shellpp.android

/**
 * What the terminal sends when a keyboard turns the text it composes from
 * [old] into [next] (architecture 6.2; the text it commits is the last
 * [next]): one Backspace per character it took back, then the text it
 * added. Added text with a line break that is more than that line break is
 * a paste (a keyboard's clipboard commits its text; D223), which asks first
 * for several lines; a lone line break is an Enter.
 */
data class ImeStep(val backspaces: Int, val added: String, val paste: Boolean)

fun imeStep(old: String, next: String): ImeStep {
    val common = old.commonPrefixWith(next).length
    val added = next.substring(common)
    val paste = added.length > 1 && (added.contains('\n') || added.contains('\r'))
    return ImeStep(old.length - common, added, paste)
}
