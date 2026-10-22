//! Headless tests of the terminal view (GPUI `TestAppContext`, the test
//! platform's clipboard and text system; no OS clipboard, no real fonts, no
//! shell). The view sits in a host view with the app's `Shell` key context
//! and bindings, so key routing is tested against the real conflicts.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ClipboardItem, Context, Entity, Focusable as _,
    InputEvent as _, InteractiveElement as _, IntoElement, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Render, ScrollDelta, Styled as _,
    Subscription, TestAppContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use gpui_kit::test::TestWindowExt as _;

use super::*;
use crate::settings::terminal::TerminalPrefs;
use crate::shell::actions::{
    self as shell_actions, ActivateTab, CloseTab, NewLocalTerminal, NextTab, PreviousTab,
    ShowVaults,
};

/// Records what the terminal sends.
#[derive(Default)]
struct Sink {
    sent: Mutex<Vec<u8>>,
    sizes: Mutex<Vec<TerminalSize>>,
}

impl Sink {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

impl TerminalSink for Sink {
    fn send(&self, bytes: Vec<u8>) {
        self.sent.lock().unwrap().extend_from_slice(&bytes);
    }

    fn resize(&self, size: TerminalSize) {
        self.sizes.lock().unwrap().push(size);
    }
}

/// A window root with the shell's key context around the terminal.
struct Host {
    /// `None` once the test closed the "tab".
    terminal: Option<Entity<TerminalView>>,
    actions: Rc<RefCell<Vec<&'static str>>>,
    events: Rc<RefCell<Vec<TerminalEvent>>>,
    _subscription: Subscription,
}

impl Host {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(|cx| TerminalView::new(TerminalOptions::default(), window, cx));
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        let subscription = cx.subscribe(&terminal, move |_, _, event: &TerminalEvent, _| {
            recorded.borrow_mut().push(event.clone());
        });
        Self {
            terminal: Some(terminal),
            actions: Rc::default(),
            events,
            _subscription: subscription,
        }
    }
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let record = |name: &'static str, actions: &Rc<RefCell<Vec<&'static str>>>| {
            let actions = actions.clone();
            move || actions.borrow_mut().push(name)
        };
        let new_local = record("new-local-terminal", &self.actions);
        let close = record("close-tab", &self.actions);
        let activate = record("activate-tab", &self.actions);
        let next = record("next-tab", &self.actions);
        let previous = record("previous-tab", &self.actions);
        let vaults = record("show-vaults", &self.actions);
        div()
            .key_context(shell_actions::KEY_CONTEXT)
            .size_full()
            .on_action(move |_: &NewLocalTerminal, _, _| new_local())
            .on_action(move |_: &CloseTab, _, _| close())
            .on_action(move |_: &ActivateTab, _, _| activate())
            .on_action(move |_: &NextTab, _, _| next())
            .on_action(move |_: &PreviousTab, _, _| previous())
            .on_action(move |_: &ShowVaults, _, _| vaults())
            .children(self.terminal.clone())
    }
}

struct Fixture {
    window: AnyWindowHandle,
    host: Entity<Host>,
    terminal: Entity<TerminalView>,
    handle: TerminalHandle,
    sink: Arc<Sink>,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_sized(cx, 800., 500.)
    }

    fn open_sized(cx: &mut TestAppContext, width: f32, height: f32) -> Self {
        // The parser runs on its own thread and wakes the view from there.
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            shell_actions::bind_keys(cx);
            super::init(cx);
        });
        let (window, host) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(width), px(height)),
                ))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Host::new(window, cx)))
                .expect("test window")
        });
        let terminal = cx.update(|cx| host.read(cx).terminal.clone().expect("terminal"));
        let handle = cx.update(|cx| terminal.read(cx).handle());
        let sink = Arc::new(Sink::default());
        handle.set_sink(Some(sink.clone()));
        let fixture = Self {
            window,
            host,
            terminal,
            handle,
            sink,
        };
        fixture.with_window(cx, |_, window, _| window.activate_window());
        cx.run_until_parked();
        fixture.with_window(cx, |terminal, window, cx| {
            terminal.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
        });
        fixture
    }

    fn with_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&Entity<TerminalView>, &mut Window, &mut gpui::App) -> R,
    ) -> R {
        let terminal = self.terminal.clone();
        cx.update_window(self.window, |_, window, cx| f(&terminal, window, cx))
            .expect("window")
    }

    /// Feeds output, waits until it is parsed and draws a frame.
    fn feed(&self, cx: &mut TestAppContext, bytes: &[u8]) {
        self.handle.feed_blocking(bytes).unwrap();
        self.handle.flush_blocking();
        cx.run_until_parked();
        self.with_window(cx, |_, window, cx| window.render_frame(cx));
    }

    fn press(&self, cx: &mut TestAppContext, key: &str) {
        self.with_window(cx, |_, window, cx| window.press(key, cx));
    }

    fn actions(&self, cx: &mut TestAppContext) -> Vec<&'static str> {
        cx.update(|cx| self.host.read(cx).actions.borrow().clone())
    }

    fn events(&self, cx: &mut TestAppContext) -> Vec<TerminalEvent> {
        cx.update(|cx| self.host.read(cx).events.borrow().clone())
    }

    /// A point in the left half of a cell (a selection starts before it).
    fn cell(
        &self,
        cx: &mut TestAppContext,
        line: usize,
        column: usize,
    ) -> gpui::Point<gpui::Pixels> {
        cx.update(|cx| self.terminal.read(cx).cell_point(line, column, 0.25))
    }

    /// A point in the right half of a cell (a selection ends after it).
    fn cell_end(
        &self,
        cx: &mut TestAppContext,
        line: usize,
        column: usize,
    ) -> gpui::Point<gpui::Pixels> {
        cx.update(|cx| self.terminal.read(cx).cell_point(line, column, 0.75))
    }

    /// A mouse press and release at a cell.
    fn click(
        &self,
        cx: &mut TestAppContext,
        (line, column): (usize, usize),
        button: MouseButton,
        click_count: usize,
        modifiers: Modifiers,
    ) {
        let position = self.cell(cx, line, column);
        self.with_window(cx, |_, window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        });
    }

    fn drag(&self, cx: &mut TestAppContext, from: (usize, usize), to: (usize, usize)) {
        let from = self.cell(cx, from.0, from.1);
        let to = self.cell_end(cx, to.0, to.1);
        self.with_window(cx, |_, window, cx| window.drag(from, to, cx));
    }

    fn selected(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.terminal.read(cx).selected_text())
    }

    fn clipboard(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
    }
}

