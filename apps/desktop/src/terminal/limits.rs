//! Limits on what terminal output can make the emulator store
//! (docs/security.md, section 12). The feed queue bounds the bytes in front
//! of the parser; these bound the state the parsed bytes become:
//!
//! - [`OscLimiter`]: vte (with `std`) collects an OSC string in a `Vec`
//!   without a limit, so `ESC ] 2 ;` followed by an endless payload would
//!   grow memory at link speed. The limiter keeps at most
//!   [`MAX_OSC_PAYLOAD`] bytes of each OSC string and drops the rest; the
//!   terminator still reaches the parser.
//! - [`Capped`]: alacritty_terminal appends every zero-width character
//!   (combining marks, joiners) to the cell before the cursor, without a
//!   limit. The handler drops a zero-width character when that cell already
//!   holds [`MAX_ZEROWIDTH`] of them.

use std::borrow::Cow;

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys, PrivateMode, Rgb,
    ScpCharPath, ScpUpdateMode, StandardCharset, TabulationClearMode, cursor_icon::CursorIcon,
};
use unicode_width::UnicodeWidthChar as _;

/// Most zero-width characters kept in one cell. Real text puts a few
/// combining marks on a character; anything beyond this is dropped.
pub const MAX_ZEROWIDTH: usize = 16;

/// Most payload bytes kept of one OSC string. Titles are cut to 256
/// characters anyway and OSC 52 is off; OSC 8 links and color
/// specifications fit.
pub const MAX_OSC_PAYLOAD: usize = 4096;

const ESC: u8 = 0x1B;

/// Where the byte stream is, as far as OSC strings are concerned. It
/// follows vte 0.15's state machine: ESC enters the escape state from any
/// state; there `]` starts an OSC string, C0 controls (except CAN and SUB),
/// DEL, ESC and bytes from 0x80 keep the escape state, and anything else
/// leaves it. An OSC string ends on BEL, CAN, SUB or ESC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OscState {
    #[default]
    Other,
    Escape,
    /// Inside an OSC string, with the payload bytes seen so far.
    Osc(usize),
}

/// Drops OSC payload bytes beyond [`MAX_OSC_PAYLOAD`]. Keeps its state
/// across calls, so a string split over several feeds is still counted.
/// Its buffer is never larger than the largest slice it was given.
#[derive(Debug, Default)]
pub struct OscLimiter {
    state: OscState,
    kept: Vec<u8>,
}

impl OscLimiter {
    /// The bytes to give the parser: `bytes` itself unless something had
    /// to be dropped.
    pub fn filter<'a>(&'a mut self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        let mut dropped = false;
        self.kept.clear();
        let mut index = 0;
        while index < bytes.len() {
            if self.state == OscState::Other {
                // Plain output: jump to the next ESC.
                match bytes[index..].iter().position(|&byte| byte == ESC) {
                    Some(offset) => {
                        if dropped {
                            self.kept.extend_from_slice(&bytes[index..index + offset]);
                        }
                        index += offset;
                    }
                    None => {
                        if dropped {
                            self.kept.extend_from_slice(&bytes[index..]);
                        }
                        break;
                    }
                }
            }
            let byte = bytes[index];
            let keep = self.step(byte);
            if !keep && !dropped {
                dropped = true;
                self.kept.extend_from_slice(&bytes[..index]);
            }
            if keep && dropped {
                self.kept.push(byte);
            }
            index += 1;
        }
        if dropped {
            Cow::Borrowed(&self.kept[..])
        } else {
            Cow::Borrowed(bytes)
        }
    }

    /// Moves to the state after `byte`; whether the byte is kept.
    fn step(&mut self, byte: u8) -> bool {
        match self.state {
            OscState::Other => {
                if byte == ESC {
                    self.state = OscState::Escape;
                }
                true
            }
            OscState::Escape => {
                self.state = match byte {
                    b']' => OscState::Osc(0),
                    0x00..=0x17 | 0x19 | 0x1B..=0x1F | 0x7F..=0xFF => OscState::Escape,
                    _ => OscState::Other,
                };
                true
            }
            OscState::Osc(seen) => match byte {
                0x07 | 0x18 | 0x1A => {
                    self.state = OscState::Other;
                    true
                }
                ESC => {
                    self.state = OscState::Escape;
                    true
                }
                _ if seen >= MAX_OSC_PAYLOAD => false,
                _ => {
                    self.state = OscState::Osc(seen + 1);
                    true
                }
            },
        }
    }
}

/// `Term` as the parser's handler, with the zero-width limit. Every other
/// call is passed on unchanged.
pub struct Capped<'a, T: EventListener>(pub &'a mut Term<T>);

