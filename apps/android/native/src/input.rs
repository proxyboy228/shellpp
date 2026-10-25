//! What the phone's keys and taps send (`docs/mobile/ui-spec.md` 9.2, 9.3):
//! the extra keys row with sticky Ctrl and Alt, text from the soft keyboard,
//! a tap that puts the cursor on a character (arrow keys along the typed
//! line) or clicks where the program asked for mouse reports.
//!
//! The bytes are the desktop's (`apps/desktop/src/terminal/input.rs` and
//! `mouse.rs`, xterm's sequences); moving that mapping into
//! `shellpp-terminal` for both clients is the mobile runs' (architecture
//! 6.2).

/// A key of the extra keys row (or a hardware keyboard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Esc,
    Tab,
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
    Delete,
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

/// The xterm modifier parameter of Ctrl and Alt (1 + 2·Alt + 4·Ctrl).
fn modifier(ctrl: bool, alt: bool) -> Option<u8> {
    let value = 1 + if alt { 2 } else { 0 } + if ctrl { 4 } else { 0 };
    (value > 1).then_some(value)
}

/// The bytes of `key` with the armed modifiers.
pub fn key_bytes(key: Key, modes: Modes, ctrl: bool, alt: bool) -> Vec<u8> {
    let cursor = |final_byte: char| match modifier(ctrl, alt) {
        Some(m) => format!("\x1b[1;{m}{final_byte}").into_bytes(),
        None if modes.app_cursor => format!("\x1bO{final_byte}").into_bytes(),
        None => format!("\x1b[{final_byte}").into_bytes(),
    };
    let tilde = |number: u8| match modifier(ctrl, alt) {
        Some(m) => format!("\x1b[{number};{m}~").into_bytes(),
        None => format!("\x1b[{number}~").into_bytes(),
    };
    let prefixed = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    match key {
        Key::Esc => prefixed(&[0x1b]),
        Key::Tab => prefixed(b"\t"),
        Key::Enter => prefixed(b"\r"),
        Key::Backspace if ctrl => prefixed(&[0x08]),
        Key::Backspace => prefixed(&[0x7f]),
        Key::Up => cursor('A'),
        Key::Down => cursor('B'),
        Key::Right => cursor('C'),
        Key::Left => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Delete => tilde(3),
    }
}

/// The control character of `c` with Ctrl (Ctrl+C is 0x03), as the
/// desktop maps it; `None` when Ctrl does not change it.
fn control(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' | '/' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Text from the soft keyboard with the armed modifiers: Ctrl turns the
/// first character into its control character, Alt puts Esc before it.
/// Line breaks become CR (Enter).
pub fn text_bytes(text: &str, ctrl: bool, alt: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut chars = text.chars();
    if let Some(first) = chars.next() {
        if alt {
            out.push(0x1b);
        }
        match (ctrl, control(first)) {
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
    );
    Some(arrow.repeat(distance.unsigned_abs() as usize))
}

/// A left click at `cell` (press and release) in the encoding the program
/// asked for: SGR (mode 1006) or the original one, which cannot say a
/// column or row past 223.
pub fn click(cell: Cell, modes: Modes, ctrl: bool, alt: bool) -> Option<Vec<u8>> {
    if !modes.mouse {
        return None;
    }
    let button = (if alt { 8 } else { 0 }) + (if ctrl { 16 } else { 0 });
    let (col, row) = (u32::from(cell.col) + 1, u32::from(cell.row) + 1);
    if modes.sgr_mouse {
        return Some(
            format!("\x1b[<{button};{col};{row}M\x1b[<{button};{col};{row}m").into_bytes(),
        );
    }
    let encode = |value: u32| u8::try_from(value + 32).ok().filter(|byte| *byte < 255);
    let (x, y) = (encode(col)?, encode(row)?);
    let mut out = vec![0x1b, b'[', b'M', 32 + button, x, y];
    out.extend_from_slice(&[0x1b, b'[', b'M', 32 + 3, x, y]);
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
        assert_eq!(key_bytes(Key::Esc, NORMAL, false, false), [0x1b]);
        assert_eq!(key_bytes(Key::Tab, NORMAL, false, false), b"\t");
        assert_eq!(key_bytes(Key::Up, NORMAL, false, false), b"\x1b[A");
        let app = Modes {
            app_cursor: true,
            ..NORMAL
        };
        assert_eq!(key_bytes(Key::Up, app, false, false), b"\x1bOA");
        assert_eq!(key_bytes(Key::Left, NORMAL, true, false), b"\x1b[1;5D");
        assert_eq!(key_bytes(Key::Home, NORMAL, false, false), b"\x1b[H");
        assert_eq!(key_bytes(Key::PageUp, NORMAL, false, false), b"\x1b[5~");
        assert_eq!(key_bytes(Key::Delete, NORMAL, false, false), b"\x1b[3~");
        assert_eq!(key_bytes(Key::Backspace, NORMAL, false, true), [0x1b, 0x7f]);
        assert_eq!(key_bytes(Key::Backspace, NORMAL, true, false), [0x08]);
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
