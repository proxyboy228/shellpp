//! What a key and a mouse event send to the program (xterm's sequences):
//! one table for the desktop and the phone (architecture 6.2; run 16,
//! D224). Each client turns its own events into a [`Key`] with [`Mods`] (the
//! desktop's GPUI keystrokes, the phone's extra keys and keyboards) and the
//! mouse into a [`MouseButton`] and [`MouseKind`]; the bytes come from here.
//!
//! The table is the desktop's from before the move: the desktop's tests
//! check every key with every modifier, in both cursor modes and both
//! Backspace settings, and every mouse report, against the files written
//! from its code then (`apps/desktop/src/terminal/key_table.txt`,
//! `mouse_table.txt`).

use alacritty_terminal::term::TermMode;

/// A key that sends a sequence (printable text and Ctrl/Alt with a
/// character go through [`control_byte`] and [`alt_prefixed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Tab,
    Escape,
    Backspace,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1-F12 (others send nothing).
    F(u8),
}

/// The modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// What plain Backspace sends: the host's Backspace setting (UI 3.1).
/// Ctrl+Backspace always sends BS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackspaceKey {
    /// DEL, 0x7f (the setting's "Default" and "^?").
    #[default]
    Del,
    /// BS, 0x08 (the setting's "^H").
    CtrlH,
}

impl BackspaceKey {
    pub fn byte(self) -> u8 {
        match self {
            Self::Del => 0x7f,
            Self::CtrlH => 0x08,
        }
    }
}

/// The xterm modifier parameter: 1 + Shift 1 + Alt 2 + Ctrl 4.
fn modifier_parameter(mods: Mods) -> u8 {
    1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl)
}

