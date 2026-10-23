//! The terminal view: a GPUI entity that draws the emulator, takes keyboard,
//! mouse and clipboard input and reports title changes and the bell
//! (UI 4.1, 4.3, 12.4, 12.5). It also runs the search of the terminal
//! sidebar's Tools tab (UI 4.2; [`super::search`]) and shows a dropped
//! session dimmed without a cursor (UI 1.1).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::Side;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Bounds, ClipboardItem, Context, DispatchPhase, ElementInputHandler, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction,
    ParentElement as _, Pixels, Render, ScrollWheelEvent, SharedString, Styled as _, Subscription,
    Task, UTF16Selection, WeakEntity, Window, canvas, div, point, px, size,
};
use gpui_kit::TestSupportExt as _;
// The app menu's Edit items (UI 1.4) dispatch these to the focused element.
use gpui_kit::component::input as edit;

use super::colors::{Palette, rgb_token};
use super::fonts::{self, FontSet};
use super::input::{self, BackspaceKey, KeyAction};
use super::journal::Buffer as _;
use super::model::{ModelOptions, TerminalHandle, TerminalSize, term_config};
use super::mouse::{self, ReportButton, ReportKind};
use super::paste::{self, PastePreview};
use super::render::{self, FrameStyle, LayoutState};
use super::search::{BufferPoint, SearchState};
use super::selection::{self, Selection};
use super::settings::{CursorStyleSetting, DEFAULT_SCROLLBACK, TerminalSettings, settings};
use crate::settings::terminal::TerminalPrefs;
use crate::theme::metrics;
use crate::theme::terminal::TerminalTheme;

/// Key context of the terminal.
pub const KEY_CONTEXT: &str = "Terminal";

/// Space between the view's edges and the grid.
const PADDING: Pixels = gpui::px(8.);

/// How often a selection drag above or below the grid scrolls.
const AUTOSCROLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);
/// Most lines one autoscroll step moves (the pointer far from the grid).
const AUTOSCROLL_MAX_LINES: i32 = 10;
/// While a search is open, new output or a reflow searches again at most
/// this often.
const SEARCH_REFRESH: std::time::Duration = std::time::Duration::from_millis(250);

gpui_kit::actions!(
    terminal,
    [
        /// Ctrl+Shift+C: copy the selection.
        Copy,
        /// Ctrl+Shift+V: paste the clipboard.
        Paste,
        /// Ctrl+Shift+A: select the screen and the scrollback.
        SelectAll,
        /// Tab and Shift+Tab go to the shell, not to focus navigation.
        SendTab,
        SendBackTab,
    ]
);

/// Marks the key bindings as installed.
struct KeysBound;

impl gpui::Global for KeysBound {}

/// The terminal's key bindings (UI 12.5), once per app. Ctrl+letter and Ctrl+Alt+letter
/// without Shift are disabled for the app while the terminal has the focus,
/// so they reach the shell (Ctrl+L, Ctrl+P, Ctrl+K, Ctrl+Alt+S, ...), and so
/// are Ctrl+=, Ctrl+- and Ctrl+0 (the interface zoom outside it); the
/// app keeps Ctrl+1..9, `Ctrl+,`, Ctrl+Shift+..., Ctrl+Alt+arrows and
/// Alt+Left/Right.
pub fn bind_keys(cx: &mut App) {
    if cx.has_global::<KeysBound>() {
        return;
    }
    cx.set_global(KeysBound);
    let context = Some(KEY_CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("ctrl-shift-c", Copy, context),
        KeyBinding::new("ctrl-shift-v", Paste, context),
        KeyBinding::new("ctrl-shift-a", SelectAll, context),
        KeyBinding::new("tab", SendTab, context),
        KeyBinding::new("shift-tab", SendBackTab, context),
    ];
    if cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new("cmd-c", Copy, context));
        bindings.push(KeyBinding::new("cmd-v", Paste, context));
        bindings.push(KeyBinding::new("cmd-a", SelectAll, context));
    }
    // The interface zoom keys go to the program too (D96).
    for keys in ["ctrl-=", "ctrl--", "ctrl-0"] {
        bindings.push(KeyBinding::new(keys, NoAction, context));
    }
    for letter in 'a'..='z' {
        bindings.push(KeyBinding::new(
            &format!("ctrl-{letter}"),
            NoAction,
            context,
        ));
        bindings.push(KeyBinding::new(
            &format!("ctrl-alt-{letter}"),
            NoAction,
            context,
        ));
    }
    cx.bind_keys(bindings);
}

/// Options of a new terminal view.
#[derive(Clone, Debug)]
pub struct TerminalOptions {
    /// The grid size until the first layout (then the view's own size).
    pub size: TerminalSize,
    /// Lines kept above the screen.
    pub scrollback: usize,
    /// The host's theme; `None` uses the default theme of the settings.
    pub theme: Option<TerminalTheme>,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            size: TerminalSize::default(),
            scrollback: DEFAULT_SCROLLBACK,
            theme: None,
        }
    }
}

