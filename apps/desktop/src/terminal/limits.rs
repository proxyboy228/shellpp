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
//!
//! [`Capped`] also counts the lines that drop off the top of a full
//! scrollback ([`count_evictions`]): positions counted from the oldest line
//! (the search's) stay valid across new output with that count. The
//! scrollback limit covers the grid and the journal together (D85): the
//! journal's lines are older, so they drop first, and clearing the
//! scrollback clears the journal.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex, PoisonError};

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys, NamedPrivateMode,
    PrivateMode, Rgb, ScpCharPath, ScpUpdateMode, StandardCharset, TabulationClearMode,
    cursor_icon::CursorIcon,
};
use unicode_width::UnicodeWidthChar as _;

use super::journal::Journal;

/// Most zero-width characters kept in one cell. Real text puts a few
/// combining marks on a character; anything beyond this is dropped.
pub const MAX_ZEROWIDTH: usize = 16;

/// Most payload bytes kept of one OSC string. Titles are cut to 256
/// characters anyway and OSC 52 is off; OSC 8 links and color
/// specifications fit.
pub const MAX_OSC_PAYLOAD: usize = 4096;

const ESC: u8 = 0x1B;

/// VARIATION SELECTOR-16: the emoji presentation of the character before it.
const VS16: char = '\u{FE0F}';

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

/// Eviction counting of this thread: the scrollback limit of the term
/// parsed here (grid and journal together), the lines dropped so far and
/// the lines of the journal.
#[derive(Clone, Copy, Debug)]
struct Evictions {
    limit: usize,
    dropped: usize,
    journal: usize,
    /// The grid's cap must be set again (the journal was cleared from the
    /// alternate screen).
    recap: bool,
}

thread_local! {
    static EVICTIONS: Cell<Option<Evictions>> = const { Cell::new(None) };
    static JOURNAL: RefCell<Option<Arc<Mutex<Journal>>>> = const { RefCell::new(None) };
    static COLUMNS: Cell<Columns> = const {
        Cell::new(Columns {
            allow: false,
            forced: None,
            changed: false,
        })
    };
}

/// DECSET 40: the program allows DECCOLM (xterm's "Allow 80 ⇒ 132").
const ALLOW_COLUMN_MODE: u16 = 40;
/// The widths of DECCOLM.
const COLUMNS_80: u16 = 80;
const COLUMNS_132: u16 = 132;

/// DECCOLM on this parser thread (D90). As in xterm, a program switches
/// the screen to 132 (DECSET 3) or 80 (DECRST 3) columns only after it
/// allowed that with DECSET 40 (vttest does; `tput reset` sends DECRST 3
/// without it, and keeps the width). The grid takes the width at once, in
/// the middle of the output, so what follows is laid out in it; the parser
/// loop then tells the view and the remote side ([`take_column_change`]).
/// RIS ends a forced width and the permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Columns {
    allow: bool,
    /// The width a DECCOLM set, while it lasts.
    forced: Option<u16>,
    /// `forced` changed since [`set_forced_columns`].
    changed: bool,
}

/// Before parsing a batch: the forced width the terminal has now (the
/// view ends it when its own size changes).
pub(super) fn set_forced_columns(forced: Option<u16>) {
    let mut columns = COLUMNS.get();
    columns.forced = forced;
    columns.changed = false;
    COLUMNS.set(columns);
}

/// After parsing: `Some(width)` when a DECCOLM or RIS changed the forced
/// width (`Some(None)`: back to the view's width).
pub(super) fn take_column_change() -> Option<Option<u16>> {
    let mut columns = COLUMNS.get();
    let changed = columns.changed.then_some(columns.forced);
    columns.changed = false;
    COLUMNS.set(columns);
    changed
}

/// Runs `change` on this thread's journal (the terminal lock is held: the
/// journal's lock is taken after it, as everywhere).
fn with_journal(change: impl FnOnce(&mut Journal)) {
    JOURNAL.with_borrow(|journal| {
        if let Some(journal) = journal {
            change(&mut journal.lock().unwrap_or_else(PoisonError::into_inner));
        }
    });
}

/// Rows moved into the journal on this thread and its oldest lines dropped
/// to keep the buffer within its limit ([`super::journal::compact`]).
pub fn journal_changed(added: usize, dropped: usize) {
    if let Some(mut count) = EVICTIONS.get() {
        count.journal = (count.journal + added).saturating_sub(dropped);
        count.dropped += dropped;
        EVICTIONS.set(Some(count));
    }
}

