//! The compact part of the scrollback (run 5, plan 6; D85). Alacritty keeps
//! every row of its grid as wide as the window, 24 bytes a cell: 10 000
//! lines of a wide window take tens of megabytes. Rows older than the
//! newest [`LIVE_HISTORY`] scrollback lines move here, as the text of the
//! row without its trailing blanks and its attributes as runs (a shell line
//! takes a hundred bytes or so instead of kilobytes).
//!
//! - Rows move in batches, by [`compact`], when the parser has nothing
//!   queued or at most every [`COMPACT_INTERVAL`]: a flood of output pays
//!   nothing per line, and memory goes down once it stops.
//! - The scrollback limit holds for the journal and the grid together: when
//!   output needs room, the journal's oldest lines go first
//!   ([`super::limits`]), so the buffer keeps its order.
//! - A row comes back as an alacritty row ([`Journal::row`]) for drawing,
//!   search and selection: the same characters, combining marks, colors,
//!   attributes and wrap. Not kept: OSC 8 links and underline colors.
//!   Rows keep the width they had: a later resize does not reflow them
//!   (a wider row is cut, a narrower one padded).
//! - Clearing the scrollback (ED 3, RIS) clears the journal too.
//!
//! Nothing here logs terminal text.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::time::Duration;

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Grid, Row, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use unicode_width::UnicodeWidthChar as _;

use super::limits::MAX_ZEROWIDTH;

/// Scrollback lines the grid keeps above the screen once the rest moved to
/// the journal: searches near the prompt and the history capture's look
/// back stay on full rows.
pub const LIVE_HISTORY: usize = 500;
/// Lines over [`LIVE_HISTORY`] a compaction waits for while output keeps
/// coming.
pub const COMPACT_BATCH: usize = 200;
/// Longest time output can keep the grid over [`LIVE_HISTORY`] +
/// [`COMPACT_BATCH`] lines.
pub const COMPACT_INTERVAL: Duration = Duration::from_millis(250);

/// Cells of one look, in a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    cells: u16,
    fg: Color,
    bg: Color,
    flags: Flags,
}

/// One row: every cell's character (wide character spacers as blanks),
/// each followed by its combining marks; the trailing blanks left out.
#[derive(Debug)]
struct Stored {
    text: Box<str>,
    runs: Box<[Run]>,
    wrapped: bool,
}

impl Stored {
    fn heap_bytes(&self) -> usize {
        self.text.len() + self.runs.len() * size_of::<Run>()
    }
}

/// The compact scrollback lines, oldest first.
#[derive(Debug, Default)]
pub struct Journal {
    lines: VecDeque<Stored>,
    heap: usize,
    /// Rows of the view above the grid's scrollback (scrolled into the
    /// journal); the grid then shows its oldest line at the top.
    view: usize,
}

/// A blank cell as a new row holds it.
fn blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.fg == Color::Named(NamedColor::Foreground)
        && cell.bg == Color::Named(NamedColor::Background)
        && (cell.flags - Flags::WRAPLINE).is_empty()
        && cell.zerowidth().is_none_or(<[char]>::is_empty)
}

impl Journal {
    /// Lines held.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Bytes the journal holds, its own structures included.
    pub fn bytes(&self) -> usize {
        self.heap + self.lines.capacity() * size_of::<Stored>() + size_of::<Self>()
    }

    /// Rows of the view above the grid's scrollback.
    pub fn view(&self) -> usize {
        self.view
    }