/// UI 12.5: Ctrl+letter without Shift goes to the shell even where the app
/// binds it (Ctrl+L = new local terminal; gpui-kit's Ctrl+C = copy); the app
/// keeps Ctrl+Shift shortcuts, Ctrl+1..9 and Alt+Left/Right.
#[gpui_kit::test]
fn ctrl_letters_go_to_the_shell_and_app_shortcuts_stay(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.press(cx, "ctrl-l");
    fixture.press(cx, "ctrl-p");
    fixture.press(cx, "ctrl-k");
    fixture.press(cx, "ctrl-c");
    assert_eq!(fixture.sink.take(), [0x0c, 0x10, 0x0b, 0x03]);
    fixture.press(cx, "ctrl-alt-s");
    assert_eq!(fixture.sink.take(), [0x1b, 0x13]);
    assert!(fixture.actions(cx).is_empty(), "no app action fired");

    fixture.press(cx, "ctrl-shift-w");
    fixture.press(cx, "ctrl-3");
    fixture.press(cx, "ctrl-1");
    fixture.press(cx, "alt-right");
    fixture.press(cx, "alt-left");
    assert_eq!(
        fixture.actions(cx),
        [
            "close-tab",
            "activate-tab",
            "show-vaults",
            "next-tab",
            "previous-tab"
        ]
    );
    assert!(fixture.sink.take().is_empty(), "app shortcuts send nothing");
}

#[gpui_kit::test]
fn keys_and_text_reach_the_shell(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    for key in [
        "tab",
        "shift-tab",
        "enter",
        "escape",
        "backspace",
        "up",
        "f5",
    ] {
        fixture.press(cx, key);
    }
    assert_eq!(
        fixture.sink.take(),
        b"\t\x1b[Z\r\x1b\x7f\x1b[A\x1b[15~".to_vec()
    );
    fixture.with_window(cx, |_, window, cx| window.input("lé 你好", cx));
    assert_eq!(fixture.sink.take(), "lé 你好".as_bytes());
    // Application cursor mode changes the arrows.
    fixture.feed(cx, b"\x1b[?1h");
    fixture.press(cx, "up");
    assert_eq!(fixture.sink.take(), b"\x1bOA");
}

/// UI 4.3: one line pastes at once; a line break asks first; bracketed
/// paste wraps the text and markers inside it are removed.
#[gpui_kit::test]
fn paste_confirmation_and_bracketed_paste(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("ls -la".to_owned())));
    fixture.press(cx, "ctrl-shift-v");
    assert_eq!(fixture.sink.take(), b"ls -la");

    // The right button pastes too.
    fixture.click(cx, (0, 0), MouseButton::Right, 1, Modifiers::default());
    assert_eq!(fixture.sink.take(), b"ls -la");

    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(
            "echo a\x1b[201~\x07\necho b\n".to_owned(),
        ));
    });
    fixture.press(cx, "ctrl-shift-v");
    assert!(
        fixture.sink.take().is_empty(),
        "nothing before the confirmation"
    );
    let pending = fixture
        .events(cx)
        .into_iter()
        .find_map(|event| match event {
            TerminalEvent::ConfirmPaste(pending) => Some(pending),
            _ => None,
        })
        .expect("a confirmation request");
    assert_eq!(pending.line_count(), 2);
    assert_eq!(pending.text(), "echo a\necho b\n");
    assert_eq!(pending.preview().lines, ["echo a", "echo b"]);

    // The program turns bracketed paste on before the user confirms.
    fixture.feed(cx, b"\x1b[?2004h");
    fixture.with_window(cx, |terminal, window, cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.paste_confirmed(&pending, window, cx)
        });
    });
    assert_eq!(
        fixture.sink.take(),
        b"\x1b[200~echo a\recho b\r\x1b[201~".to_vec()
    );
}

/// UI 4.3: double click selects a word, triple click a line, drag
/// characters; Ctrl+Shift+C copies; Ctrl+Shift+A selects everything.
#[gpui_kit::test]
fn mouse_selection_and_copy(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"hello world\r\nsecond line here");
    let none = Modifiers::default();

    fixture.click(cx, (0, 8), MouseButton::Left, 2, none);
    assert_eq!(fixture.selected(cx).as_deref(), Some("world"));
    fixture.click(cx, (1, 2), MouseButton::Left, 3, none);
    assert_eq!(fixture.selected(cx).as_deref(), Some("second line here"));
    fixture.drag(cx, (0, 0), (0, 4));
    assert_eq!(fixture.selected(cx).as_deref(), Some("hello"));
    assert_eq!(fixture.clipboard(cx), None, "copy on select is off");

    fixture.press(cx, "ctrl-shift-c");
    assert_eq!(fixture.clipboard(cx).as_deref(), Some("hello"));
    assert!(fixture.sink.take().is_empty(), "copy sends nothing");

    fixture.press(cx, "ctrl-shift-a");
    let all = fixture.selected(cx).unwrap();
    assert!(all.starts_with("hello world\nsecond line here"), "{all:?}");

    // A click without a drag clears the selection.
    fixture.click(cx, (0, 1), MouseButton::Left, 1, none);
    assert_eq!(fixture.selected(cx), None);
}

#[gpui_kit::test]
fn copy_on_select_follows_the_setting(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    cx.update(|cx| {
        cx.set_global(TerminalPrefs {
            copy_on_select: true,
            ..TerminalPrefs::default()
        });
    });
    fixture.feed(cx, b"copy me please");
    fixture.drag(cx, (0, 0), (0, 6));
    assert_eq!(fixture.clipboard(cx).as_deref(), Some("copy me"));
}

