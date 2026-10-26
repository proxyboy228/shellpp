//! The visible screen as a binary snapshot for the Kotlin view
//! (`docs/mobile/architecture.md` 6.1): copied under the terminal's lock in
//! one pass, colors resolved by the model's palette, nothing for the view to
//! parse but fixed-size little-endian fields.
//!
//! Layout:
//!
//! - header, 44 bytes: `cols` u16, `rows` u16, cursor column u16, cursor row
//!   u16, cursor visible u8, cursor shape u8 (0 block, 1 underline, 2 beam,
//!   3 hollow block), modes u8 ([`MODE_APP_CURSOR`] and the others), bell
//!   u8 (1 when the program rang the bell since the last snapshot, D223);
//!   then u32 colors `0x00RRGGBB`: background, foreground, cursor, cursor
//!   text, the selection (`0xAARRGGBB`), and the scroll offset u32, the
//!   number of rows of the scrollback above u32, 0 u32;
//! - `rows` bytes: 1 when that row continues on the next (the line wraps);
//! - `rows × cols` cells of 14 bytes: the character u32 (a code point, 0 for
//!   an empty cell), foreground u32, background u32, flags u16
//!   ([`FLAG_BOLD`] and the others);
//! - the combining marks and emoji sequences: a count u32, then per entry
//!   column u16, row u16, length u16 and that many bytes of UTF-8: the
//!   cell's whole text.

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use shellpp_terminal::TerminalHandle;

use crate::input::{Cell, Modes};

pub const HEADER_BYTES: usize = 44;
pub const CELL_BYTES: usize = 14;

pub const MODE_APP_CURSOR: u8 = 1;
pub const MODE_MOUSE: u8 = 2;
pub const MODE_SGR_MOUSE: u8 = 4;
pub const MODE_BRACKETED_PASTE: u8 = 8;
pub const MODE_ALT_SCREEN: u8 = 16;

pub const FLAG_BOLD: u16 = 1;
pub const FLAG_ITALIC: u16 = 2;
pub const FLAG_UNDERLINE: u16 = 4;
pub const FLAG_STRIKEOUT: u16 = 8;
pub const FLAG_WIDE: u16 = 16;
pub const FLAG_WIDE_SPACER: u16 = 32;
pub const FLAG_COMBINED: u16 = 64;

fn rgb(color: Rgb) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

fn mode_bits(mode: TermMode) -> u8 {
    let mut bits = 0;
    if mode.contains(TermMode::APP_CURSOR) {
        bits |= MODE_APP_CURSOR;
    }
    if mode.intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
    {
        bits |= MODE_MOUSE;
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        bits |= MODE_SGR_MOUSE;
    }
    if mode.contains(TermMode::BRACKETED_PASTE) {
        bits |= MODE_BRACKETED_PASTE;
    }
    if mode.contains(TermMode::ALT_SCREEN) {
        bits |= MODE_ALT_SCREEN;
    }
    bits
}

/// The modes that change what keys and taps send.
pub fn modes(handle: &TerminalHandle) -> Modes {
    let bits = mode_bits(*handle.term().lock().mode());
    Modes {
        app_cursor: bits & MODE_APP_CURSOR != 0,
        mouse: bits & MODE_MOUSE != 0,
        sgr_mouse: bits & MODE_SGR_MOUSE != 0,
    }
}

/// Whether the program switched to the alternate screen (a full-screen
/// program: an editor, a pager, `htop`).
pub fn alternate_screen(handle: &TerminalHandle) -> bool {
    handle.term().lock().mode().contains(TermMode::ALT_SCREEN)
}

/// The cursor's cell on the screen and which rows wrap onto the next, for
/// [`crate::input::walk_to`]; `None` while the view is scrolled back.
pub fn cursor_and_wraps(handle: &TerminalHandle) -> Option<(Cell, u16, Vec<bool>)> {
    let term = handle.term().lock();
    let content = term.renderable_content();
    if content.display_offset != 0 {
        return None;
    }
    let cols = term.columns();
    let rows = term.screen_lines();
    let last = alacritty_terminal::index::Column(cols.saturating_sub(1));
    let wraps = (0..rows)
        .map(|row| {
            let line = alacritty_terminal::index::Line(row as i32);
            term.grid()[line][last].flags.contains(Flags::WRAPLINE)
        })
        .collect();
    let point = content.cursor.point;
    let cell = Cell {
        col: u16::try_from(point.column.0).ok()?,
        row: u16::try_from(point.line.0).ok()?,
    };
    Some((cell, u16::try_from(cols).ok()?, wraps))
}

