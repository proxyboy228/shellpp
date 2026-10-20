//! Mouse reports for full-screen programs (htop, mc, vim with `mouse=a`):
//! the xterm protocol in the SGR encoding (mode 1006), the UTF-8 encoding
//! (mode 1005) and the original one. While a program asked for mouse
//! events, holding Shift keeps the mouse for the terminal itself
//! (selection, paste).

use alacritty_terminal::term::TermMode;
use gpui::Modifiers;

/// A mouse button as the protocol numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Motion without a pressed button (mode 1003).
    None,
}

impl ReportButton {
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

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Press,
    Release,
    Motion,
}

/// Whether the program asked for mouse events.
pub fn reporting(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// Whether a motion event is reported: with a pressed button in mode 1002
/// or 1003, without one only in 1003.
pub fn reports_motion(mode: TermMode, button_pressed: bool) -> bool {
    mode.contains(TermMode::MOUSE_MOTION) || (button_pressed && mode.contains(TermMode::MOUSE_DRAG))
}

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
    let mut code = button.code();
    if modifiers.shift {
        code += 4;
    }
    if modifiers.alt {
        code += 8;
    }
    if modifiers.control {
        code += 16;
    }
    if kind == ReportKind::Motion {
        code += 32;
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if kind == ReportKind::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{};{}{suffix}", column + 1, line + 1).into_bytes());
    }
    // The older encodings report every release as button 3.
    if kind == ReportKind::Release {
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

    fn sgr() -> TermMode {
        TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE
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