/// What a terminal view reports to its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// The program set (`Some`) or reset (`None`) the title (OSC 0/2):
    /// plain text, at most 256 characters.
    TitleChanged(Option<SharedString>),
    /// The program rang the bell (BEL).
    Bell,
    /// Pasted text with a line break waits for confirmation (UI 4.3). The
    /// owner shows the dialog (line count, preview) and calls
    /// [`TerminalView::paste_confirmed`] on "Paste"; Cancel drops it.
    ConfirmPaste(PendingPaste),
}

/// The matches of the search changed (a new query, a step, or new output
/// searched again): the Tools tab updates its "N of M".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchUpdated;

/// What the Tools tab shows about a search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchSummary {
    /// Matches found (at most `search::MAX_MATCHES`).
    pub count: usize,
    /// The current match, counted from 1 (the oldest match first).
    pub current: Option<usize>,
    /// More matches exist than are counted.
    pub truncated: bool,
}

/// A paste waiting for the user's confirmation: already cleaned (no
/// control characters except tab, CR and LF, no bracketed paste markers).
#[derive(Clone, PartialEq, Eq)]
pub struct PendingPaste {
    text: Rc<str>,
    /// The other panes it also goes to (Broadcast input on); 0 when off.
    broadcast: usize,
}

impl std::fmt::Debug for PendingPaste {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingPaste")
            .field("lines", &self.line_count())
            .field("broadcast", &self.broadcast)
            .finish_non_exhaustive()
    }
}

impl PendingPaste {
    /// The cleaned text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines (a final line break does not start a line).
    pub fn line_count(&self) -> usize {
        paste::lines(&self.text).len().max(1)
    }

    /// The line count and the first lines for the dialog.
    pub fn preview(&self) -> PastePreview {
        PastePreview::new(&self.text)
    }

    /// The other panes the paste also goes to (Broadcast input, UI 4.4).
    pub fn broadcast(&self) -> usize {
        self.broadcast
    }
}

/// A mouse drag in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    /// Extending a selection.
    Selecting,
    /// A button reported to the program, and the last reported cell.
    Reporting(ReportButton, (usize, usize)),
}

/// The terminal view. Create it with [`TerminalView::new`], feed it through
/// [`TerminalView::handle`] and show it in a tab as a child element.
pub struct TerminalView {
    handle: TerminalHandle,
    focus: FocusHandle,
    scrollback: usize,
    theme: Option<TerminalTheme>,
    settings: TerminalSettings,
    fonts: Rc<FontSet>,
    palette: Palette,
    layout: Rc<RefCell<LayoutState>>,
    drag: Option<Drag>,
    /// Where the pointer of a selection drag was last.
    drag_position: Option<gpui::Point<Pixels>>,
    /// Scrolls while a selection drag is above or below the grid.
    autoscroll: Option<Task<()>>,
    /// What plain Backspace sends (the host's setting).
    backspace: BackspaceKey,
    /// Broadcast input (UI 4.4, D81): the other panes' terminals that what
    /// is typed here also goes to; each encodes it with its own modes and
    /// Backspace setting.
    broadcast: Vec<WeakEntity<TerminalView>>,
    /// Broadcast input stopped at a password prompt until Enter or Ctrl+C
    /// is typed here or the shell is back at its prompt (a prompt that
    /// echoes `*` stays private too).
    broadcast_paused: bool,
    /// The last cell reported for motion without a button (mode 1003).
    hover_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
    marked_text: Option<String>,
    title: Option<SharedString>,
    /// A dropped session: the content is dimmed and the cursor hidden.
    dimmed: bool,
    /// The selection (UI 4.3), in buffer lines; its part on the screen is
    /// mirrored in the emulator.
    selection: Option<Selection>,
    /// The first and last cell of the emulator's copy of the selection's
    /// screen part, counted as the selection is, when it has one.
    selection_mirror: Option<(BufferPoint, BufferPoint)>,
    /// The screen (alternate or not) and the columns of the selection.
    selection_grid: (bool, usize),
    /// The open search, if any (UI 4.2 Tools).
    search: Option<SearchState>,
    /// New output arrived since the search last ran.
    search_stale: bool,
    /// The pending search refresh after new output.
    search_refresh: Option<Task<()>>,
    _pump: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}
impl EventEmitter<SearchUpdated> for TerminalView {}

impl std::fmt::Debug for TerminalView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalView").finish_non_exhaustive()
    }
}