/// Mouse reports (SGR) in full-screen programs; Shift keeps the mouse for
/// selection.
#[gpui_kit::test]
fn mouse_reports_in_sgr_mode(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"\x1b[?1000h\x1b[?1006hsome text");
    fixture.click(cx, (2, 5), MouseButton::Left, 1, Modifiers::default());
    assert_eq!(fixture.sink.take(), b"\x1b[<0;6;3M\x1b[<0;6;3m".to_vec());
    fixture.click(cx, (0, 1), MouseButton::Right, 1, Modifiers::default());
    assert_eq!(fixture.sink.take(), b"\x1b[<2;2;1M\x1b[<2;2;1m".to_vec());

    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    fixture.click(cx, (0, 2), MouseButton::Left, 2, shift);
    assert!(fixture.sink.take().is_empty());
    assert_eq!(fixture.selected(cx).as_deref(), Some("some"));

    // The wheel is reported as buttons 64/65.
    fixture.with_window(cx, |_, window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., 1.)), cx);
    });
    let sent = String::from_utf8(fixture.sink.take()).unwrap();
    assert!(sent.starts_with("\x1b[<64;"), "{sent:?}");
}

/// The wheel and Shift+PgUp scroll the scrollback; typing returns to the
/// bottom; in the alternate screen the wheel sends arrow keys.
#[gpui_kit::test]
fn scrollback_wheel_and_keys(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let mut text = String::new();
    for line in 0..200 {
        text.push_str(&format!("line {line}\r\n"));
    }
    fixture.feed(cx, text.as_bytes());
    let offset =
        |cx: &mut TestAppContext| cx.update(|cx| fixture.terminal.read(cx).display_offset());
    assert_eq!(offset(cx), 0);

    fixture.with_window(cx, |_, window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., 3.)), cx);
    });
    assert_eq!(offset(cx), 3);
    assert!(fixture.sink.take().is_empty());

    fixture.press(cx, "shift-pageup");
    let page = fixture.handle.size().rows as usize - 1;
    assert_eq!(offset(cx), 3 + page);
    fixture.press(cx, "shift-pagedown");
    assert_eq!(offset(cx), 3);

    fixture.with_window(cx, |_, window, cx| window.input("x", cx));
    assert_eq!(offset(cx), 0, "typing scrolls to the bottom");
    assert_eq!(fixture.sink.take(), b"x");

    // vim, less: the alternate screen with alternate scroll.
    fixture.feed(cx, b"\x1b[?1049h");
    fixture.with_window(cx, |_, window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., -2.)), cx);
    });
    assert_eq!(fixture.sink.take(), b"\x1b[B\x1b[B".to_vec());
}

/// The grid follows the view's size and the sink hears about it.
#[gpui_kit::test]
fn resize_reaches_the_sink(cx: &mut TestAppContext) {
    let fixture = Fixture::open_sized(cx, 800., 500.);
    let first = fixture.handle.size();
    assert!(first.cols > 50 && first.rows > 20, "{first:?}");
    assert_eq!(fixture.sink.sizes.lock().unwrap().last(), Some(&first));
    cx.simulate_window_resize(fixture.window, size(px(400.), px(300.)));
    fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
    let second = fixture.handle.size();
    assert!(second.cols < first.cols && second.rows < first.rows);
    assert_eq!(fixture.sink.sizes.lock().unwrap().last(), Some(&second));
    assert!(second.pixel_width > 0 && second.pixel_height > 0);
    assert_eq!(fixture.handle.screen_text().len(), second.rows as usize);
}

#[gpui_kit::test]
fn title_and_bell_events(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"\x1b]2;build \x1b[31mlog\x07\x07");
    let events = fixture.events(cx);
    assert!(events.contains(&TerminalEvent::Bell), "{events:?}");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TerminalEvent::TitleChanged(Some(title)) if title.starts_with("build"))),
        "{events:?}"
    );
    let title = cx.update(|cx| fixture.terminal.read(cx).title()).unwrap();
    assert!(!title.contains('\x1b'));
}

/// Emoji sequences the shell counts over several cells (D90): a ZWJ
/// family, a skin tone modifier and a flag are each one segment at their
/// first cell (one glyph two cells wide); the cells keep the columns the
/// shell counts, so the text after them stays in place; two flags in a
/// row stay two.
#[gpui_kit::test]
fn emoji_sequences_are_one_glyph(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let thumb = "\u{1F44D}\u{1F3FD}";
    let flag = "\u{1F1FA}\u{1F1E6}";
    let line = format!("a{family}b{thumb}c{flag}{flag}d");
    fixture.feed(cx, line.as_bytes());
    let frame = fixture.with_window(cx, |terminal, window, cx| {
        let style = terminal.read(cx).frame_style(window);
        let layout = terminal.read(cx).layout_state();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(200.)));
        render::prepaint(
            bounds,
            &fixture.handle,
            &mut layout.borrow_mut(),
            &style,
            window,
        )
    });
    let segments = frame.segments(0);
    let texts: Vec<&str> = segments.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(texts, ["a", family, "b", thumb, "c", flag, flag, "d"]);
    // The family's cells: 2 + 2 + 2 columns, as wcwidth counts them.
    assert_eq!(segments[0].0, 0);
    assert_eq!(segments[1].0, 1);
    assert_eq!(segments[2].0, 7);
    assert_eq!(segments[3].0, 8);
    assert_eq!(segments[4].0, 12);
    let d = segments[7].0;
    assert_eq!(
        fixture.handle.screen_text()[0]
            .chars()
            .filter(|ch| *ch == 'd')
            .count(),
        1
    );
    assert!(d > segments[6].0);
}

/// Wide characters: one segment per wide character, two cells each; the
/// narrow text around them keeps its columns; the cursor follows.
#[gpui_kit::test]
fn wide_characters_take_two_cells(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, "ab你好🙂cd".as_bytes());
    assert_eq!(fixture.handle.screen_text()[0], "ab你好🙂cd");
    let frame = fixture.with_window(cx, |terminal, window, cx| {
        let style = terminal.read(cx).frame_style(window);
        let layout = terminal.read(cx).layout_state();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(200.)));
        render::prepaint(
            bounds,
            &fixture.handle,
            &mut layout.borrow_mut(),
            &style,
            window,
        )
    });
    assert_eq!(
        frame.segments(0),
        [
            (0, "ab".to_owned()),
            (2, "你".to_owned()),
            (4, "好".to_owned()),
            (6, "🙂".to_owned()),
            (8, "cd".to_owned()),
        ]
    );
    assert_eq!(frame.cursor(), Some((0, 10, false)));
    fixture.feed(cx, b"\r\x1b[2C");
    let frame = fixture.with_window(cx, |terminal, window, cx| {
        let style = terminal.read(cx).frame_style(window);
        let layout = terminal.read(cx).layout_state();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(200.)));
        render::prepaint(
            bounds,
            &fixture.handle,
            &mut layout.borrow_mut(),
            &style,
            window,
        )
    });
    assert_eq!(
        frame.cursor(),
        Some((0, 2, true)),
        "the cursor covers both cells"
    );
}

