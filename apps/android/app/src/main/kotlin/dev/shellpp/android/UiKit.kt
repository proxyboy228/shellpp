package dev.shellpp.android

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Dns
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.json.JSONObject

/*
 * The building blocks of the screens of run 15 (docs/mobile/reference-1.md
 * and reference-2.md: structure only; the look is Shell++'s own design
 * system, docs/design-system.md, UI spec 2).
 */

/** Radius of cards and tiles (design-system radii in dp). */
val CardShape = RoundedCornerShape(12.dp)
val TileShape = RoundedCornerShape(10.dp)

/**
 * A screen's top bar: [left] (a back arrow and its label), a centred
 * [title], and [actions] on the right. 64 dp high, on the screen's
 * background.
 */
@Composable
fun TopBar(
    title: String,
    modifier: Modifier = Modifier,
    left: (@Composable RowScope.() -> Unit)? = null,
    titleContent: (@Composable () -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Box(modifier.fillMaxWidth().height(64.dp).background(Tokens.bgBase)) {
        Row(
            Modifier.align(Alignment.CenterStart).padding(start = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            content = { left?.invoke(this) },
        )
        Box(Modifier.align(Alignment.Center).padding(horizontal = 96.dp)) {
            if (titleContent != null) {
                titleContent()
            } else {
                Text(
                    title,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    textAlign = TextAlign.Center,
                )
            }
        }
        Row(
            Modifier.align(Alignment.CenterEnd).padding(end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            content = actions,
        )
    }
}

/** A back arrow, with [label] after it when given (the screen it returns to). */
@Composable
fun BackButton(onClick: () -> Unit, label: String? = null, tag: String = "screen_close") {
    Row(
        Modifier.clip(RoundedCornerShape(24.dp)).clickable(onClick = onClick).heightIn(min = 48.dp)
            .padding(start = 8.dp, end = if (label == null) 8.dp else 12.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
        if (label != null) {
            Spacer(Modifier.width(8.dp))
            Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.width(88.dp), color = Tokens.textSecondary)
        }
    }
}

@Composable
fun BarIcon(icon: ImageVector, description: String, tag: String, enabled: Boolean = true, tint: Color? = null, onClick: () -> Unit) {
    IconButton(onClick = onClick, enabled = enabled, modifier = Modifier.testTag(tag)) {
        Icon(icon, contentDescription = description, tint = tint ?: if (enabled) Tokens.text else Tokens.textMuted)
    }
}

/** A screen without the app's bottom bar: its top bar and its content. */
@Composable
fun Page(top: @Composable () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    // Text and icons in the theme's text color, as inside a Scaffold.
    CompositionLocalProvider(LocalContentColor provides Tokens.text) {
        Column(Modifier.fillMaxSize().background(Tokens.bgBase)) {
            top()
            content()
        }
    }
}

/** A section header of a list ("Groups", "Hosts"). */
@Composable
fun SectionHeader(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold),
        color = Tokens.textSecondary,
        modifier = modifier.padding(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 8.dp),
    )
}

/**
 * A full-width card of a list (a group, a host, a known host, a key): a
 * [tile] on the left, a title and a muted subtitle, [trailing] on the right.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ListCard(
    title: AnnotatedString,
    subtitle: String?,
    tag: String,
    tile: @Composable () -> Unit,
    onClick: () -> Unit,
    onLongClick: (() -> Unit)? = null,
    selected: Boolean = false,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        Modifier
            .padding(horizontal = 16.dp, vertical = 4.dp)
            .fillMaxWidth()
            .clip(CardShape)
            .background(if (selected) Tokens.accentSubtle else Tokens.bgRaised)
            .combinedClickable(onClick = onClick, onLongClick = onLongClick)
            .heightIn(min = 72.dp)
            .padding(horizontal = 12.dp, vertical = 10.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        tile()
        Column(Modifier.weight(1f).padding(horizontal = 14.dp), verticalArrangement = Arrangement.Center) {
            Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyLarge)
            if (!subtitle.isNullOrEmpty()) {
                Text(subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis, color = Tokens.textSecondary, style = MaterialTheme.typography.bodyMedium)
            }
        }
        trailing?.invoke()
    }
}

/** One rounded container of rows with thin separators (the vault home, pickers). */
@Composable
fun GroupedList(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier.padding(horizontal = 16.dp).fillMaxWidth().clip(CardShape).background(Tokens.bgRaised),
        content = content,
    )
}

/** A row of a [GroupedList]: icon, label, a muted [value], a chevron or [trailing]. */
@Composable
fun GroupedRow(
    label: String,
    tag: String,
    icon: ImageVector? = null,
    value: String? = null,
    divider: Boolean = true,
    enabled: Boolean = true,
    trailing: (@Composable () -> Unit)? = null,
    onClick: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 16.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, contentDescription = null, tint = Tokens.accent, modifier = Modifier.size(22.dp))
            Spacer(Modifier.width(16.dp))
        }
        Text(label, Modifier.weight(1f), color = if (enabled) Tokens.text else Tokens.textMuted)
        if (value != null) Text(value, color = Tokens.textSecondary, modifier = Modifier.padding(end = 8.dp))
        trailing?.invoke() ?: Text("›", color = Tokens.textMuted, fontSize = 22.sp)
    }
    if (divider) HorizontalDivider(Modifier.padding(start = if (icon != null) 54.dp else 16.dp), color = Tokens.border.copy(alpha = 0.35f))
}

