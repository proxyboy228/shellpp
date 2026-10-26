//! Putting the cursor on a tapped character (`docs/mobile/ui-spec.md` 9.3,
//! architecture 6.3): a click where the program asked for mouse reports,
//! else a walk with the arrow keys with feedback from the model.

use std::time::{Duration, Instant};

use shellpp_terminal::TerminalHandle;

use crate::input::{self, Cell, Key};
use crate::screen;

/// Arrows sent before the walk looks at the cursor again, once the first
/// arrow has moved it (the first step is a single arrow: a pager, which
/// scrolls with arrows, gets one).
const BATCH: u16 = 8;
/// How often the walk reads the cursor while it waits for the program.
const POLL: Duration = Duration::from_millis(10);
/// Time for the rest of a batch's echo once the cursor started moving.
const SETTLE: Duration = Duration::from_millis(15);

/// Cells between `a` and `b` in reading order.
fn cells(a: Cell, b: Cell, cols: u16) -> u64 {
    let index = |cell: Cell| i64::from(cell.row) * i64::from(cols) + i64::from(cell.col);
    (index(a) - index(b)).unsigned_abs()
}

/// How far the cursor is from `target`, in the arrows the walk would send:
/// along the typed line, the cells between; on the alternate screen, the
/// rows plus the columns.
fn distance(cursor: Cell, target: Cell, cols: u16, alternate: bool) -> u64 {
    if alternate {
        u64::from(cursor.row.abs_diff(target.row)) + u64::from(cursor.col.abs_diff(target.col))
    } else {
        cells(cursor, target, cols)
    }
}