/// Colors and attributes become backgrounds and text runs; truecolor and
/// 256 colors pass through.
#[gpui_kit::test]
fn styled_cells_become_backgrounds_and_runs(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(
        cx,
        b"\x1b[41mred\x1b[0m \x1b[48;2;1;2;3mtc\x1b[0m \x1b[48;5;196mx\x1b[0m \x1b[1;4mbold\x1b[0m   ",
    );
    let frame = fixture.with_window(cx, |terminal, window, cx| {
        let style = terminal.read(cx).frame_style(window);
        let layout = terminal.read(cx).layout_state();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(200.)));
        render::prepaint(
            bounds,
            &fixture.handle,
            &mut layout.borrow_mut(),
            &style,
            window,
        )
    });
    assert_eq!(frame.backgrounds(0), [(0, 3), (4, 2), (7, 1)]);
    // One narrow segment; the trailing blanks are dropped.
    assert_eq!(frame.segments(0), [(0, "red tc x bold".to_owned())]);
}

/// Rows that did not change are not shaped again; scrolling output only
/// shapes the new row.
#[gpui_kit::test]
fn unchanged_rows_come_from_the_cache(cx: &mut TestAppContext) {
    let fixture = Fixture::open_sized(cx, 400., 200.);
    let rows = fixture.handle.size().rows as usize;
    let mut text = String::new();
    for line in 0..rows {
        text.push_str(&format!("row {line}\r\n"));
    }
    fixture.feed(cx, text.as_bytes());
    let shaped = |cx: &mut TestAppContext| cx.update(|cx| fixture.terminal.read(cx).shaped_rows());
    let before = shaped(cx);
    fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
    assert_eq!(shaped(cx), before, "a repaint shapes nothing");
    // One more line scrolls the screen by one row.
    fixture.feed(cx, b"one more\r\n");
    let after = shaped(cx);
    assert!(after - before <= 2, "shaped {} rows", after - before);
}

/// A cached row is reused only for the same cells: a row whose hash
/// collides with another row's is shaped from its own text.
#[gpui_kit::test]
fn a_hash_collision_does_not_show_another_rows_text(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"echo harmless\r\nrm -rf data");
    let prepaint = |cx: &mut TestAppContext| {
        fixture.with_window(cx, |terminal, window, cx| {
            let style = terminal.read(cx).frame_style(window);
            let layout = terminal.read(cx).layout_state();
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(200.)));
            render::prepaint(
                bounds,
                &fixture.handle,
                &mut layout.borrow_mut(),
                &style,
                window,
            )
        })
    };
    prepaint(cx);
    let layout = cx.update(|cx| fixture.terminal.read(cx).layout_state());
    layout.borrow_mut().forge_collision(0, 1);
    let shaped = layout.borrow().shaped_rows;
    let frame = prepaint(cx);
    assert_eq!(frame.segments(1), [(0, "rm -rf data".to_owned())]);
    assert_eq!(layout.borrow().shaped_rows, shaped + 1);
}

/// A flood of combining marks on one character: the frame draws at most
/// `MAX_ZEROWIDTH` of them and stays quick.
#[gpui_kit::test]
fn a_zero_width_flood_draws_quickly(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let flood = format!("a{}b", "\u{301}".repeat(512 * 1024));
    fixture.feed(cx, flood.as_bytes());
    let started = std::time::Instant::now();
    let frame = fixture.with_window(cx, |terminal, window, cx| {
        let style = terminal.read(cx).frame_style(window);
        let layout = terminal.read(cx).layout_state();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(200.)));
        render::prepaint(
            bounds,
            &fixture.handle,
            &mut layout.borrow_mut(),
            &style,
            window,
        )
    });
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let expected = format!("a{}b", "\u{301}".repeat(limits::MAX_ZEROWIDTH));
    assert_eq!(frame.segments(0), [(0, expected)]);
}

/// A selection drag from the bottom row to above the grid scrolls into the
/// scrollback (at once and then on a timer while the button stays down
/// there) and the selection takes the history lines with it.
#[gpui_kit::test]
fn a_selection_drag_above_the_grid_scrolls_into_history(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let mut text = String::new();
    for line in 0..200 {
        text.push_str(&format!("line {line}\r\n"));
    }
    fixture.feed(cx, text.as_bytes());
    let rows = fixture.handle.size().rows as usize;
    let start = fixture.cell(cx, rows - 2, 0);
    let line_height = cx.update(|cx| {
        let layout = fixture.terminal.read(cx).layout_state();
        layout.borrow().metrics.expect("a frame").line_height
    });
    let top = fixture.cell(cx, 0, 3);
    let above = point(top.x, top.y - line_height * 3.);
    let offset =
        |cx: &mut TestAppContext| cx.update(|cx| fixture.terminal.read(cx).display_offset());
    fixture.with_window(cx, |_, window, cx| {
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position: start,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseMoveEvent {
                position: above,
                pressed_button: Some(MouseButton::Left),
                modifiers: Modifiers::default(),
            }
            .to_platform_input(),
            cx,
        );
    });
    let first_step = offset(cx);
    assert!(first_step > 0, "the first step scrolls at once");
    // The button stays down above the grid: the timer keeps scrolling.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    let later = offset(cx);
    assert!(later > first_step, "{later} > {first_step}");
    fixture.with_window(cx, |_, window, cx| {
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Left,
                position: above,
                modifiers: Modifiers::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    });
    let selected = fixture.selected(cx).expect("a selection");
    let history_line = format!("line {}", 200 + 1 - rows - later);
    assert!(
        selected.lines().count() > rows,
        "more than a screen selected: {} lines",
        selected.lines().count()
    );
    // It reaches the top row of the scrolled view, from the pointer's
    // column (3) on.
    assert_eq!(selected.lines().next(), Some(&history_line[3..]));
    // The timer stopped with the button.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(offset(cx), later);
}

