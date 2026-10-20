//! Keyboard input (UI 12.5, "Terminal focus"): which keys the terminal
//! sends to the shell and as which bytes (xterm conventions).
//!
//! While the terminal has the focus, the app keeps only Ctrl+1..9, `Ctrl+,`,
//! the Ctrl+Shift shortcuts, Ctrl+Alt+arrows and Alt+Left/Right (bound in
//! the shell's key context). Everything else goes to the shell, in
//! particular Ctrl+letter without Shift (Ctrl+L clears the screen, Ctrl+P
//! recalls the previous command, Ctrl+K deletes to the end of the line) and
//! Ctrl+Alt+letter. The terminal's key context disables app bindings of
//! those keys ([`super::view::bind_keys`]); this module maps them.
//!
//! Printable text is not taken here: it arrives through the platform input
//! handler (IME aware). With AltGr (Ctrl+Alt on Windows) a key that types a
//! character also goes that way, so no shortcut fires (UI 12.5).

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

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
    fn byte(self) -> u8 {
        match self {
            Self::Del => 0x7f,
            Self::CtrlH => 0x08,
        }
    }
}

/// What a key press does in the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Send these bytes to the shell.
    Send(Vec<u8>),
    /// Scroll the scrollback by one page (Shift+PgUp / Shift+PgDn).
    ScrollPageUp,
    ScrollPageDown,
    /// Scroll to the top / bottom of the scrollback (Shift+Home / Shift+End).
    ScrollTop,
    ScrollBottom,
    /// Not a terminal key: the text input handler or the app handles it.
    Ignore,
}

/// The xterm modifier parameter: 1 + Shift 1 + Alt 2 + Ctrl 4.
fn modifier_parameter(keystroke: &Keystroke) -> u8 {
    let mods = keystroke.modifiers;
    1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.control)
}

