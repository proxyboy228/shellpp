//! Search in the terminal output (UI 4.2 Tools, Ctrl+Shift+F): plain text,
//! case-insensitive, over the scrollback (its journal included, D85) and
//! the screen.
//!
//! - Matching runs over logical lines: a row that wrapped (alacritty's
//!   `WRAPLINE`) continues into the next one, so a word broken by the
//!   right edge is found; a line break ends a match. Trailing blanks of a
//!   line are not part of it.
//! - A wide character (CJK, emoji) is one character of two cells; its match
//!   covers both cells. Combining marks are not compared (a search for `e`
//!   finds `é` written as `e` + U+0301). Hidden text (SGR 8) reads as
//!   blanks, so a search never reveals it.
//! - Case: each character is folded to its lower case when that is one
//!   character (`İ` stays as it is); the query is folded the same way.
//! - Matches do not overlap and are kept in buffer order, at most
//!   [`MAX_MATCHES`] ([`Matches::truncated`] says when there were more).
//! - Positions: [`find`] counts lines from the oldest line of the buffer
//!   ([`BufferPoint`]). A [`SearchState`] adds the lines that had dropped
//!   off the top of the scrollback ([`Origin::evicted`], counted by the
//!   terminal), so its positions stay on their text while a full
//!   scrollback scrolls on: the current match keeps its text until it
//!   drops off itself.
//! - Searching again after new output ([`SearchState::refresh`]) reads only
//!   what can have changed: rows that were in the scrollback do not change,
//!   so it searches from the logical line at the previous screen top down
//!   (and the first logical line, when its start dropped off) and keeps
//!   the other matches. A reflow (another grid size), a switch of screen or
//!   a list cut at [`MAX_MATCHES`] searches everything; after a reflow the
//!   current match is the one with the same ordinal (a reflow moves every
//!   match and keeps their order), and the highlights wait for that search
//!   ([`SearchState::spans`] is empty on another grid size).
//!
//! Nothing here logs the query or the terminal text.

use std::ops::Range;
use std::rc::Rc;

use alacritty_terminal::grid::Row;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::cell::{Cell, Flags};

use super::journal::Buffer;

/// Most matches kept for one query; the counter shows "10000+" beyond.
pub const MAX_MATCHES: usize = 10_000;
/// Longest query, in characters (a longer one is cut).
pub const MAX_QUERY_CHARS: usize = 1024;

/// A cell position counted from the oldest line of the buffer: line 0 is
/// the topmost scrollback line. In a [`SearchState`], the line also counts
/// the lines that dropped off before (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferPoint {
    pub line: usize,
    pub column: usize,
}

/// One match: its first and last cell (a wide character's second cell
/// included), inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: BufferPoint,
    pub end: BufferPoint,
}

/// The matches of a query, in buffer order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matches {
    pub list: Vec<SearchMatch>,
    /// There were more than [`MAX_MATCHES`].
    pub truncated: bool,
}

impl Matches {
    /// Adds a match, or notes that there were more than `limit`.
    fn push(&mut self, found: SearchMatch, limit: usize) {
        if self.list.len() >= limit {
            self.truncated = true;
        } else {
            self.list.push(found);
        }
    }
}

/// Where the buffer of a grid stands: the lines that dropped off the top of
/// its scrollback so far (counted by the terminal) and whether it is the
/// alternate screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Origin {
    pub evicted: usize,
    pub alt_screen: bool,
}

/// What a search ran over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Basis {
    columns: usize,
    screen_lines: usize,
    history: usize,
    origin: Origin,
}

impl Basis {
    fn of<B: Buffer + ?Sized>(buffer: &B, origin: Origin) -> Self {
        Self {
            columns: buffer.cols(),
            screen_lines: buffer.rows(),
            history: buffer.above(),
            origin,
        }
    }

    /// The line (dropped lines included) of the first screen row.
    fn screen_top(&self) -> usize {
        self.origin.evicted + self.history
    }

    /// The grid still has the size the search ran on.
    fn same_size<B: Buffer + ?Sized>(&self, buffer: &B) -> bool {
        (self.columns, self.screen_lines) == (buffer.cols(), buffer.rows())
    }
}

/// A character folded for comparison.
fn fold(ch: char) -> char {
    if ch.is_ascii() {
        return ch.to_ascii_lowercase();
    }
    let mut lower = ch.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(lower), None) => lower,
        _ => ch,
    }
}