/// The app menu's Edit items (UI 1.4) act on a focused terminal: Paste goes
/// through the same cleaning and confirmation; Select all and Copy too.
#[gpui_kit::test]
fn edit_menu_actions_reach_a_focused_terminal(cx: &mut TestAppContext) {
    use gpui_kit::component::input as edit;

    let fixture = Fixture::open(cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("ls\x1b[201~ -la".into())));
    fixture.with_window(cx, |_, window, cx| {
        window.dispatch_action(Box::new(edit::Paste), cx);
    });
    assert_eq!(fixture.sink.take(), b"ls -la");

    fixture.feed(cx, b"menu copy");
    fixture.with_window(cx, |_, window, cx| {
        window.dispatch_action(Box::new(edit::SelectAll), cx);
        window.dispatch_action(Box::new(edit::Copy), cx);
    });
    let copied = fixture.clipboard(cx).expect("copied");
    assert!(copied.contains("menu copy"), "{copied:?}");

    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo".into())));
    fixture.with_window(cx, |_, window, cx| {
        window.dispatch_action(Box::new(edit::Paste), cx);
    });
    assert!(
        fixture.sink.take().is_empty(),
        "a multi-line paste asks first"
    );
    assert!(
        fixture
            .events(cx)
            .iter()
            .any(|event| matches!(event, TerminalEvent::ConfirmPaste(_)))
    );
}

/// Focus in/out reports (mode 1004).
#[gpui_kit::test]
fn focus_reports(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"\x1b[?1004h");
    fixture.with_window(cx, |_, window, cx| {
        window.blur(cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    assert_eq!(fixture.sink.take(), b"\x1b[O");
    fixture.with_window(cx, |terminal, window, cx| {
        terminal.read(cx).focus_handle(cx).focus(window, cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    assert_eq!(fixture.sink.take(), b"\x1b[I");
}

/// Mode 1002: a drag reports motion with the pressed button, once per cell.
#[gpui_kit::test]
fn drag_motion_reports(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"\x1b[?1002h\x1b[?1006h");
    fixture.drag(cx, (1, 1), (1, 3));
    let sent = String::from_utf8(fixture.sink.take()).unwrap();
    assert!(sent.starts_with("\x1b[<0;2;2M"), "{sent:?}");
    assert!(sent.contains("\x1b[<32;3;2M"), "{sent:?}");
    assert!(sent.contains("\x1b[<32;4;2M"), "{sent:?}");
    assert!(sent.ends_with("\x1b[<0;4;2m"), "{sent:?}");
    assert_eq!(sent.matches("\x1b[<32;3;2M").count(), 1, "{sent:?}");
    assert_eq!(
        fixture.selected(cx),
        None,
        "no selection while the program has the mouse"
    );
}

/// Closing the tab (dropping the view) closes the terminal: the session's
/// feeds fail and the sink is released.
#[gpui_kit::test]
fn dropping_the_view_closes_the_terminal(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"some output");
    assert!(!fixture.handle.is_closed());
    let Fixture {
        window,
        host,
        terminal,
        handle,
        sink,
    } = fixture;
    let weak = terminal.downgrade();
    drop(terminal);
    cx.update(|cx| {
        host.update(cx, |host, cx| {
            host.terminal = None;
            cx.notify();
        });
    });
    // The next frame replaces the previous frame's input handler and mouse
    // listeners, which hold the view.
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("window");
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the view was dropped");
    assert!(handle.is_closed());
    assert_eq!(handle.feed_blocking(b"late"), Err(FeedError::Closed));
    assert_eq!(Arc::strong_count(&sink), 1, "the sink was released");
}

/// Prepares a frame of the fixture's terminal in the text area of an
/// 800 x 500 view (8 px padding) and returns it.
fn frame(fixture: &Fixture, cx: &mut TestAppContext) -> render::Frame {
    fixture.with_window(cx, |terminal, window, cx| {
        let style = terminal.read(cx).frame_style(window);
        let layout = terminal.read(cx).layout_state();
        let bounds = Bounds::new(point(px(8.), px(8.)), size(px(784.), px(484.)));
        render::prepaint(
            bounds,
            &fixture.handle,
            &mut layout.borrow_mut(),
            &style,
            window,
        )
    })
}

/// UI 4.2 Tools: the search finds matches in the scrollback and on screen,
/// highlights the visible ones (the current one in its own color), steps
/// with wrap-around, scrolls the current match into view and reports
/// every change; clearing it removes the highlights.
#[gpui_kit::test]
fn search_highlights_steps_and_scrolls(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let updates = Rc::new(RefCell::new(0usize));
    let counted = updates.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&fixture.terminal, move |_, _: &SearchUpdated, _| {
            *counted.borrow_mut() += 1;
        })
    });
    let mut text = String::new();
    for line in 0..150 {
        if line == 5 || line == 140 {
            text.push_str(&format!("line {line} Needle here\r\n"));
        } else {
            text.push_str(&format!("line {line}\r\n"));
        }
    }
    fixture.feed(cx, text.as_bytes());
    assert!(fixture.handle.history_size() > 100);

    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("NEEDLE", cx));
    assert_eq!(
        summary,
        SearchSummary {
            count: 2,
            current: Some(2),
            truncated: false
        }
    );
    assert_eq!(*updates.borrow(), 1);
    let offset =
        |cx: &mut TestAppContext| cx.update(|cx| fixture.terminal.read(cx).display_offset());
    assert_eq!(offset(cx), 0, "the newest match is visible: no scrolling");
    let spans = frame(&fixture, cx).matches().to_vec();
    assert_eq!(spans.len(), 1);
    assert!(spans[0].current);
    assert_eq!((spans[0].first, spans[0].last), (9, 14));

    // Previous: the match in the scrollback becomes current and visible.
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search_step(false, cx));
    assert_eq!(summary.current, Some(1));
    assert!(offset(cx) > 0, "scrolled into the scrollback");
    let spans = frame(&fixture, cx).matches().to_vec();
    assert_eq!(spans.len(), 1);
    assert!(spans[0].current);
    assert_eq!((spans[0].first, spans[0].last), (7, 12));

    // Previous again wraps to the last match; next wraps to the first.
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search_step(false, cx));
    assert_eq!(summary.current, Some(2));
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search_step(true, cx));
    assert_eq!(summary.current, Some(1));
    assert_eq!(*updates.borrow(), 4);

    // Clearing removes the highlights.
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.clear_search(cx));
    assert!(frame(&fixture, cx).matches().is_empty());
    assert_eq!(
        cx.update(|cx| fixture.terminal.read(cx).search_summary()),
        None
    );
    // An empty query is no search.
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("", cx));
    assert_eq!(summary, SearchSummary::default());
}

