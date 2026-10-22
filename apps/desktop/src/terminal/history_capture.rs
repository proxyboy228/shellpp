//! Command history capture in the terminal (docs/security.md, section 10.1;
//! UI 4.2; stage 2b lead decision 7).
//!
//! The terminal sees keystrokes and the screen, not commands, so the capture
//! is a heuristic. It never keeps what the user typed: typed input is only
//! classified (printable or not, Enter, Ctrl+C, bracketed paste markers), and
//! the command is read from the screen, so input the server does not echo
//! (a password) never reaches it.
//!
//! - **Prompt boundary.** The first key after an Enter (or after the session
//!   starts) fixes the boundary: the cursor position in the normal screen,
//!   kept as a row offset inside the cursor's logical line (wrapped rows
//!   joined) and a column. It follows the line when the screen scrolls or the
//!   shell redraws the line elsewhere (Ctrl+L).
//! - **Enter.** At Enter the line becomes *pending*: the text from the
//!   boundary to the end of the logical line is read now and again after
//!   every piece of output, until the cursor leaves the line (the shell
//!   echoes the Enter as a line feed, or moves the cursor to another line).
//!   The last text read while the cursor was still on the line is the
//!   command. Waiting for that moment takes in the echo of keys typed just
//!   before Enter on a slow link. While a line is pending, the parser feeds
//!   output in pieces that end before each LF, VT, FF and ESC
//!   ([`advance`]); otherwise it parses as usual.
//! - **Not recorded** (the app drops the line; the core checks the same
//!   rules again, `shellpp_core::history::evaluate`): the alternate screen
//!   was active when typing started or at Enter; the line holds input the
//!   app injected ([`TerminalHandle::inject_input`]: a History insert,
//!   later snippets); a key was typed while the line was pending (its echo
//!   could land on the line); the line started while the previous line was
//!   still pending, or after an Enter in the same write (type-ahead and
//!   multi-line pastes: the boundary is unknown); a bracketed paste put a
//!   line break into it; Ctrl+C abandoned it; the cursor did not leave the
//!   line within [`PENDING_TIMEOUT`]; echo was off (printable keys were
//!   typed but the text after the boundary is empty or only mask
//!   characters); the prompt or the line itself looks like a password
//!   prompt; the command is empty, starts with a space, is longer than 4096
//!   characters or looks like a secret (docs/security.md, section 10.2).
//!   Keys typed in a full-screen program that has ended (`q` in a pager)
//!   do not start the next line.
//! - **Held lines.** A finished line with nothing before its boundary on
//!   its own row (no shell prompt there: input typed ahead, for example a
//!   password typed with its Enter before `su` turned echo off) is held
//!   back until the next key or until [`PENDING_TIMEOUT`] has passed, and
//!   dropped when the row the output is on looks like a password prompt
//!   meanwhile (`Password:` follows the echoed password), or when the
//!   alternate screen comes on. While a line is held, output is fed in
//!   pieces as for a pending line. A line with the shell's prompt before
//!   its boundary is passed on at once.
//!
//! The capture runs only while a [`HistorySink`] is set (a session to a
//! saved host). Nothing here logs commands, prompts or typed input.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use alacritty_terminal::Term;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use gpui::Context;
use shellpp_core::history::{
    MAX_COMMAND_CHARS, clean_command, ends_in_password_prompt, evaluate, prompt_looks_like_password,
};
use shellpp_core::service::dto::HistoryCandidate;
use zeroize::Zeroizing;

use super::limits::{Capped, MAX_ZEROWIDTH};
use super::model::{Listener, TerminalHandle};
use super::paste;
use super::view::TerminalView;

/// A pending line that the cursor has not left after this long is dropped.
pub const PENDING_TIMEOUT: Duration = Duration::from_secs(5);
/// Rows of one logical line the capture follows; a longer line (endless
/// output without a line break) is not read.
pub const MAX_LINE_ROWS: usize = 512;
/// Rows above the line searched for the last non-empty line (the prompt
/// check when the text before the boundary is empty).
pub const PROMPT_LOOKBACK_ROWS: usize = 64;
/// Characters read from the screen per line: one more than a command may
/// have, so a longer line is still recognized as too long.
const MAX_READ_CHARS: usize = MAX_COMMAND_CHARS + 1;