/// `ESC [ 1 ; m X` with modifiers, else `ESC [ X` (or `ESC O X` for the
/// cursor keys in application cursor mode).
fn cursor_key(final_byte: char, mods: Mods, app_cursor: bool) -> Vec<u8> {
    let modifier = modifier_parameter(mods);
    if modifier > 1 {
        format!("\x1b[1;{modifier}{final_byte}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

/// `ESC [ n ~` or `ESC [ n ; m ~`.
fn tilde_key(number: u8, mods: Mods) -> Vec<u8> {
    let modifier = modifier_parameter(mods);
    if modifier > 1 {
        format!("\x1b[{number};{modifier}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

/// F1-F4 are `ESC O P..S` (`ESC [ 1 ; m P..S` with modifiers).
fn function_key(number: u8, mods: Mods) -> Option<Vec<u8>> {
    let modifier = modifier_parameter(mods);
    let ss3 = |final_byte: char| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else {
            format!("\x1bO{final_byte}").into_bytes()
        }
    };
    Some(match number {
        1 => ss3('P'),
        2 => ss3('Q'),
        3 => ss3('R'),
        4 => ss3('S'),
        5 => tilde_key(15, mods),
        6 => tilde_key(17, mods),
        7 => tilde_key(18, mods),
        8 => tilde_key(19, mods),
        9 => tilde_key(20, mods),
        10 => tilde_key(21, mods),
        11 => tilde_key(23, mods),
        12 => tilde_key(24, mods),
        _ => return None,
    })
}

/// `bytes` after ESC when Alt is held (Alt+key: ESC and what the key sends).
pub fn alt_prefixed(alt: bool, bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend(bytes);
        prefixed
    } else {
        bytes
    }
}

/// The bytes of `key` with `mods`, in application cursor mode or not, with
/// the host's Backspace setting; `None` for a key that sends nothing (F13
/// and above).
pub fn key_bytes(
    key: Key,
    mods: Mods,
    app_cursor: bool,
    backspace: BackspaceKey,
) -> Option<Vec<u8>> {
    Some(match key {
        Key::Enter => alt_prefixed(mods.alt, vec![b'\r']),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt_prefixed(mods.alt, vec![b'\t']),
        Key::Escape => alt_prefixed(mods.alt, vec![0x1b]),
        Key::Backspace if mods.ctrl => alt_prefixed(mods.alt, vec![0x08]),
        Key::Backspace => alt_prefixed(mods.alt, vec![backspace.byte()]),
        Key::Up => cursor_key('A', mods, app_cursor),
        Key::Down => cursor_key('B', mods, app_cursor),
        Key::Right => cursor_key('C', mods, app_cursor),
        Key::Left => cursor_key('D', mods, app_cursor),
        Key::Home => cursor_key('H', mods, app_cursor),
        Key::End => cursor_key('F', mods, app_cursor),
        Key::PageUp => tilde_key(5, mods),
        Key::PageDown => tilde_key(6, mods),
        Key::Insert => tilde_key(2, mods),
        Key::Delete => tilde_key(3, mods),
        Key::F(number) => function_key(number, mods)?,
    })
}

/// The C0 byte of Ctrl+`key`, a key name or one character (Ctrl+A = 0x01,
/// Ctrl+[ = ESC, Ctrl+Space = NUL).
pub fn control_byte(key: &str) -> Option<u8> {
    if key == "space" {
        return Some(0);
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    control_of(ch)
}

/// The C0 byte of Ctrl with the character `ch`, if it has one.
pub fn control_of(ch: char) -> Option<u8> {
    match ch.to_ascii_lowercase() {
        letter @ 'a'..='z' => Some(letter as u8 - b'a' + 1),
        '@' | '2' | ' ' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '/' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// A mouse button as the protocol numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Motion without a pressed button (mode 1003).
    None,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::None => 3,
            Self::WheelUp => 64,
            Self::WheelDown => 65,
        }
    }
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Motion,
}

/// Whether the program asked for mouse events.
pub fn mouse_reporting(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// Whether a motion event is reported: with a pressed button in mode 1002
/// or 1003, without one only in 1003.
pub fn reports_motion(mode: TermMode, button_pressed: bool) -> bool {
    mode.contains(TermMode::MOUSE_MOTION) || (button_pressed && mode.contains(TermMode::MOUSE_DRAG))
}

/// The report of a mouse event at 0-based `column` and `line` in the
/// encoding the program asked for: SGR (mode 1006), UTF-8 (mode 1005) or the
/// original one; `None` when the encoding cannot express it (the original
/// encoding stops at column and line 223).
pub fn mouse_report(
    mode: TermMode,
    button: MouseButton,
    kind: MouseKind,
    column: usize,
    line: usize,
    mods: Mods,
) -> Option<Vec<u8>> {
    let mut code = button.code();
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    if kind == MouseKind::Motion {
        code += 32;
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if kind == MouseKind::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{};{}{suffix}", column + 1, line + 1).into_bytes());
    }
    // The older encodings report every release as button 3.
    if kind == MouseKind::Release {
        code = (code & !3) | 3;
    }
    let mut bytes = b"\x1b[M".to_vec();
    let values = [u32::from(code) + 32, column as u32 + 33, line as u32 + 33];
    if mode.contains(TermMode::UTF8_MOUSE) {
        for value in values {
            let ch = char::from_u32(value).filter(|_| value < 2048)?;
            let mut buffer = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
        }
    } else {
        for value in values {
            bytes.push(u8::try_from(value).ok()?);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_with_modifiers() {
        let none = Mods::default();
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(
            key_bytes(Key::Up, none, false, BackspaceKey::Del).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            key_bytes(Key::Up, none, true, BackspaceKey::Del).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes(Key::Left, ctrl, true, BackspaceKey::Del).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            key_bytes(Key::F(5), ctrl, false, BackspaceKey::Del).unwrap(),
            b"\x1b[15;5~"
        );
        assert_eq!(key_bytes(Key::F(13), none, false, BackspaceKey::Del), None);
        assert_eq!(
            key_bytes(Key::Backspace, none, false, BackspaceKey::CtrlH).unwrap(),
            [0x08]
        );
        assert_eq!(
            key_bytes(Key::Backspace, ctrl, false, BackspaceKey::Del).unwrap(),
            [0x08]
        );
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(
            key_bytes(Key::Tab, shift, false, BackspaceKey::Del).unwrap(),
            b"\x1b[Z"
        );
        assert_eq!(control_of('c'), Some(0x03));
        assert_eq!(control_of('é'), None);
    }

    #[test]
    fn a_click_in_each_encoding() {
        let click = TermMode::MOUSE_REPORT_CLICK;
        let none = Mods::default();
        assert_eq!(
            mouse_report(
                click | TermMode::SGR_MOUSE,
                MouseButton::Left,
                MouseKind::Press,
                9,
                4,
                none
            )
            .unwrap(),
            b"\x1b[<0;10;5M"
        );
        assert_eq!(
            mouse_report(click, MouseButton::Left, MouseKind::Release, 9, 4, none).unwrap(),
            [0x1b, b'[', b'M', 35, 42, 37]
        );
        assert_eq!(
            mouse_report(click, MouseButton::Left, MouseKind::Press, 300, 0, none),
            None
        );
    }
}