/// New output while the search is open is searched again shortly after.
#[gpui_kit::test]
fn new_output_is_searched_again(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"error one\r\n");
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("error", cx));
    fixture.feed(cx, b"fine\r\nerror two\r\n");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    let summary = cx.update(|cx| fixture.terminal.read(cx).search_summary());
    assert_eq!(summary.map(|summary| summary.count), Some(2));
    // The current match stayed on the first one.
    assert_eq!(summary.and_then(|summary| summary.current), Some(1));
}

/// A narrower grid reflows wrapped lines: the search runs again at once,
/// without new output (the sidebar opening, a split, a window resize), so
/// the highlights stay on the text.
#[gpui_kit::test]
fn a_reflow_searches_again(cx: &mut TestAppContext) {
    let fixture = Fixture::open_sized(cx, 800., 500.);
    let wide = fixture.handle.size().cols as usize;
    assert!(wide > 60, "{wide}");
    let line = format!("{}NEEDLE\r\n", "a".repeat(50));
    fixture.feed(cx, line.as_bytes());
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("needle", cx));
    let start = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let terminal = fixture.terminal.read(cx);
            terminal.search_state().and_then(|search| search.current())
        })
        .map(|found| found.start)
    };
    let before = start(cx).expect("a match");
    assert_eq!(before.column, 50);

    cx.simulate_window_resize(fixture.window, size(px(300.), px(500.)));
    fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
    cx.run_until_parked();
    let narrow = fixture.handle.size().cols as usize;
    assert!(narrow < 50, "{narrow}");
    let after = start(cx).expect("still a match");
    // The match moved with the text to a later row.
    assert_eq!(
        (after.line - before.line, after.column),
        (50 / narrow, 50 % narrow)
    );
}

/// Gap 8: a reflow keeps the current match by its ordinal (three wrapped
/// matches, the last one current, stay "3 of 3"). A second resize within
/// the refresh period waits for the period (one search per period during a
/// drag), without highlights on the moved text meanwhile.
#[gpui_kit::test]
fn a_reflow_keeps_the_current_match(cx: &mut TestAppContext) {
    use alacritty_terminal::grid::Dimensions as _;

    let fixture = Fixture::open_sized(cx, 800., 500.);
    let line = format!("{}NEEDLE\r\n", "a".repeat(50));
    fixture.feed(cx, line.repeat(3).as_bytes());
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("needle", cx));
    assert_eq!((summary.count, summary.current), (3, Some(3)));
    let summary = |cx: &mut TestAppContext| {
        cx.update(|cx| fixture.terminal.read(cx).search_summary())
            .map(|summary| (summary.count, summary.current))
    };
    // Whether the current match is highlighted on the grid as it is.
    let highlighted = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let search = fixture.terminal.read(cx).search_state()?;
            let term = fixture.handle.term().lock();
            let grid = term.grid();
            let origin = fixture.handle.search_origin(&term);
            let spans = search.spans(grid, origin, grid.screen_lines(), grid.columns());
            Some(spans.iter().any(|span| span.current))
        })
    };
    assert_eq!(highlighted(cx), Some(true));
    let resize = |cx: &mut TestAppContext, width: f32| {
        cx.simulate_window_resize(fixture.window, size(px(width), px(500.)));
        fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
        cx.run_until_parked();
    };

    resize(cx, 300.);
    assert!(fixture.handle.size().cols < 50);
    assert_eq!(summary(cx), Some((3, Some(3))));
    assert_eq!(highlighted(cx), Some(true));

    resize(cx, 800.);
    assert_eq!(highlighted(cx), Some(false), "waits for the period");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(summary(cx), Some((3, Some(3))));
    assert_eq!(highlighted(cx), Some(true));
}

/// Gap 13: with a full scrollback and output going on, the oldest lines
/// drop off, and the current match stays on its text (its ordinal moves
/// down by the matches that dropped off).
#[gpui_kit::test]
fn the_current_match_keeps_its_text_in_a_full_scrollback(cx: &mut TestAppContext) {
    use alacritty_terminal::index::Column;

    let fixture = Fixture::open(cx);
    let lines = |range: std::ops::Range<usize>| -> String {
        range
            .map(|index| {
                if index % 50 == 0 {
                    format!("line {index:05} hit\r\n")
                } else {
                    format!("line {index:05}\r\n")
                }
            })
            .collect()
    };
    fixture.feed(cx, lines(0..10_500).as_bytes());
    assert_eq!(fixture.handle.history_size(), DEFAULT_SCROLLBACK);
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("hit", cx));
    // The text of the row the current match is on.
    let current_text = |cx: &mut TestAppContext| -> Option<String> {
        cx.update(|cx| {
            let found = fixture.terminal.read(cx).search_state()?.current()?;
            let term = fixture.handle.term().lock();
            let buffer = found
                .start
                .line
                .checked_sub(fixture.handle.evicted_lines())?;
            // The journal and the grid (D85).
            let text: String = fixture.handle.with_lines(&term, |lines| {
                use crate::terminal::journal::Buffer as _;
                let row = lines.row(buffer);
                (0..lines.cols())
                    .map(|column| row[Column(column)].c)
                    .collect()
            });
            Some(text.trim_end().to_owned())
        })
    };
    for _ in 0..1000 {
        if current_text(cx).as_deref() == Some("line 05000 hit") {
            break;
        }
        fixture
            .terminal
            .update(cx, |terminal, cx| terminal.search_step(false, cx));
    }
    assert_eq!(current_text(cx).as_deref(), Some("line 05000 hit"));
    let before = cx
        .update(|cx| fixture.terminal.read(cx).search_summary())
        .unwrap();

    fixture.feed(cx, lines(10_500..11_500).as_bytes());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(current_text(cx).as_deref(), Some("line 05000 hit"));
    let after = cx
        .update(|cx| fixture.terminal.read(cx).search_summary())
        .unwrap();
    assert_eq!(after.count, before.count);
    assert_eq!(after.current, before.current.map(|current| current - 20));
}