/// Where a captured command line goes: the session to a saved host, which
/// records it through the core's history API (that checks every rule again).
pub trait HistorySink: Send + Sync + 'static {
    /// A command line that passed the capture rules. Called on the
    /// terminal's parser thread: must not block.
    fn record(&self, candidate: HistoryCandidate);
}

/// Who sent input to the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOrigin {
    /// Keys, pastes and IME text of the user.
    Typed,
    /// Text the app sends on its own (a History insert, later snippets).
    Injected,
}

/// The boundary of a line: a row offset from the first row of the logical
/// line and a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Boundary {
    row: usize,
    column: usize,
}

/// The line being typed.
#[derive(Debug)]
struct TypedLine {
    boundary: Boundary,
    typed_printable: bool,
    injected: bool,
    /// The alternate screen was active when typing started.
    alt_screen: bool,
    /// The boundary is not known (type-ahead, a pasted line break, a line
    /// too long to follow): the line is never recorded.
    unreliable: bool,
}

/// The text of a line as read from the screen (wiped on drop).
#[derive(Default)]
struct Snapshot {
    command: Zeroizing<String>,
    prompt_text: Zeroizing<String>,
    /// Text stood before the boundary on the line's own rows (a prompt);
    /// `false` when `prompt_text` is the line above.
    prompt_on_line: bool,
}

/// A line after Enter, until the cursor leaves it.
struct Pending {
    line: TypedLine,
    since: Instant,
    snapshot: Snapshot,
}

/// A finished line without a prompt on its own rows, held back until the
/// next key or [`PENDING_TIMEOUT`] (a password prompt can still follow it).
/// The candidate is wiped when it is dropped.
struct Held {
    candidate: HistoryCandidate,
    since: Instant,
}

impl Drop for Held {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.candidate.command);
        zeroize::Zeroize::zeroize(&mut self.candidate.prompt_text);
    }
}

impl Held {
    /// The candidate for the sink (the held copy is left empty).
    fn release(mut self) -> HistoryCandidate {
        HistoryCandidate {
            command: std::mem::take(&mut self.candidate.command),
            prompt_text: std::mem::take(&mut self.candidate.prompt_text),
            typed_printable: self.candidate.typed_printable,
            alt_screen: self.candidate.alt_screen,
        }
    }
}

#[derive(Default)]
struct State {
    sink: Option<Arc<dyn HistorySink>>,
    line: Option<TypedLine>,
    pending: Option<Pending>,
    held: Option<Held>,
    /// Inside a bracketed paste of the typed input.
    in_paste: bool,
}

/// The history capture of one terminal (shared by the view, the handle and
/// the parser thread).
#[derive(Default)]
pub(super) struct Capture {
    /// A sink is set: typed input is watched.
    enabled: AtomicBool,
    /// A line is pending or held: the parser feeds output in pieces.
    watching: AtomicBool,
    state: Mutex<State>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture")
            .field("enabled", &self.enabled())
            .field("watching", &self.watching())
            .finish_non_exhaustive()
    }
}

/// One key (or marker) of typed input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Printable,
    Enter,
    /// Ctrl+C: the shell abandons the line.
    Interrupt,
    PasteStart,
    PasteEnd,
    /// Control keys, escape sequences (arrows, Alt+key, function keys).
    Other,
}

/// Classifies typed input. Escape sequences are skipped whole; the
/// bracketed paste markers are reported.
fn keys(bytes: &[u8]) -> Vec<Key> {
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    let mut keys = Vec::new();
    while let Some(c) = chars.next() {
        let key = match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut sequence = String::new();
                    for c in chars.by_ref() {
                        sequence.push(c);
                        if ('\u{40}'..='\u{7e}').contains(&c) || sequence.len() > 32 {
                            break;
                        }
                    }
                    match sequence.as_str() {
                        "200~" => Key::PasteStart,
                        "201~" => Key::PasteEnd,
                        _ => Key::Other,
                    }
                }
                Some('O') => {
                    chars.next();
                    Key::Other
                }
                // Alt+key, or ESC alone.
                Some(_) | None => Key::Other,
            },
            '\r' | '\n' => Key::Enter,
            '\u{3}' => Key::Interrupt,
            c if c.is_control() => Key::Other,
            _ => Key::Printable,
        };
        keys.push(key);
    }
    keys
}