/// The tab is gone: input and size changes stop, the parser thread ends
/// and the session's feeds return [`super::FeedError::Closed`].
impl Drop for TerminalView {
    fn drop(&mut self) {
        self.handle.close();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn cursor_shape(setting: CursorStyleSetting) -> CursorShape {
    match setting {
        CursorStyleSetting::Block => CursorShape::Block,
        CursorStyleSetting::Bar => CursorShape::Beam,
        CursorStyleSetting::Underline => CursorShape::Underline,
    }
}

impl TerminalView {
    /// Creates the emulator, its parser thread and the view.
    pub fn new(options: TerminalOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Both are no-ops after `terminal::init`.
        fonts::register(cx);
        bind_keys(cx);
        let settings = settings(cx);
        let palette =
            Palette::from_theme(options.theme.as_ref().unwrap_or(&settings.default_theme));
        let (wake, woken) = async_channel::bounded(1);
        let handle = TerminalHandle::spawn(
            ModelOptions {
                size: options.size,
                scrollback: options.scrollback,
                cursor_shape: cursor_shape(settings.cursor_style),
            },
            palette.clone(),
            wake,
        );
        let pump = cx.spawn(async move |this, cx| {
            while woken.recv().await.is_ok() {
                if this.update(cx, |view, cx| view.on_wake(cx)).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        let subscriptions = vec![
            // Settings → Terminal replaces this global after every change.
            cx.observe_global::<TerminalPrefs>(|view, cx| {
                view.apply_settings(super::settings::settings(cx), cx);
            }),
            cx.on_focus(&focus, window, |view, _, cx| view.focus_changed(true, cx)),
            cx.on_blur(&focus, window, |view, _, cx| view.focus_changed(false, cx)),
        ];
        Self {
            handle,
            focus,
            scrollback: options.scrollback,
            theme: options.theme,
            fonts: Rc::new(FontSet::new(&settings.font_family)),
            settings,
            palette,
            layout: Rc::new(RefCell::new(LayoutState::default())),
            drag: None,
            drag_position: None,
            autoscroll: None,
            backspace: BackspaceKey::default(),
            broadcast: Vec::new(),
            broadcast_paused: false,
            hover_cell: None,
            scroll_remainder: 0.,
            marked_text: None,
            title: None,
            dimmed: false,
            selection: None,
            selection_mirror: None,
            selection_grid: (false, 0),
            search: None,
            search_stale: false,
            search_refresh: None,
            _pump: pump,
            _subscriptions: subscriptions,
        }
    }

    /// The handle for feeding output and setting the sink (any thread).
    pub fn handle(&self) -> TerminalHandle {
        self.handle.clone()
    }

    /// Sizes the grid for a view of `area` (its padding included) before
    /// the view is drawn, so a session opens its shell at the size the tab
    /// will have rather than 80x24. Drawing sizes the grid again from the
    /// view's real bounds; for the same area the result is the same.
    pub fn fit(&self, area: gpui::Size<Pixels>, window: &Window) {
        let metrics = render::metrics(&self.frame_style(window), window);
        let text = size(area.width - PADDING * 2., area.height - PADDING * 2.);
        self.handle.resize(render::grid_size(text, &metrics));
    }

    /// The last title the program set.
    pub fn title(&self) -> Option<SharedString> {
        self.title.clone()
    }

    /// Moves the keyboard focus into the terminal (UI 4.1).
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    /// Whether the shell waits at its prompt: it turned bracketed paste on
    /// (bash, zsh, fish do at each prompt).
    pub fn at_prompt(&self) -> bool {
        self.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Broadcast input (UI 4.4): the terminals typed input also goes to;
    /// empty stops it.
    pub fn set_broadcast(&mut self, targets: Vec<WeakEntity<TerminalView>>) {
        if targets.is_empty() {
            self.broadcast_paused = false;
        }
        self.broadcast = targets;
    }

    /// The broadcast targets for `bytes` typed now; none from a password
    /// prompt until Enter or Ctrl+C ends it here or the shell is back at
    /// its prompt (a `sudo` password stays in its pane, its Enter too).
    /// While the shell edits its own line (bracketed paste on) no program
    /// asks a password: a typed `TOKEN:` is mirrored.
    fn broadcast_targets(&mut self, bytes: &[u8]) -> Vec<gpui::Entity<TerminalView>> {
        if self.broadcast.is_empty() {
            self.broadcast_paused = false;
            return Vec::new();
        }
        if self.at_prompt() {
            self.broadcast_paused = false;
        } else if self.broadcast_paused || self.handle.cursor_at_password_prompt() {
            self.broadcast_paused = !bytes.iter().any(|byte| matches!(byte, b'\r' | 0x03));
            return Vec::new();
        }
        self.broadcast
            .iter()
            .filter_map(WeakEntity::upgrade)
            .collect()
    }

    /// A key typed in a broadcasting pane, encoded for this terminal. As
    /// injected input: the line it is on is never recorded here.
    fn mirrored_key(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) {
        if let KeyAction::Send(bytes) = input::map_key(keystroke, self.mode(), self.backspace) {
            self.mirrored_bytes(bytes, cx);
        }
    }

    /// A paste in a broadcasting pane: bracketed per this terminal's mode;
    /// several lines are left out where the shell takes no bracketed paste
    /// (each line would run as it arrives).
    fn mirrored_paste(&mut self, cleaned: &str, cx: &mut Context<Self>) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        if !bracketed && paste::has_line_break(cleaned) {
            return;
        }
        self.mirrored_bytes(paste::paste_bytes(cleaned, bracketed), cx);
    }

    fn mirrored_bytes(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.handle.scroll_to_bottom();
        self.handle.inject_input(bytes);
        cx.notify();
    }

    /// Typed bytes that are the same in every terminal (text, Tab,
    /// Shift+Tab), mirrored to the broadcast targets.
    pub(super) fn type_bytes(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let targets = self.broadcast_targets(&bytes);
        for target in targets {
            target.update(cx, |target, cx| target.mirrored_bytes(bytes.clone(), cx));
        }
        self.send_input(bytes, cx);
    }

    /// Typed text (the input handler's path).
    fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.type_bytes(text.as_bytes().to_vec(), cx);
    }

    /// Tests: text as the input handler types it.
    #[cfg(test)]
    pub(crate) fn type_for_test(&mut self, text: &str, cx: &mut Context<Self>) {
        self.type_text(text, cx);
    }

    /// Tests: the multi-line paste setting.
    #[cfg(test)]
    pub(crate) fn set_confirm_multiline_paste_for_test(&mut self, on: bool) {
        self.settings.confirm_multiline_paste = on;
    }

    /// What plain Backspace sends from now on (the host's setting, UI 3.1).
    pub fn set_backspace(&mut self, backspace: BackspaceKey) {
        self.backspace = backspace;
    }

    /// What plain Backspace sends.
    pub fn backspace(&self) -> BackspaceKey {
        self.backspace
    }

    /// Uses the host's theme, or the settings' default theme for `None`.
    pub fn set_theme(&mut self, theme: Option<TerminalTheme>, cx: &mut Context<Self>) {
        self.theme = theme;
        self.update_palette(cx);
    }

    /// The theme the terminal is drawn with: its own, else the default
    /// theme of the settings.
    pub fn theme(&self) -> &TerminalTheme {
        self.theme.as_ref().unwrap_or(&self.settings.default_theme)
    }

    /// The settings the view draws with (font, size, spacing, contrast).
    pub fn settings(&self) -> &TerminalSettings {
        &self.settings
    }

    /// Dims the content and hides the cursor while the session is dropped
    /// (UI 1.1, 4.6); `false` restores both (on reconnect).
    pub fn set_dimmed(&mut self, dimmed: bool, cx: &mut Context<Self>) {
        if self.dimmed != dimmed {
            self.dimmed = dimmed;
            cx.notify();
        }
    }

    pub fn is_dimmed(&self) -> bool {
        self.dimmed
    }

    // Search (UI 4.2 Tools) ---------------------------------------------------

    /// Searches the scrollback and the screen for `query` (plain text,
    /// case-insensitive) and highlights the matches; an empty query ends
    /// the search. The view scrolls to the current match when it is not
    /// visible.
    pub fn search(&mut self, query: &str, cx: &mut Context<Self>) -> SearchSummary {
        if query.is_empty() {
            self.clear_search(cx);
            return SearchSummary::default();
        }
        let state = {
            let term = self.handle.term().lock();
            let origin = self.handle.search_origin(&term);
            self.handle
                .with_lines(&term, |lines| SearchState::new(query, lines, origin))
        };
        self.search = Some(state);
        self.search_stale = false;
        self.scroll_to_current();
        cx.emit(SearchUpdated);
        cx.notify();
        self.search_summary().unwrap_or_default()
    }

    /// Moves to the next match (`forward`, toward newer output) or the
    /// previous one, wrapping around, and scrolls it into view.
    pub fn search_step(&mut self, forward: bool, cx: &mut Context<Self>) -> SearchSummary {
        if self.search_stale {
            self.refresh_search(cx);
        }
        let Some(search) = self.search.as_mut() else {
            return SearchSummary::default();
        };
        search.step(forward);
        self.scroll_to_current();
        cx.emit(SearchUpdated);
        cx.notify();
        self.search_summary().unwrap_or_default()
    }

    /// Ends the search: the highlights go away.
    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search_refresh = None;
        self.search_stale = false;
        if self.search.take().is_some() {
            cx.emit(SearchUpdated);
            cx.notify();
        }
    }

    /// The open search's query, if any.
    pub fn search_query(&self) -> Option<&str> {
        self.search.as_ref().map(SearchState::query)
    }

    /// The counter of the open search, if any.
    pub fn search_summary(&self) -> Option<SearchSummary> {
        self.search.as_ref().map(|search| SearchSummary {
            count: search.count(),
            current: search.current_index().map(|index| index + 1),
            truncated: search.truncated(),
        })
    }

    /// The open search (tests and the sidebar).
    pub fn search_state(&self) -> Option<&SearchState> {
        self.search.as_ref()
    }

    /// Scrolls so the current match is visible (in the middle of the
    /// screen when it was not).
    fn scroll_to_current(&self) {
        let Some(found) = self.search.as_ref().and_then(SearchState::current) else {
            return;
        };
        let (offset, target) = {
            let term = self.handle.term().lock();
            let evicted = self.handle.evicted_lines();
            self.handle.with_lines(&term, |lines| {
                (
                    lines.view_offset(),
                    SearchState::offset_showing(lines, evicted, found),
                )
            })
        };
        if target != offset {
            self.handle
                .scroll(Scroll::Delta(target as i32 - offset as i32));
        }
    }

    /// New output or a reflow while a search is open: search again, at
    /// most once per [`SEARCH_REFRESH`], so the highlights follow the text
    /// while a flood or a resize drag does not search on every frame. A
    /// reflow searches at once when no search ran in the last period (its
    /// highlights wait for that search); output searches at the end of the
    /// period.
    fn search_content_changed(&mut self, reflowed: bool, cx: &mut Context<Self>) {
        if self.search.is_none() {
            return;
        }
        self.search_stale = true;
        if self.search_refresh.is_some() {
            return;
        }
        if reflowed {
            self.refresh_search(cx);
        }
        self.schedule_search_refresh(cx);
    }

    /// Searches again at the end of the period if anything changed in it,
    /// and then waits another period.
    fn schedule_search_refresh(&mut self, cx: &mut Context<Self>) {
        self.search_refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_REFRESH).await;
            let _ = this.update(cx, |view, cx| {
                view.search_refresh = None;
                if view.search_stale {
                    view.refresh_search(cx);
                    view.schedule_search_refresh(cx);
                }
            });
        }));
    }

    /// Searches again for the same query (after new output or a reflow):
    /// only the rows that can have changed, unless the grid reflowed
    /// ([`SearchState::refresh`]).
    pub fn refresh_search(&mut self, cx: &mut Context<Self>) {
        self.search_stale = false;
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let changed = {
            let term = self.handle.term().lock();
            let origin = self.handle.search_origin(&term);
            self.handle
                .with_lines(&term, |lines| search.refresh(lines, origin))
        };
        if changed {
            cx.emit(SearchUpdated);
            cx.notify();
        }
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let term = self.handle.term().lock();
        let evicted = self.handle.evicted_lines();
        self.handle
            .with_lines(&term, |lines| selection.text(lines, evicted))
    }

    /// The buffer point of a viewport cell (its line counted with the
    /// lines dropped before).
    fn buffer_point(&self, line: usize, column: usize) -> BufferPoint {
        let term = self.handle.term().lock();
        let evicted = self.handle.evicted_lines();
        self.handle.with_lines(&term, |lines| BufferPoint {
            line: evicted + lines.view_top() + line,
            column,
        })
    }

    /// Sets the selection and mirrors its part on the screen in the
    /// emulator, which drops it when output writes over it and moves it
    /// with text that scrolls on the screen.
    fn set_selection(&mut self, selection: Option<Selection>) {
        let mut term = self.handle.term().lock();
        let evicted = self.handle.evicted_lines();
        let mirror = selection.and_then(|selection| {
            self.handle
                .with_lines(&term, |lines| selection.mirror(lines, evicted))
        });
        term.selection = mirror;
        self.selection_mirror = self.mirrored_range(&term);
        self.selection_grid = (
            term.mode().contains(TermMode::ALT_SCREEN),
            term.grid().cols(),
        );
        self.selection = selection;
    }

    /// The cells the emulator's selection covers, counted as the view's
    /// selection is.
    fn mirrored_range(
        &self,
        term: &alacritty_terminal::Term<super::model::Listener>,
    ) -> Option<(BufferPoint, BufferPoint)> {
        let range = term.selection.as_ref()?.to_range(term)?;
        let evicted = self.handle.evicted_lines() as i64;
        let screen = self.handle.with_lines(term, |lines| lines.above()) as i64;
        let point = |point: alacritty_terminal::index::Point| BufferPoint {
            line: (evicted + screen + i64::from(point.line.0)).max(0) as usize,
            column: point.column.0,
        };
        Some((point(range.start), point(range.end)))
    }

    /// Follows the emulator's copy of the selection: it ends when output
    /// wrote over it, a reset or the other screen; it moves with text that
    /// scrolled on the screen (a scroll region, the alternate screen). A
    /// copy that only moved into the journal (D85) leaves the selection as
    /// it is; a new width (rows that reflowed) ends it.
    fn check_selection(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let term = self.handle.term().lock();
        let grid = (
            term.mode().contains(TermMode::ALT_SCREEN),
            term.grid().cols(),
        );
        let moved = self.handle.outbox().take_selection_moved();
        let now = self.mirrored_range(&term);
        match (self.selection_mirror, now) {
            _ if grid != self.selection_grid => self.selection = None,
            (Some(_), None) if !moved => self.selection = None,
            (Some(before), Some(after)) if before != after => {
                // An end the emulator clamped (at the edge of the screen or
                // of a scroll region) moved less: the other one tells.
                let start = after.0.line as i64 - before.0.line as i64;
                let end = after.1.line as i64 - before.1.line as i64;
                let lines = if start.abs() >= end.abs() { start } else { end };
                self.selection = Some(selection.shifted(lines));
            }
            _ => {}
        }
        self.selection_mirror = now;
    }

    /// Rows shaped so far (the row cache's misses; tests and benchmarks).
    pub fn shaped_rows(&self) -> u64 {
        self.layout.borrow().shaped_rows
    }

    fn apply_settings(&mut self, settings: TerminalSettings, cx: &mut Context<Self>) {
        if settings == self.settings {
            return;
        }
        if settings.cursor_style != self.settings.cursor_style {
            let config = term_config(self.scrollback, cursor_shape(settings.cursor_style));
            self.handle.term().lock().set_options(config);
        }
        if settings.font_family != self.settings.font_family {
            self.fonts = Rc::new(FontSet::new(&settings.font_family));
        }
        self.settings = settings;
        self.update_palette(cx);
    }

    fn update_palette(&mut self, cx: &mut Context<Self>) {
        let theme = self.theme.as_ref().unwrap_or(&self.settings.default_theme);
        self.palette = Palette::from_theme(theme);
        self.handle.set_palette(self.palette.clone());
        cx.notify();
    }

    fn on_wake(&mut self, cx: &mut Context<Self>) {
        self.check_selection();
        let events = self.handle.outbox().take_events();
        if let Some(title) = events.title {
            self.title = title.map(SharedString::from);
            cx.emit(TerminalEvent::TitleChanged(self.title.clone()));
        }
        if events.bell {
            cx.emit(TerminalEvent::Bell);
        }
        // After a reflow the search runs again soon (its positions moved);
        // after output, shortly after.
        let resized = std::mem::take(&mut self.layout.borrow_mut().resized);
        self.search_content_changed(resized, cx);
        cx.notify();
    }

    fn focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        let reports = self
            .handle
            .term()
            .lock()
            .mode()
            .contains(TermMode::FOCUS_IN_OUT);
        if reports {
            self.handle
                .send_input(if focused { b"\x1b[I" } else { b"\x1b[O" }.to_vec());
        }
        cx.notify();
    }

    fn mode(&self) -> TermMode {
        *self.handle.term().lock().mode()
    }

    /// Sends typed input: back to the bottom of the scrollback first.
    fn send_input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.handle.scroll_to_bottom();
        // Typed input: watched by the history capture.
        self.handle.send_typed(bytes);
        cx.notify();
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.handle.scroll(scroll);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // → takes a shown suggestion (autocomplete, D142).
        let keystroke = &event.keystroke;
        if keystroke.key == "right" && !keystroke.modifiers.modified() && self.accept_suggestion(cx)
        {
            cx.stop_propagation();
            return;
        }
        let page = self.layout.borrow().rows.max(2) as i32 - 1;
        match input::map_key(&event.keystroke, self.mode(), self.backspace) {
            KeyAction::Send(bytes) => {
                let targets = self.broadcast_targets(&bytes);
                self.send_input(bytes, cx);
                for target in targets {
                    target.update(cx, |target, cx| target.mirrored_key(&event.keystroke, cx));
                }
            }
            KeyAction::ScrollPageUp => self.scroll(Scroll::Delta(page), cx),
            KeyAction::ScrollPageDown => self.scroll(Scroll::Delta(-page), cx),
            KeyAction::ScrollTop => self.scroll(Scroll::Top, cx),
            KeyAction::ScrollBottom => self.scroll(Scroll::Bottom, cx),
            KeyAction::Ignore => return,
        }
        cx.stop_propagation();
    }

    // Clipboard ---------------------------------------------------------------

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste_action(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        self.paste_clipboard(cx);
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        let all = {
            let term = self.handle.term().lock();
            let evicted = self.handle.evicted_lines();
            self.handle
                .with_lines(&term, |lines| Selection::all(lines, evicted))
        };
        self.set_selection(Some(all));
        cx.notify();
    }

    /// Pastes `text` (UI 4.3): cleaned; with a line break, after the user
    /// confirms (when `terminal.confirm_multiline_paste` is on).
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let cleaned = paste::sanitize(text);
        if cleaned.is_empty() {
            return;
        }
        // While broadcasting, several lines always ask first (they run in
        // every pane).
        let confirm = self.settings.confirm_multiline_paste || !self.broadcast.is_empty();
        if paste::needs_confirmation(&cleaned, confirm) {
            self.confirm_paste(cleaned, cx);
        } else {
            self.send_paste(&cleaned, true, cx);
        }
    }

    /// Sends a paste; `mirror`: also to the broadcast targets.
    fn send_paste(&mut self, cleaned: &str, mirror: bool, cx: &mut Context<Self>) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        let bytes = paste::paste_bytes(cleaned, bracketed);
        let targets = if mirror {
            self.broadcast_targets(&bytes)
        } else {
            Vec::new()
        };
        self.send_input(bytes, cx);
        for target in targets {
            target.update(cx, |target, cx| target.mirrored_paste(cleaned, cx));
        }
    }

    /// Asks the owner to confirm a paste (UI 4.3): the tab shows the
    /// dialog and calls [`Self::paste_confirmed`] on "Paste".
    fn confirm_paste(&mut self, cleaned: String, cx: &mut Context<Self>) {
        cx.emit(TerminalEvent::ConfirmPaste(PendingPaste {
            text: cleaned.into(),
            broadcast: self.broadcast.len(),
        }));
    }

    /// Sends a paste the user confirmed (bracketed per the mode at this
    /// moment) and returns the focus to the terminal. It goes to the other
    /// panes only if the dialog said so: Broadcast input turned on while
    /// it was open does not take it along.
    pub fn paste_confirmed(
        &mut self,
        paste: &PendingPaste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_paste(&paste.text, paste.broadcast > 0, cx);
        self.focus.focus(window, cx);
    }

    // Mouse -------------------------------------------------------------------

    fn cell_at(&self, position: gpui::Point<Pixels>) -> Option<(usize, usize, bool)> {
        self.layout.borrow().cell_at(position)
    }

    fn report(
        &mut self,
        button: ReportButton,
        kind: ReportKind,
        cell: (usize, usize),
        modifiers: Modifiers,
    ) {
        if let Some(bytes) = mouse::report(self.mode(), button, kind, cell.1, cell.0, modifiers) {
            self.handle.send_input(bytes);
        }
    }

    fn reports_mouse(&self, modifiers: Modifiers) -> bool {
        !modifiers.shift && mouse::reporting(self.mode())
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let Some((line, column, right_half)) = self.cell_at(event.position) else {
            return;
        };
        let button = match event.button {
            MouseButton::Left => ReportButton::Left,
            MouseButton::Middle => ReportButton::Middle,
            MouseButton::Right => ReportButton::Right,
            _ => return,
        };
        if self.reports_mouse(event.modifiers) {
            self.report(button, ReportKind::Press, (line, column), event.modifiers);
            self.drag = Some(Drag::Reporting(button, (line, column)));
            cx.stop_propagation();
            return;
        }
        match event.button {
            MouseButton::Left => {
                let point = self.buffer_point(line, column);
                let side = if right_half { Side::Right } else { Side::Left };
                let kind = selection::selection_type(event.click_count);
                self.set_selection(Some(Selection::new(kind, point, side)));
                self.drag = Some(Drag::Selecting);
                cx.notify();
            }
            // UI 4.3: the right button pastes.
            MouseButton::Right => self.paste_clipboard(cx),
            _ => {}
        }
        cx.stop_propagation();
    }

    fn mouse_moved(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.drag {
            Some(Drag::Selecting) if event.pressed_button == Some(MouseButton::Left) => {
                self.drag_position = Some(event.position);
                if self.autoscroll_lines(event.position) == 0 {
                    self.autoscroll = None;
                    self.extend_selection(event.position, cx);
                } else if self.autoscroll.is_none() {
                    // Above or below the grid: scroll now, then on a timer
                    // while the button stays down there.
                    self.autoscroll_step(cx);
                    self.autoscroll = Some(cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor().timer(AUTOSCROLL_INTERVAL).await;
                            let going = this
                                .update(cx, |view, cx| view.autoscroll_step(cx))
                                .unwrap_or(false);
                            if !going {
                                break;
                            }
                        }
                    }));
                } else {
                    self.extend_selection(event.position, cx);
                }
            }
            Some(Drag::Reporting(button, last)) => {
                let Some((line, column, _)) = self.cell_at(event.position) else {
                    return;
                };
                if (line, column) != last && mouse::reports_motion(self.mode(), true) {
                    self.report(button, ReportKind::Motion, (line, column), event.modifiers);
                    self.drag = Some(Drag::Reporting(button, (line, column)));
                }
            }
            _ => {}
        }
    }

    /// Extends the selection to the cell under `position` (clamped to the
    /// grid).
    fn extend_selection(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some((line, column, right_half)) = self.cell_at(position) else {
            return;
        };
        let point = self.buffer_point(line, column);
        let side = if right_half { Side::Right } else { Side::Left };
        if let Some(mut selection) = self.selection {
            selection.update(point, side);
            self.set_selection(Some(selection));
        }
        cx.notify();
    }

    /// The lines one autoscroll step moves for a drag at `position`:
    /// positive above the grid (into the scrollback), negative below it,
    /// 0 inside; one more per line height of distance, at most
    /// [`AUTOSCROLL_MAX_LINES`].
    fn autoscroll_lines(&self, position: gpui::Point<Pixels>) -> i32 {
        let layout = self.layout.borrow();
        let Some(metrics) = layout.metrics else {
            return 0;
        };
        let line_height = metrics.line_height.as_f32();
        let top = layout.origin.y.as_f32();
        let bottom = top + line_height * layout.rows as f32;
        let y = position.y.as_f32();
        let lines = |distance: f32| (1 + (distance / line_height) as i32).min(AUTOSCROLL_MAX_LINES);
        if y < top {
            lines(top - y)
        } else if y >= bottom {
            -lines(y - bottom)
        } else {
            0
        }
    }

    /// One autoscroll step of a selection drag outside the grid: scrolls
    /// and extends the selection to the row at that edge. Whether the drag
    /// is still outside (the timer goes on).
    fn autoscroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(position) = self
            .drag_position
            .filter(|_| self.drag == Some(Drag::Selecting))
        else {
            return false;
        };
        let lines = self.autoscroll_lines(position);
        if lines == 0 {
            return false;
        }
        self.handle.scroll(Scroll::Delta(lines));
        self.extend_selection(position, cx);
        true
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.drag_position = None;
        self.autoscroll = None;
        match self.drag.take() {
            Some(Drag::Selecting) => {
                // A click without a drag selects nothing.
                let selected = self
                    .selection
                    .is_some_and(|selection| !selection.is_empty());
                if !selected {
                    self.set_selection(None);
                }
                if selected
                    && self.settings.copy_on_select
                    && let Some(text) = self.selected_text()
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                cx.notify();
            }
            Some(Drag::Reporting(button, last)) => {
                let cell = self
                    .cell_at(event.position)
                    .map_or(last, |(l, c, _)| (l, c));
                self.report(button, ReportKind::Release, cell, event.modifiers);
            }
            None => {}
        }
    }

    /// Motion without a pressed button (mode 1003) over the terminal.
    fn hover(&mut self, event: &MouseMoveEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        if self.drag.is_some() || event.pressed_button.is_some() {
            return;
        }
        if self.reports_mouse(event.modifiers)
            && mouse::reports_motion(self.mode(), false)
            && let Some((line, column, _)) = self.cell_at(event.position)
            && self.hover_cell != Some((line, column))
        {
            // One report per cell, not per pixel.
            self.hover_cell = Some((line, column));
            self.report(
                ReportButton::None,
                ReportKind::Motion,
                (line, column),
                event.modifiers,
            );
        }
    }

    fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(metrics) = self.layout.borrow().metrics else {
            return;
        };
        let line_height = metrics.line_height.as_f32();
        self.scroll_remainder += event.delta.pixel_delta(metrics.line_height).y.as_f32();
        let lines = (self.scroll_remainder / line_height).trunc() as i32;
        if lines == 0 {
            cx.stop_propagation();
            return;
        }
        self.scroll_remainder -= lines as f32 * line_height;
        let mode = self.mode();
        if self.reports_mouse(event.modifiers) {
            let cell = self
                .cell_at(event.position)
                .map_or((0, 0), |(line, column, _)| (line, column));
            let button = if lines > 0 {
                ReportButton::WheelUp
            } else {
                ReportButton::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                self.report(button, ReportKind::Press, cell, event.modifiers);
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs without mouse support get arrow keys.
            let arrow: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let bytes = arrow.repeat(lines.unsigned_abs() as usize);
            self.handle.send_input(bytes);
        } else {
            self.scroll(Scroll::Delta(lines), cx);
            // A selection drag follows the text under the pointer.
            if self.drag == Some(Drag::Selecting)
                && let Some(position) = self.drag_position
            {
                self.extend_selection(position, cx);
            }
        }
        cx.stop_propagation();
    }

    pub(super) fn frame_style(&self, window: &Window) -> FrameStyle {
        FrameStyle {
            palette: self.palette.clone(),
            fonts: self.fonts.clone(),
            font_size: px(self.settings.font_size),
            line_height: self.settings.line_height,
            bold_is_bright: self.settings.bold_is_bright,
            focused: self.focus.is_focused(window),
            marked_text: self.marked_text.clone(),
            ghost_text: None,
            min_contrast: self.settings.min_contrast,
            cursor_hidden: self.dimmed,
            search: self.search.clone(),
            selection: self.selection,
            highlighting: self.settings.highlighting,
        }
    }
}