/// UI 1.1: a dropped session is dimmed and has no cursor; both come back.
#[gpui_kit::test]
fn a_dimmed_terminal_hides_the_cursor(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"$ ");
    assert!(frame(&fixture, cx).cursor().is_some());
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.set_dimmed(true, cx));
    assert!(cx.update(|cx| fixture.terminal.read(cx).is_dimmed()));
    assert_eq!(frame(&fixture, cx).cursor(), None);
    fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.set_dimmed(false, cx));
    assert!(frame(&fixture, cx).cursor().is_some());
}

/// Settings (or the Appearance tab) change the line spacing, the size and
/// the font family of open terminals at once.
#[gpui_kit::test]
fn font_size_spacing_and_family_apply_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::open_sized(cx, 800., 500.);
    let metrics = |cx: &mut TestAppContext| {
        let _ = frame(&fixture, cx);
        let layout = cx.update(|cx| fixture.terminal.read(cx).layout_state());
        layout.borrow().metrics.expect("a frame was drawn")
    };
    let before = metrics(cx);
    let rows_before = fixture.handle.size().rows;
    assert_eq!(before.line_height, px(17.), "14 px x 1.2, snapped");

    cx.update(|cx| {
        let prefs = TerminalPrefs {
            line_height: 1.5,
            ..TerminalPrefs::current(cx)
        };
        cx.set_global(prefs);
    });
    fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
    let spaced = metrics(cx);
    assert_eq!(spaced.line_height, px(21.), "14 px x 1.5");
    assert!(fixture.handle.size().rows < rows_before, "fewer rows fit");

    cx.update(|cx| {
        let prefs = TerminalPrefs {
            font_size: 20,
            font_family: "Courier New".to_owned(),
            ..TerminalPrefs::current(cx)
        };
        cx.set_global(prefs);
    });
    let larger = metrics(cx);
    assert_eq!(larger.font_size, px(20.));
    assert_eq!(larger.line_height, px(30.));
    let family = fixture.with_window(cx, |terminal, window, cx| {
        terminal
            .read(cx)
            .frame_style(window)
            .fonts
            .family()
            .to_owned()
    });
    assert_eq!(family, "Courier New");
}

/// Robustness (plan of run 3, section 10, item 4): output made of escape
/// sequences, mutated with a fixed seed (CSI, OSC titles and hyperlinks,
/// DCS, the alternate screen, wide and combining characters, keywords for
/// the highlighting), is parsed and drawn without a panic, and none of it
/// reaches the session as input except the terminal's own replies.
#[gpui_kit::test]
fn mutated_escape_sequences_never_panic(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    const PIECES: &[&str] = &[
        "\x1b[31m",
        "\x1b[0m",
        "\x1b[1;4;7m",
        "\x1b[38;2;1;2;3m",
        "\x1b[48;5;200m",
        "\x1b[2J",
        "\x1b[H",
        "\x1b[10;20H",
        "\x1b[999;999H",
        "\x1b[?1049h",
        "\x1b[?1049l",
        "\x1b[?25l",
        "\x1b[?2004h",
        "\x1b[K",
        "\x1b[3S",
        "\x1b[3T",
        "\x1b]0;title\x07",
        "\x1b]2;t\x1b\\",
        "\x1b]8;;http://example.com\x1b\\link\x1b]8;;\x1b\\",
        "\x1bP+q544e\x1b\\",
        "\x1b[6n",
        "\x1b[c",
        "\x1b(0lqk\x1b(B",
        "\r\n",
        "\t",
        "\x08",
        "\x07",
        "你好",
        "e\u{301}",
        "🙂",
        "error ",
        "warning ",
        "ok ",
        "192.168.1.1 ",
        "aa:bb:cc:dd:ee:ff ",
        "\x1b",
        "\x1b[",
        "\x1b]",
        "\u{9b}31m",
        "\u{202e}",
    ];
    let mut state = 20_261_021_u64;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..300 {
        let mut chunk = Vec::new();
        for _ in 0..1 + next() % 24 {
            match next() % 5 {
                0 => chunk.push(next() as u8),
                _ => chunk
                    .extend_from_slice(PIECES[(next() % PIECES.len() as u64) as usize].as_bytes()),
            }
        }
        fixture.feed(cx, &chunk);
    }
    // Replies (cursor position, device attributes) are the only bytes sent
    // back: never the output's own text.
    let sent = fixture.sink.take();
    assert!(sent.is_empty() || sent.starts_with(b"\x1b"));
    assert!(!sent.windows(7).any(|window| window == b"warning"));
}