fn alt_screen(term: &Term<Listener>) -> bool {
    term.mode().contains(TermMode::ALT_SCREEN)
}

/// Whether row `line` continues on the next row (soft wrap).
fn wraps(term: &Term<Listener>, line: Line) -> bool {
    let grid = term.grid();
    let last = grid.columns().saturating_sub(1);
    grid[line][Column(last)].flags.contains(Flags::WRAPLINE)
}

/// The first row of the logical line that holds `line`; `None` when the
/// line has more than [`MAX_LINE_ROWS`] rows above.
fn logical_start(term: &Term<Listener>, line: Line) -> Option<Line> {
    let top = term.grid().topmost_line();
    let mut start = line;
    for _ in 0..MAX_LINE_ROWS {
        if start <= top || !wraps(term, start - 1) {
            return Some(start);
        }
        start -= 1;
    }
    None
}

/// The last row of the logical line that holds `line` (at most
/// [`MAX_LINE_ROWS`] rows below it).
fn logical_end(term: &Term<Listener>, line: Line) -> Line {
    let bottom = term.grid().bottommost_line();
    let mut end = line;
    for _ in 0..MAX_LINE_ROWS {
        if end >= bottom || !wraps(term, end) {
            break;
        }
        end += 1;
    }
    end
}

/// Appends the text of row `line` from `from` to its last column (wide
/// character spacers skipped, combining marks kept). Stops at `limit`
/// characters in `out`; returns the new count.
fn push_row(
    term: &Term<Listener>,
    line: Line,
    from: usize,
    to: usize,
    out: &mut String,
    mut count: usize,
) -> usize {
    let row = &term.grid()[line];
    for column in from..to {
        if count >= MAX_READ_CHARS {
            break;
        }
        let cell = &row[Column(column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        out.push(cell.c);
        count += 1;
        if let Some(extra) = cell.zerowidth() {
            out.extend(extra.iter().take(MAX_ZEROWIDTH));
        }
    }
    count
}

/// The text of rows `start..=end`, from column `from` of the first row.
fn rows_text(term: &Term<Listener>, start: Line, end: Line, from: usize) -> String {
    let columns = term.grid().columns();
    let mut out = String::new();
    let mut count = 0;
    let mut line = start;
    let mut column = from.min(columns);
    while line <= end && count < MAX_READ_CHARS {
        count = push_row(term, line, column, columns, &mut out, count);
        column = 0;
        line += 1;
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    out
}

/// The boundary at the cursor (the next typed character lands there).
fn boundary_now(term: &Term<Listener>) -> Option<Boundary> {
    let cursor = &term.grid().cursor;
    let start = logical_start(term, cursor.point.line)?;
    let mut row = (cursor.point.line.0 - start.0) as usize;
    let mut column = cursor.point.column.0;
    if cursor.input_needs_wrap {
        row += 1;
        column = 0;
    }
    Some(Boundary { row, column })
}

/// The last non-empty logical line above row `start` (within
/// [`PROMPT_LOOKBACK_ROWS`]).
fn line_above(term: &Term<Listener>, start: Line) -> String {
    let top = term.grid().topmost_line();
    let mut line = start;
    for _ in 0..PROMPT_LOOKBACK_ROWS {
        if line <= top {
            break;
        }
        line -= 1;
        let first = logical_start(term, line).unwrap_or(line);
        let text = rows_text(term, first, line, 0);
        if !text.trim().is_empty() {
            return text;
        }
        line = first;
    }
    String::new()
}

/// Reads the command (from the boundary to the end of the cursor's logical
/// line) and the prompt text (before the boundary on that line, or the last
/// non-empty line above when that is empty). `None` in the alternate screen
/// or when the line is too long to follow.
fn read_line(term: &Term<Listener>, boundary: Boundary) -> Option<Snapshot> {
    if alt_screen(term) {
        return None;
    }
    let cursor = term.grid().cursor.point.line;
    let start = logical_start(term, cursor)?;
    let end = logical_end(term, cursor);
    let row = start + boundary.row;
    let command = if row <= end {
        rows_text(term, row, end, boundary.column)
    } else {
        String::new()
    };
    let mut before = String::new();
    if row <= end || boundary.row > 0 {
        let mut count = 0;
        let mut line = start;
        let columns = term.grid().columns();
        while line < row.min(end + 1) {
            count = push_row(term, line, 0, columns, &mut before, count);
            line += 1;
        }
        if row <= end {
            push_row(
                term,
                row,
                0,
                boundary.column.min(columns),
                &mut before,
                count,
            );
        }
    }
    let prompt_on_line = !before.trim().is_empty();
    let prompt_text = if prompt_on_line {
        before.trim().to_owned()
    } else {
        line_above(term, start)
    };
    Some(Snapshot {
        command: Zeroizing::new(command),
        prompt_text: Zeroizing::new(prompt_text),
        prompt_on_line,
    })
}

impl TerminalHandle {
    /// Broadcast input (UI 4.4): whether the text before the cursor ends
    /// in a password prompt (`sudo`, `su`, `mysql -p`); typed input then
    /// stays in its own pane. Only the end of the line is read (at most
    /// [`MAX_READ_CHARS`] cells before the cursor); a line too long to
    /// follow counts as one.
    pub fn cursor_at_password_prompt(&self) -> bool {
        let term = self.term().lock();
        let cursor = &term.grid().cursor;
        let Some(start) = logical_start(&term, cursor.point.line) else {
            return true;
        };
        let columns = term.grid().columns();
        let to = if cursor.input_needs_wrap {
            columns
        } else {
            cursor.point.column.0.min(columns)
        };
        let rows = (MAX_READ_CHARS / columns.max(1)).max(1) as i32;
        let mut before = Zeroizing::new(String::new());
        let mut count = 0;
        let mut line = Line(start.0.max(cursor.point.line.0 - rows + 1));
        while line < cursor.point.line {
            count = push_row(&term, line, 0, columns, &mut before, count);
            line += 1;
        }
        push_row(&term, cursor.point.line, 0, to, &mut before, count);
        ends_in_password_prompt(&before)
    }
}

/// Whether the logical line the cursor is on looks like a password prompt
/// (the check of a held line); also `true` in the alternate screen and for
/// a line too long to follow (fail closed).
fn cursor_line_is_password_prompt(term: &Term<Listener>) -> bool {
    if alt_screen(term) {
        return true;
    }
    let cursor = term.grid().cursor.point.line;
    let Some(start) = logical_start(term, cursor) else {
        return true;
    };
    let text = Zeroizing::new(rows_text(term, start, logical_end(term, cursor), 0));
    prompt_looks_like_password(&text)
}

/// Where the cursor is before a piece of output.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    cursor: Line,
    start: Line,
    end: Line,
}

impl Geometry {
    fn of(term: &Term<Listener>) -> Option<Self> {
        if alt_screen(term) {
            return None;
        }
        let cursor = term.grid().cursor.point.line;
        Some(Self {
            cursor,
            start: logical_start(term, cursor)?,
            end: logical_end(term, cursor),
        })
    }
}

/// Whether the piece of output that began with `first` moved the cursor off
/// the logical line it was on (`before`).
fn left_line(term: &Term<Listener>, first: u8, before: Geometry) -> bool {
    if alt_screen(term) {
        return true;
    }
    match first {
        // A line feed from the line's last row starts another line (it
        // moves down or scrolls); from an earlier row it stays inside.
        b'\n' | 0x0b | 0x0c => before.cursor >= before.end,
        _ => {
            let cursor = term.grid().cursor.point.line;
            match logical_start(term, cursor) {
                None => true,
                Some(start) => {
                    // Same line, or the same line moved up by an autowrap
                    // at the bottom (the cursor stays on the bottom row, on
                    // a continuation row).
                    let same = start == before.start
                        || (cursor == before.cursor && cursor > start && start < before.start);
                    !same
                }
            }
        }
    }
}

impl State {
    /// Drops a pending line older than [`PENDING_TIMEOUT`].
    fn expire(&mut self, now: Instant) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now.duration_since(pending.since) > PENDING_TIMEOUT)
        {
            self.pending = None;
        }
    }

    /// Output is fed in pieces: a line is pending or held.
    fn watching(&self) -> bool {
        self.pending.is_some() || self.held.is_some()
    }

    /// Checks the held line against the screen: dropped when the cursor's
    /// line looks like a password prompt (or the alternate screen is on).
    fn check_held(&mut self, term: &Term<Listener>) {
        if self.held.is_some() && cursor_line_is_password_prompt(term) {
            self.held = None;
        }
    }

    /// The held line, once it is due: `now` is past its timeout, or `force`
    /// (the next key). Check it with [`Self::check_held`] first.
    fn take_held(&mut self, now: Instant, force: bool) -> Option<HistoryCandidate> {
        let due = self
            .held
            .as_ref()
            .is_some_and(|held| force || now.duration_since(held.since) > PENDING_TIMEOUT);
        if due {
            self.held.take().map(Held::release)
        } else {
            None
        }
    }

    /// The current line, started at the cursor when there is none.
    fn touch(
        &mut self,
        term: &Term<Listener>,
        origin: InputOrigin,
        entered: bool,
    ) -> &mut TypedLine {
        let pending = self.pending.is_some();
        // Keys typed in a full-screen program that has ended (`q` in less)
        // do not start the next command line.
        if self
            .line
            .as_ref()
            .is_some_and(|line| line.alt_screen && !alt_screen(term))
        {
            self.line = None;
        }
        let line = self.line.get_or_insert_with(|| {
            let boundary = boundary_now(term);
            TypedLine {
                boundary: boundary.unwrap_or_default(),
                typed_printable: false,
                injected: false,
                alt_screen: alt_screen(term),
                unreliable: boundary.is_none() || entered || pending,
            }
        });
        if origin == InputOrigin::Injected {
            line.injected = true;
        }
        line
    }

    /// Enter: the line becomes pending unless it is dropped at once.
    fn enter(&mut self, term: &Term<Listener>, origin: InputOrigin, now: Instant) {
        // Enter on a line where nothing was typed after the boundary: there
        // is nothing of this line to record (a pending line keeps waiting).
        let Some(mut line) = self.line.take() else {
            return;
        };
        if origin == InputOrigin::Injected {
            line.injected = true;
        }
        if self.pending.is_some()
            || line.unreliable
            || line.injected
            || line.alt_screen
            || alt_screen(term)
        {
            return;
        }
        if let Some(snapshot) = read_line(term, line.boundary) {
            self.pending = Some(Pending {
                line,
                since: now,
                snapshot,
            });
        }
    }
}