/// The folded characters of a query (at most [`MAX_QUERY_CHARS`]).
pub fn fold_query(query: &str) -> Vec<char> {
    query.chars().take(MAX_QUERY_CHARS).map(fold).collect()
}

/// The KMP failure function of `pattern`.
fn failure(pattern: &[char]) -> Vec<usize> {
    let mut table = vec![0; pattern.len()];
    let mut k = 0;
    for i in 1..pattern.len() {
        while k > 0 && pattern[i] != pattern[k] {
            k = table[k - 1];
        }
        if pattern[i] == pattern[k] {
            k += 1;
        }
        table[i] = k;
    }
    table
}

/// Streams characters through a KMP matcher, remembering where each of the
/// last `pattern.len()` characters started.
struct Matcher<'a> {
    pattern: &'a [char],
    table: Vec<usize>,
    matched: usize,
    /// Start points of the last characters, a ring of `pattern.len()`.
    starts: Vec<BufferPoint>,
    seen: usize,
    limit: usize,
    out: Matches,
}

impl<'a> Matcher<'a> {
    fn new(pattern: &'a [char], limit: usize, out: Matches) -> Self {
        Self {
            pattern,
            table: failure(pattern),
            matched: 0,
            starts: vec![BufferPoint { line: 0, column: 0 }; pattern.len()],
            seen: 0,
            limit,
            out,
        }
    }

    fn full(&self) -> bool {
        self.out.truncated
    }

    /// A line break: no match continues across it.
    fn break_line(&mut self) {
        self.matched = 0;
    }

    /// The next character, its first cell and its last cell.
    fn push(&mut self, ch: char, start: BufferPoint, end: BufferPoint) {
        let size = self.pattern.len();
        self.starts[self.seen % size] = start;
        self.seen += 1;
        while self.matched > 0 && self.pattern[self.matched] != ch {
            self.matched = self.table[self.matched - 1];
        }
        if self.pattern[self.matched] == ch {
            self.matched += 1;
        }
        if self.matched == size {
            // The match began `size` characters ago.
            let first = self.starts[(self.seen - size) % size];
            self.out.push(SearchMatch { start: first, end }, self.limit);
            // Matches do not overlap.
            self.matched = 0;
        }
    }
}

/// The last column of a row (`columns` wide) that holds anything but a
/// blank, if any.
fn last_used_column(row: &Row<Cell>, columns: usize) -> Option<usize> {
    (0..columns).rev().find(|&column| {
        let cell = &row[Column(column)];
        !(cell.c == ' ' || cell.c == '\0' || cell.flags.contains(Flags::HIDDEN))
            || cell.flags.contains(Flags::WRAPLINE)
    })
}

/// The first buffer line of the logical line that holds `line`.
fn logical_start<B: Buffer + ?Sized>(buffer: &B, mut line: usize) -> usize {
    while line > 0 && buffer.wraps(line - 1) {
        line -= 1;
    }
    line
}

/// The buffer line after the logical line that holds `line`.
fn logical_end<B: Buffer + ?Sized>(buffer: &B, mut line: usize) -> usize {
    let last = buffer.total() - 1;
    while line < last && buffer.wraps(line) {
        line += 1;
    }
    line + 1
}

/// Searches the buffer lines `lines` of `grid` for `pattern` and adds the
/// matches to `out`, their lines raised by `base`, up to `limit` in all.
/// `lines` starts at the start of a logical line.
fn scan<B: Buffer + ?Sized>(
    buffer: &B,
    pattern: &[char],
    lines: Range<usize>,
    base: usize,
    limit: usize,
    out: Matches,
) -> Matches {
    if pattern.is_empty() {
        return out;
    }
    let mut matcher = Matcher::new(pattern, limit, out);
    let columns = buffer.cols();
    for buffer_line in lines {
        if matcher.full() {
            break;
        }
        let row = buffer.row(buffer_line);
        let wraps = row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        // A line that ends here drops its trailing blanks; a wrapped row
        // keeps every cell (the blanks are inside the logical line).
        let used = if wraps {
            Some(columns - 1)
        } else {
            last_used_column(&row, columns)
        };
        if let Some(used) = used {
            for column in 0..=used {
                let cell = &row[Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let ch = if cell.flags.contains(Flags::HIDDEN) || cell.c == '\0' {
                    ' '
                } else {
                    fold(cell.c)
                };
                let start = BufferPoint {
                    line: buffer_line + base,
                    column,
                };
                let end = if cell.flags.contains(Flags::WIDE_CHAR) {
                    BufferPoint {
                        line: buffer_line + base,
                        column: (column + 1).min(columns - 1),
                    }
                } else {
                    start
                };
                matcher.push(ch, start, end);
            }
        }
        if !wraps {
            matcher.break_line();
        }
    }
    matcher.out
}

/// All matches of `query` in the scrollback and the screen of `buffer`.
pub fn find<B: Buffer + ?Sized>(buffer: &B, query: &str, limit: usize) -> Matches {
    let pattern = fold_query(query);
    scan(
        buffer,
        &pattern,
        0..buffer.total(),
        0,
        limit,
        Matches::default(),
    )
}

/// A highlighted span on screen: the viewport row, the first and last
/// column, and whether it belongs to the current match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    pub row: usize,
    pub first: usize,
    pub last: usize,
    pub current: bool,
}

