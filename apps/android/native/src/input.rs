//! What the phone's keys and taps send (`docs/mobile/ui-spec.md` 9.2, 9.3):
//! the extra keys row with sticky Ctrl and Alt, text from the soft keyboard,
//! a tap that puts the cursor on a character (arrow keys along the typed
//! line) or clicks where the program asked for mouse reports.
//!
//! The bytes come from the table the desktop uses too
//! (`shellpp_terminal::keys`, run 16, D224): the keys with the armed
//! modifiers, the host's Backspace setting and the mouse's encodings.

use shellpp_terminal::keys::{self as table, BackspaceKey, Mods, MouseButton, MouseKind};

/// A key of the extra keys row (or a hardware keyboard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Esc,
    Tab,
    /// Shift+Tab.
    BackTab,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1-F12.
    F(u8),
}

impl Key {
    /// The key of the code the Kotlin side sends (`Native.KEY_*`).
    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            1 => Self::Esc,
            2 => Self::Tab,
            3 => Self::Enter,
            4 => Self::Backspace,
            5 => Self::Up,
            6 => Self::Down,
            7 => Self::Left,
            8 => Self::Right,
            9 => Self::Home,
            10 => Self::End,
            11 => Self::PageUp,
            12 => Self::PageDown,
            13 => Self::Delete,
            14 => Self::Insert,
            15 => Self::BackTab,
            101..=112 => Self::F(u8::try_from(code - 100).ok()?),
            _ => return None,
        })
    }
}

/// The modes of the terminal that change what keys and taps send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Application cursor keys (DECCKM): arrows as `ESC O x`.
    pub app_cursor: bool,
    /// The program asked for mouse button reports.
    pub mouse: bool,
    /// ... in the SGR encoding (mode 1006).
    pub sgr_mouse: bool,
}

/// The bytes of `key` with the armed modifiers and the host's Backspace
/// setting (the shared table, D224).
pub fn key_bytes(
    key: Key,
    modes: Modes,
    ctrl: bool,
    alt: bool,
    backspace: BackspaceKey,
) -> Vec<u8> {
    let mods = Mods {
        shift: key == Key::BackTab,
        alt,
        ctrl,
    };
    let shared = match key {
        Key::Esc => table::Key::Escape,
        Key::Tab | Key::BackTab => table::Key::Tab,
        Key::Enter => table::Key::Enter,
        Key::Backspace => table::Key::Backspace,
        Key::Up => table::Key::Up,
        Key::Down => table::Key::Down,
        Key::Left => table::Key::Left,
        Key::Right => table::Key::Right,
        Key::Home => table::Key::Home,
        Key::End => table::Key::End,
        Key::PageUp => table::Key::PageUp,
        Key::PageDown => table::Key::PageDown,
        Key::Insert => table::Key::Insert,
        Key::Delete => table::Key::Delete,
        Key::F(number) => table::Key::F(number),
    };
    table::key_bytes(shared, mods, modes.app_cursor, backspace).unwrap_or_default()
}

/// Text from the soft keyboard with the armed modifiers: Ctrl turns the
/// first character into its control character (the desktop's, D224), Alt
/// puts Esc before it. Line breaks become CR (Enter).
pub fn text_bytes(text: &str, ctrl: bool, alt: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut chars = text.chars();
    if let Some(first) = chars.next() {
        if alt {
            out.push(0x1b);
        }
        match (ctrl, table::control_of(first)) {
            (true, Some(byte)) => out.push(byte),
            _ => push_char(&mut out, first),
        }
    }
    for c in chars {
        push_char(&mut out, c);
    }
    out
}

fn push_char(out: &mut Vec<u8>, c: char) {
    if c == '\n' {
        out.push(b'\r');
    } else {
        let mut buffer = [0; 4];
        out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
    }
}

/// A cell of the visible screen (0-based column and row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub col: u16,
    pub row: u16,
}