impl<T: EventListener> std::fmt::Debug for Capped<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capped").finish_non_exhaustive()
    }
}

impl<T: EventListener> Capped<'_, T> {
    /// Whether the cell a zero-width character would join is full. The cell
    /// is found the way `Term::input` finds it: the one before the cursor
    /// (the cursor's own when a wrap is pending), and the first cell of a
    /// wide character when that is its spacer.
    fn zerowidth_full(&self) -> bool {
        let grid = self.0.grid();
        let cursor = &grid.cursor;
        let mut column = cursor.point.column;
        if !cursor.input_needs_wrap {
            column.0 = column.0.saturating_sub(1);
        }
        let row = &grid[cursor.point.line];
        if row[column].flags.contains(Flags::WIDE_CHAR_SPACER) {
            column.0 = column.0.saturating_sub(1);
        }
        row[column]
            .zerowidth()
            .is_some_and(|extra| extra.len() >= MAX_ZEROWIDTH)
    }
}

/// Passes the listed `Handler` methods on to the term.
macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(
            #[inline]
            fn $name(&mut self, $($arg: $ty),*) {
                Handler::$name(&mut *self.0, $($arg),*)
            }
        )*
    };
}

impl<T: EventListener> Handler for Capped<'_, T> {
    #[inline]
    fn input(&mut self, c: char) {
        if c.width() == Some(0) && self.zerowidth_full() {
            return;
        }
        Handler::input(&mut *self.0, c);
    }

    forward! {
        set_title(title: Option<String>);
        set_cursor_style(style: Option<CursorStyle>);
        set_cursor_shape(shape: CursorShape);
        goto(line: i32, col: usize);
        goto_line(line: i32);
        goto_col(col: usize);
        insert_blank(count: usize);
        move_up(rows: usize);
        move_down(rows: usize);
        identify_terminal(intermediate: Option<char>);
        device_status(arg: usize);
        move_forward(cols: usize);
        move_backward(cols: usize);
        move_down_and_cr(rows: usize);
        move_up_and_cr(rows: usize);
        put_tab(count: u16);
        backspace();
        carriage_return();
        linefeed();
        bell();
        substitute();
        newline();
        set_horizontal_tabstop();
        scroll_up(rows: usize);
        scroll_down(rows: usize);
        insert_blank_lines(count: usize);
        delete_lines(count: usize);
        erase_chars(count: usize);
        delete_chars(count: usize);
        move_backward_tabs(count: u16);
        move_forward_tabs(count: u16);
        save_cursor_position();
        restore_cursor_position();
        clear_line(mode: LineClearMode);
        clear_screen(mode: ClearMode);
        clear_tabs(mode: TabulationClearMode);
        set_tabs(interval: u16);
        reset_state();
        reverse_index();
        terminal_attribute(attr: Attr);
        set_mode(mode: Mode);
        unset_mode(mode: Mode);
        report_mode(mode: Mode);
        set_private_mode(mode: PrivateMode);
        unset_private_mode(mode: PrivateMode);
        report_private_mode(mode: PrivateMode);
        set_scrolling_region(top: usize, bottom: Option<usize>);
        set_keypad_application_mode();
        unset_keypad_application_mode();
        set_active_charset(index: CharsetIndex);
        configure_charset(index: CharsetIndex, charset: StandardCharset);
        set_color(index: usize, color: Rgb);
        dynamic_color_sequence(prefix: String, index: usize, terminator: &str);
        reset_color(index: usize);
        clipboard_store(clipboard: u8, data: &[u8]);
        clipboard_load(clipboard: u8, terminator: &str);
        decaln();
        push_title();
        pop_title();
        text_area_size_pixels();
        text_area_size_chars();
        set_hyperlink(link: Option<Hyperlink>);
        set_mouse_cursor_icon(icon: CursorIcon);
        report_keyboard_mode();
        push_keyboard_mode(mode: KeyboardModes);
        pop_keyboard_modes(to_pop: u16);
        set_keyboard_mode(mode: KeyboardModes, behavior: KeyboardModesApplyBehavior);
        set_modify_other_keys(mode: ModifyOtherKeys);
        report_modify_other_keys();
        set_scp(char_path: ScpCharPath, update_mode: ScpUpdateMode);
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::terminal::model::GridDims;

    fn filtered(limiter: &mut OscLimiter, bytes: &[u8]) -> Vec<u8> {
        limiter.filter(bytes).into_owned()
    }

    #[test]
    fn short_osc_strings_and_plain_output_pass_unchanged() {
        let mut limiter = OscLimiter::default();
        let input = b"plain \x1b[1mbold\x1b]2;title\x07 \x1b]8;;https://example.test\x1b\\link";
        assert!(matches!(limiter.filter(input), Cow::Borrowed(kept) if kept == input));
        assert_eq!(limiter.state, OscState::Other);
    }

    #[test]
    fn long_osc_payload_is_cut_and_the_terminator_kept() {
        let mut limiter = OscLimiter::default();
        let mut input = b"\x1b]2;".to_vec();
        input.extend(std::iter::repeat_n(b'a', MAX_OSC_PAYLOAD * 3));
        input.extend_from_slice(b"\x07after");
        let out = filtered(&mut limiter, &input);
        // "2;" counts as payload too.
        assert_eq!(out.len(), 2 + MAX_OSC_PAYLOAD + b"\x07after".len());
        assert!(out.ends_with(b"\x07after"));
        assert_eq!(limiter.state, OscState::Other);
    }

    /// The count survives feeds split anywhere; each terminator vte knows
    /// ends the string; a new OSC string starts a new count.
    #[test]
    fn the_state_carries_across_calls_and_every_terminator_ends_the_string() {
        for terminator in [&b"\x07"[..], b"\x18", b"\x1a", b"\x1b\\"] {
            let mut limiter = OscLimiter::default();
            let mut kept = filtered(&mut limiter, b"\x1b");
            kept.extend(filtered(&mut limiter, b"]0;"));
            for _ in 0..10 {
                kept.extend(filtered(&mut limiter, &[b'x'; 1000]));
            }
            kept.extend(filtered(&mut limiter, terminator));
            kept.extend(filtered(&mut limiter, b"\x1b]0;short\x07"));
            // ESC ], the payload up to the cap ("0;" included), the
            // terminator and the short string.
            assert_eq!(kept.len(), 2 + MAX_OSC_PAYLOAD + terminator.len() + 10);
            assert!(kept.ends_with(b"\x1b]0;short\x07"));
        }
    }

    /// Bytes that keep vte in the escape state keep the limiter there too;
    /// an intermediate byte means `]` is a final byte, not an OSC string.
    #[test]
    fn escape_state_follows_vte() {
        let mut limiter = OscLimiter::default();
        let mut input = b"\x1b\x05\xc3]".to_vec();
        input.extend(std::iter::repeat_n(b'a', MAX_OSC_PAYLOAD + 10));
        assert_eq!(filtered(&mut limiter, &input).len(), input.len() - 10);

        let mut limiter = OscLimiter::default();
        let mut input = b"\x1b(]".to_vec();
        input.extend(std::iter::repeat_n(b'a', MAX_OSC_PAYLOAD + 10));
        assert_eq!(filtered(&mut limiter, &input), input);
    }

    fn term(cols: usize) -> Term<VoidListener> {
        Term::new(Config::default(), &GridDims { cols, rows: 2 }, VoidListener)
    }

    fn feed(term: &mut Term<VoidListener>, text: &str) {
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut Capped(term), text.as_bytes());
    }

    fn marks(term: &Term<VoidListener>, line: i32, column: usize) -> usize {
        term.grid()[Line(line)][Column(column)]
            .zerowidth()
            .map_or(0, <[char]>::len)
    }

    #[test]
    fn zero_width_characters_are_capped_per_cell() {
        let mut term = term(10);
        let flood = "\u{301}".repeat(1000);
        feed(&mut term, &format!("a{flood}b\u{301}"));
        assert_eq!(marks(&term, 0, 0), MAX_ZEROWIDTH);
        assert_eq!(marks(&term, 0, 1), 1, "the next cell has its own count");
        // Moving the cursor back to the full cell does not reopen it.
        feed(&mut term, &format!("\x1b[1;2H{flood}"));
        assert_eq!(marks(&term, 0, 0), MAX_ZEROWIDTH);
    }

    /// The marks go where alacritty puts them: on a wide character (not its
    /// spacer) and on the last column while a wrap is pending.
    #[test]
    fn the_cap_applies_to_the_cell_alacritty_writes() {
        let mut term = term(4);
        let flood = "\u{301}".repeat(100);
        feed(&mut term, &format!("你{flood}"));
        assert_eq!(marks(&term, 0, 0), MAX_ZEROWIDTH);
        assert_eq!(marks(&term, 0, 1), 0);
        feed(&mut term, &format!("\rabcd{flood}"));
        assert_eq!(marks(&term, 0, 3), MAX_ZEROWIDTH);
        assert_eq!(marks(&term, 1, 0), 0);
    }
}
