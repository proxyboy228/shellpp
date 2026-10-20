//! Selecting text with the mouse (UI 4.3): drag selects characters, a
//! double click a word (alacritty's semantic selection, bounded by spaces
//! and `,│`|:"' ()[]{}<>`), a triple click a line (a wrapped line as a
//! whole). Ctrl+Shift+A selects the screen and the scrollback. The selection
//! lives in the emulator, so it moves with the text when output scrolls.

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};

/// The selection kind of a click.
pub fn selection_type(click_count: usize) -> SelectionType {
    match click_count {
        0 | 1 => SelectionType::Simple,
        2 => SelectionType::Semantic,
        _ => SelectionType::Lines,
    }
}

/// Starts a selection at `point` (a click). A single click starts an empty
/// selection that a drag extends; double and triple clicks select at once.
pub fn start<T: EventListener>(term: &mut Term<T>, point: Point, side: Side, click_count: usize) {
    term.selection = Some(Selection::new(selection_type(click_count), point, side));
}

/// Extends the selection to `point` (a drag).
pub fn extend<T: EventListener>(term: &mut Term<T>, point: Point, side: Side) {
    if let Some(selection) = term.selection.as_mut() {
        selection.update(point, side);
    }
}

/// Drops an empty selection (a click without a drag); whether one remains.
pub fn finish<T: EventListener>(term: &mut Term<T>) -> bool {
    if term.selection.as_ref().is_some_and(Selection::is_empty) {
        term.selection = None;
    }
    term.selection.is_some()
}

/// Selects everything: the scrollback and the screen.
pub fn select_all<T: EventListener>(term: &mut Term<T>) {
    let start = Point::new(term.topmost_line(), Column(0));
    let end = Point::new(term.bottommost_line(), term.last_column());
    let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
    selection.update(end, Side::Right);
    term.selection = Some(selection);
}

/// The selected text (trailing blanks of each line removed by alacritty).
/// A line selection leaves out its final line break, so pasting a copied
/// line does not run it at once.
pub fn text<T: EventListener>(term: &Term<T>) -> Option<String> {
    let lines = term
        .selection
        .as_ref()
        .is_some_and(|selection| selection.ty == SelectionType::Lines);
    term.selection_to_string()
        .map(|text| {
            if lines {
                text.strip_suffix('\n').map(str::to_owned).unwrap_or(text)
            } else {
                text
            }
        })
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Line;
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

    fn at(line: i32, column: usize) -> Point {
        Point::new(Line(line), Column(column))
    }

    #[test]
    fn double_click_selects_a_word() {
        let mut term = term("ls /usr/local/bin (done)");
        start(&mut term, at(0, 7), Side::Left, 2);
        assert_eq!(text(&term).as_deref(), Some("/usr/local/bin"));
        start(&mut term, at(0, 20), Side::Left, 2);
        assert_eq!(text(&term).as_deref(), Some("done"));
    }

    #[test]
    fn triple_click_selects_the_line() {
        let mut term = term("first line\r\nsecond line here\r\nthird");
        start(&mut term, at(1, 3), Side::Left, 3);
        assert_eq!(text(&term).as_deref(), Some("second line here"));
    }

    #[test]
    fn a_wrapped_line_is_selected_as_a_whole() {
        // 40 characters wrap at 30 columns.
        let long = "abcdefghij".repeat(4);
        let mut term = term(&long);
        start(&mut term, at(1, 2), Side::Left, 3);
        assert_eq!(text(&term), Some(long));
    }

    #[test]
    fn drag_selects_characters_and_a_click_selects_nothing() {
        let mut term = term("hello world");
        start(&mut term, at(0, 0), Side::Left, 1);
        assert!(!finish(&mut term), "a click alone");
        start(&mut term, at(0, 0), Side::Left, 1);
        extend(&mut term, at(0, 4), Side::Right);
        assert!(finish(&mut term));
        assert_eq!(text(&term).as_deref(), Some("hello"));
    }

    #[test]
    fn select_all_includes_the_scrollback() {
        let mut lines = String::new();
        for n in 0..12 {
            lines.push_str(&format!("line {n}\r\n"));
        }
        let mut term = term(&lines);
        select_all(&mut term);
        let all = text(&term).unwrap();
        assert!(all.starts_with("line 0\n"), "{all:?}");
        assert!(all.contains("line 11"));
    }

    #[test]
    fn wide_characters_select_whole() {
        let mut term = term("ab 你好 cd");
        // Column 4 is the second half of 你.
        start(&mut term, at(0, 4), Side::Left, 2);
        assert_eq!(text(&term).as_deref(), Some("你好"));
        start(&mut term, at(0, 3), Side::Left, 1);
        extend(&mut term, at(0, 4), Side::Right);
        assert_eq!(text(&term).as_deref(), Some("你"));
    }
}