/// `ESC [ 1 ; m X` with modifiers, else `ESC [ X` (or `ESC O X` for the
/// cursor keys in application cursor mode).
fn cursor_key(final_byte: char, keystroke: &Keystroke, mode: TermMode) -> Vec<u8> {
    let modifier = modifier_parameter(keystroke);
    if modifier > 1 {
        format!("\x1b[1;{modifier}{final_byte}").into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

/// `ESC [ n ~` or `ESC [ n ; m ~`.
fn tilde_key(number: u8, keystroke: &Keystroke) -> Vec<u8> {
    let modifier = modifier_parameter(keystroke);
    if modifier > 1 {
        format!("\x1b[{number};{modifier}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

/// F1-F4 are `ESC O P..S` (`ESC [ 1 ; m P..S` with modifiers).
fn function_key(number: u8, keystroke: &Keystroke) -> Option<Vec<u8>> {
    let modifier = modifier_parameter(keystroke);
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
        5 => tilde_key(15, keystroke),
        6 => tilde_key(17, keystroke),
        7 => tilde_key(18, keystroke),
        8 => tilde_key(19, keystroke),
        9 => tilde_key(20, keystroke),
        10 => tilde_key(21, keystroke),
        11 => tilde_key(23, keystroke),
        12 => tilde_key(24, keystroke),
        _ => return None,
    })
}

/// The C0 byte of Ctrl+`key` (Ctrl+A = 0x01, Ctrl+[ = ESC, Ctrl+Space = NUL).
pub fn control_byte(key: &str) -> Option<u8> {
    if key == "space" {
        return Some(0);
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match ch.to_ascii_lowercase() {
        letter @ 'a'..='z' => Some(letter as u8 - b'a' + 1),
        '@' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '/' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Whether a keystroke with Ctrl+Alt types a character (AltGr on Windows).
fn is_alt_gr_text(keystroke: &Keystroke) -> bool {
    let mods = keystroke.modifiers;
    mods.control
        && mods.alt
        && keystroke
            .key_char
            .as_deref()
            .is_some_and(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// Maps a key press to what the terminal does with it.
pub fn map_key(keystroke: &Keystroke, mode: TermMode, backspace: BackspaceKey) -> KeyAction {
    let mods = keystroke.modifiers;
    if mods.platform || mods.function {
        return KeyAction::Ignore;
    }
    if is_alt_gr_text(keystroke) {
        return KeyAction::Ignore;
    }
    let alt_screen = mode.contains(TermMode::ALT_SCREEN);
    let key = keystroke.key.as_str();
    // Ctrl+Shift+letter belongs to the app (UI 12.5), bound or not.
    if mods.control && mods.shift && key.chars().count() == 1 {
        return KeyAction::Ignore;
    }
    let escape_prefixed = |bytes: Vec<u8>| -> Vec<u8> {
        if mods.alt {
            let mut prefixed = Vec::with_capacity(bytes.len() + 1);
            prefixed.push(0x1b);
            prefixed.extend(bytes);
            prefixed
        } else {
            bytes
        }
    };
    let bytes = match key {
        "enter" => escape_prefixed(vec![b'\r']),
        "tab" if mods.shift => b"\x1b[Z".to_vec(),
        "tab" => escape_prefixed(vec![b'\t']),
        "escape" => escape_prefixed(vec![0x1b]),
        "backspace" if mods.control => escape_prefixed(vec![0x08]),
        "backspace" => escape_prefixed(vec![backspace.byte()]),
        "up" => cursor_key('A', keystroke, mode),
        "down" => cursor_key('B', keystroke, mode),
        "right" => cursor_key('C', keystroke, mode),
        "left" => cursor_key('D', keystroke, mode),
        "home" if mods.shift && !alt_screen && !mods.control && !mods.alt => {
            return KeyAction::ScrollTop;
        }
        "end" if mods.shift && !alt_screen && !mods.control && !mods.alt => {
            return KeyAction::ScrollBottom;
        }
        "home" => cursor_key('H', keystroke, mode),
        "end" => cursor_key('F', keystroke, mode),
        "pageup" if mods.shift && !alt_screen && !mods.control && !mods.alt => {
            return KeyAction::ScrollPageUp;
        }
        "pagedown" if mods.shift && !alt_screen && !mods.control && !mods.alt => {
            return KeyAction::ScrollPageDown;
        }
        "pageup" => tilde_key(5, keystroke),
        "pagedown" => tilde_key(6, keystroke),
        "insert" => tilde_key(2, keystroke),
        "delete" => tilde_key(3, keystroke),
        function if function.len() > 1 && function.starts_with('f') => {
            match function[1..]
                .parse::<u8>()
                .ok()
                .and_then(|n| function_key(n, keystroke))
            {
                Some(bytes) => bytes,
                None => return KeyAction::Ignore,
            }
        }
        // Ctrl+letter (and Ctrl+Alt+letter with an ESC prefix): the C0 byte.
        _ if mods.control => match control_byte(key) {
            Some(byte) => escape_prefixed(vec![byte]),
            None => return KeyAction::Ignore,
        },
        // Alt+key: ESC and the character the key types.
        _ if mods.alt => {
            let text = keystroke
                .key_char
                .clone()
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    (key == "space").then(|| " ".to_owned()).or_else(|| {
                        (key.chars().count() == 1).then(|| {
                            if mods.shift {
                                key.to_uppercase()
                            } else {
                                key.to_owned()
                            }
                        })
                    })
                });
            match text {
                Some(text) => escape_prefixed(text.into_bytes()),
                None => return KeyAction::Ignore,
            }
        }
        // Printable text arrives through the input handler.
        _ => return KeyAction::Ignore,
    };
    KeyAction::Send(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Keystroke {
        Keystroke::parse(text).expect("keystroke")
    }

    fn send(text: &str) -> Vec<u8> {
        match map_key(&key(text), TermMode::default(), BackspaceKey::Del) {
            KeyAction::Send(bytes) => bytes,
            other => panic!("{text}: {other:?}"),
        }
    }

    /// UI 12.5: Ctrl+letter without Shift goes to the shell.
    #[test]
    fn ctrl_letters_go_to_the_shell() {
        assert_eq!(send("ctrl-l"), [0x0c]);
        assert_eq!(send("ctrl-p"), [0x10]);
        assert_eq!(send("ctrl-k"), [0x0b]);
        assert_eq!(send("ctrl-c"), [0x03]);
        assert_eq!(send("ctrl-a"), [0x01]);
        assert_eq!(send("ctrl-z"), [0x1a]);
        assert_eq!(send("ctrl-space"), [0x00]);
        assert_eq!(send("ctrl-["), [0x1b]);
        // Ctrl+Alt+letter: ESC and the control byte (Ctrl+Alt+S is the shell's).
        assert_eq!(send("ctrl-alt-s"), [0x1b, 0x13]);
    }

    /// UI 12.5: Ctrl+Shift shortcuts belong to the app.
    #[test]
    fn ctrl_shift_letters_stay_with_the_app() {
        for text in [
            "ctrl-shift-c",
            "ctrl-shift-v",
            "ctrl-shift-l",
            "ctrl-shift-p",
        ] {
            assert_eq!(
                map_key(&key(text), TermMode::default(), BackspaceKey::Del),
                KeyAction::Ignore
            );
        }
    }

    #[test]
    fn altgr_text_is_typed_not_sent_as_a_shortcut() {
        let mut altgr_q = key("ctrl-alt-q");
        altgr_q.key_char = Some("@".to_owned());
        assert_eq!(
            map_key(&altgr_q, TermMode::default(), BackspaceKey::Del),
            KeyAction::Ignore
        );
    }

    #[test]
    fn printable_keys_are_left_to_the_input_handler() {
        for text in ["a", "shift-a", "1", "space"] {
            assert_eq!(
                map_key(&key(text), TermMode::default(), BackspaceKey::Del),
                KeyAction::Ignore
            );
        }
    }

    /// UI 3.1: a host with Backspace = ^H gets BS for Backspace (also with
    /// Alt); Ctrl+Backspace stays BS either way.
    #[test]
    fn backspace_follows_the_host_setting() {
        let send_h = |text: &str| map_key(&key(text), TermMode::default(), BackspaceKey::CtrlH);
        assert_eq!(send_h("backspace"), KeyAction::Send(vec![0x08]));
        assert_eq!(send_h("alt-backspace"), KeyAction::Send(vec![0x1b, 0x08]));
        assert_eq!(send_h("ctrl-backspace"), KeyAction::Send(vec![0x08]));
        assert_eq!(BackspaceKey::default(), BackspaceKey::Del);
    }

    #[test]
    fn special_keys() {
        assert_eq!(send("enter"), b"\r");
        assert_eq!(send("backspace"), [0x7f]);
        assert_eq!(send("ctrl-backspace"), [0x08]);
        assert_eq!(send("alt-backspace"), [0x1b, 0x7f]);
        assert_eq!(send("escape"), [0x1b]);
        assert_eq!(send("tab"), b"\t");
        assert_eq!(send("shift-tab"), b"\x1b[Z");
        assert_eq!(send("up"), b"\x1b[A");
        assert_eq!(send("ctrl-left"), b"\x1b[1;5D");
        assert_eq!(send("shift-up"), b"\x1b[1;2A");
        assert_eq!(send("home"), b"\x1b[H");
        assert_eq!(send("delete"), b"\x1b[3~");
        assert_eq!(send("pageup"), b"\x1b[5~");
        assert_eq!(send("ctrl-pagedown"), b"\x1b[6;5~");
        assert_eq!(send("f1"), b"\x1bOP");
        assert_eq!(send("shift-f1"), b"\x1b[1;2P");
        assert_eq!(send("f5"), b"\x1b[15~");
        assert_eq!(send("f12"), b"\x1b[24~");
        assert_eq!(send("alt-x"), b"\x1bx");
        assert_eq!(send("alt-shift-x"), b"\x1bX");
    }

    #[test]
    fn application_cursor_mode_and_the_alternate_screen() {
        let app = TermMode::default() | TermMode::APP_CURSOR;
        assert_eq!(
            map_key(&key("up"), app, BackspaceKey::Del),
            KeyAction::Send(b"\x1bOA".to_vec())
        );
        // Shift+PgUp scrolls the scrollback, except in full-screen programs.
        assert_eq!(
            map_key(&key("shift-pageup"), TermMode::default(), BackspaceKey::Del),
            KeyAction::ScrollPageUp
        );
        assert_eq!(
            map_key(
                &key("shift-pagedown"),
                TermMode::default(),
                BackspaceKey::Del
            ),
            KeyAction::ScrollPageDown
        );
        let alt = TermMode::default() | TermMode::ALT_SCREEN;
        assert_eq!(
            map_key(&key("shift-pageup"), alt, BackspaceKey::Del),
            KeyAction::Send(b"\x1b[5;2~".to_vec())
        );
    }
}