/// The candidate of a finished line (cleaned: rule 3 of docs/security.md,
/// section 10.1), when the app-side check passes (the core checks again
/// before it writes).
fn candidate(pending: Pending) -> Option<HistoryCandidate> {
    let Pending { line, snapshot, .. } = pending;
    let candidate = HistoryCandidate {
        command: clean_command(&snapshot.command),
        prompt_text: clean_command(&snapshot.prompt_text),
        typed_printable: line.typed_printable,
        alt_screen: line.alt_screen,
    };
    match evaluate(&candidate, line.injected, true) {
        Ok(_) => Some(candidate),
        Err(_) => {
            let mut candidate = candidate;
            zeroize::Zeroize::zeroize(&mut candidate.command);
            zeroize::Zeroize::zeroize(&mut candidate.prompt_text);
            None
        }
    }
}

impl Capture {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// A line is pending or held: output must go through [`advance`].
    pub(super) fn watching(&self) -> bool {
        self.watching.load(Ordering::Acquire)
    }

    /// Sets (or removes) the sink; the capture starts over.
    pub(super) fn set_sink(&self, sink: Option<Arc<dyn HistorySink>>) {
        let mut state = self.lock();
        self.enabled.store(sink.is_some(), Ordering::Release);
        self.watching.store(false, Ordering::Release);
        *state = State {
            sink,
            ..State::default()
        };
    }

