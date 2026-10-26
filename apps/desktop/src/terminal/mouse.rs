//! Mouse reports for full-screen programs (htop, mc, vim with `mouse=a`):
//! the xterm protocol in the SGR encoding (mode 1006), the UTF-8 encoding
//! (mode 1005) and the original one. While a program asked for mouse
//! events, holding Shift keeps the mouse for the terminal itself
//! (selection, paste).

use alacritty_terminal::term::TermMode;
use gpui::Modifiers;
use shellpp_terminal::keys::{self, Mods};

/// A mouse button and what it did, as the protocol numbers them (the shared
/// table, `shellpp_terminal::keys`, D224).
pub use shellpp_terminal::keys::{
    MouseButton as ReportButton, MouseKind as ReportKind, mouse_reporting as reporting,
    reports_motion,
};

/// The report of a mouse event at 0-based `column` and `line`, or `None`
/// when the encoding cannot express it (the original encoding stops at
/// column and line 223).
pub fn report(
    mode: TermMode,
    button: ReportButton,
    kind: ReportKind,
    column: usize,
    line: usize,
    modifiers: Modifiers,
) -> Option<Vec<u8>> {
    let mods = Mods {
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.control,
    };
    keys::mouse_report(mode, button, kind, column, line, mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sgr() -> TermMode {
        TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE
    }

    /// Every report: each button, kind and modifier, in the three
    /// encodings, at the corners and past the limits of the old ones.
    fn report_table() -> String {
        let mut out = String::new();
        let buttons = [
            ReportButton::Left,
            ReportButton::Middle,
            ReportButton::Right,
            ReportButton::WheelUp,
            ReportButton::WheelDown,
            ReportButton::None,
        ];
        for (mode_name, mode) in [
            ("x10", TermMode::MOUSE_REPORT_CLICK),
            ("sgr", sgr()),
            ("utf8", TermMode::MOUSE_REPORT_CLICK | TermMode::UTF8_MOUSE),
        ] {
            for button in buttons {
                for kind in [ReportKind::Press, ReportKind::Release, ReportKind::Motion] {
                    for bits in 0..8u8 {
                        let modifiers = Modifiers {
                            shift: bits & 1 != 0,
                            alt: bits & 2 != 0,
                            control: bits & 4 != 0,
                            ..Modifiers::default()
                        };
                        for (column, line) in [
                            (0, 0),
                            (9, 4),
                            (222, 222),
                            (223, 0),
                            (300, 1),
                            (2014, 3),
                            (2015, 3),
                        ] {
                            let shown = match report(mode, button, kind, column, line, modifiers) {
                                Some(bytes) => bytes
                                    .iter()
                                    .map(|byte| format!("{byte:02x}"))
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                None => "none".to_owned(),
                            };
                            out.push_str(&format!("{mode_name}\t{button:?}\t{kind:?}\t{bits}\t{column},{line}\t{shown}\n"));
                        }
                    }
                }
            }
        }
        out
    }

    /// The reports stay what the desktop sent before the encoding moved
    /// into `shellpp-terminal` (run 16, D224); `SHELLPP_WRITE_KEY_TABLE=1`
    /// writes `mouse_table.txt` again.
    #[test]
    fn every_report_is_what_the_table_says() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/terminal/mouse_table.txt");
        let table = report_table();
        if std::env::var_os("SHELLPP_WRITE_KEY_TABLE").is_some() {
            std::fs::write(path, &table).expect("written");
        }
        let expected = std::fs::read_to_string(path)
            .expect("mouse_table.txt")
            .replace("\r\n", "\n");
        for (line, (got, want)) in table.lines().zip(expected.lines()).enumerate() {
            assert_eq!(got, want, "line {}", line + 1);
        }
        assert_eq!(table.lines().count(), expected.lines().count());
    }

    #[test]
    fn sgr_reports() {
        let none = Modifiers::default();
        assert_eq!(
            report(sgr(), ReportButton::Left, ReportKind::Press, 0, 0, none).unwrap(),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(
            report(sgr(), ReportButton::Left, ReportKind::Release, 9, 4, none).unwrap(),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(
            report(
                sgr(),
                ReportButton::WheelDown,
                ReportKind::Press,
                2,
                3,
                none
            )
            .unwrap(),
            b"\x1b[<65;3;4M"
        );
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(
            report(sgr(), ReportButton::Right, ReportKind::Motion, 300, 1, ctrl).unwrap(),
            b"\x1b[<50;301;2M"
        );
    }

    #[test]
    fn original_encoding_and_its_limit() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        let none = Modifiers::default();
        assert_eq!(
            report(mode, ReportButton::Left, ReportKind::Press, 0, 0, none).unwrap(),
            [0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            report(mode, ReportButton::Left, ReportKind::Release, 1, 2, none).unwrap(),
            [0x1b, b'[', b'M', 35, 34, 35]
        );
        assert_eq!(
            report(mode, ReportButton::Left, ReportKind::Press, 300, 0, none),
            None
        );
        let utf8 = mode | TermMode::UTF8_MOUSE;
        let bytes = report(utf8, ReportButton::Left, ReportKind::Press, 300, 0, none).unwrap();
        assert_eq!(&bytes[..4], [0x1b, b'[', b'M', 32]);
        assert_eq!(
            std::str::from_utf8(&bytes[4..]).unwrap().chars().next(),
            Some('\u{14D}')
        );
    }

    #[test]
    fn which_modes_report_motion() {
        assert!(!reporting(TermMode::default()));
        assert!(reporting(TermMode::MOUSE_DRAG));
        assert!(reports_motion(TermMode::MOUSE_DRAG, true));
        assert!(!reports_motion(TermMode::MOUSE_DRAG, false));
        assert!(reports_motion(TermMode::MOUSE_MOTION, false));
    }
}