#[cfg(test)]
impl TerminalView {
    /// A window position inside a viewport cell (last frame): `fraction`
    /// of the cell's width from its left edge, at half its height.
    pub(super) fn cell_point(
        &self,
        line: usize,
        column: usize,
        fraction: f32,
    ) -> gpui::Point<Pixels> {
        let layout = self.layout.borrow();
        let metrics = layout.metrics.expect("a frame was drawn");
        point(
            layout.origin.x + metrics.cell_width * (column as f32 + fraction),
            layout.origin.y + metrics.line_height * (line as f32 + 0.5),
        )
    }

    pub(super) fn layout_state(&self) -> Rc<RefCell<LayoutState>> {
        self.layout.clone()
    }

    /// Rows the view is scrolled up (the journal's included; tests).
    #[cfg(test)]
    pub(super) fn display_offset(&self) -> usize {
        self.handle.view_offset()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut style = self.frame_style(window);
        style.ghost_text = self.suggestion(cx);
        let background = rgb_token(self.palette.background()).hsla();
        let handle = self.handle.clone();
        let layout = self.layout.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let dimmed = self.dimmed;
        let grid = canvas(
            move |bounds, window, _cx| {
                render::prepaint(bounds, &handle, &mut layout.borrow_mut(), &style, window)
            },
            move |bounds, frame, window, cx| {
                render::paint(bounds, &frame, window, cx);
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                // Drags continue outside the terminal (selection, reports).
                let view = entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        view.update(cx, |view, cx| view.mouse_moved(event, window, cx));
                    }
                });
                let view = entity.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        view.update(cx, |view, cx| view.mouse_up(event, window, cx));
                    }
                });
            },
        )
        .size_full();
        // A dropped session (UI 1.1): the text at `--opacity-disabled` over
        // the theme background.
        let grid = div()
            .size_full()
            .when(dimmed, |this| this.opacity(metrics::OPACITY_DISABLED))
            .child(grid);

        div()
            .id("terminal")
            .test_support()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .bg(background)
            .p(PADDING)
            .cursor_text()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste_action))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(|this, _: &edit::Copy, window, cx| this.copy(&Copy, window, cx)))
            .on_action(cx.listener(|this, _: &edit::Paste, _, cx| this.paste_clipboard(cx)))
            .on_action(cx.listener(|this, _: &edit::SelectAll, window, cx| {
                this.select_all(&SelectAll, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SendTab, _, cx| {
                // Tab takes a shown suggestion (autocomplete, D142).
                if !this.accept_suggestion(cx) {
                    this.type_bytes(b"\t".to_vec(), cx)
                }
            }))
            .on_action(
                cx.listener(|this, _: &SendBackTab, _, cx| this.type_bytes(b"\x1b[Z".to_vec(), cx)),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::hover))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(grid)
    }
}

/// The platform text input: committed text goes to the shell; IME
/// composition text is drawn at the cursor until it is committed.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if !text.is_empty() {
            self.type_text(text, cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.borrow();
        let metrics = layout.metrics?;
        let (line, column) = layout.cursor;
        Some(Bounds::new(
            point(
                layout.origin.x + metrics.cell_width * column as f32,
                layout.origin.y + metrics.line_height * line as f32,
            ),
            size(metrics.cell_width, metrics.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn paste(&mut self, item: ClipboardItem, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.paste_text(&text, cx);
        }
    }
}