    /// Input about to be sent to the session. Call it before sending, with
    /// the terminal locked (the boundary is the cursor before the echo).
    pub(super) fn on_input(&self, term: &Term<Listener>, bytes: &[u8], origin: InputOrigin) {
        let mut state = self.lock();
        if state.sink.is_none() {
            return;
        }
        let now = Instant::now();
        state.expire(now);
        // The next key: a held line goes now, unless a password prompt
        // showed up after it.
        let released = if bytes.is_empty() {
            None
        } else {
            state.check_held(term);
            state.take_held(now, true)
        };
        // The boundary of a line started from here on is unknown: after an
        // Enter in this write, or while a line was still pending.
        let mut entered = false;
        for key in keys(bytes) {
            // Input before the pending line ended on screen: its echo can
            // land on that line, so the line is not recorded (fail closed).
            if state.pending.take().is_some() {
                entered = true;
            }
            match key {
                Key::PasteStart => {
                    state.in_paste = true;
                    state.touch(term, origin, entered);
                }
                Key::PasteEnd => state.in_paste = false,
                // A line break inside a bracketed paste: the shell keeps
                // the line open over several screen lines.
                Key::Enter if state.in_paste => {
                    state.touch(term, origin, entered).unreliable = true
                }
                Key::Enter => {
                    state.enter(term, origin, now);
                    entered = true;
                }
                Key::Interrupt => state.line = None,
                Key::Printable => state.touch(term, origin, entered).typed_printable = true,
                Key::Other => {
                    state.touch(term, origin, entered);
                }
            }
        }
        self.watching.store(state.watching(), Ordering::Release);
        let sink = state.sink.clone();
        drop(state);
        if let Some((sink, candidate)) = sink.zip(released) {
            sink.record(candidate);
        }
    }