/// Writes the snapshot of the visible screen.
pub fn snapshot(handle: &TerminalHandle) -> Vec<u8> {
    let palette = handle.outbox().palette();
    let term = handle.term().lock();
    let cols = term.columns();
    let rows = term.screen_lines();
    let history = term.history_size();
    let content = term.renderable_content();
    let offset = content.display_offset;
    let mode = content.mode;
    let colors = content.colors;
    let cursor = content.cursor;

    let mut out = Vec::with_capacity(HEADER_BYTES + rows + rows * cols * CELL_BYTES + 4);
    let u16_of = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    let cursor_row = cursor.point.line.0 + offset as i32;
    let cursor_visible = mode.contains(TermMode::SHOW_CURSOR)
        && cursor.shape != CursorShape::Hidden
        && (0..rows as i32).contains(&cursor_row);
    out.extend_from_slice(&u16_of(cols).to_le_bytes());
    out.extend_from_slice(&u16_of(rows).to_le_bytes());
    out.extend_from_slice(&u16_of(cursor.point.column.0).to_le_bytes());
    out.extend_from_slice(&u16::try_from(cursor_row.max(0)).unwrap_or(0).to_le_bytes());
    out.push(u8::from(cursor_visible));
    out.push(match cursor.shape {
        CursorShape::Underline => 1,
        CursorShape::Beam => 2,
        CursorShape::HollowBlock => 3,
        _ => 0,
    });
    out.push(mode_bits(mode));
    // Snapshots are taken while the session is seen: its bell rings then.
    out.push(u8::from(handle.outbox().take_events().bell));
    let selection = palette.selection();
    for color in [
        rgb(palette.background()),
        rgb(palette.foreground()),
        rgb(palette.cursor()),
        rgb(palette.cursor_text()),
        (u32::from(selection.alpha) << 24) | rgb(selection.rgb),
        u32::try_from(offset).unwrap_or(u32::MAX),
        u32::try_from(history).unwrap_or(u32::MAX),
        0,
    ] {
        out.extend_from_slice(&color.to_le_bytes());
    }

    let wraps_at = out.len();
    out.resize(wraps_at + rows, 0);
    let cells_at = out.len();
    out.resize(cells_at + rows * cols * CELL_BYTES, 0);
    let mut combined: Vec<(u16, u16, String)> = Vec::new();
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset as i32;
        let (Ok(row), col) = (usize::try_from(row), indexed.point.column.0) else {
            continue;
        };
        if row >= rows || col >= cols {
            continue;
        }
        let cell = indexed.cell;
        if col + 1 == cols && cell.flags.contains(Flags::WRAPLINE) {
            out[wraps_at + row] = 1;
        }
        let (fg, bg) = palette.cell_colors(cell.fg, cell.bg, cell.flags, true, colors);
        let mut flags = 0u16;
        for (flag, bit) in [
            (Flags::BOLD, FLAG_BOLD),
            (Flags::ITALIC, FLAG_ITALIC),
            (Flags::ALL_UNDERLINES, FLAG_UNDERLINE),
            (Flags::STRIKEOUT, FLAG_STRIKEOUT),
            (Flags::WIDE_CHAR, FLAG_WIDE),
            (Flags::WIDE_CHAR_SPACER, FLAG_WIDE_SPACER),
        ] {
            if cell.flags.intersects(flag) {
                flags |= bit;
            }
        }
        if let Some(marks) = cell.zerowidth() {
            flags |= FLAG_COMBINED;
            let mut text = String::from(cell.c);
            text.extend(marks.iter());
            combined.push((u16_of(col), u16_of(row), text));
        }
        let c = if cell.flags.contains(Flags::WIDE_CHAR_SPACER) || cell.c == ' ' {
            0
        } else {
            u32::from(cell.c)
        };
        let at = cells_at + (row * cols + col) * CELL_BYTES;
        out[at..at + 4].copy_from_slice(&c.to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&rgb(fg).to_le_bytes());
        out[at + 8..at + 12]
            .copy_from_slice(&rgb(bg.unwrap_or(palette.background())).to_le_bytes());
        out[at + 12..at + 14].copy_from_slice(&flags.to_le_bytes());
    }
    out.extend_from_slice(&u32::try_from(combined.len()).unwrap_or(0).to_le_bytes());
    for (col, row, text) in combined {
        let bytes = text.as_bytes();
        let length = u16::try_from(bytes.len()).unwrap_or(0);
        out.extend_from_slice(&col.to_le_bytes());
        out.extend_from_slice(&row.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&bytes[..usize::from(length)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::vte::ansi::CursorShape;
    use shellpp_terminal::model::{ModelOptions, TerminalSize};

    use super::*;

    fn terminal(cols: u16, rows: u16) -> (TerminalHandle, async_channel::Receiver<()>) {
        let (wake, woken) = async_channel::bounded(1);
        let handle = TerminalHandle::spawn(
            ModelOptions {
                size: TerminalSize::cells(cols, rows),
                scrollback: 100,
                cursor_shape: CursorShape::Block,
            },
            crate::theme::palette(true),
            wake,
        );
        (handle, woken)
    }

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn cell_at(rows: usize, cols: usize, col: usize, row: usize) -> usize {
        HEADER_BYTES + rows + (row * cols + col) * CELL_BYTES
    }

    #[test]
    fn the_snapshot_holds_text_colors_cursor_and_wraps() {
        let (handle, _woken) = terminal(10, 4);
        // "ab" in red bold, then a line that wraps, then a combining mark.
        handle
            .feed_blocking(b"\x1b[1;31mab\x1b[0m\r\n0123456789xy\r\ne\xcc\x81")
            .unwrap();
        handle.flush_blocking();
        let bytes = snapshot(&handle);
        assert_eq!(u16_at(&bytes, 0), 10);
        assert_eq!(u16_at(&bytes, 2), 4);
        // The cursor after "é" on row 2.
        assert_eq!((u16_at(&bytes, 4), u16_at(&bytes, 6)), (1, 3));
        assert_eq!(bytes[8], 1);
        let palette = crate::theme::palette(true);
        let a = cell_at(4, 10, 0, 0);
        assert_eq!(u32_at(&bytes, a), u32::from('a'));
        // Bold red is drawn bright red (the theme's bold-is-bright rule).
        assert_eq!(u32_at(&bytes, a + 4), rgb(palette.indexed(9)));
        assert_eq!(u32_at(&bytes, a + 8), rgb(palette.background()));
        assert_eq!(u16_at(&bytes, a + 12) & FLAG_BOLD, FLAG_BOLD);
        // Row 1 wraps onto row 2.
        assert_eq!(&bytes[HEADER_BYTES..HEADER_BYTES + 4], [0, 1, 0, 0]);
        let x = cell_at(4, 10, 0, 2);
        assert_eq!(u32_at(&bytes, x), u32::from('x'));
        // "é" as e plus a combining acute accent.
        let e = cell_at(4, 10, 0, 3);
        assert_eq!(u16_at(&bytes, e + 12) & FLAG_COMBINED, FLAG_COMBINED);
        let table = HEADER_BYTES + 4 + 4 * 10 * CELL_BYTES;
        assert_eq!(u32_at(&bytes, table), 1);
        assert_eq!(
            (u16_at(&bytes, table + 4), u16_at(&bytes, table + 6)),
            (0, 3)
        );
        let length = usize::from(u16_at(&bytes, table + 8));
        assert_eq!(
            &bytes[table + 10..table + 10 + length],
            "e\u{301}".as_bytes()
        );
        let (cursor, cols, wraps) = cursor_and_wraps(&handle).unwrap();
        assert_eq!((cursor, cols), (Cell { col: 1, row: 3 }, 10));
        assert_eq!(wraps, [false, true, false, false]);
    }

    #[test]
    fn modes_follow_the_program() {
        let (handle, _woken) = terminal(10, 4);
        assert_eq!(modes(&handle), Modes::default());
        handle
            .feed_blocking(b"\x1b[?1h\x1b[?1000h\x1b[?1006h")
            .unwrap();
        handle.flush_blocking();
        assert_eq!(
            modes(&handle),
            Modes {
                app_cursor: true,
                mouse: true,
                sgr_mouse: true,
            }
        );
        let bytes = snapshot(&handle);
        assert_eq!(bytes[10], MODE_APP_CURSOR | MODE_MOUSE | MODE_SGR_MOUSE);
    }
}