    /// Adds `row` (of a grid `columns` wide) as the newest line.
    pub fn push(&mut self, row: &Row<Cell>, columns: usize) {
        let columns = columns.min(row.len());
        let wrapped = columns > 0 && row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        let end = if wrapped {
            columns
        } else {
            (0..columns)
                .rev()
                .find(|&column| !blank(&row[Column(column)]))
                .map_or(0, |column| column + 1)
        };
        let mut text = String::with_capacity(end);
        let mut runs: Vec<Run> = Vec::new();
        for column in 0..end {
            let cell = &row[Column(column)];
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter().take(MAX_ZEROWIDTH));
            }
            let look = Run {
                cells: 1,
                fg: cell.fg,
                bg: cell.bg,
                flags: cell.flags - Flags::WRAPLINE,
            };
            match runs.last_mut() {
                Some(run)
                    if (run.fg, run.bg, run.flags) == (look.fg, look.bg, look.flags)
                        && run.cells < u16::MAX =>
                {
                    run.cells += 1;
                }
                _ => runs.push(look),
            }
        }
        let stored = Stored {
            text: text.into_boxed_str(),
            runs: runs.into_boxed_slice(),
            wrapped,
        };
        self.heap += stored.heap_bytes();
        self.lines.push_back(stored);
    }

    /// Drops the `count` oldest lines.
    pub fn pop_front(&mut self, count: usize) {
        for _ in 0..count.min(self.lines.len()) {
            if let Some(line) = self.lines.pop_front() {
                self.heap -= line.heap_bytes();
            }
        }
        self.view = self.view.min(self.lines.len());
        self.shrink();
    }

    /// Drops every line (the scrollback was cleared).
    pub fn clear(&mut self) {
        self.lines = VecDeque::new();
        self.heap = 0;
        self.view = 0;
    }

    /// Gives back spare room once most of it is unused.
    fn shrink(&mut self) {
        if self.lines.capacity() > 64 && self.lines.len() < self.lines.capacity() / 4 {
            self.lines.shrink_to(self.lines.len() * 2);
        }
    }

    /// Whether line `index` continues on the next line.
    pub fn wraps(&self, index: usize) -> bool {
        self.lines.get(index).is_some_and(|line| line.wrapped)
    }

    /// Line `index` as a row `columns` wide (cut or padded).
    pub fn row(&self, index: usize, columns: usize) -> Row<Cell> {
        let mut row = Row::new(columns);
        let Some(line) = self.lines.get(index) else {
            return row;
        };
        let mut chars = line.text.chars().peekable();
        let mut column = 0;
        for run in line.runs.iter() {
            for _ in 0..run.cells {
                let Some(ch) = chars.next() else {
                    break;
                };
                let mut marks = 0;
                while let Some(&mark) = chars.peek() {
                    if mark.width() != Some(0) {
                        break;
                    }
                    chars.next();
                    if column < columns && marks < MAX_ZEROWIDTH {
                        row[Column(column)].push_zerowidth(mark);
                        marks += 1;
                    }
                }
                if column < columns {
                    let cell = &mut row[Column(column)];
                    cell.c = ch;
                    cell.fg = run.fg;
                    cell.bg = run.bg;
                    cell.flags = run.flags;
                }
                column += 1;
            }
        }
        if line.wrapped && columns > 0 {
            row[Column(columns - 1)].flags.insert(Flags::WRAPLINE);
        }
        row
    }
}

/// Lines as drawing, search and selection read them: buffer line 0 is the
/// oldest line of the scrollback, the screen's rows are the last ones.
pub trait Buffer {
    fn cols(&self) -> usize;
    fn rows(&self) -> usize;
    /// Lines above the screen.
    fn above(&self) -> usize;
    /// Rows the view is scrolled up from the bottom.
    fn view_offset(&self) -> usize;
    /// The row of buffer line `line` (below [`Self::total`]).
    fn row(&self, line: usize) -> Cow<'_, Row<Cell>>;

    /// Whether buffer line `line` continues on the next one.
    fn wraps(&self, line: usize) -> bool {
        let columns = self.cols();
        columns > 0
            && self.row(line)[Column(columns - 1)]
                .flags
                .contains(Flags::WRAPLINE)
    }

    /// Lines of the buffer: the scrollback and the screen.
    fn total(&self) -> usize {
        self.above() + self.rows()
    }

    /// The buffer line of the top row of the view.
    fn view_top(&self) -> usize {
        self.above().saturating_sub(self.view_offset())
    }
}

impl Buffer for Grid<Cell> {
    fn cols(&self) -> usize {
        Dimensions::columns(self)
    }

    fn rows(&self) -> usize {
        Dimensions::screen_lines(self)
    }

    fn above(&self) -> usize {
        self.history_size()
    }

    fn view_offset(&self) -> usize {
        self.display_offset()
    }

    fn row(&self, line: usize) -> Cow<'_, Row<Cell>> {
        Cow::Borrowed(&self[Line(line as i32 - self.history_size() as i32)])
    }
}

/// The journal and the grid of a terminal as one buffer (read while both
/// are locked).
#[derive(Clone, Copy)]
pub struct Lines<'a> {
    pub grid: &'a Grid<Cell>,
    pub journal: &'a Journal,
}

impl std::fmt::Debug for Lines<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lines").finish_non_exhaustive()
    }
}