    /// After a piece of output while a line is pending: the line is read
    /// again, or finished when the cursor left it. A finished line without
    /// a prompt on its own rows is held; while a line is held, each piece
    /// checks the row the output is on for a password prompt.
    fn after_piece(&self, term: &Term<Listener>, first: u8, before: Option<Geometry>) {
        let delivery = {
            let mut state = self.lock();
            let now = Instant::now();
            state.expire(now);
            let released = if let Some(pending) = state.pending.as_mut() {
                let left = before.is_none_or(|before| left_line(term, first, before));
                if !left && let Some(snapshot) = read_line(term, pending.line.boundary) {
                    pending.snapshot = snapshot;
                    return;
                }
                let finished = state.pending.take();
                let hold = finished
                    .as_ref()
                    .is_some_and(|pending| !pending.snapshot.prompt_on_line);
                match finished.and_then(candidate) {
                    Some(candidate) if hold => {
                        state.held = Some(Held {
                            candidate,
                            since: now,
                        });
                        // This piece may hold the prompt already.
                        state.check_held(term);
                        None
                    }
                    finished => finished,
                }
            } else {
                state.check_held(term);
                state.take_held(now, false)
            };
            self.watching.store(state.watching(), Ordering::Release);
            state.sink.clone().zip(released)
        };
        if let Some((sink, candidate)) = delivery {
            sink.record(candidate);
        }
    }

    /// Nothing typed, nothing pending or held (tests).
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        let state = self.lock();
        state.line.is_none() && state.pending.is_none() && state.held.is_none()
    }

    /// A line is held back (tests).
    #[cfg(test)]
    pub(super) fn holds_a_line(&self) -> bool {
        self.lock().held.is_some()
    }
}

/// Parses output while a line may be pending or held: in pieces that end
/// before each LF, VT, FF and ESC, checking the line after each piece. Falls
/// back to one call when nothing is pending or held.
pub(super) fn advance(
    capture: &Capture,
    processor: &mut Processor<StdSyncHandler>,
    term: &mut Term<Listener>,
    bytes: &[u8],
) {
    let mut rest = bytes;
    while !rest.is_empty() {
        if !capture.watching() {
            processor.advance(&mut Capped(term), rest);
            return;
        }
        let end = rest[1..]
            .iter()
            .position(|byte| matches!(*byte, b'\n' | 0x0b | 0x0c | 0x1b))
            .map_or(rest.len(), |at| at + 1);
        let (piece, tail) = rest.split_at(end);
        let before = Geometry::of(term);
        processor.advance(&mut Capped(term), piece);
        capture.after_piece(term, piece[0], before);
        rest = tail;
    }
}