/// The state of a search in one terminal: the query, its matches and the
/// current match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchState {
    query: String,
    matches: Rc<Matches>,
    current: Option<usize>,
    basis: Basis,
}

impl SearchState {
    /// Searches `grid` (whose buffer stands at `origin`) for `query`. The
    /// current match is the last one that starts at or above the bottom of
    /// the visible rows (the newest output the user sees), else the first
    /// one.
    pub fn new<B: Buffer + ?Sized>(query: &str, buffer: &B, origin: Origin) -> Self {
        let pattern = fold_query(query);
        let matches = scan(
            buffer,
            &pattern,
            0..buffer.total(),
            origin.evicted,
            MAX_MATCHES,
            Matches::default(),
        );
        let bottom = visible_bottom(buffer) + origin.evicted;
        let current = match matches.list.partition_point(|m| m.start.line <= bottom) {
            0 if matches.list.is_empty() => None,
            0 => Some(0),
            after => Some(after - 1),
        };
        Self {
            query: query.to_owned(),
            matches: Rc::new(matches),
            current,
            basis: Basis::of(buffer, origin),
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn matches(&self) -> &[SearchMatch] {
        &self.matches.list
    }

    /// Matches found (at most [`MAX_MATCHES`]).
    pub fn count(&self) -> usize {
        self.matches.list.len()
    }

    /// More matches exist than are kept.
    pub fn truncated(&self) -> bool {
        self.matches.truncated
    }

    /// The index of the current match (0-based), if any.
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current(&self) -> Option<SearchMatch> {
        self.current
            .and_then(|index| self.matches.list.get(index).copied())
    }

    /// Moves to the next (`forward`, toward newer output) or the previous
    /// match, wrapping around at either end.
    pub fn step(&mut self, forward: bool) -> Option<SearchMatch> {
        let count = self.count();
        if count == 0 {
            self.current = None;
            return None;
        }
        self.current = Some(match (self.current, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
        });
        self.current()
    }

    /// Searches again after the content changed (see the module
    /// documentation for what is read again). The current match stays the
    /// one at the same position, else the next one after it (else the
    /// last); after a reflow, the one with the same ordinal (clamped).
    /// Returns whether the matches changed.
    pub fn refresh<B: Buffer + ?Sized>(&mut self, buffer: &B, origin: Origin) -> bool {
        let pattern = fold_query(&self.query);
        let old = self.basis;
        let new = Basis::of(buffer, origin);
        self.basis = new;
        let reflowed = (old.columns, old.screen_lines) != (new.columns, new.screen_lines);
        let matches = self
            .search_changed_rows(buffer, &pattern, old, new)
            .unwrap_or_else(|| {
                scan(
                    buffer,
                    &pattern,
                    0..buffer.total(),
                    origin.evicted,
                    MAX_MATCHES,
                    Matches::default(),
                )
            });
        if matches == *self.matches {
            return false;
        }
        self.current = match self.current() {
            _ if matches.list.is_empty() => None,
            None => None,
            Some(_) if reflowed => self.current.map(|index| index.min(matches.list.len() - 1)),
            Some(previous) => {
                let after = matches.list.partition_point(|m| m.start < previous.start);
                Some(after.min(matches.list.len() - 1))
            }
        };
        self.matches = Rc::new(matches);
        true
    }

    /// The matches after new output, searching only the rows that can have
    /// changed since `old`; `None` when everything must be searched.
    fn search_changed_rows<B: Buffer + ?Sized>(
        &self,
        buffer: &B,
        pattern: &[char],
        old: Basis,
        new: Basis,
    ) -> Option<Matches> {
        let same_grid = (old.columns, old.screen_lines, old.origin.alt_screen)
            == (new.columns, new.screen_lines, new.origin.alt_screen);
        let top = new.origin.evicted;
        // Lines only drop off the top and the screen top only moves down;
        // anything else changed the buffer in a way that is not counted.
        if !same_grid
            || self.matches.truncated
            || top < old.origin.evicted
            || new.screen_top() < old.screen_top()
            || old.screen_top() < top
        {
            return None;
        }
        // The rows from the logical line at the previous screen top down.
        let rescan = logical_start(buffer, old.screen_top() - top);
        // A logical line whose first rows dropped off is searched anew.
        let head = if top > old.origin.evicted {
            logical_end(buffer, 0)
        } else {
            0
        };
        if head >= rescan {
            return None;
        }
        let mut out = scan(
            buffer,
            pattern,
            0..head,
            top,
            MAX_MATCHES,
            Matches::default(),
        );
        let kept = (top + head)..(top + rescan);
        for found in &self.matches.list {
            if kept.contains(&found.start.line) && found.end.line < kept.end {
                out.push(*found, MAX_MATCHES);
            }
        }
        Some(scan(
            buffer,
            pattern,
            rescan..buffer.total(),
            top,
            MAX_MATCHES,
            out,
        ))
    }

    /// The highlighted spans of the visible rows: `rows` rows from the top
    /// of the viewport, `columns` wide, on a grid whose buffer stands at
    /// `origin`. None while the grid has another size than the one the
    /// matches were found on, or shows the other screen.
    pub fn spans<B: Buffer + ?Sized>(
        &self,
        buffer: &B,
        origin: Origin,
        rows: usize,
        columns: usize,
    ) -> Vec<MatchSpan> {
        let list = &self.matches.list;
        if list.is_empty()
            || rows == 0
            || columns == 0
            || !self.basis.same_size(buffer)
            || origin.alt_screen != self.basis.origin.alt_screen
        {
            return Vec::new();
        }
        let top = buffer.view_top() + origin.evicted;
        let bottom = top + rows - 1;
        let first = list.partition_point(|m| m.end.line < top);
        let mut spans = Vec::new();
        for (index, found) in list.iter().enumerate().skip(first) {
            if found.start.line > bottom {
                break;
            }
            let current = self.current == Some(index);
            for line in found.start.line.max(top)..=found.end.line.min(bottom) {
                let first_column = if line == found.start.line {
                    found.start.column
                } else {
                    0
                };
                let last_column = if line == found.end.line {
                    found.end.column
                } else {
                    columns - 1
                };
                if first_column <= last_column && first_column < columns {
                    spans.push(MatchSpan {
                        row: line - top,
                        first: first_column,
                        last: last_column.min(columns - 1),
                        current,
                    });
                }
            }
        }
        spans
    }

    /// The display offset that shows `found` (a match of a state on a grid
    /// whose buffer has dropped `evicted` lines): the current one when the
    /// match is already visible or has dropped off, else one that puts it
    /// in the middle of the screen.
    pub fn offset_showing<B: Buffer + ?Sized>(
        buffer: &B,
        evicted: usize,
        found: SearchMatch,
    ) -> usize {
        let history = buffer.above();
        let rows = buffer.rows();
        let offset = buffer.view_offset();
        let (Some(start), Some(end)) = (
            found.start.line.checked_sub(evicted),
            found.end.line.checked_sub(evicted),
        ) else {
            return offset;
        };
        let top = history - offset;
        let bottom = top + rows - 1;
        if start >= top && end <= bottom {
            return offset;
        }
        (history + rows / 2).saturating_sub(start).min(history)
    }
}

/// The buffer line of the bottom visible row.
fn visible_bottom<B: Buffer + ?Sized>(buffer: &B) -> usize {
    buffer.view_top() + buffer.rows().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::Term;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::{Dimensions, Scroll};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::limits::{self, Capped};
    use crate::model::GridDims;

    fn term(cols: usize, rows: usize, scrollback: usize, text: &str) -> Term<VoidListener> {
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let mut term = Term::new(config, &GridDims { cols, rows }, VoidListener);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, text.as_bytes());
        term
    }

    fn point(line: usize, column: usize) -> BufferPoint {
        BufferPoint { line, column }
    }

    #[test]
    fn finds_matches_in_the_scrollback_and_on_screen_ignoring_case() {
        let mut text = String::new();
        for line in 0..30 {
            text.push_str(&format!("line {line} Error\r\n"));
        }
        let term = term(20, 5, 100, &text);
        let grid = term.grid();
        assert!(grid.history_size() > 20, "most lines are in the scrollback");
        let found = find(grid, "error", MAX_MATCHES);
        assert_eq!(found.list.len(), 30);
        assert!(!found.truncated);
        assert_eq!(
            found.list[0],
            SearchMatch {
                start: point(0, 7),
                end: point(0, 11)
            }
        );
        // Upper case query, lower case text.
        assert_eq!(find(grid, "LINE 2", MAX_MATCHES).list.len(), 11);
        assert!(find(grid, "", MAX_MATCHES).list.is_empty());
        assert!(find(grid, "absent", MAX_MATCHES).list.is_empty());
    }

    #[test]
    fn wide_characters_and_wrapped_lines() {
        // 10 columns: "ab你好cd" is 8 cells; "wrapped-word" wraps.
        let term = term(10, 6, 100, "ab你好cd\r\nxx wrapped-word\r\nx你好你好你好");
        let grid = term.grid();
        let found = find(grid, "你好c", MAX_MATCHES).list;
        assert_eq!(
            found,
            [SearchMatch {
                start: point(0, 2),
                end: point(0, 6)
            }]
        );
        // The second 好 ends on column 5 (its spacer).
        let found = find(grid, "好", MAX_MATCHES).list;
        assert_eq!(found[0].end, point(0, 5));
        // A word broken by the right edge is one match over two rows.
        let found = find(grid, "wrapped-word", MAX_MATCHES).list;
        assert_eq!(
            found,
            [SearchMatch {
                start: point(1, 3),
                end: point(2, 4)
            }]
        );
        // "x你好你好" fills columns 0-8; the next 你 does not fit in column 9
        // (a leading spacer) and continues on the next row: still one
        // logical line.
        let found = find(grid, "好你好你", MAX_MATCHES).list;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].start, point(3, 3));
        assert_eq!(found[0].end, point(4, 1));
    }

    #[test]
    fn matches_do_not_cross_line_breaks_or_trailing_blanks() {
        let term = term(20, 4, 10, "abc\r\ndef");
        let grid = term.grid();
        assert!(find(grid, "cd", MAX_MATCHES).list.is_empty());
        assert!(find(grid, "c ", MAX_MATCHES).list.is_empty());
        // Non-overlapping.
        let term = super::tests::term(20, 2, 10, "aaaa");
        assert_eq!(find(term.grid(), "aa", MAX_MATCHES).list.len(), 2);
    }

    #[test]
    fn hidden_text_is_not_found() {
        let term = term(20, 2, 10, "pw \x1b[8msecret\x1b[0m end");
        assert!(find(term.grid(), "secret", MAX_MATCHES).list.is_empty());
        assert_eq!(find(term.grid(), "end", MAX_MATCHES).list.len(), 1);
    }

    #[test]
    fn the_match_list_is_capped() {
        let term = term(80, 4, 10, &"x ".repeat(30));
        let found = find(term.grid(), "x", 5);
        assert_eq!(found.list.len(), 5);
        assert!(found.truncated);
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let term = term(20, 4, 10, "one\r\ntwo one\r\none");
        let mut state = SearchState::new("one", term.grid(), Origin::default());
        assert_eq!(state.count(), 3);
        // The newest visible match is current.
        assert_eq!(state.current_index(), Some(2));
        assert_eq!(state.step(true).map(|m| m.start), Some(point(0, 0)));
        assert_eq!(state.current_index(), Some(0));
        assert_eq!(state.step(false).map(|m| m.start), Some(point(2, 0)));
        assert_eq!(state.step(false).map(|m| m.start), Some(point(1, 4)));
        let mut empty = SearchState::new("none", term.grid(), Origin::default());
        assert_eq!(empty.step(true), None);
        assert_eq!(empty.current_index(), None);
    }

    #[test]
    fn spans_cover_visible_rows_and_mark_the_current_match() {
        let term = term(10, 3, 10, "ab\r\nxx wrapped-word");
        let state = SearchState::new("wrapped-word", term.grid(), Origin::default());
        let spans = state.spans(term.grid(), Origin::default(), 3, 10);
        assert_eq!(
            spans,
            [
                MatchSpan {
                    row: 1,
                    first: 3,
                    last: 9,
                    current: true
                },
                MatchSpan {
                    row: 2,
                    first: 0,
                    last: 4,
                    current: true
                },
            ]
        );
    }

    #[test]
    fn refresh_keeps_the_current_match_while_output_grows() {
        let mut term = term(20, 3, 100, "hit 1\r\nhit 2\r\n");
        let mut state = SearchState::new("hit", term.grid(), Origin::default());
        state.step(false);
        let current = state.current().unwrap();
        assert_eq!(current.start.line, 0);
        // More output scrolls the screen into the scrollback.
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, b"x\r\ny\r\nhit 3\r\nz\r\n");
        assert!(state.refresh(term.grid(), Origin::default()));
        assert_eq!(state.count(), 3);
        assert_eq!(state.current(), Some(current), "same text, same match");
        assert!(
            !state.refresh(term.grid(), Origin::default()),
            "nothing new"
        );
    }

    #[test]
    fn the_offset_that_shows_a_match() {
        let mut text = String::new();
        for line in 0..50 {
            text.push_str(&format!("row {line}\r\n"));
        }
        let mut term = term(20, 5, 100, &text);
        let found = find(term.grid(), "row 3", MAX_MATCHES).list;
        let first = found[0];
        let offset = SearchState::offset_showing(term.grid(), 0, first);
        assert!(offset > 0);
        term.scroll_display(Scroll::Delta(offset as i32));
        let state = SearchState::new("row 3", term.grid(), Origin::default());
        let spans = state.spans(term.grid(), Origin::default(), 5, 20);
        assert!(spans.iter().any(|span| span.first == 0 && span.last == 4));
        // Already visible: the offset stays.
        assert_eq!(SearchState::offset_showing(term.grid(), 0, first), offset);
    }

    /// Parses `text` through the app's handler, which counts the lines that
    /// drop off the scrollback when [`limits::count_evictions`] is on.
    fn feed_counted(term: &mut Term<VoidListener>, text: &str) {
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut Capped(term), text.as_bytes());
    }

    /// Where the buffer of a term fed by [`feed_counted`] stands.
    fn origin_of(term: &Term<VoidListener>) -> Origin {
        Origin {
            evicted: limits::evictions(),
            alt_screen: term
                .mode()
                .contains(alacritty_terminal::term::TermMode::ALT_SCREEN),
        }
    }

    /// The text of the row a match of a state starts on.
    fn row_text(term: &Term<VoidListener>, origin: Origin, found: SearchMatch) -> String {
        let grid = term.grid();
        let row = grid.row(found.start.line - origin.evicted);
        (0..grid.columns())
            .map(|column| row[Column(column)].c)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    /// A reflow moves every match: the current one is found again by its
    /// ordinal, not by its old position (three wrapped matches, the last
    /// one current).
    #[test]
    fn a_reflow_keeps_the_current_match_by_its_ordinal() {
        let line = format!("{}NEEDLE\r\n", "a".repeat(50));
        let mut term = term(80, 10, 100, &line.repeat(3));
        let mut state = SearchState::new("needle", term.grid(), Origin::default());
        assert_eq!((state.count(), state.current_index()), (3, Some(2)));
        assert_eq!(state.current().unwrap().start, point(2, 50));

        term.resize(GridDims { cols: 30, rows: 10 });
        // Until the search runs again, nothing is highlighted on the
        // reflowed text.
        assert!(
            state
                .spans(term.grid(), Origin::default(), 10, 30)
                .is_empty()
        );
        assert!(state.refresh(term.grid(), Origin::default()));
        assert_eq!((state.count(), state.current_index()), (3, Some(2)));
        let current = state.current().unwrap();
        assert_eq!(current.start.column, 50 % 30);
        assert_eq!(
            state.matches(),
            SearchState::new("needle", term.grid(), Origin::default()).matches()
        );
        assert!(
            state
                .spans(term.grid(), Origin::default(), 10, 30)
                .iter()
                .any(|span| span.current)
        );

        // Wider again: still the third.
        term.resize(GridDims {
            cols: 100,
            rows: 10,
        });
        assert!(state.refresh(term.grid(), Origin::default()));
        assert_eq!(state.current_index(), Some(2));
        assert_eq!(state.current().unwrap().start, point(2, 50));
    }

    /// Owner's gap 13: once the scrollback is full, output drops the oldest
    /// lines; the current match stays on its text (and the list equals a
    /// fresh search) instead of drifting to a newer match.
    #[test]
    fn the_current_match_keeps_its_text_while_lines_drop_off() {
        limits::count_evictions(20, Default::default());
        let mut term = term(20, 4, 20, "");
        let lines = |range: Range<usize>| -> String {
            range.map(|index| format!("hit {index:03}\r\n")).collect()
        };
        feed_counted(&mut term, &lines(0..40));
        let origin = origin_of(&term);
        assert_eq!(term.grid().history_size(), 20);
        assert!(origin.evicted > 0, "the scrollback is full");
        let mut state = SearchState::new("hit", term.grid(), origin);
        let count = state.count();
        while state
            .current()
            .is_some_and(|found| row_text(&term, origin, found) != "hit 030")
        {
            state.step(false);
        }
        let index = state.current_index().unwrap();

        feed_counted(&mut term, &lines(40..50));
        let later = origin_of(&term);
        assert_eq!(later.evicted, origin.evicted + 10);
        assert!(state.refresh(term.grid(), later));
        let current = state.current().unwrap();
        assert_eq!(row_text(&term, later, current), "hit 030");
        assert_eq!(state.current_index(), Some(index - 10));
        assert_eq!(state.count(), count);
        assert_eq!(
            state.matches(),
            SearchState::new("hit", term.grid(), later).matches()
        );
        // The view would keep it in sight.
        let offset = SearchState::offset_showing(term.grid(), later.evicted, current);
        assert!(offset > 0);

        // Once it drops off itself, the oldest match left is current.
        feed_counted(&mut term, &lines(50..80));
        let last = origin_of(&term);
        assert!(state.refresh(term.grid(), last));
        let current = state.current().unwrap();
        assert_eq!(state.current_index(), Some(0));
        assert_eq!(current.start.line, last.evicted);
    }

    /// A small deterministic generator for the test below.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    /// Searching only the rows that can have changed finds exactly what a
    /// fresh search finds: wrapped lines across the previous screen top and
    /// across the dropped lines, a self-overlapping query, wide characters,
    /// screen edits, clears (ED 2, ED 3), scrolls (SU, DL) and the
    /// alternate screen.
    #[test]
    fn searching_the_changed_rows_matches_a_fresh_search() {
        for query in ["aa", "ab", "你a"] {
            limits::count_evictions(12, Default::default());
            let mut term = term(7, 4, 12, "");
            let mut random = Lcg(0x5eed);
            let mut state = SearchState::new(query, term.grid(), origin_of(&term));
            for _ in 0..400 {
                let mut chunk = String::new();
                for _ in 0..random.next(4) + 1 {
                    match random.next(20) {
                        0 => chunk.push_str("\x1b[2J"),
                        1 => chunk.push_str("\x1b[3J"),
                        2 => chunk.push_str("\x1b[2S"),
                        3 => chunk.push_str("\x1b[H\x1b[M"),
                        4 => chunk.push_str("\x1b[2Aab\x1b[2B"),
                        5 => chunk.push_str("\x1b[?1049h"),
                        6 => chunk.push_str("\x1b[?1049l"),
                        _ => {
                            for _ in 0..random.next(18) {
                                chunk.push(match random.next(5) {
                                    0 | 1 => 'a',
                                    2 => 'b',
                                    3 => '你',
                                    _ => 'x',
                                });
                            }
                            chunk.push_str("\r\n");
                        }
                    }
                }
                feed_counted(&mut term, &chunk);
                assert!(term.grid().history_size() <= 12);
                let origin = origin_of(&term);
                state.refresh(term.grid(), origin);
                let fresh = SearchState::new(query, term.grid(), origin);
                assert_eq!(state.matches(), fresh.matches(), "{query:?} {chunk:?}");
                assert!(
                    state
                        .current_index()
                        .is_none_or(|index| index < state.count())
                );
            }
        }
    }
}