/// The arrow keys that walk the cursor from `cursor` to `target` along the
/// typed line (UI spec 9.3, case 2), or `None` when `target` is not on it.
/// `wrapped[row]` says whether `row` continues on the next row (the line
/// wraps there); the line is the cursor's row and the rows joined to it by
/// wraps. One arrow per cell, at most the cells between the two.
pub fn walk_to(
    cursor: Cell,
    target: Cell,
    cols: u16,
    wrapped: &[bool],
    modes: Modes,
) -> Option<Vec<u8>> {
    let joined = |row: u16| wrapped.get(usize::from(row)).copied().unwrap_or(false);
    let (from, to) = if target.row <= cursor.row {
        (target.row, cursor.row)
    } else {
        (cursor.row, target.row)
    };
    if (from..to).any(|row| !joined(row)) {
        return None;
    }
    let index = |cell: Cell| i64::from(cell.row) * i64::from(cols) + i64::from(cell.col);
    let distance = index(target) - index(cursor);
    let arrow = key_bytes(
        if distance < 0 { Key::Left } else { Key::Right },
        modes,
        false,
        false,
        BackspaceKey::Del,
    );
    Some(arrow.repeat(distance.unsigned_abs() as usize))
}

/// A left click at `cell` (press and release) in the encoding the program
/// asked for (the shared table, D224): SGR (mode 1006) or the original one,
/// which cannot say a column or row past 222.
pub fn click(cell: Cell, modes: Modes, ctrl: bool, alt: bool) -> Option<Vec<u8>> {
    use alacritty_terminal::term::TermMode;
    if !modes.mouse {
        return None;
    }
    let mut mode = TermMode::MOUSE_REPORT_CLICK;
    if modes.sgr_mouse {
        mode |= TermMode::SGR_MOUSE;
    }
    let mods = Mods {
        shift: false,
        alt,
        ctrl,
    };
    let (col, row) = (usize::from(cell.col), usize::from(cell.row));
    let mut out = table::mouse_report(mode, MouseButton::Left, MouseKind::Press, col, row, mods)?;
    out.extend(table::mouse_report(
        mode,
        MouseButton::Left,
        MouseKind::Release,
        col,
        row,
        mods,
    )?);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMAL: Modes = Modes {
        app_cursor: false,
        mouse: false,
        sgr_mouse: false,
    };

    #[test]
    fn keys_send_the_desktops_sequences() {
        assert_eq!(
            key_bytes(Key::Esc, NORMAL, false, false, BackspaceKey::Del),
            [0x1b]
        );
        assert_eq!(
            key_bytes(Key::Tab, NORMAL, false, false, BackspaceKey::Del),
            b"\t"
        );
        assert_eq!(
            key_bytes(Key::Up, NORMAL, false, false, BackspaceKey::Del),
            b"\x1b[A"
        );
        let app = Modes {
            app_cursor: true,
            ..NORMAL
        };
        assert_eq!(
            key_bytes(Key::Up, app, false, false, BackspaceKey::Del),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes(Key::Left, NORMAL, true, false, BackspaceKey::Del),
            b"\x1b[1;5D"
        );
        assert_eq!(
            key_bytes(Key::Home, NORMAL, false, false, BackspaceKey::Del),
            b"\x1b[H"
        );
        assert_eq!(
            key_bytes(Key::PageUp, NORMAL, false, false, BackspaceKey::Del),
            b"\x1b[5~"
        );
        assert_eq!(
            key_bytes(Key::Delete, NORMAL, false, false, BackspaceKey::Del),
            b"\x1b[3~"
        );
        assert_eq!(
            key_bytes(Key::Backspace, NORMAL, false, true, BackspaceKey::Del),
            [0x1b, 0x7f]
        );
        assert_eq!(
            key_bytes(Key::Backspace, NORMAL, true, false, BackspaceKey::Del),
            [0x08]
        );
    }

    /// Every key of the phone, with Ctrl and Alt armed or not, in both
    /// cursor modes and with both Backspace settings, sends what the
    /// desktop's table says (`apps/desktop/src/terminal/key_table.txt`,
    /// written from the desktop's code before the mapping moved, D224).
    #[test]
    fn every_key_sends_the_desktops_bytes() {
        let table = include_str!("../../../desktop/src/terminal/key_table.txt");
        let shown = |bytes: &[u8]| -> String {
            bytes
                .iter()
                .map(|byte| match byte {
                    0x21..=0x7e if *byte != b'\\' => char::from(*byte).to_string(),
                    _ => format!("\\x{byte:02x}"),
                })
                .collect()
        };
        let mut keys = vec![
            (Key::Esc, "escape"),
            (Key::Tab, "tab"),
            (Key::BackTab, "tab"),
            (Key::Enter, "enter"),
            (Key::Backspace, "backspace"),
            (Key::Up, "up"),
            (Key::Down, "down"),
            (Key::Left, "left"),
            (Key::Right, "right"),
            (Key::Home, "home"),
            (Key::End, "end"),
            (Key::PageUp, "pageup"),
            (Key::PageDown, "pagedown"),
            (Key::Insert, "insert"),
            (Key::Delete, "delete"),
        ];
        let names: Vec<String> = (1..=12).map(|n| format!("f{n}")).collect();
        for (n, name) in names.iter().enumerate() {
            keys.push((Key::F(u8::try_from(n + 1).unwrap()), name.as_str()));
        }
        let mut checked = 0;
        for (key, name) in keys {
            for (ctrl, alt) in [(false, false), (true, false), (false, true), (true, true)] {
                for (mode_name, app_cursor) in [("normal", false), ("app-cursor", true)] {
                    for (backspace, backspace_name) in
                        [(BackspaceKey::Del, "Del"), (BackspaceKey::CtrlH, "CtrlH")]
                    {
                        let line = format!(
                            "{}{}{}{name}\t{mode_name}\t{backspace_name}\t",
                            if ctrl { "ctrl-" } else { "" },
                            if alt { "alt-" } else { "" },
                            if key == Key::BackTab { "shift-" } else { "" },
                        );
                        let want = table
                            .lines()
                            .find_map(|row| row.strip_prefix(line.as_str()))
                            .unwrap_or_else(|| panic!("no row {line}"));
                        let modes = Modes {
                            app_cursor,
                            ..NORMAL
                        };
                        assert_eq!(
                            shown(&key_bytes(key, modes, ctrl, alt, backspace)),
                            want,
                            "{line}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 27 * 4 * 2 * 2);
    }

    #[test]
    fn ctrl_and_alt_change_typed_text() {
        assert_eq!(text_bytes("c", true, false), [0x03]);
        assert_eq!(text_bytes("C", true, false), [0x03]);
        assert_eq!(text_bytes("x", false, true), [0x1b, b'x']);
        assert_eq!(text_bytes("k", true, true), [0x1b, 0x0b]);
        assert_eq!(text_bytes("6", true, false), [0x1e]);
        assert_eq!(text_bytes("é1\n", false, false), "é1\r".as_bytes());
        // Ctrl changes the first character only; one without a control
        // character goes as it is.
        assert_eq!(text_bytes("ab", true, false), [0x01, b'b']);
        assert_eq!(text_bytes("é", true, false), "é".as_bytes());
    }

    #[test]
    fn a_tap_on_the_typed_line_walks_the_cursor_there() {
        let cursor = Cell { col: 10, row: 3 };
        let wrapped = [false, false, true, true, false];
        let left = walk_to(cursor, Cell { col: 6, row: 3 }, 80, &wrapped, NORMAL).unwrap();
        assert_eq!(left, b"\x1b[D".repeat(4));
        // Across the wrap above: row 2 continues on row 3.
        let up = walk_to(cursor, Cell { col: 78, row: 2 }, 80, &wrapped, NORMAL).unwrap();
        assert_eq!(up, b"\x1b[D".repeat(12));
        // Onto row 4, which row 3 continues on, in application cursor mode.
        let app = Modes {
            app_cursor: true,
            ..NORMAL
        };
        let down = walk_to(cursor, Cell { col: 2, row: 4 }, 80, &wrapped, app).unwrap();
        assert_eq!(down, b"\x1bOC".repeat(72));
        // Row 1 is another line: nothing.
        assert_eq!(
            walk_to(cursor, Cell { col: 0, row: 1 }, 80, &wrapped, NORMAL),
            None
        );
        assert_eq!(
            walk_to(cursor, cursor, 80, &wrapped, NORMAL),
            Some(Vec::new())
        );
    }

    #[test]
    fn a_tap_clicks_where_the_program_takes_the_mouse() {
        let cell = Cell { col: 9, row: 4 };
        assert_eq!(click(cell, NORMAL, false, false), None);
        let sgr = Modes {
            mouse: true,
            sgr_mouse: true,
            ..NORMAL
        };
        assert_eq!(
            click(cell, sgr, false, false).unwrap(),
            b"\x1b[<0;10;5M\x1b[<0;10;5m"
        );
        assert_eq!(
            click(cell, sgr, true, false).unwrap(),
            b"\x1b[<16;10;5M\x1b[<16;10;5m"
        );
        let original = Modes {
            mouse: true,
            ..NORMAL
        };
        assert_eq!(
            click(cell, original, false, false).unwrap(),
            [0x1b, b'[', b'M', 32, 42, 37, 0x1b, b'[', b'M', 35, 42, 37]
        );
        assert_eq!(
            click(Cell { col: 300, row: 0 }, original, false, false),
            None
        );
    }
}