#[cfg(test)]
impl TerminalHandle {
    /// A terminal without a view (tests of the session layer): the default
    /// theme, 1000 lines of scrollback. Keep the receiver while it is used.
    pub(crate) fn headless(cols: u16, rows: u16) -> (Self, async_channel::Receiver<()>) {
        let (wake, woken) = async_channel::bounded(1);
        let handle = Self::spawn(
            super::model::ModelOptions {
                size: super::TerminalSize::cells(cols, rows),
                scrollback: 1000,
                cursor_shape: alacritty_terminal::vte::ansi::CursorShape::Block,
            },
            super::colors::Palette::from_theme(&crate::theme::terminal::builtin()[0]),
            wake,
        );
        (handle, woken)
    }
}

impl TerminalHandle {
    /// Typed input (keys, pastes, IME text) for the session: watched by the
    /// history capture, then sent.
    pub fn send_typed(&self, bytes: Vec<u8>) {
        self.watch_input(&bytes, InputOrigin::Typed);
        self.send_input(bytes);
    }

    /// Input the app sends on its own (a History insert; snippets later):
    /// sent like typed input, and the line that holds it is never recorded
    /// (docs/security.md, section 10.1).
    pub fn inject_input(&self, bytes: Vec<u8>) {
        self.watch_input(&bytes, InputOrigin::Injected);
        self.send_input(bytes);
    }

    /// Where captured command lines go; `None` stops the capture (local
    /// terminals, quick connects).
    pub fn set_history_sink(&self, sink: Option<Arc<dyn HistorySink>>) {
        self.outbox().capture.set_sink(sink);
    }

    fn watch_input(&self, bytes: &[u8], origin: InputOrigin) {
        let capture = &self.outbox().capture;
        if capture.enabled() {
            let term = self.term().lock();
            capture.on_input(&term, bytes, origin);
        }
    }
}

impl TerminalView {
    /// Inserts `text` at the prompt without running it (UI 4.2, a History
    /// entry): as injected input, on one line (line breaks become spaces),
    /// bracketed when the program asked for bracketed paste.
    pub fn insert_injected(&mut self, text: &str, cx: &mut Context<Self>) {
        let one_line: String = paste::sanitize(text)
            .chars()
            .map(|c| if matches!(c, '\r' | '\n') { ' ' } else { c })
            .collect();
        if one_line.is_empty() {
            return;
        }
        let handle = self.handle();
        handle.scroll_to_bottom();
        let bracketed = handle
            .term()
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        handle.inject_input(paste::paste_bytes(&one_line, bracketed));
        cx.notify();
    }

    /// Whether [`Self::insert_snippet`] takes `script` without Enter now: a
    /// script of several lines needs the shell's bracketed paste (D68: the
    /// sidebar shows its insert as unavailable otherwise).
    pub fn can_insert(&self, script: &str) -> bool {
        let text = paste::sanitize(script).replace("\r\n", "\n");
        let text = text.trim_end_matches(['\n', '\r']);
        !paste::has_line_break(text)
            || self
                .handle()
                .term()
                .lock()
                .mode()
                .contains(TermMode::BRACKETED_PASTE)
    }

    /// A snippet from the terminal sidebar (UI 4.2), as injected input:
    /// `run` sends Enter after it. A script of several lines goes as one
    /// bracketed paste when the shell takes one, so nothing runs before
    /// Enter; without bracketed paste each line would run as it arrives, so
    /// an insert of such a script is refused (`false`).
    pub fn insert_snippet(&mut self, script: &str, run: bool, cx: &mut Context<Self>) -> bool {
        let text = paste::sanitize(script).replace("\r\n", "\n");
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() {
            return false;
        }
        let handle = self.handle();
        handle.scroll_to_bottom();
        let bracketed = handle
            .term()
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        if !run && !bracketed && paste::has_line_break(text) {
            return false;
        }
        let mut bytes = paste::paste_bytes(text, bracketed);
        if run {
            bytes.push(b'\r');
        }
        handle.inject_input(bytes);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests;