impl Lines<'_> {
    /// The grid line of buffer line `line`, when it is in the grid.
    pub fn grid_line(&self, line: usize) -> Option<Line> {
        line.checked_sub(self.journal.len())
            .map(|line| Line(line as i32 - self.grid.history_size() as i32))
    }

    /// The buffer line of grid line `line`.
    pub fn buffer_line(&self, line: Line) -> usize {
        (self.journal.len() as i32 + self.grid.history_size() as i32 + line.0).max(0) as usize
    }
}

impl Buffer for Lines<'_> {
    fn cols(&self) -> usize {
        Dimensions::columns(self.grid)
    }

    fn rows(&self) -> usize {
        Dimensions::screen_lines(self.grid)
    }

    fn above(&self) -> usize {
        self.journal.len() + self.grid.history_size()
    }

    fn view_offset(&self) -> usize {
        self.grid.display_offset() + self.journal.view()
    }

    fn row(&self, line: usize) -> Cow<'_, Row<Cell>> {
        match self.grid_line(line) {
            Some(line) => Cow::Borrowed(&self.grid[line]),
            None => Cow::Owned(self.journal.row(line, self.cols())),
        }
    }

    fn wraps(&self, line: usize) -> bool {
        match self.grid_line(line) {
            Some(line) => {
                let columns = self.cols();
                columns > 0
                    && self.grid[line][Column(columns - 1)]
                        .flags
                        .contains(Flags::WRAPLINE)
            }
            None => self.journal.wraps(line),
        }
    }
}

/// Scrolls the view of the grid and the journal together: the view's
/// offset from the bottom covers the grid's scrollback first, then the
/// journal's lines (the grid then shows its oldest line at the top).
pub fn scroll<T: EventListener>(term: &mut Term<T>, journal: &mut Journal, scroll: Scroll) {
    let grid = term.grid();
    let history = grid.history_size();
    let current = grid.display_offset();
    let offset = current + journal.view;
    let most = history + journal.len();
    let page = Dimensions::screen_lines(grid);
    let target = match scroll {
        Scroll::Delta(lines) => (offset as i64 + i64::from(lines)).clamp(0, most as i64) as usize,
        Scroll::PageUp => (offset + page).min(most),
        Scroll::PageDown => offset.saturating_sub(page),
        Scroll::Top => most,
        Scroll::Bottom => 0,
    };
    let in_grid = target.min(history);
    if in_grid != current {
        term.scroll_display(Scroll::Delta(in_grid as i32 - current as i32));
    }
    journal.view = target - in_grid;
}

