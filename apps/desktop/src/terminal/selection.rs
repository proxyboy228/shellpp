//! Selecting text with the mouse (UI 4.3): drag selects characters, a
//! double click a word (bounded by spaces and `,│`|:"' ()[]{}<>`, the
//! semantic characters of alacritty), a triple click a line (a wrapped line
//! as a whole). Ctrl+Shift+A selects the screen and the scrollback.
//!
//! The selection is kept in buffer lines counted with the lines that
//! dropped off the top before (as the search's positions), so it stays on
//! its text while output scrolls, and it reaches into the compact part of
//! the scrollback (D85). Its part on the screen is mirrored in the emulator
//! ([`Selection::mirror`]): alacritty drops a selection when output writes
//! over it, and the view then ends its own.

use alacritty_terminal::grid::Row;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection as Emulated, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};

use super::journal::Buffer;
use super::search::BufferPoint;

/// Characters that end a word for a double click (alacritty's default).
pub const SEMANTIC_ESCAPE_CHARS: &str = ",│`|:\"' ()[]{}<>\t";

/// The selection kind of a click.
pub fn selection_type(click_count: usize) -> SelectionType {
    match click_count {
        0 | 1 => SelectionType::Simple,
        2 => SelectionType::Semantic,
        _ => SelectionType::Lines,
    }
}

/// One end of a selection: a cell (its line counted with the lines dropped
/// before) and the half of it the pointer was on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    point: BufferPoint,
    side: Side,
}

/// A selection in the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    kind: SelectionType,
    anchor: Anchor,
    head: Anchor,
}

impl Selection {
    /// A selection that starts at `point` (a click): a single click starts
    /// an empty one that a drag extends; double and triple clicks select
    /// at once.
    pub fn new(kind: SelectionType, point: BufferPoint, side: Side) -> Self {
        let anchor = Anchor { point, side };
        Self {
            kind,
            anchor,
            head: anchor,
        }
    }

    /// Everything: the scrollback and the screen of `buffer`, which has
    /// dropped `evicted` lines before.
    pub fn all<B: Buffer + ?Sized>(buffer: &B, evicted: usize) -> Self {
        let mut selection = Self::new(
            SelectionType::Simple,
            BufferPoint {
                line: evicted,
                column: 0,
            },
            Side::Left,
        );
        selection.update(
            BufferPoint {
                line: evicted + buffer.total().saturating_sub(1),
                column: buffer.cols().saturating_sub(1),
            },
            Side::Right,
        );
        selection
    }

    pub fn kind(&self) -> SelectionType {
        self.kind
    }

    /// Extends the selection to `point` (a drag).
    pub fn update(&mut self, point: BufferPoint, side: Side) {
        self.head = Anchor { point, side };
    }

    /// The two ends in buffer order.
    fn ordered(&self) -> (Anchor, Anchor) {
        if self.head.point < self.anchor.point {
            (self.head, self.anchor)
        } else {
            (self.anchor, self.head)
        }
    }

    /// Whether nothing is selected: a click without a drag, or a drag that
    /// stayed between two cells.
    pub fn is_empty(&self) -> bool {
        if self.kind != SelectionType::Simple {
            return false;
        }
        let (start, end) = self.ordered();
        start == end
            || (start.side == Side::Right
                && end.side == Side::Left
                && start.point.line == end.point.line
                && start.point.column + 1 == end.point.column)
    }

