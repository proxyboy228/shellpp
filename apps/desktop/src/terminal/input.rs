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
use shellpp_terminal::keys::{self, Key, Mods};

/// What plain Backspace sends: the host's Backspace setting (UI 3.1).
pub use shellpp_terminal::keys::{BackspaceKey, control_byte};

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

/// The key a GPUI key name stands for, with its sequence in the shared
/// table (`shellpp_terminal::keys`, D224).
fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "up" => Key::Up,
        "down" => Key::Down,
        "right" => Key::Right,
        "left" => Key::Left,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        function if function.len() > 1 && function.starts_with('f') => {
            Key::F(function[1..].parse().ok()?)
        }
        _ => return None,
    })
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
    let shift_scrolls = mods.shift && !alt_screen && !mods.control && !mods.alt;
    match key {
        "home" if shift_scrolls => return KeyAction::ScrollTop,
        "end" if shift_scrolls => return KeyAction::ScrollBottom,
        "pageup" if shift_scrolls => return KeyAction::ScrollPageUp,
        "pagedown" if shift_scrolls => return KeyAction::ScrollPageDown,
        _ => {}
    }
    let shared = Mods {
        shift: mods.shift,
        alt: mods.alt,
        ctrl: mods.control,
    };
    let bytes = if let Some(named) = named_key(key) {
        match keys::key_bytes(
            named,
            shared,
            mode.contains(TermMode::APP_CURSOR),
            backspace,
        ) {
            Some(bytes) => bytes,
            None => return KeyAction::Ignore,
        }
    } else if mods.control {
        // Ctrl+letter (and Ctrl+Alt+letter with an ESC prefix): the C0 byte.
        match control_byte(key) {
            Some(byte) => keys::alt_prefixed(mods.alt, vec![byte]),
            None => return KeyAction::Ignore,
        }
    } else if mods.alt {
        // Alt+key: ESC and the character the key types.
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
            Some(text) => keys::alt_prefixed(true, text.into_bytes()),
            None => return KeyAction::Ignore,
        }
    } else {
        // Printable text arrives through the input handler.
        return KeyAction::Ignore;
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

    /// Bytes as text: printable ASCII as it is, the rest as `\xNN`.
    fn shown(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|byte| match byte {
                0x21..=0x7e if *byte != b'\\' => char::from(*byte).to_string(),
                _ => format!("\\x{byte:02x}"),
            })
            .collect()
    }

    /// What every key does with every modifier, in normal and application
    /// cursor mode and on the alternate screen, with both Backspace
    /// settings: one line per case.
    fn key_table() -> String {
        let mut names: Vec<String> = [
            "enter",
            "tab",
            "escape",
            "backspace",
            "up",
            "down",
            "right",
            "left",
            "home",
            "end",
            "pageup",
            "pagedown",
            "insert",
            "delete",
            "space",
        ]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
        names.extend((1..=13).map(|n| format!("f{n}")));
        names.extend(
            [
                "a", "c", "k", "z", "1", "2", "3", "4", "5", "6", "7", "8", "9", "@", "[", "\\",
                "]", "^", "_", "-", "/", "?", ";", "'", ",", ".", "`",
            ]
            .iter()
            .map(|name| (*name).to_owned()),
        );
        let mut out = String::new();
        for name in &names {
            for bits in 0..8u8 {
                let mut keystroke = key("a");
                keystroke.key = name.clone();
                keystroke.modifiers.shift = bits & 1 != 0;
                keystroke.modifiers.alt = bits & 2 != 0;
                keystroke.modifiers.control = bits & 4 != 0;
                for (mode_name, mode) in [
                    ("normal", TermMode::default()),
                    ("app-cursor", TermMode::APP_CURSOR),
                    ("alt-screen", TermMode::ALT_SCREEN),
                ] {
                    for backspace in [BackspaceKey::Del, BackspaceKey::CtrlH] {
                        let action = match map_key(&keystroke, mode, backspace) {
                            KeyAction::Send(bytes) => shown(&bytes),
                            other => format!("{other:?}"),
                        };
                        out.push_str(&format!(
                            "{}{}{}{name}\t{mode_name}\t{backspace:?}\t{action}\n",
                            if bits & 4 != 0 { "ctrl-" } else { "" },
                            if bits & 2 != 0 { "alt-" } else { "" },
                            if bits & 1 != 0 { "shift-" } else { "" },
                        ));
                    }
                }
            }
        }
        out
    }

    /// The whole table stays what the desktop sent before the mapping moved
    /// into `shellpp-terminal` (run 16, D224): `key_table.txt` was written
    /// from the desktop's own code then. `SHELLPP_WRITE_KEY_TABLE=1` writes
    /// it again (only for a change made on purpose).
    #[test]
    fn every_key_sends_what_the_table_says() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/terminal/key_table.txt");
        let table = key_table();
        if std::env::var_os("SHELLPP_WRITE_KEY_TABLE").is_some() {
            std::fs::write(path, &table).expect("written");
        }
        let expected = std::fs::read_to_string(path)
            .expect("key_table.txt")
            .replace("\r\n", "\n");
        for (line, (got, want)) in table.lines().zip(expected.lines()).enumerate() {
            assert_eq!(got, want, "line {}", line + 1);
        }
        assert_eq!(table.lines().count(), expected.lines().count());
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