/// Puts the cursor on `target`: a click where the program asked for mouse
/// reports; otherwise one arrow, then a few at a time, waiting after each step
/// until the program has moved its cursor (at most `step_wait`) and reading
/// where it went, until it is on `target`, an arrow no longer moves it or
/// moves it no closer (a program that redraws on its own), or the arrows
/// sent pass the distance between the two. On the shell's screen
/// only along the typed line (the cursor's row and the rows it wraps onto);
/// on the alternate screen rows first, then columns. Blocks the calling
/// thread; `true` when the cursor ended on `target` or the click was sent.
pub fn put(terminal: &TerminalHandle, target: Cell, step_wait: Duration) -> bool {
    let modes = screen::modes(terminal);
    if let Some(bytes) = input::click(target, modes, false, false) {
        terminal.send_input(bytes);
        return true;
    }
    let alternate = screen::alternate_screen(terminal);
    let Some((start, cols, wraps)) = screen::cursor_and_wraps(terminal) else {
        return false;
    };
    if !alternate && input::walk_to(start, target, cols, &wraps, modes).is_none() {
        return false;
    }
    let budget = distance(start, target, cols, alternate) + u64::from(BATCH);
    let origin = Cell { col: 0, row: 0 };
    let mut sent = 0u64;
    let mut batch = 1u16;
    let mut cursor = start;
    while cursor != target && sent <= budget {
        let away = distance(cursor, target, cols, alternate);
        let (key, count) = if alternate && cursor.row != target.row {
            let key = if target.row < cursor.row {
                Key::Up
            } else {
                Key::Down
            };
            (key, cursor.row.abs_diff(target.row))
        } else if alternate {
            let key = if target.col < cursor.col {
                Key::Left
            } else {
                Key::Right
            };
            (key, cursor.col.abs_diff(target.col))
        } else {
            let key = if cells(target, origin, cols) < cells(cursor, origin, cols) {
                Key::Left
            } else {
                Key::Right
            };
            (
                key,
                u16::try_from(cells(cursor, target, cols)).unwrap_or(u16::MAX),
            )
        };
        let count = count.clamp(1, batch);
        let arrow = input::key_bytes(
            key,
            modes,
            false,
            false,
            shellpp_terminal::keys::BackspaceKey::Del,
        );
        terminal.send_typed(arrow.repeat(usize::from(count)));
        sent += u64::from(count);
        let before = cursor;
        let deadline = Instant::now() + step_wait;
        loop {
            std::thread::sleep(POLL);
            match screen::cursor_and_wraps(terminal) {
                Some((now, _, _)) if now != before => {
                    std::thread::sleep(SETTLE);
                    cursor =
                        screen::cursor_and_wraps(terminal).map_or(now, |(settled, _, _)| settled);
                    batch = BATCH;
                    break;
                }
                // Scrolled back meanwhile: the screen no longer shows the cursor.
                None => return false,
                _ if Instant::now() >= deadline => return cursor == target,
                _ => {}
            }
        }
        if distance(cursor, target, cols, alternate) >= away {
            // It moved, but no closer: not our arrows (a program that
            // redraws on its own), so the walk stops.
            return false;
        }
    }
    cursor == target
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use alacritty_terminal::vte::ansi::CursorShape;
    use shellpp_terminal::TerminalSink;
    use shellpp_terminal::model::{ModelOptions, TerminalSize};

    use super::*;

    /// A program that moves its cursor one cell per arrow and puts it there
    /// itself (a line editor across its wrapped rows, or an editor on the
    /// alternate screen); `moves == false` is a program whose cursor stays
    /// (a pager). Counts the escape sequences it gets.
    struct Program {
        terminal: Mutex<Option<TerminalHandle>>,
        moves: bool,
        arrows: AtomicUsize,
    }

    impl TerminalSink for Program {
        fn send(&self, bytes: Vec<u8>) {
            let keys: Vec<u8> = bytes
                .windows(3)
                .filter(|seq| seq[0] == 0x1b && seq[1] == b'[')
                .map(|seq| seq[2])
                .collect();
            self.arrows.fetch_add(keys.len(), Ordering::SeqCst);
            let guard = self.terminal.lock().unwrap();
            let (Some(terminal), true) = (guard.as_ref(), self.moves) else {
                return;
            };
            let (mut at, cols, _) = screen::cursor_and_wraps(terminal).unwrap();
            let cols = i32::from(cols);
            for key in keys {
                let index = i32::from(at.row) * cols + i32::from(at.col);
                let index = match key {
                    b'D' => index - 1,
                    b'C' => index + 1,
                    b'A' => index - cols,
                    b'B' => index + cols,
                    _ => index,
                }
                .max(0);
                at = Cell {
                    col: u16::try_from(index % cols).unwrap(),
                    row: u16::try_from(index / cols).unwrap(),
                };
            }
            let place = format!("\x1b[{};{}H", at.row + 1, at.col + 1);
            terminal.feed_blocking(place.as_bytes()).unwrap();
        }

        fn resize(&self, _size: TerminalSize) {}
    }

    fn terminal(moves: bool) -> (TerminalHandle, Arc<Program>, async_channel::Receiver<()>) {
        let (wake, woken) = async_channel::bounded(16);
        let terminal = TerminalHandle::spawn(
            ModelOptions {
                size: TerminalSize::cells(20, 6),
                scrollback: 100,
                cursor_shape: CursorShape::Block,
            },
            crate::theme::palette(true),
            wake,
        );
        let program = Arc::new(Program {
            terminal: Mutex::new(Some(terminal.clone())),
            moves,
            arrows: AtomicUsize::new(0),
        });
        terminal.set_sink(Some(program.clone()));
        (terminal, program, woken)
    }

    fn cursor(terminal: &TerminalHandle) -> Cell {
        screen::cursor_and_wraps(terminal).unwrap().0
    }

    const WAIT: Duration = Duration::from_secs(2);

    #[test]
    fn a_tap_on_the_typed_line_walks_back_across_a_wrap() {
        let (terminal, program, _woken) = terminal(true);
        // "$ " and a command that wraps from row 0 onto row 1.
        terminal.feed_blocking(b"$ echo 0123456789abcdef").unwrap();
        terminal.flush_blocking();
        assert_eq!(cursor(&terminal), Cell { col: 3, row: 1 });
        assert!(put(&terminal, Cell { col: 7, row: 0 }, WAIT));
        assert_eq!(cursor(&terminal), Cell { col: 7, row: 0 });
        // 16 cells back: one arrow, then 8, then the last 7.
        assert_eq!(program.arrows.load(Ordering::SeqCst), 16);
        // A row that is not the typed line: nothing is sent.
        terminal.feed_blocking(b"\r\n\r\n$ ").unwrap();
        terminal.flush_blocking();
        let before = program.arrows.load(Ordering::SeqCst);
        assert!(!put(&terminal, Cell { col: 5, row: 1 }, WAIT));
        assert_eq!(program.arrows.load(Ordering::SeqCst), before);
        program.terminal.lock().unwrap().take();
    }

    #[test]
    fn a_tap_in_a_full_screen_program_walks_rows_then_columns() {
        let (terminal, program, _woken) = terminal(true);
        terminal.feed_blocking(b"\x1b[?1049h\x1b[H").unwrap();
        terminal.flush_blocking();
        assert!(put(&terminal, Cell { col: 12, row: 4 }, WAIT));
        assert_eq!(cursor(&terminal), Cell { col: 12, row: 4 });
        assert!(put(&terminal, Cell { col: 2, row: 1 }, WAIT));
        assert_eq!(cursor(&terminal), Cell { col: 2, row: 1 });
        program.terminal.lock().unwrap().take();
    }

    #[test]
    fn a_program_that_does_not_move_its_cursor_gets_one_arrow() {
        let (terminal, program, _woken) = terminal(false);
        terminal.feed_blocking(b"\x1b[?1049h\x1b[H").unwrap();
        terminal.flush_blocking();
        assert!(!put(
            &terminal,
            Cell { col: 0, row: 5 },
            Duration::from_millis(200)
        ));
        assert_eq!(program.arrows.load(Ordering::SeqCst), 1);
        program.terminal.lock().unwrap().take();
    }

    /// A program that moves its cursor somewhere else on every key (one
    /// that redraws on its own) gets two steps, not the screen's worth.
    #[test]
    fn a_walk_stops_when_the_cursor_moves_no_closer() {
        let (terminal, program, _woken) = terminal(false);
        terminal.feed_blocking(b"\x1b[?1049h\x1b[1;1H").unwrap();
        terminal.flush_blocking();
        let other = terminal.clone();
        let wanderer = std::thread::spawn(move || {
            for row in 2..40 {
                std::thread::sleep(Duration::from_millis(30));
                let place = format!("\x1b[{};1H", 1 + row % 2);
                let _ = other.feed_blocking(place.as_bytes());
            }
        });
        assert!(!put(
            &terminal,
            Cell { col: 15, row: 5 },
            Duration::from_millis(300)
        ));
        assert!(program.arrows.load(Ordering::SeqCst) <= 1 + BATCH as usize);
        wanderer.join().unwrap();
        program.terminal.lock().unwrap().take();
    }

    #[test]
    fn a_tap_clicks_where_the_program_takes_the_mouse() {
        let (terminal, program, _woken) = terminal(false);
        terminal
            .feed_blocking(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h")
            .unwrap();
        terminal.flush_blocking();
        assert!(put(&terminal, Cell { col: 3, row: 2 }, WAIT));
        // The click's press and release, no arrows.
        assert_eq!(program.arrows.load(Ordering::SeqCst), 2);
        program.terminal.lock().unwrap().take();
    }
}