/// D85: once output pauses, the scrollback beyond the newest lines moves
/// into the journal; the view scrolls, searches and selects over it as over
/// the grid, with its colors, keeps its place while output goes on, and a
/// cleared scrollback clears the journal too.
#[gpui_kit::test]
fn the_compact_scrollback_scrolls_searches_and_selects(cx: &mut TestAppContext) {
    use alacritty_terminal::grid::{Dimensions as _, Scroll};
    use alacritty_terminal::index::Column;
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    use crate::terminal::journal::{Buffer as _, LIVE_HISTORY};

    let fixture = Fixture::open(cx);
    let lines = |range: std::ops::Range<usize>| -> String {
        range
            .map(|index| format!("\x1b[31mline {index:05}\x1b[0m tail\r\n"))
            .collect()
    };
    fixture.feed(cx, lines(0..10_500).as_bytes());
    assert_eq!(fixture.handle.history_size(), DEFAULT_SCROLLBACK);
    let (journal, live) = {
        let term = fixture.handle.term().lock();
        let journal = fixture.handle.outbox().journal().len();
        (journal, term.grid().history_size())
    };
    assert_eq!(live, LIVE_HISTORY, "the parser was idle after the feed");
    assert_eq!(journal + live, DEFAULT_SCROLLBACK);
    assert!(fixture.handle.buffer_bytes() <= 5 * 1024 * 1024);

    // The top: the oldest line kept, red as it came, from the journal.
    let redraw = |cx: &mut TestAppContext| {
        fixture.with_window(cx, |_, window, cx| window.render_frame(cx));
    };
    fixture.handle.scroll(Scroll::Top);
    redraw(cx);
    let rows = fixture.handle.screen_text();
    let first = 10_500 + 1 - (DEFAULT_SCROLLBACK + rows.len());
    assert_eq!(rows[0], format!("line {first:05} tail"));
    {
        let term = fixture.handle.term().lock();
        fixture.handle.with_lines(&term, |lines| {
            let row = lines.row(0);
            assert_eq!(row[Column(0)].fg, Color::Named(NamedColor::Red));
            assert_eq!(row[Column(11)].fg, Color::Named(NamedColor::Foreground));
        });
    }

    // Search finds a line in the journal and scrolls to it.
    let wanted = format!("line {:05} tail", first + 100);
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search(&wanted, cx));
    assert_eq!(summary.count, 1);
    fixture.handle.scroll(Scroll::Bottom);
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search(&wanted, cx));
    redraw(cx);
    assert!(fixture.handle.screen_text().contains(&wanted));
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.clear_search(cx));

    // A selection across the end of the journal and the grid's top.
    let view_top = journal - 2;
    fixture.handle.scroll(Scroll::Bottom);
    fixture
        .handle
        .scroll(Scroll::Delta((DEFAULT_SCROLLBACK - view_top) as i32));
    redraw(cx);
    let rows = fixture.handle.screen_text();
    fixture.drag(cx, (0, 0), (3, 9));
    let expected = format!("{}\n{}\n{}\n{}", rows[0], rows[1], rows[2], &rows[3][..10]);
    assert_eq!(fixture.selected(cx), Some(expected.clone()));

    // More output: the view and the selection keep their text, also when
    // the rows on screen move into the journal; the buffer stays at its
    // limit, the oldest lines dropped and counted.
    let evicted = fixture.handle.evicted_lines();
    fixture.feed(cx, lines(10_500..10_800).as_bytes());
    assert_eq!(fixture.handle.screen_text()[0], rows[0]);
    assert_eq!(fixture.selected(cx), Some(expected));
    assert_eq!(fixture.handle.history_size(), DEFAULT_SCROLLBACK);
    assert_eq!(fixture.handle.evicted_lines(), evicted + 300);
    {
        let term = fixture.handle.term().lock();
        let journal = fixture.handle.outbox().journal();
        assert!(journal.view() > 0);
        assert_eq!(term.grid().display_offset(), term.grid().history_size());
    }

    // A resize keeps the view in the journal on the grid's top.
    fixture.handle.resize(TerminalSize::cells(60, 12));
    {
        let term = fixture.handle.term().lock();
        assert_eq!(term.grid().display_offset(), term.grid().history_size());
    }

    // The alternate screen has no journal: nothing of it is searched or
    // scrolled to there.
    fixture.feed(cx, b"\x1b[?1049h");
    let summary = fixture
        .terminal
        .update(cx, |terminal, cx| terminal.search("line", cx));
    assert_eq!(summary.count, 0);
    fixture.handle.scroll(Scroll::Top);
    assert_eq!(fixture.handle.view_offset(), 0);
    fixture
        .terminal
        .update(cx, |terminal, cx| terminal.clear_search(cx));
    // A resize there gives the alternate screen no scrollback, and nothing
    // of it reaches the journal.
    let journal = fixture.handle.outbox().journal().len();
    fixture.handle.resize(TerminalSize::cells(50, 10));
    fixture.feed(cx, lines(0..1_200).as_bytes());
    {
        let term = fixture.handle.term().lock();
        assert_eq!(term.grid().history_size(), 0);
    }
    assert_eq!(fixture.handle.outbox().journal().len(), journal);
    fixture.feed(cx, b"\x1b[?1049l");

    // RIS from the alternate screen clears the journal too.
    fixture.feed(cx, lines(0..2_000).as_bytes());
    assert!(!fixture.handle.outbox().journal().is_empty());
    fixture.feed(cx, b"\x1b[?1049h\x1bc");
    assert!(fixture.handle.outbox().journal().is_empty());
    fixture.feed(cx, lines(0..2_000).as_bytes());
    assert!(
        fixture.handle.history_size() >= 1_900,
        "the grid's cap is back"
    );

    // ED 3 clears the whole scrollback, the journal too.
    fixture.feed(cx, b"\x1b[3J");
    assert_eq!(fixture.handle.history_size(), 0);
    assert_eq!(fixture.handle.view_offset(), 0);
}

/// D85: the selection on the screen follows the emulator's copy of it:
/// output that clears it ends it, output that scrolls the screen into
/// the scrollback keeps it on its text, and text that scrolls inside the
/// screen (the alternate screen, a scroll region) takes it along.
#[gpui_kit::test]
fn a_selection_on_the_screen_follows_its_text(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.feed(cx, b"alpha one\r\nbravo two\r\ncharlie three");
    fixture.drag(cx, (1, 0), (1, 4));
    assert_eq!(fixture.selected(cx).as_deref(), Some("bravo"));
    // New lines scroll the screen: the selection stays on "bravo".
    let mut more = String::new();
    for n in 0..40 {
        more.push_str(&format!("\r\nmore {n}"));
    }
    fixture.feed(cx, more.as_bytes());
    assert_eq!(fixture.selected(cx).as_deref(), Some("bravo"));

    // Output that clears the selected text ends the selection (as
    // alacritty's own did; plain characters over it do not).
    fixture.feed(cx, b"\x1b[2J\x1b[Hdelta four\r\necho five");
    fixture.drag(cx, (1, 0), (1, 3));
    assert_eq!(fixture.selected(cx).as_deref(), Some("echo"));
    fixture.feed(cx, b"\x1b[2;1H\x1b[2Kxxxx");
    assert_eq!(fixture.selected(cx), None);

    // The alternate screen scrolls inside its screen: the selection moves.
    fixture.feed(cx, b"\x1b[?1049h\x1b[Hfirst\r\nsecond\r\nthird");
    fixture.drag(cx, (1, 0), (1, 5));
    assert_eq!(fixture.selected(cx).as_deref(), Some("second"));
    // Scroll up one line (SU): "second" is now on the first row.
    fixture.feed(cx, b"\x1b[S");
    assert_eq!(fixture.selected(cx).as_deref(), Some("second"));
    assert_eq!(fixture.handle.screen_text()[0], "second");
    fixture.feed(cx, b"\x1b[?1049l");
    assert_eq!(fixture.selected(cx), None, "the other screen");
}
