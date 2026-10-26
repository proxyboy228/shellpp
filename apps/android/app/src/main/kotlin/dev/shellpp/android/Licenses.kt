package dev.shellpp.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** The project's page (architecture.md 8; the About rows, D223 item 5). */
const val PROJECT_PAGE = "https://github.com/proxyboy228/shellpp"

/** One line of the licenses list: a heading, a paragraph, or a component of a table. */
sealed interface NoticeItem {
    data class Heading(val text: String) : NoticeItem
    data class Paragraph(val text: String) : NoticeItem
    data class Component(val name: String, val version: String, val license: String, val source: String) : NoticeItem
    /** A license text, shown as it is. */
    data class Verbatim(val text: String) : NoticeItem
}

/**
 * The Android app's part of THIRD_PARTY_NOTICES.md (D223 item 5: the same
 * source as the repository's list) and the license texts after it, which
 * the build puts in the app's assets: `##` and `###` headings, paragraphs,
 * the rows of its tables (component, version, license, source; links
 * without their angle brackets) and fenced texts as they are. HTML
 * comments, table headers and separators are left out.
 */
fun parseNotices(text: String): List<NoticeItem> {
    val out = ArrayList<NoticeItem>()
    val paragraph = StringBuilder()
    fun flush() {
        if (paragraph.isNotBlank()) out += NoticeItem.Paragraph(paragraph.toString().trim())
        paragraph.clear()
    }
    var header = true
    var fence: StringBuilder? = null
    for (raw in text.lines()) {
        val line = raw.trim()
        val open = fence
        if (open != null) {
            if (line.startsWith("```")) {
                out += NoticeItem.Verbatim(open.toString().trimEnd())
                fence = null
            } else {
                open.append(raw).append('\n')
            }
            continue
        }
        when {
            line.startsWith("```") -> {
                flush()
                fence = StringBuilder()
            }
            line.startsWith("<!--") -> flush()
            line.startsWith("## ") || line.startsWith("### ") -> {
                flush()
                out += NoticeItem.Heading(line.trimStart('#').trim())
            }
            line.startsWith("|") -> {
                flush()
                val cells = line.trim('|').split('|').map { it.trim().removePrefix("<").removeSuffix(">") }
                when {
                    cells.all { cell -> cell.isEmpty() || cell.all { it == '-' || it == ':' } } -> header = false
                    header -> {}
                    // The terminal themes: theme, author, license, copyright, source.
                    cells.size >= 5 -> out += NoticeItem.Component(
                        "${cells[0]} (${cells[1]})", "", cells[2], cells.drop(3).joinToString("; "),
                    )
                    cells.size == 4 -> out += NoticeItem.Component(cells[0], cells[1], cells[2], cells.drop(3).joinToString(" "))
                    cells.size == 3 -> out += NoticeItem.Component(cells[0], cells[1], cells[2], "")
                }
            }
            line.isEmpty() -> {
                flush()
                header = true
            }
            else -> {
                if (paragraph.isNotEmpty()) paragraph.append(' ')
                paragraph.append(line)
            }
        }
    }
    flush()
    return out
}

/**
 * The Rust crates' license files (assets/notices/rust-licenses.txt): its
 * first lines as a paragraph, then a heading per block (`== crates ==`) and
 * the text.
 */
fun parseLicenseFiles(text: String): List<NoticeItem> {
    val out = ArrayList<NoticeItem>()
    out += NoticeItem.Heading("License files of the Rust crates")
    val blocks = text.split(Regex("(?m)^== ")).map { it.trimEnd() }
    blocks.firstOrNull()?.takeIf { it.isNotBlank() }?.let { out += NoticeItem.Verbatim(it) }
    for (block in blocks.drop(1)) {
        val title = block.substringBefore(" ==")
        out += NoticeItem.Paragraph(title)
        out += NoticeItem.Verbatim(block.substringAfter(" ==").trim())
    }
    return out
}

/** Settings → About → Open-source licenses: the components the APK ships, with their licenses and texts. */
@Composable
fun LicensesScreen(onClose: () -> Unit) {
    val context = LocalContext.current
    val items = remember {
        val list = runCatching { context.assets.open(NOTICES_ASSET).bufferedReader().use { parseNotices(it.readText()) } }.getOrDefault(emptyList())
        val files = runCatching { context.assets.open(LICENSE_FILES_ASSET).bufferedReader().use { parseLicenseFiles(it.readText()) } }.getOrDefault(emptyList())
        list + files
    }
    BackHandler(onBack = onClose)
    Page(top = { TopBar("Open-source licenses", left = { BackButton(onClose) }) }) {
        LazyColumn(Modifier.fillMaxSize().testTag("licenses")) {
            items(items) { item ->
                when (item) {
                    is NoticeItem.Heading -> Text(
                        item.text, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 20.dp, bottom = 4.dp),
                    )
                    is NoticeItem.Paragraph -> Text(
                        item.text, color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                    )
                    is NoticeItem.Verbatim -> Text(
                        item.text, color = Tokens.textSecondary, fontFamily = FontFamily.Monospace, fontSize = 11.sp,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                    )
                    is NoticeItem.Component -> Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp)) {
                        Text(if (item.version.isEmpty()) item.name else "${item.name} ${item.version}")
                        Text(item.license, color = Tokens.accent, style = MaterialTheme.typography.bodyMedium)
                        if (item.source.isNotEmpty()) Text(item.source, color = Tokens.textSecondary, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
        }
    }
}

/** The asset the build makes from THIRD_PARTY_NOTICES.md (apps/android/app/build.gradle.kts). */
const val NOTICES_ASSET = "notices/android.md"

/** The Rust crates' license files (scripts/rust-notices.mjs). */
const val LICENSE_FILES_ASSET = "notices/rust-licenses.txt"