/// Moves the scrollback rows of `term` over [`LIVE_HISTORY`] into
/// `journal` (oldest first) and keeps the view on the same text; returns
/// the lines moved. `limit` is the scrollback of the whole buffer: the grid
/// may grow again by what the journal does not hold.
pub fn compact<T: EventListener>(term: &mut Term<T>, journal: &mut Journal, limit: usize) -> usize {
    let grid = term.grid();
    let history = grid.history_size();
    if history <= LIVE_HISTORY {
        return 0;
    }
    let moving = history - LIVE_HISTORY;
    let columns = grid.columns();
    let offset = grid.display_offset();
    for line in 0..moving {
        let row = &grid[Line(-(history as i32) + line as i32)];
        journal.push(row, columns);
    }
    // A selection that reaches into the moved rows ends there (its text is
    // in the scrollback and stays as it is).
    if term
        .selection
        .as_ref()
        .and_then(|selection| selection.to_range(term))
        .is_some_and(|range| range.start.line.0 < -(LIVE_HISTORY as i32))
    {
        term.selection = None;
    }
    let grid = term.grid_mut();
    grid.update_history(LIVE_HISTORY);
    grid.update_history(limit.saturating_sub(journal.len()).max(LIVE_HISTORY));
    // The rows of the view that moved are now above the grid's top.
    if offset > LIVE_HISTORY {
        journal.view += offset - LIVE_HISTORY;
    }
    moving
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::terminal::model::GridDims;

    fn row_text(row: &Row<Cell>) -> String {
        let mut text = String::new();
        for column in 0..row.len() {
            let cell = &row[Column(column)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra);
            }
        }
        text.trim_end().to_owned()
    }

    fn term(cols: usize, output: &str) -> Term<VoidListener> {
        let mut term = Term::new(
            Config {
                scrolling_history: 10_000,
                ..Config::default()
            },
            &GridDims { cols, rows: 3 },
            VoidListener,
        );
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, output.as_bytes());
        term
    }

    #[test]
    fn a_row_comes_back_with_its_text_colors_and_marks() {
        let term = term(
            20,
            "\x1b[1;31mred\x1b[0m e\u{301} 你好 \x1b[44m  \x1b[0m\r\n\r\n\r\n",
        );
        let grid = term.grid();
        let original = &grid[Line(-1)];
        let mut journal = Journal::default();
        journal.push(original, 20);
        let back = journal.row(0, 20);
        assert_eq!(row_text(&back), "red e\u{301} 你好");
        for column in 0..20 {
            let (a, b) = (&original[Column(column)], &back[Column(column)]);
            assert_eq!(
                (a.c, a.fg, a.bg, a.flags),
                (b.c, b.fg, b.bg, b.flags),
                "{column}"
            );
            assert_eq!(a.zerowidth(), b.zerowidth(), "{column}");
        }
        // The blue blanks are kept: they are not blanks of a new row.
        assert!(journal.bytes() < 400, "{}", journal.bytes());
    }

    #[test]
    fn a_wrapped_row_keeps_its_wrap_and_other_widths_cut_or_pad() {
        let term = term(10, &format!("{}\r\n\r\n\r\n", "abcdefghij".repeat(2)));
        let grid = term.grid();
        let mut journal = Journal::default();
        // The first row wrapped into the second.
        journal.push(&grid[Line(-2)], 10);
        journal.push(&grid[Line(-1)], 10);
        assert!(journal.wraps(0));
        assert!(!journal.wraps(1));
        let narrow = journal.row(0, 6);
        assert_eq!(row_text(&narrow), "abcdef");
        assert!(narrow[Column(5)].flags.contains(Flags::WRAPLINE));
        let wide = journal.row(1, 14);
        assert_eq!(row_text(&wide), "abcdefghij");
        assert!(!wide[Column(13)].flags.contains(Flags::WRAPLINE));
    }

    #[test]
    fn ten_thousand_shell_lines_take_a_few_megabytes() {
        // Typical shell output in a wide window: a colored prompt, a
        // command, a listing line.
        let mut output = String::new();
        for n in 0..3_400 {
            output.push_str(&format!(
                "\x1b[32mdev@web-01\x1b[0m:\x1b[34m~/srv/app\x1b[0m$ tail -n 20 /var/log/app/{n}.log\r\n"
            ));
            output.push_str(&format!(
                "2026-10-22T10:{:02}:{:02}Z INFO request id={n} path=/api/v1/items status=200 took=12ms\r\n",
                n % 60,
                n % 60
            ));
            output.push_str(
                "-rw-r--r-- 1 dev dev   4096 Oct 22 10:00 \x1b[01;34mreleases\x1b[0m\r\n",
            );
        }
        let mut term = term(200, &output);
        let mut journal = Journal::default();
        let history = term.grid().history_size();
        assert!(history >= 10_000, "{history}");
        let moved = compact(&mut term, &mut journal, 10_000);
        assert_eq!(moved, history - LIVE_HISTORY);
        let grid_bytes =
            (term.grid().history_size() + term.grid().screen_lines()) * 200 * size_of::<Cell>();
        let total = grid_bytes + journal.bytes();
        assert!(total <= 5 * 1024 * 1024, "{total} bytes");
        // Before: every line a full row.
        assert!(history * 200 * size_of::<Cell>() > 40 * 1024 * 1024);
    }

    #[test]
    fn compaction_keeps_the_view_on_its_text() {
        let mut output = String::new();
        for n in 0..1_000 {
            output.push_str(&format!("line {n}\r\n"));
        }
        let mut term = term(20, &output);
        let history = term.grid().history_size();
        // The view shows the line 700 rows above the screen.
        term.scroll_display(alacritty_terminal::grid::Scroll::Delta(700));
        let mut journal = Journal::default();
        compact(&mut term, &mut journal, 10_000);
        assert_eq!(term.grid().history_size(), LIVE_HISTORY);
        assert_eq!(term.grid().display_offset(), LIVE_HISTORY);
        assert_eq!(journal.view(), 200);
        // The top row of the view: journal line len - view.
        let top = journal.len() - journal.view();
        assert_eq!(
            row_text(&journal.row(top, 20)),
            format!("line {}", history - 700)
        );
    }
}