    /// The first and the last selected cell (inclusive) in lines of
    /// `buffer`, which has dropped `evicted` lines before; `None` when empty
    /// or dropped off.
    pub fn range<B: Buffer + ?Sized>(
        &self,
        buffer: &B,
        evicted: usize,
    ) -> Option<(BufferPoint, BufferPoint)> {
        let columns = buffer.cols();
        let total = buffer.total();
        if columns == 0 || total == 0 || self.is_empty() {
            return None;
        }
        let (mut start, mut end) = self.ordered();
        if self.kind == SelectionType::Simple {
            // A drag from the right half of a cell leaves it out, one to the
            // left half of a cell leaves that one out.
            if end.side == Side::Left && start.point != end.point {
                if end.point.column == 0 {
                    end.point.line = end.point.line.checked_sub(1)?;
                    end.point.column = columns - 1;
                } else {
                    end.point.column -= 1;
                }
            }
            if start.side == Side::Right && start.point != end.point {
                start.point.column += 1;
                if start.point.column >= columns {
                    start.point.column = 0;
                    start.point.line += 1;
                }
            }
        }
        // To lines of the buffer, without what dropped off.
        let end_line = end.point.line.checked_sub(evicted)?.min(total - 1);
        let mut first = match start.point.line.checked_sub(evicted) {
            Some(line) => BufferPoint {
                line,
                column: start.point.column.min(columns - 1),
            },
            None => BufferPoint { line: 0, column: 0 },
        };
        let mut last = BufferPoint {
            line: end_line,
            column: end.point.column.min(columns - 1),
        };
        if first > last {
            return None;
        }
        match self.kind {
            SelectionType::Simple | SelectionType::Block => {}
            SelectionType::Semantic => {
                first = word_start(buffer, first);
                last = word_end(buffer, last);
            }
            SelectionType::Lines => {
                first = BufferPoint {
                    line: logical_start(buffer, first.line),
                    column: 0,
                };
                last = BufferPoint {
                    line: logical_end(buffer, last.line),
                    column: columns - 1,
                };
            }
        }
        Some((first, last))
    }

    /// The selected text: the trailing blanks of each line left out, line
    /// breaks where a line ends (a wrapped row goes on); a line selection
    /// without its final line break, so pasting a copied line does not run
    /// it at once.
    pub fn text<B: Buffer + ?Sized>(&self, buffer: &B, evicted: usize) -> Option<String> {
        let (start, end) = self.range(buffer, evicted)?;
        let columns = buffer.cols();
        let mut text = String::new();
        for line in start.line..=end.line {
            let row = buffer.row(line);
            let mut first = if line == start.line { start.column } else { 0 };
            let last = if line == end.line {
                end.column
            } else {
                columns - 1
            };
            // The second half of a wide character takes the character.
            if first > 0 && row[Column(first)].flags.contains(Flags::WIDE_CHAR_SPACER) {
                first -= 1;
            }
            let length = line_length(&row, columns).min(last + 1);
            for column in first..length {
                let cell = &row[Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
            }
            let wrapped = length > 0 && row[Column(length - 1)].flags.contains(Flags::WRAPLINE);
            if last >= columns - 1 && !wrapped {
                text.push('\n');
            }
        }
        if text.ends_with('\n') {
            text.pop();
        }
        (!text.is_empty()).then_some(text)
    }

    /// The selected columns of each visible row of `buffer`: (row, first,
    /// last), for `rows` rows from the top of the view.
    pub fn spans<B: Buffer + ?Sized>(
        &self,
        buffer: &B,
        evicted: usize,
        rows: usize,
    ) -> Vec<(usize, usize, usize)> {
        let Some((start, end)) = self.range(buffer, evicted) else {
            return Vec::new();
        };
        let columns = buffer.cols();
        let top = buffer.view_top();
        let mut spans = Vec::new();
        for row in 0..rows {
            let line = top + row;
            if line < start.line || line > end.line {
                continue;
            }
            let first = if line == start.line { start.column } else { 0 };
            let last = if line == end.line {
                end.column
            } else {
                columns - 1
            };
            if first <= last {
                spans.push((row, first, last));
            }
        }
        spans
    }

    /// The emulator's copy of the part on the screen of `buffer` (whose
    /// grid's screen starts at buffer line `buffer.above()`), or `None`
    /// when no part of it is on the screen.
    pub fn mirror<B: Buffer + ?Sized>(&self, buffer: &B, evicted: usize) -> Option<Emulated> {
        let (start, end) = self.range(buffer, evicted)?;
        let screen = buffer.above();
        if end.line < screen {
            return None;
        }
        let first = if start.line < screen {
            Point::new(Line(0), Column(0))
        } else {
            Point::new(Line((start.line - screen) as i32), Column(start.column))
        };
        let last = Point::new(Line((end.line - screen) as i32), Column(end.column));
        let mut emulated = Emulated::new(SelectionType::Simple, first, Side::Left);
        emulated.update(last, Side::Right);
        Some(emulated)
    }
}

/// Where a line's text ends: every cell of a wrapped row, else the cell
/// after the last one that is not a plain blank.
fn line_length(row: &Row<Cell>, columns: usize) -> usize {
    if columns == 0 {
        return 0;
    }
    if row[Column(columns - 1)].flags.contains(Flags::WRAPLINE) {
        return columns;
    }
    (0..columns)
        .rev()
        .find(|&column| {
            let cell = &row[Column(column)];
            cell.c != ' ' || cell.zerowidth().is_some_and(|extra| !extra.is_empty())
        })
        .map_or(0, |column| column + 1)
}

/// The first buffer line of the logical line that holds `line`.
fn logical_start<B: Buffer + ?Sized>(buffer: &B, mut line: usize) -> usize {
    while line > 0 && buffer.wraps(line - 1) {
        line -= 1;
    }
    line
}

/// The last buffer line of the logical line that holds `line`.
fn logical_end<B: Buffer + ?Sized>(buffer: &B, mut line: usize) -> usize {
    let last = buffer.total().saturating_sub(1);
    while line < last && buffer.wraps(line) {
        line += 1;
    }
    line
}

/// Whether the cell at `point` ends a word; spacers of wide characters
/// belong to their character.
fn escape<B: Buffer + ?Sized>(buffer: &B, point: BufferPoint) -> Option<bool> {
    let cell = buffer.row(point.line)[Column(point.column)].clone();
    if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        return None;
    }
    Some(SEMANTIC_ESCAPE_CHARS.contains(cell.c))
}