/** [text] with the parts that match [query] (case-insensitive) in the accent. */
@Composable
fun highlighted(text: String, query: String): AnnotatedString {
    val accent = Tokens.accent
    val needle = query.trim()
    if (needle.isEmpty()) return AnnotatedString(text)
    // Indices of the text itself (a lowercase of another length, as "İ",
    // must not shift them).
    return buildAnnotatedString {
        var at = 0
        while (at < text.length) {
            val hit = text.indexOf(needle, at, ignoreCase = true)
            if (hit < 0) {
                append(text.substring(at))
                break
            }
            append(text.substring(at, hit))
            withStyle(SpanStyle(color = accent, fontWeight = FontWeight.SemiBold)) { append(text.substring(hit, hit + needle.length)) }
            at = hit + needle.length
        }
    }
}

/** A tile with an icon: groups, known hosts, keys. */
@Composable
fun IconTile(icon: ImageVector, size: Dp = 44.dp, fill: Color = Tokens.accentSubtle, tint: Color = Tokens.accent) {
    Box(Modifier.size(size).clip(TileShape).background(fill), contentAlignment = Alignment.Center) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(size * 0.5f))
    }
}

/** A tile with a check mark: a selected card (selection mode). */
@Composable
fun CheckTile(size: Dp = 44.dp) = IconTile(Icons.Filled.Check, size, fill = Tokens.accent, tint = Tokens.onAccent)

/**
 * The OS mark of a host (reference-1 2.2; plan 1.3): Shell++'s own simple
 * glyphs, never a distribution's logo. A detected Linux distribution shows
 * its name's first letters on a tile of one of the design system's tag
 * colors; without detection, a generic server glyph.
 */
data class OsMark(val letters: String?, val color: String?) {
    companion object {
        private val names = mapOf(
            "ubuntu" to ("Ub" to "coral"), "debian" to ("De" to "rose"), "almalinux" to ("Al" to "ocean"),
            "rocky" to ("Ro" to "moss"), "centos" to ("Ce" to "plum"), "fedora" to ("Fe" to "ocean"),
            "rhel" to ("RH" to "coral"), "arch" to ("Ar" to "ocean"), "alpine" to ("Ap" to "ocean"),
            "opensuse-leap" to ("Su" to "moss"), "opensuse-tumbleweed" to ("Su" to "moss"), "gentoo" to ("Ge" to "plum"),
            "nixos" to ("Nx" to "ocean"), "raspbian" to ("Rp" to "rose"), "kali" to ("Ka" to "graphite"),
            "linuxmint" to ("Mi" to "moss"), "manjaro" to ("Mj" to "jade"), "amzn" to ("Az" to "amber"),
            "ol" to ("OL" to "coral"), "freebsd" to ("FB" to "coral"), "openbsd" to ("OB" to "amber"), "netbsd" to ("NB" to "amber"),
        )

        /** The mark of a host's `os` member (data-model 7.6), or the generic one. */
        fun of(os: JSONObject?): OsMark {
            if (os == null) return OsMark(null, null)
            val distro = os.text("distro")?.lowercase()
            names[distro]?.let { (letters, color) -> return OsMark(letters, color) }
            return when (os.text("family")) {
                "linux" -> OsMark(distro?.take(2)?.replaceFirstChar { it.uppercase() } ?: "Lx", "jade")
                "windows" -> OsMark("Wn", "ocean")
                "macos" -> OsMark("mc", "graphite")
                "bsd" -> OsMark("BS", "coral")
                else -> OsMark(null, null)
            }
        }
    }
}

/**
 * A host's tile: its [mark] (or the generic glyph), a ring while it has
 * open sessions ([active]); [sftp] draws it inside a folder outline, the
 * tile of an SFTP session (reference-2 7.5).
 */
@Composable
fun OsTile(mark: OsMark, size: Dp = 44.dp, active: Boolean = false, sftp: Boolean = false, onTerminal: Boolean = false) {
    // A tag colour a quarter darker under white letters: at least 4.8:1 for
    // every colour of the palette (design-system 7.3).
    val fill = tagColor(mark.color)?.let { lerp(it, Color.Black, 0.25f) } ?: Tokens.bgInput
    val ring = if (active) Modifier.border(2.dp, Tokens.accent, TileShape) else Modifier
    Box(Modifier.size(size).then(ring).padding(if (active) 3.dp else 0.dp).clip(TileShape).background(if (onTerminal) Color.Transparent else fill), contentAlignment = Alignment.Center) {
        if (sftp) {
            Icon(Icons.Filled.Folder, contentDescription = null, tint = if (mark.letters != null) Color.White.copy(alpha = 0.35f) else Tokens.accentSubtle, modifier = Modifier.size(size * 0.8f))
        }
        when {
            mark.letters != null -> Text(
                mark.letters,
                color = Color.White,
                fontWeight = FontWeight.Bold,
                fontSize = (size.value * 0.36f).sp,
                modifier = Modifier.padding(top = if (sftp) 3.dp else 0.dp),
            )
            else -> Icon(
                if (sftp) Icons.Filled.Dns else Icons.Filled.Dns,
                contentDescription = null,
                tint = Tokens.accent,
                modifier = Modifier.size(size * if (sftp) 0.34f else 0.5f).padding(top = if (sftp) 2.dp else 0.dp),
            )
        }
    }
}

/** The generic glyph of the Connections tab while no session is open. */
val TerminalGlyph: ImageVector get() = Icons.Filled.Terminal

/** A muted line in the middle of an empty screen: a tile, a title, an explanation. */
@Composable
fun EmptyState(icon: ImageVector, title: String, text: String, modifier: Modifier = Modifier, extra: @Composable ColumnScope.() -> Unit = {}) {
    Column(
        modifier.fillMaxWidth().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        IconTile(icon, 64.dp)
        Text(title, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        Text(text, color = Tokens.textSecondary, textAlign = TextAlign.Center)
        extra()
    }
}