/// Starts counting, on this thread, the lines that drop off the top of the
/// primary screen's scrollback of the term parsed here through [`Capped`]
/// (its scrollback holds at most `limit` lines). The terminal's parser
/// thread does this once; [`evictions`] reads the count.
///
/// Lines drop when output scrolls a full scrollback (alacritty then reuses
/// the oldest line without telling) and when the scrollback is cleared (ED
/// 3, RIS). To see the first, [`Capped`] gives the grid room for the lines
/// one call can push (a screen) before a call that can scroll, and trims it
/// back to `limit` right after: what it trims is what alacritty would have
/// dropped, and the result is the same grid.
pub fn count_evictions(limit: usize, journal: Arc<Mutex<Journal>>) {
    EVICTIONS.set(Some(Evictions {
        limit,
        dropped: 0,
        journal: 0,
        recap: false,
    }));
    JOURNAL.set(Some(journal));
}

/// Lines dropped off the scrollback so far on this thread (0 when not
/// counting).
pub fn evictions() -> usize {
    EVICTIONS.get().map_or(0, |count| count.dropped)
}

/// `Term` as the parser's handler, with the zero-width limit and the
/// eviction count. Every other call is passed on unchanged.
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

    /// The cell VS16 (U+FE0F) makes wide, if any: the narrow character right
    /// before the cursor, with nothing joined to it yet, which has an emoji
    /// presentation (`❤` before `❤️`), and room for its second cell before
    /// the end of the row (D206). Its column.
    fn widened_by_vs16(&self) -> Option<usize> {
        let grid = self.0.grid();
        let cursor = &grid.cursor;
        if cursor.input_needs_wrap || cursor.point.column.0 == 0 {
            return None;
        }
        let column = cursor.point.column.0 - 1;
        let cell = &grid[cursor.point.line][alacritty_terminal::index::Column(column)];
        let narrow = !cell.flags.intersects(
            Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER,
        );
        let emoji = unicode_width::UnicodeWidthStr::width(
            [cell.c, VS16].iter().collect::<String>().as_str(),
        ) == 2;
        (narrow
            && cell.c.width() == Some(1)
            && cell.zerowidth().is_none_or(<[char]>::is_empty)
            && emoji)
            .then_some(column)
    }

    /// DECSET or DECRST of mode 40 or 3 (see [`Columns`]). Returns whether
    /// the term should see the mode too: DECCOLM clears the screen and the
    /// scrolling region there.
    fn column_mode(&mut self, mode: PrivateMode, set: bool) -> bool {
        let mut columns = COLUMNS.get();
        match mode {
            PrivateMode::Unknown(ALLOW_COLUMN_MODE) => {
                columns.allow = set;
                // DECRST 40 also ends a forced width (the program is done
                // with column switching; a reconnect sends it too).
                if !set && columns.forced.take().is_some() {
                    columns.changed = true;
                }
                COLUMNS.set(columns);
                false
            }
            PrivateMode::Named(NamedPrivateMode::ColumnMode) if columns.allow => {
                let width = if set { COLUMNS_132 } else { COLUMNS_80 };
                if usize::from(width) != self.0.columns() {
                    // As a resize of the view does (`model::apply_size`):
                    // the grid may hold every reflowed line for now, and the
                    // next output trims the buffer to its limit, the
                    // journal's oldest lines first. Not in `counting`, which
                    // would take the shorter history of a wider reflow for a
                    // clear of the scrollback.
                    if !self.0.mode().contains(TermMode::ALT_SCREEN) {
                        let history = self.0.grid().history_size();
                        let limit = EVICTIONS.get().map_or(history, |count| count.limit);
                        self.0.grid_mut().update_history(limit.max(history));
                    }
                    let rows = self.0.screen_lines();
                    self.0.resize(super::model::GridDims {
                        cols: usize::from(width),
                        rows,
                    });
                }
                columns.changed |= columns.forced != Some(width);
                columns.forced = Some(width);
                COLUMNS.set(columns);
                // xterm homes the cursor as well.
                Handler::goto(&mut *self.0, 0, 0);
                true
            }
            _ => true,
        }
    }

    /// Runs a call that can scroll lines into the scrollback or clear it,
    /// counting the lines it drops ([`count_evictions`]).
    #[inline]
    fn counting(&mut self, call: impl FnOnce(&mut Term<T>)) {
        let Some(mut count) = EVICTIONS.get() else {
            return call(self.0);
        };
        // The alternate screen has no scrollback.
        if count.limit == 0 || self.0.mode().contains(TermMode::ALT_SCREEN) {
            return call(self.0);
        }
        // What the grid may hold: the limit less the journal's lines.
        let limit = count.limit.saturating_sub(count.journal);
        if count.recap {
            self.0.grid_mut().update_history(limit);
            count.recap = false;
            EVICTIONS.set(Some(count));
        }
        let room = self.0.screen_lines();
        let before = self.0.grid().history_size();
        let full = before + room > limit;
        if full {
            // Room for what the call can push, never less than the grid
            // holds: new options or a resize may have left it over its cap,
            // and only a clear may shrink it during the call.
            self.0.grid_mut().update_history(limit.max(before) + room);
        }
        call(self.0);
        let after = self.0.grid().history_size();
        let mut dropped = 0;
        let cleared = before.saturating_sub(after);
        if cleared > 0 {
            // ED 3 or RIS: the whole scrollback goes, the journal too.
            dropped += cleared + count.journal;
            if count.journal > 0 {
                with_journal(Journal::clear);
                count.journal = 0;
            }
            self.0.grid_mut().update_history(count.limit);
        } else if full {
            // Back to the limit: the oldest lines go, the journal's first
            // (they are older than the grid's), as alacritty drops them.
            let over = (count.journal + after).saturating_sub(count.limit);
            let from_journal = over.min(count.journal);
            if from_journal > 0 {
                with_journal(|journal| journal.pop_front(from_journal));
                count.journal -= from_journal;
            }
            self.0
                .grid_mut()
                .update_history(count.limit.saturating_sub(count.journal));
            dropped += over;
        }
        if dropped > 0 {
            count.dropped += dropped;
            EVICTIONS.set(Some(count));
        }
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
        let width = c.width();
        if width == Some(0) && self.zerowidth_full() {
            return;
        }
        // VS16 after a narrow emoji: the emoji takes two cells (D206). The
        // selector joins its cell, the cell becomes a wide character and a
        // spacer is written after it as `Term::input` writes one, so insert
        // mode, the attributes and the wrap at the row's end are its own.
        if c == VS16
            && let Some(column) = self.widened_by_vs16()
        {
            Handler::input(&mut *self.0, c);
            let line = self.0.grid().cursor.point.line;
            self.0.grid_mut()[line][alacritty_terminal::index::Column(column)]
                .flags
                .insert(Flags::WIDE_CHAR);
            let template = &mut self.0.grid_mut().cursor.template.flags;
            template.insert(Flags::WIDE_CHAR_SPACER);
            Handler::input(&mut *self.0, ' ');
            self.0
                .grid_mut()
                .cursor
                .template
                .flags
                .remove(Flags::WIDE_CHAR_SPACER);
            return;
        }
        // A pending wrap, or a wide character at the last column, can
        // scroll.
        if self.0.grid().cursor.input_needs_wrap || width == Some(2) {
            self.counting(|term| Handler::input(term, c));
        } else {
            Handler::input(&mut *self.0, c);
        }
    }

    #[inline]
    fn linefeed(&mut self) {
        self.counting(Handler::linefeed);
    }

    #[inline]
    fn newline(&mut self) {
        self.counting(Handler::newline);
    }

    #[inline]
    fn scroll_up(&mut self, rows: usize) {
        self.counting(|term| Handler::scroll_up(term, rows));
    }

    #[inline]
    fn delete_lines(&mut self, count: usize) {
        self.counting(|term| Handler::delete_lines(term, count));
    }

    #[inline]
    fn clear_screen(&mut self, mode: ClearMode) {
        self.counting(|term| Handler::clear_screen(term, mode));
    }

    #[inline]
    fn set_private_mode(&mut self, mode: PrivateMode) {
        if self.column_mode(mode, true) {
            Handler::set_private_mode(&mut *self.0, mode);
        }
    }

    #[inline]
    fn unset_private_mode(&mut self, mode: PrivateMode) {
        if self.column_mode(mode, false) {
            Handler::unset_private_mode(&mut *self.0, mode);
        }
    }

    #[inline]
    fn reset_state(&mut self) {
        let mut columns = COLUMNS.get();
        columns.allow = false;
        if columns.forced.take().is_some() {
            columns.changed = true;
        }
        COLUMNS.set(columns);
        // RIS from the alternate screen clears the primary screen's
        // scrollback too (out of `counting`'s sight): its journal goes, and
        // the grid's cap is set again at the next call.
        if self.0.mode().contains(TermMode::ALT_SCREEN)
            && let Some(mut count) = EVICTIONS.get()
            && count.journal > 0
        {
            with_journal(Journal::clear);
            count.dropped += count.journal;
            count.journal = 0;
            count.recap = true;
            EVICTIONS.set(Some(count));
        }
        self.counting(Handler::reset_state);
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
        bell();
        substitute();
        set_horizontal_tabstop();
        scroll_down(rows: usize);
        insert_blank_lines(count: usize);
        erase_chars(count: usize);
        delete_chars(count: usize);
        move_backward_tabs(count: u16);
        move_forward_tabs(count: u16);
        save_cursor_position();
        restore_cursor_position();
        clear_line(mode: LineClearMode);
        clear_tabs(mode: TabulationClearMode);
        set_tabs(interval: u16);
        reverse_index();
        terminal_attribute(attr: Attr);
        set_mode(mode: Mode);
        unset_mode(mode: Mode);
        report_mode(mode: Mode);
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

    fn flags(term: &Term<VoidListener>, column: usize) -> Flags {
        term.grid()[Line(0)][Column(column)].flags
    }

    fn cursor(term: &Term<VoidListener>) -> (usize, bool) {
        let cursor = &term.grid().cursor;
        (cursor.point.column.0, cursor.input_needs_wrap)
    }

    /// VS16 after a narrow emoji makes it a wide character (D206): the
    /// selector joins it, a spacer follows, and the text after it starts two
    /// columns on.
    #[test]
    fn vs16_makes_a_narrow_emoji_two_cells_wide() {
        let mut term = term(10);
        feed(&mut term, "\u{2764}\u{FE0F}x\u{26A0}\u{FE0F}y");
        let grid = term.grid();
        assert_eq!(grid[Line(0)][Column(0)].c, '\u{2764}');
        assert_eq!(
            grid[Line(0)][Column(0)].zerowidth(),
            Some(&['\u{FE0F}'][..])
        );
        assert!(flags(&term, 0).contains(Flags::WIDE_CHAR));
        assert!(flags(&term, 1).contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(grid[Line(0)][Column(2)].c, 'x');
        assert!(flags(&term, 3).contains(Flags::WIDE_CHAR));
        assert!(flags(&term, 4).contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(grid[Line(0)][Column(5)].c, 'y');
        assert_eq!(cursor(&term), (6, false));
    }

    /// Only an emoji widens: VS16 after a letter, after an emoji that is
    /// wide already, a second VS16, and VS16 after a mark joined to the
    /// emoji leave the columns as they were.
    #[test]
    fn vs16_leaves_other_characters_as_they_are() {
        let mut term = term(12);
        feed(
            &mut term,
            "a\u{FE0F}b\u{2705}\u{FE0F}c\u{2764}\u{FE0F}\u{FE0F}d",
        );
        let text: String = (0..8)
            .map(|column| term.grid()[Line(0)][Column(column)].c)
            .collect();
        // a b ✅ (two cells) c ❤ (two cells) d.
        assert_eq!(text, "ab\u{2705} c\u{2764} d");
        assert_eq!(marks(&term, 0, 0), 1, "the selector stays on the letter");
        assert_eq!(marks(&term, 0, 5), 2, "a second selector joins, no wider");
        assert_eq!(cursor(&term), (8, false));

        let mut marked = self::term(4);
        feed(&mut marked, "\u{2764}\u{301}\u{FE0F}z");
        assert!(!flags(&marked, 0).contains(Flags::WIDE_CHAR));
        assert_eq!(marked.grid()[Line(0)][Column(1)].c, 'z');
    }

    /// At the last column there is no second cell: the emoji stays narrow
    /// and the wrap stays pending, as before. One column earlier it widens
    /// into the last column and the next character wraps.
    #[test]
    fn vs16_at_the_end_of_a_row() {
        let mut last = term(4);
        feed(&mut last, "abc\u{2764}\u{FE0F}");
        assert!(!flags(&last, 3).contains(Flags::WIDE_CHAR));
        assert_eq!(cursor(&last), (3, true));

        let mut before = term(4);
        feed(&mut before, "ab\u{2764}\u{FE0F}x");
        assert!(flags(&before, 2).contains(Flags::WIDE_CHAR));
        assert!(flags(&before, 3).contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(before.grid()[Line(1)][Column(0)].c, 'x');
    }

    /// In insert mode the spacer pushes the rest of the row right, as a
    /// wide character's own does; writing over the spacer later makes the
    /// emoji narrow again (alacritty clears a broken wide character).
    #[test]
    fn vs16_in_insert_mode_and_overwritten() {
        let mut term = term(8);
        feed(&mut term, "abc\r\x1b[4h\u{2764}\u{FE0F}\x1b[4l");
        let text: String = (0..5)
            .map(|column| term.grid()[Line(0)][Column(column)].c)
            .collect();
        assert_eq!(text, "\u{2764} abc");
        assert!(flags(&term, 0).contains(Flags::WIDE_CHAR));
        feed(&mut term, "\x1b[1;2Hq");
        assert!(!flags(&term, 0).contains(Flags::WIDE_CHAR));
        assert!(!flags(&term, 1).contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(term.grid()[Line(0)][Column(1)].c, 'q');
    }

    /// Every row of the buffer, oldest first: its text and whether it wraps.
    fn rows(term: &Term<VoidListener>) -> Vec<(String, bool)> {
        let grid = term.grid();
        let last = Column(grid.columns() - 1);
        (grid.topmost_line().0..=grid.bottommost_line().0)
            .map(|line| {
                let row = &grid[Line(line)];
                let text: String = (0..grid.columns())
                    .map(|column| row[Column(column)].c)
                    .collect();
                (text, row[last].flags.contains(Flags::WRAPLINE))
            })
            .collect()
    }

    fn sized(cols: usize, rows: usize, scrollback: usize) -> Term<VoidListener> {
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        Term::new(config, &GridDims { cols, rows }, VoidListener)
    }

    /// Feeds `text` to a counted term (through [`Capped`]) and to a plain
    /// one (alacritty alone): the grids stay the same.
    fn both(counted: &mut Term<VoidListener>, plain: &mut Term<VoidListener>, text: &str) {
        feed(counted, text);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(plain, text.as_bytes());
        assert_eq!(rows(counted), rows(plain), "{text:?}");
        assert_eq!(
            counted.grid().display_offset(),
            plain.grid().display_offset()
        );
    }

    /// The count of lines that drop off a full scrollback (output that
    /// scrolls on, wraps, SU, DL at the top, ED 2, ED 3, RIS), and the grid
    /// is the one alacritty makes without the count.
    #[test]
    fn lines_dropped_off_the_scrollback_are_counted() {
        count_evictions(5, Default::default());
        let mut counted = sized(6, 3, 5);
        let mut plain = sized(6, 3, 5);
        // 20 lines from the top of 3 rows: 18 scroll into the scrollback
        // of 5, so 13 drop off.
        both(&mut counted, &mut plain, &"x\r\n".repeat(20));
        assert_eq!(counted.grid().history_size(), 5);
        assert_eq!(evictions(), 13);
        // A wrapped line: "abcdefghij" takes two rows.
        both(&mut counted, &mut plain, "abcdefghij\r\n");
        assert_eq!(evictions(), 15);
        // A wide character that does not fit the last column wraps too.
        both(&mut counted, &mut plain, "abcde\u{4f60}\r\n");
        assert_eq!(evictions(), 17);
        // While scrolled back, the view stays on its text as before.
        counted.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        plain.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        both(&mut counted, &mut plain, "\x1b[2S\x1b[H\x1b[M");
        assert_eq!(evictions(), 20);
        // ED 2 moves the screen into the scrollback.
        both(&mut counted, &mut plain, "y\r\nz\x1b[2J");
        assert_eq!(counted.grid().history_size(), 5);
        let before = evictions();
        assert!(before > 20);
        // ED 3 drops the whole scrollback.
        both(&mut counted, &mut plain, "\x1b[3J");
        assert_eq!(counted.grid().history_size(), 0);
        assert_eq!(evictions(), before + 5);
        // The alternate screen has no scrollback: nothing drops there, and
        // the primary screen's lines stay.
        both(&mut counted, &mut plain, &"w\r\n".repeat(4));
        let before = evictions();
        let kept = counted.grid().history_size();
        assert!(kept > 0);
        both(&mut counted, &mut plain, "\x1b[?1049h");
        both(&mut counted, &mut plain, &"v\r\n".repeat(30));
        assert_eq!(evictions(), before);
        both(&mut counted, &mut plain, "\x1b[?1049l");
        assert_eq!(counted.grid().history_size(), kept);
        // RIS drops the scrollback.
        both(&mut counted, &mut plain, "\x1bc");
        assert_eq!(evictions(), before + kept);
    }

    /// Without [`count_evictions`] on the thread, nothing is counted.
    #[test]
    fn nothing_is_counted_unless_asked() {
        let mut term = sized(6, 3, 2);
        feed(&mut term, &"x\r\n".repeat(10));
        assert_eq!(term.grid().history_size(), 2);
        assert_eq!(evictions(), 0);
    }
}