/// The first cell of the word at `point`, across wrapped rows; `point`
/// itself when it is a word end character.
fn word_start<B: Buffer + ?Sized>(buffer: &B, point: BufferPoint) -> BufferPoint {
    if escape(buffer, point) == Some(true) {
        return point;
    }
    let columns = buffer.cols();
    let mut at = point;
    loop {
        let previous = if at.column > 0 {
            BufferPoint {
                line: at.line,
                column: at.column - 1,
            }
        } else if at.line > 0 && buffer.wraps(at.line - 1) {
            BufferPoint {
                line: at.line - 1,
                column: columns - 1,
            }
        } else {
            return at;
        };
        match escape(buffer, previous) {
            Some(true) => return at,
            Some(false) => at = previous,
            // A spacer: its character decides.
            None => at = previous,
        }
    }
}

/// The last cell of the word at `point`, across wrapped rows (a wide
/// character's second cell included).
fn word_end<B: Buffer + ?Sized>(buffer: &B, point: BufferPoint) -> BufferPoint {
    if escape(buffer, point) == Some(true) {
        return point;
    }
    let columns = buffer.cols();
    let last_line = buffer.total().saturating_sub(1);
    let mut at = point;
    loop {
        let next = if at.column + 1 < columns {
            BufferPoint {
                line: at.line,
                column: at.column + 1,
            }
        } else if at.line < last_line && buffer.wraps(at.line) {
            BufferPoint {
                line: at.line + 1,
                column: 0,
            }
        } else {
            return at;
        };
        match escape(buffer, next) {
            Some(true) => return at,
            Some(false) | None => at = next,
        }
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::Term;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::terminal::model::GridDims;

    fn term(text: &str) -> Term<VoidListener> {
        let mut term = Term::new(
            Config::default(),
            &GridDims { cols: 30, rows: 5 },
            VoidListener,
        );
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, text.as_bytes());
        term
    }

    /// A cell of the screen row `row` of `term`.
    fn at(term: &Term<VoidListener>, row: usize, column: usize) -> BufferPoint {
        BufferPoint {
            line: term.grid().above() + row,
            column,
        }
    }

    fn text(term: &Term<VoidListener>, selection: &Selection) -> Option<String> {
        selection.text(term.grid(), 0)
    }

    #[test]
    fn double_click_selects_a_word() {
        let term = term("ls /usr/local/bin (done)");
        let word = Selection::new(SelectionType::Semantic, at(&term, 0, 7), Side::Left);
        assert_eq!(text(&term, &word).as_deref(), Some("/usr/local/bin"));
        let word = Selection::new(SelectionType::Semantic, at(&term, 0, 20), Side::Left);
        assert_eq!(text(&term, &word).as_deref(), Some("done"));
    }

    #[test]
    fn triple_click_selects_the_line() {
        let term = term("first line\r\nsecond line here\r\nthird");
        let line = Selection::new(SelectionType::Lines, at(&term, 1, 3), Side::Left);
        assert_eq!(text(&term, &line).as_deref(), Some("second line here"));
    }

    #[test]
    fn a_wrapped_line_is_selected_as_a_whole() {
        // 40 characters wrap at 30 columns.
        let long = "abcdefghij".repeat(4);
        let term = term(&long);
        let line = Selection::new(SelectionType::Lines, at(&term, 1, 2), Side::Left);
        assert_eq!(text(&term, &line), Some(long.clone()));
        let word = Selection::new(SelectionType::Semantic, at(&term, 1, 2), Side::Left);
        assert_eq!(text(&term, &word), Some(long));
    }

    #[test]
    fn drag_selects_characters_and_a_click_selects_nothing() {
        let term = term("hello world");
        let click = Selection::new(SelectionType::Simple, at(&term, 0, 0), Side::Left);
        assert!(click.is_empty(), "a click alone");
        let mut drag = click;
        drag.update(at(&term, 0, 4), Side::Right);
        assert!(!drag.is_empty());
        assert_eq!(text(&term, &drag).as_deref(), Some("hello"));
        // Backwards, from the left half of a cell: that cell is left out.
        let mut back = Selection::new(SelectionType::Simple, at(&term, 0, 10), Side::Left);
        back.update(at(&term, 0, 6), Side::Left);
        assert_eq!(text(&term, &back).as_deref(), Some("worl"));
    }

    #[test]
    fn select_all_includes_the_scrollback() {
        let mut lines = String::new();
        for n in 0..12 {
            lines.push_str(&format!("line {n}\r\n"));
        }
        let term = term(&lines);
        let all = Selection::all(term.grid(), 0);
        let all = text(&term, &all).unwrap();
        assert!(all.starts_with("line 0\n"), "{all:?}");
        assert!(all.contains("line 11"));
    }

    #[test]
    fn wide_characters_select_whole() {
        let term = term("ab 你好 cd");
        // Column 4 is the second half of 你.
        let word = Selection::new(SelectionType::Semantic, at(&term, 0, 4), Side::Left);
        assert_eq!(text(&term, &word).as_deref(), Some("你好"));
        let mut drag = Selection::new(SelectionType::Simple, at(&term, 0, 3), Side::Left);
        drag.update(at(&term, 0, 4), Side::Right);
        assert_eq!(text(&term, &drag).as_deref(), Some("你"));
    }

    #[test]
    fn lines_that_dropped_off_leave_the_selection() {
        let term = term("one\r\ntwo\r\nthree");
        let mut drag = Selection::new(SelectionType::Simple, at(&term, 0, 0), Side::Left);
        drag.update(at(&term, 2, 4), Side::Right);
        // Two lines dropped off before: the selection starts at the top.
        let shifted = |line: usize| BufferPoint { line, column: 0 };
        let mut later = Selection::new(SelectionType::Simple, shifted(0), Side::Left);
        later.update(BufferPoint { line: 2, column: 4 }, Side::Right);
        assert_eq!(later.text(term.grid(), 2).as_deref(), Some("one"));
        assert!(
            later.text(term.grid(), 3).is_none(),
            "all of it dropped off"
        );
        assert_eq!(text(&term, &drag).as_deref(), Some("one\ntwo\nthree"));
    }
}
