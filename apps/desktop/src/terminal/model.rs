//! The emulator side of a terminal: alacritty_terminal's `Term` behind a
//! fair mutex, the parser thread that feeds it, and the handle the session
//! layer uses from any thread (decision 30).
//!
//! Data flow:
//!
//! ```text
//! session (Tokio task / PTY thread) --feed--> bounded queue --> parser thread
//!     parser thread: Term (FairMutex) <-- UI thread (render snapshot, input)
//!     parser thread --wake (bounded(1), coalesced)--> UI pump --> cx.notify()
//! keyboard / paste / mouse / terminal replies --> TerminalSink::send
//! grid size changes --> TerminalSink::resize
//! ```
//!
//! Flow control: the queue holds at most [`QUEUE_CHUNKS`] chunks of at most
//! [`CHUNK_BYTES`]; [`TerminalHandle::feed`] waits while it is full, so a
//! flood (`seq 1 3000000`) slows the SSH channel or the PTY reader instead
//! of growing memory. The parser takes at most [`BATCH_BYTES`] from the
//! queue at a time and holds the terminal lock for at most
//! [`LOCKED_BYTES`] of parsing, so the UI thread never waits long for a
//! frame.
//!
//! What parsed output can store is bounded too ([`super::limits`]): OSC
//! strings keep at most `MAX_OSC_PAYLOAD` bytes and a cell at most
//! `MAX_ZEROWIDTH` zero-width characters.
//!
//! Nothing here logs terminal data, titles or sizes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Column;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor, Rgb, StdSyncHandler};

use super::colors::Palette;
use super::journal::{
    self, COMPACT_BATCH, COMPACT_INTERVAL, COMPACT_QUIET, Journal, LIVE_HISTORY, Lines,
};
use super::limits::{self, Capped, MAX_ZEROWIDTH, OscLimiter};

/// Largest chunk the queue carries; bigger feeds are split.
pub const CHUNK_BYTES: usize = 64 * 1024;
/// Chunks the queue holds before [`TerminalHandle::feed`] waits.
pub const QUEUE_CHUNKS: usize = 32;
/// Bytes the parser takes from the queue before it parses.
pub const BATCH_BYTES: usize = 1024 * 1024;
/// Bytes parsed per hold of the terminal lock.
pub const LOCKED_BYTES: usize = 64 * 1024;
/// Longest window or tab title kept (docs/security.md, section 12).
pub const TITLE_MAX_CHARS: usize = 256;
/// While a synchronized update (mode 2026) is open, the parser checks the
/// queue at this interval until the update's deadline.
const SYNC_POLL: Duration = Duration::from_millis(4);

/// The size of the terminal grid, as sent to the remote side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
    /// Width of the text area in logical pixels (0 when unknown).
    pub pixel_width: u16,
    /// Height of the text area in logical pixels (0 when unknown).
    pub pixel_height: u16,
}

impl TerminalSize {
    /// A size without pixel dimensions.
    pub const fn cells(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self::cells(80, 24)
    }
}

/// The grid at `size` (the view's, or the width a DECCOLM set); tells the
/// sink. Returns whether it changed.
fn apply_size(term: &SharedTerm, outbox: &Outbox, scrollback: usize, size: TerminalSize) -> bool {
    {
        let Ok(mut current) = outbox.size.lock() else {
            return false;
        };
        // The grid may differ from the size last told: a DECCOLM resized it
        // in the parser, and a RIS in the same output ended the width.
        let grid_differs = {
            let term = term.lock();
            term.columns() != usize::from(size.cols)
                || term.screen_lines() != usize::from(size.rows)
        };
        if *current == size && !grid_differs {
            return false;
        }
        let cells_changed = grid_differs || current.cols != size.cols || current.rows != size.rows;
        *current = size;
        if cells_changed {
            let mut term = term.lock();
            // A reflow or fewer rows push more lines into the grid's
            // scrollback: let it hold them, so none is lost between the
            // journal and the grid; the next output trims the buffer to its
            // limit, the journal's oldest lines first. Not on the alternate
            // screen: its grid has no scrollback.
            if !term.mode().contains(TermMode::ALT_SCREEN) {
                let history = term.grid().history_size();
                term.grid_mut().update_history(scrollback.max(history));
            }
            term.resize(GridDims::from(size));
            // A view in the journal shows the grid from its top.
            let journal = outbox.journal();
            if journal.view() > 0 {
                term.scroll_display(Scroll::Top);
            }
        }
    }
    outbox.resize(size);
    true
}

/// A DECCOLM (or RIS) changed the forced width (D90): the grid takes it
/// (the parser's handler already gave the grid a forced width) and the sink
/// is told.
fn columns_changed(term: &SharedTerm, outbox: &Outbox, scrollback: usize, forced: Option<u16>) {
    let size = {
        let Ok(mut columns) = outbox.columns.lock() else {
            return;
        };
        columns.0 = forced;
        let natural = columns
            .1
            .unwrap_or_else(|| outbox.size.lock().map(|size| *size).unwrap_or_default());
        with_columns(natural, forced)
    };
    apply_size(term, outbox, scrollback, size);
}

/// `size` with the width a DECCOLM set, if any; the pixel width in
/// proportion.
fn with_columns(size: TerminalSize, forced: Option<u16>) -> TerminalSize {
    match forced {
        Some(cols) if cols != size.cols => TerminalSize {
            cols,
            pixel_width: (u32::from(size.pixel_width) * u32::from(cols)
                / u32::from(size.cols.max(1)))
            .try_into()
            .unwrap_or(u16::MAX),
            ..size
        },
        _ => size,
    }
}

/// Grid dimensions for alacritty (`Dimensions` without scrollback).
#[derive(Clone, Copy, Debug)]
pub(super) struct GridDims {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for GridDims {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

impl From<TerminalSize> for GridDims {
    fn from(size: TerminalSize) -> Self {
        Self {
            cols: usize::from(size.cols.max(2)),
            rows: usize::from(size.rows.max(1)),
        }
    }
}

/// Where a terminal sends what it produces. Implemented by the session
/// layer (SSH channel, local PTY). Both methods are called from the UI
/// thread and from the parser thread and must not block.
pub trait TerminalSink: Send + Sync + 'static {
    /// Bytes for the remote side, UTF-8: keyboard input, pasted text, mouse
    /// reports and replies to terminal queries (cursor position, device
    /// attributes, colors). The session encodes them to the host charset.
    fn send(&self, bytes: Vec<u8>);

    /// The grid size changed: forward it (SSH window-change, PTY resize).
    fn resize(&self, size: TerminalSize);
}

/// Why bytes could not be fed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FeedError {
    /// The queue is full (only [`TerminalHandle::try_feed`]).
    #[error("the terminal queue is full")]
    Full,
    /// The terminal was closed.
    #[error("the terminal is closed")]
    Closed,
}

/// Events for the UI thread, collected on the parser thread.
#[derive(Debug, Default)]
pub(super) struct PendingEvents {
    /// `Some(None)` resets the title.
    pub title: Option<Option<String>>,
    pub bell: bool,
}

type ColorFormatter = Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>;

/// State shared by the handle, the parser thread, the listener and the view.
pub(super) struct Outbox {
    sink: RwLock<Option<Arc<dyn TerminalSink>>>,
    pending: Mutex<PendingEvents>,
    color_requests: Mutex<Vec<(usize, ColorFormatter)>>,
    palette: RwLock<Palette>,
    wake: async_channel::Sender<()>,
    size: Mutex<TerminalSize>,
    /// The width a DECCOLM set (D90; `limits::Columns`) and the size the
    /// view asked for: `size` is the view's, with the forced width.
    columns: Mutex<(Option<u16>, Option<TerminalSize>)>,
    /// Lines dropped off the top of the scrollback so far
    /// ([`limits::count_evictions`]); written by the parser thread while it
    /// holds the terminal lock, so a reader holding the lock sees the count
    /// of the grid it reads.
    evicted: AtomicUsize,
    /// The compact part of the scrollback (D85). Locked after the terminal,
    /// never before it.
    journal: Arc<Mutex<Journal>>,
    /// A compaction ended the emulator's selection (its rows moved into the
    /// journal): the view keeps its own.
    selection_moved: std::sync::atomic::AtomicBool,
    /// Command history capture (docs/security.md, section 10.1).
    pub(super) capture: super::history_capture::Capture,
    /// Autocomplete's history source and list (D142).
    pub(super) autocomplete: Arc<super::autocomplete::Slot>,
}

impl Outbox {
    /// The journal (lock the terminal first).
    pub(super) fn journal(&self) -> MutexGuard<'_, Journal> {
        self.journal.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a compaction ended the emulator's selection since the last
    /// call.
    pub(super) fn take_selection_moved(&self) -> bool {
        self.selection_moved.swap(false, Ordering::AcqRel)
    }

    /// Sends bytes to the sink (dropped while there is none).
    pub fn send(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let sink = self.sink.read().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            sink.send(bytes);
        }
    }

    /// The width a DECCOLM set, if any.
    fn forced_columns(&self) -> Option<u16> {
        self.columns.lock().map(|columns| columns.0).unwrap_or(None)
    }

    fn resize(&self, size: TerminalSize) {
        let sink = self.sink.read().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            sink.resize(size);
        }
    }

    /// Wakes the UI pump; a wake that is already pending covers this one.
    pub fn wake(&self) {
        let _ = self.wake.try_send(());
    }

    pub fn take_events(&self) -> PendingEvents {
        self.pending
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default()
    }

    pub fn palette(&self) -> Palette {
        self.palette
            .read()
            .map(|palette| palette.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }
}

/// Receives the emulator's events on the parser thread (inside the lock).
pub(super) struct Listener(Arc<Outbox>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // Replies to queries (DSR, DA, ...): straight to the remote side.
            Event::PtyWrite(text) => self.0.send(text.into_bytes()),
            Event::Title(title) => {
                if let Ok(mut pending) = self.0.pending.lock() {
                    pending.title = Some(Some(sanitize_title(&title)));
                }
            }
            Event::ResetTitle => {
                if let Ok(mut pending) = self.0.pending.lock() {
                    pending.title = Some(None);
                }
            }
            Event::Bell => {
                if let Ok(mut pending) = self.0.pending.lock() {
                    pending.bell = true;
                }
            }
            // Answered after the parse step, from the same lock hold.
            Event::ColorRequest(index, format) => {
                if let Ok(mut requests) = self.0.color_requests.lock() {
                    requests.push((index, format));
                }
            }
            // OSC 52 is disabled in the config, so no clipboard events come;
            // text area size reports (a window operation) are never answered
            // (docs/security.md, section 12).
            Event::ClipboardStore(..)
            | Event::ClipboardLoad(..)
            | Event::TextAreaSizeRequest(_)
            | Event::MouseCursorDirty
            | Event::CursorBlinkingChange
            | Event::Wakeup
            | Event::Exit
            | Event::ChildExit(_) => {}
        }
    }
}

/// A remote title as plain text: no control or bidirectional override
/// characters, at most [`TITLE_MAX_CHARS`] characters.
pub fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .filter(|ch| !ch.is_control() && !is_bidi_control(*ch))
        .take(TITLE_MAX_CHARS)
        .collect()
}

fn is_bidi_control(ch: char) -> bool {
    matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}')
}

pub(super) type SharedTerm = Arc<FairMutex<Term<Listener>>>;

enum Message {
    Bytes(Vec<u8>),
    /// Answered once everything queued before it is parsed.
    Flush(mpsc::Sender<()>),
}

/// The emulator configuration.
pub(super) fn term_config(scrollback: usize, cursor_shape: CursorShape) -> Config {
    Config {
        scrolling_history: scrollback,
        default_cursor_style: CursorStyle {
            shape: cursor_shape,
            blinking: false,
        },
        // OSC 52: a host may neither read nor replace the clipboard
        // (docs/security.md, section 12).
        osc52: Osc52::Disabled,
        kitty_keyboard: false,
        ..Config::default()
    }
}

/// Options of a new terminal.
#[derive(Clone, Debug)]
pub struct ModelOptions {
    pub size: TerminalSize,
    pub scrollback: usize,
    pub cursor_shape: CursorShape,
}

/// The handle the session layer feeds from any thread. Cloning is cheap;
/// the parser thread ends when the view and every handle are gone.
#[derive(Clone)]
pub struct TerminalHandle {
    term: SharedTerm,
    queue: async_channel::Sender<Message>,
    outbox: Arc<Outbox>,
    /// The scrollback limit (lines, the journal's included).
    scrollback: usize,
}

impl std::fmt::Debug for TerminalHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalHandle").finish_non_exhaustive()
    }
}

impl TerminalHandle {
    /// Creates the emulator and starts its parser thread. `wake` receives
    /// a value (coalesced) whenever the screen or the pending events changed.
    pub(super) fn spawn(
        options: ModelOptions,
        palette: Palette,
        wake: async_channel::Sender<()>,
    ) -> Self {
        let outbox = Arc::new(Outbox {
            sink: RwLock::new(None),
            pending: Mutex::new(PendingEvents::default()),
            color_requests: Mutex::new(Vec::new()),
            palette: RwLock::new(palette),
            wake,
            size: Mutex::new(options.size),
            columns: Mutex::new((None, None)),
            evicted: AtomicUsize::new(0),
            journal: Arc::new(Mutex::new(Journal::default())),
            selection_moved: Default::default(),
            capture: Default::default(),
            autocomplete: Default::default(),
        });
        let config = term_config(options.scrollback, options.cursor_shape);
        let dims = GridDims::from(options.size);
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &dims,
            Listener(outbox.clone()),
        )));
        let (queue, receiver) = async_channel::bounded(QUEUE_CHUNKS);
        let thread_term = term.clone();
        let thread_outbox = outbox.clone();
        let scrollback = options.scrollback;
        let spawned = thread::Builder::new()
            .name("shellpp-term-parse".into())
            .spawn(move || parse_loop(thread_term, receiver, thread_outbox, scrollback));
        if spawned.is_err() {
            tracing::error!("the terminal parser thread did not start");
        }
        Self {
            term,
            queue,
            outbox,
            scrollback,
        }
    }

    pub(super) fn term(&self) -> &SharedTerm {
        &self.term
    }

    pub(super) fn outbox(&self) -> &Arc<Outbox> {
        &self.outbox
    }

    /// Feeds output of the remote side (UTF-8). Waits while the queue is
    /// full (flow control); works on any async runtime.
    pub async fn feed(&self, bytes: &[u8]) -> Result<(), FeedError> {
        for chunk in bytes.chunks(CHUNK_BYTES) {
            self.queue
                .send(Message::Bytes(chunk.to_vec()))
                .await
                .map_err(|_| FeedError::Closed)?;
        }
        Ok(())
    }

    /// [`Self::feed`] for a plain thread (for example a PTY reader): blocks
    /// while the queue is full. Never call it on the UI thread.
    pub fn feed_blocking(&self, bytes: &[u8]) -> Result<(), FeedError> {
        for chunk in bytes.chunks(CHUNK_BYTES) {
            self.queue
                .send_blocking(Message::Bytes(chunk.to_vec()))
                .map_err(|_| FeedError::Closed)?;
        }
        Ok(())
    }

    /// Feeds without waiting; [`FeedError::Full`] when the queue is full.
    /// Bytes larger than one chunk are refused as a whole unless they fit.
    pub fn try_feed(&self, bytes: &[u8]) -> Result<(), FeedError> {
        if bytes.len() > CHUNK_BYTES {
            let free = QUEUE_CHUNKS.saturating_sub(self.queue.len());
            if bytes.len().div_ceil(CHUNK_BYTES) > free {
                return Err(FeedError::Full);
            }
        }
        for chunk in bytes.chunks(CHUNK_BYTES) {
            self.queue
                .try_send(Message::Bytes(chunk.to_vec()))
                .map_err(|error| match error {
                    async_channel::TrySendError::Full(_) => FeedError::Full,
                    async_channel::TrySendError::Closed(_) => FeedError::Closed,
                })?;
        }
        Ok(())
    }

    /// Blocks until everything fed so far is parsed (tests, benchmarks).
    pub fn flush_blocking(&self) {
        let (done, wait) = mpsc::channel();
        if self.queue.send_blocking(Message::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }

    /// Closes the terminal (its view is gone): the sink is removed (which
    /// also ends a reference cycle when the sink holds a handle), feeding
    /// returns [`FeedError::Closed`], and the parser thread ends once it has
    /// parsed what is queued.
    pub fn close(&self) {
        self.set_sink(None);
        self.outbox.capture.set_sink(None);
        self.queue.close();
    }

    /// Whether [`Self::close`] was called.
    pub fn is_closed(&self) -> bool {
        self.queue.is_closed()
    }

    /// Sets (or removes) where input and size changes go. A new sink gets
    /// the current size right away.
    pub fn set_sink(&self, sink: Option<Arc<dyn TerminalSink>>) {
        let size = self.size();
        if let Some(sink) = &sink {
            sink.resize(size);
        }
        if let Ok(mut slot) = self.outbox.sink.write() {
            *slot = sink;
        }
    }

    /// Sends bytes to the sink as if typed (snippets, tests).
    pub fn send_input(&self, bytes: Vec<u8>) {
        self.outbox.send(bytes);
    }

    /// The current grid size.
    pub fn size(&self) -> TerminalSize {
        self.outbox
            .size
            .lock()
            .map(|size| *size)
            .unwrap_or_default()
    }

    /// Resizes the grid for the view's size and tells the sink. Returns
    /// whether it changed. A width a DECCOLM set (D90) stays until the
    /// view's own size changes.
    pub(super) fn resize(&self, size: TerminalSize) -> bool {
        let size = {
            let Ok(mut columns) = self.outbox.columns.lock() else {
                return false;
            };
            let (forced, natural) = &mut *columns;
            if natural.is_some_and(|natural| (natural.cols, natural.rows) != (size.cols, size.rows))
            {
                *forced = None;
            }
            *natural = Some(size);
            with_columns(size, *forced)
        };
        apply_size(&self.term, &self.outbox, self.scrollback, size)
    }

    pub(super) fn set_palette(&self, palette: Palette) {
        if let Ok(mut current) = self.outbox.palette.write() {
            *current = palette;
        }
    }

    /// The visible screen as text, one string per row with trailing blanks
    /// trimmed (tests and diagnostics; never logged).
    pub fn screen_text(&self) -> Vec<String> {
        use super::journal::Buffer as _;
        let term = self.term.lock();
        let journal = self.outbox.journal();
        let lines = Lines::new(&term, &journal);
        let top = lines.view_top();
        (0..lines.rows())
            .map(|line| {
                let row = lines.row(top + line);
                let mut text = String::new();
                for column in 0..lines.cols() {
                    let cell = &row[Column(column)];
                    if cell
                        .flags
                        .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
                    {
                        continue;
                    }
                    text.push(cell.c);
                    if let Some(extra) = cell.zerowidth() {
                        text.extend(extra.iter().take(MAX_ZEROWIDTH));
                    }
                }
                text.trim_end().to_owned()
            })
            .collect()
    }

    /// Lines in the scrollback (above the screen), the journal's included.
    pub fn history_size(&self) -> usize {
        let term = self.term.lock();
        term.grid().history_size() + self.outbox.journal().len()
    }

    /// Bytes the scrollback and the screen take: the grid's rows (each as
    /// wide as the window) and the journal (tests and benchmarks). Rows
    /// alacritty keeps for reuse are not counted; a compaction frees them.
    pub fn buffer_bytes(&self) -> usize {
        let term = self.term.lock();
        let grid = term.grid();
        (grid.history_size() + grid.screen_lines())
            * grid.columns()
            * size_of::<alacritty_terminal::term::cell::Cell>()
            + self.outbox.journal().bytes()
    }

    /// Scrolls the view: the scrollback and its journal together.
    pub(super) fn scroll(&self, scroll: Scroll) {
        let mut term = self.term.lock();
        journal::scroll(&mut term, &mut self.outbox.journal(), scroll);
    }

    /// Back to the bottom of the scrollback (typed input, a paste).
    pub(super) fn scroll_to_bottom(&self) {
        let mut term = self.term.lock();
        let mut journal = self.outbox.journal();
        let alt = term.mode().contains(TermMode::ALT_SCREEN);
        if term.grid().display_offset() != 0 || (!alt && journal.view() != 0) {
            journal::scroll(&mut term, &mut journal, Scroll::Bottom);
        }
    }

    /// Whether the view is scrolled up from the bottom (into the
    /// scrollback or its journal).
    pub(super) fn scrolled_back(&self) -> bool {
        let term = self.term.lock();
        let alt = term.mode().contains(TermMode::ALT_SCREEN);
        term.grid().display_offset() != 0 || (!alt && self.outbox.journal().view() != 0)
    }

    /// Rows the view is scrolled up from the bottom (tests).
    #[cfg(test)]
    pub(super) fn view_offset(&self) -> usize {
        let term = self.term.lock();
        use super::journal::Buffer as _;
        Lines::new(&term, &self.outbox.journal()).view_offset()
    }

    /// Runs `read` on the buffer of `term` (this handle's terminal,
    /// locked): the journal and the grid.
    pub(super) fn with_lines<R>(
        &self,
        term: &Term<Listener>,
        read: impl FnOnce(&Lines<'_>) -> R,
    ) -> R {
        let journal = self.outbox.journal();
        read(&Lines::new(term, &journal))
    }

    /// Lines dropped off the top of the scrollback so far (a full
    /// scrollback that scrolls on, or a cleared one). Read it while holding
    /// the terminal lock to match the grid.
    pub(super) fn evicted_lines(&self) -> usize {
        self.outbox.evicted.load(Ordering::Acquire)
    }

    /// Where the buffer of `term` (this handle's terminal, locked) stands,
    /// for the search.
    pub(super) fn search_origin(&self, term: &Term<Listener>) -> super::search::Origin {
        super::search::Origin {
            evicted: self.evicted_lines(),
            alt_screen: term.mode().contains(TermMode::ALT_SCREEN),
        }
    }
}

/// Waits for the next message; `None` when `deadline` passed first.
fn receive_until(
    receiver: &async_channel::Receiver<Message>,
    deadline: Instant,
) -> Option<Result<Message, async_channel::RecvError>> {
    loop {
        match receiver.try_recv() {
            Ok(message) => return Some(Ok(message)),
            Err(async_channel::TryRecvError::Closed) => {
                return Some(Err(async_channel::RecvError));
            }
            Err(async_channel::TryRecvError::Empty) => {}
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        thread::sleep(SYNC_POLL.min(deadline - now));
    }
}

fn parse_loop(
    term: SharedTerm,
    receiver: async_channel::Receiver<Message>,
    outbox: Arc<Outbox>,
    scrollback: usize,
) {
    limits::count_evictions(scrollback, outbox.journal.clone());
    // The scrollback beyond the newest lines moves to the journal after a
    // pause in the output, now and then during a long run of it, and before
    // a flush answers (D85).
    let mut compacted = Instant::now();
    let mut compact_at: Option<Instant> = None;
    let compact = |outbox: &Outbox| {
        let mut locked = term.lock();
        let had_selection = locked.selection.is_some();
        let (moved, dropped) = journal::compact(&mut locked, &mut outbox.journal(), scrollback);
        limits::journal_changed(moved, dropped);
        outbox.evicted.store(limits::evictions(), Ordering::Release);
        if had_selection && locked.selection.is_none() {
            outbox.selection_moved.store(true, Ordering::Release);
        }
    };
    // Published before the lock is let go (see `Outbox::evicted`).
    let publish = |outbox: &Outbox| {
        outbox.evicted.store(limits::evictions(), Ordering::Release);
    };
    let mut processor: Processor<StdSyncHandler> = Processor::new();
    let mut osc = OscLimiter::default();
    let mut batch: Vec<u8> = Vec::with_capacity(CHUNK_BYTES);
    let mut flushes: Vec<mpsc::Sender<()>> = Vec::new();
    loop {
        let sync = processor.sync_timeout().sync_timeout();
        let deadline = match (sync, compact_at) {
            (Some(sync), Some(compact)) => Some(sync.min(compact)),
            (sync, compact) => sync.or(compact),
        };
        let first = match deadline {
            None => receiver.recv_blocking(),
            Some(deadline) => match receive_until(&receiver, deadline) {
                Some(message) => message,
                None => {
                    let now = Instant::now();
                    if sync.is_some_and(|sync| now >= sync) {
                        // The synchronized update timed out: show what it
                        // has.
                        let mut locked = term.lock();
                        limits::set_forced_columns(outbox.forced_columns());
                        processor.stop_sync(&mut Capped(&mut *locked));
                        let columns = limits::take_column_change();
                        publish(&outbox);
                        drop(locked);
                        if let Some(forced) = columns {
                            columns_changed(&term, &outbox, scrollback, forced);
                        }
                        outbox.wake();
                    }
                    if compact_at.is_some_and(|at| now >= at) {
                        compact_at = None;
                        compact(&outbox);
                        compacted = now;
                    }
                    continue;
                }
            },
        };
        let Ok(first) = first else {
            break;
        };
        batch.clear();
        let mut take = |message: Message, batch: &mut Vec<u8>| match message {
            Message::Bytes(bytes) => batch.extend_from_slice(&bytes),
            Message::Flush(done) => flushes.push(done),
        };
        take(first, &mut batch);
        while batch.len() < BATCH_BYTES {
            match receiver.try_recv() {
                Ok(message) => take(message, &mut batch),
                Err(_) => break,
            }
        }
        for slice in batch.chunks(LOCKED_BYTES) {
            let slice = osc.filter(slice);
            let mut locked = term.lock();
            limits::set_forced_columns(outbox.forced_columns());
            if outbox.capture.watching() {
                // A command line waits for its end (history capture).
                super::history_capture::advance(
                    &outbox.capture,
                    &mut processor,
                    &mut locked,
                    &slice,
                );
            } else {
                processor.advance(&mut Capped(&mut *locked), &slice);
            }
            answer_color_requests(&locked, &outbox);
            // The screen now shows what was typed before (its echo).
            outbox.autocomplete.output();
            let columns = limits::take_column_change();
            publish(&outbox);
            drop(locked);
            if let Some(forced) = columns {
                columns_changed(&term, &outbox, scrollback, forced);
            }
            outbox.wake();
        }
        let history = term.lock().grid().history_size();
        if history > LIVE_HISTORY {
            let long_run =
                history > LIVE_HISTORY + COMPACT_BATCH && compacted.elapsed() >= COMPACT_INTERVAL;
            if long_run || !flushes.is_empty() {
                compact(&outbox);
                compacted = Instant::now();
                compact_at = None;
            } else {
                compact_at = Some(Instant::now() + COMPACT_QUIET);
            }
        }
        if batch.capacity() > BATCH_BYTES * 2 {
            batch = Vec::with_capacity(CHUNK_BYTES);
        }
        for done in flushes.drain(..) {
            let _ = done.send(());
        }
    }
    for done in flushes.drain(..) {
        let _ = done.send(());
    }
}

/// Answers OSC 4/10/11/12 color queries with the remote overrides or the
/// theme colors.
fn answer_color_requests(term: &Term<Listener>, outbox: &Outbox) {
    let requests = match outbox.color_requests.lock() {
        Ok(mut requests) if !requests.is_empty() => std::mem::take(&mut *requests),
        _ => return,
    };
    let palette = outbox.palette();
    for (index, format) in requests {
        let color = palette.table_entry(index, term.colors());
        outbox.send(format(color).into_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::limits::MAX_ZEROWIDTH;
    use crate::theme::terminal::builtin;

    /// Records what the terminal sends.
    #[derive(Default)]
    pub(crate) struct RecordingSink {
        pub sent: Mutex<Vec<u8>>,
        pub sizes: Mutex<Vec<TerminalSize>>,
    }

    impl TerminalSink for RecordingSink {
        fn send(&self, bytes: Vec<u8>) {
            self.sent.lock().unwrap().extend_from_slice(&bytes);
        }

        fn resize(&self, size: TerminalSize) {
            self.sizes.lock().unwrap().push(size);
        }
    }

    fn handle(
        cols: u16,
        rows: u16,
        scrollback: usize,
    ) -> (TerminalHandle, async_channel::Receiver<()>) {
        let (wake, woken) = async_channel::bounded(1);
        let palette = Palette::from_theme(&builtin()[0]);
        let handle = TerminalHandle::spawn(
            ModelOptions {
                size: TerminalSize::cells(cols, rows),
                scrollback,
                cursor_shape: CursorShape::Block,
            },
            palette,
            wake,
        );
        (handle, woken)
    }

    #[test]
    fn feeds_parse_on_the_parser_thread_and_wake_the_ui() {
        let (handle, woken) = handle(20, 4, 100);
        handle.feed_blocking(b"hello\r\n\x1b[1mworld").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.screen_text()[..2], ["hello", "world"]);
        assert!(woken.try_recv().is_ok(), "the UI was woken");
        assert!(woken.try_recv().is_err(), "wakes are coalesced");
    }

    #[test]
    fn replies_go_to_the_sink_and_titles_are_sanitized() {
        let (handle, _woken) = handle(20, 4, 100);
        let sink = Arc::new(RecordingSink::default());
        handle.set_sink(Some(sink.clone()));
        assert_eq!(sink.sizes.lock().unwrap()[..], [TerminalSize::cells(20, 4)]);
        // Cursor position report and an OSC 11 background query.
        handle.feed_blocking(b"ab\x1b[6n\x1b]11;?\x07").unwrap();
        // A title with control and bidi characters.
        handle.feed_blocking(b"\x1b]2;bad\x1b\\").unwrap();
        handle
            .feed_blocking("\x1b]0;ti\u{202E}tle\x07".as_bytes())
            .unwrap();
        handle.flush_blocking();
        let sent = String::from_utf8(sink.sent.lock().unwrap().clone()).unwrap();
        assert!(sent.starts_with("\x1b[1;3R"), "{sent:?}");
        assert!(sent.contains("\x1b]11;rgb:"), "{sent:?}");
        let events = handle.outbox().take_events();
        assert_eq!(events.title, Some(Some("title".to_owned())));
        let long: String = "x".repeat(400);
        assert_eq!(sanitize_title(&long).chars().count(), TITLE_MAX_CHARS);
    }

    #[test]
    fn osc_52_and_size_reports_are_ignored() {
        let (handle, _woken) = handle(20, 4, 100);
        let sink = Arc::new(RecordingSink::default());
        handle.set_sink(Some(sink.clone()));
        // Clipboard write, clipboard read and a text area size query.
        handle
            .feed_blocking(b"\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07\x1b[14t")
            .unwrap();
        handle.flush_blocking();
        assert!(sink.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn scrollback_is_capped() {
        let (handle, _woken) = handle(20, 10, 10_000);
        let mut text = String::new();
        for line in 0..20_000 {
            text.push_str(&format!("line {line}\r\n"));
        }
        handle.feed_blocking(text.as_bytes()).unwrap();
        handle.flush_blocking();
        assert_eq!(handle.history_size(), 10_000);
    }

    /// Endless combining marks on one character stay at the cap (the cell
    /// would otherwise grow by every mark the output carries).
    #[test]
    fn a_zero_width_flood_stays_capped() {
        let (handle, _woken) = handle(20, 4, 100);
        handle.feed_blocking(b"a").unwrap();
        let flood = "\u{301}".repeat(1024 * 1024);
        for _ in 0..16 {
            // 16 x 2 MiB = 32 MiB of U+0301.
            handle.feed_blocking(flood.as_bytes()).unwrap();
        }
        handle.feed_blocking(b"b").unwrap();
        handle.flush_blocking();
        let expected = format!("a{}b", "\u{301}".repeat(MAX_ZEROWIDTH));
        assert_eq!(handle.screen_text()[0], expected);
    }

    /// An OSC string without an end keeps at most `MAX_OSC_PAYLOAD` bytes;
    /// its terminator and the output after it still work.
    #[test]
    fn an_endless_osc_string_is_cut() {
        let (handle, _woken) = handle(20, 4, 100);
        handle.feed_blocking(b"\x1b]2;").unwrap();
        let payload = vec![b'a'; 1024 * 1024];
        for _ in 0..64 {
            handle.feed_blocking(&payload).unwrap();
        }
        handle.feed_blocking(b"\x07after").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.screen_text()[0], "after");
        let events = handle.outbox().take_events();
        assert_eq!(events.title, Some(Some("a".repeat(TITLE_MAX_CHARS))));
    }

    /// The mode reset a reconnect feeds before the new shell's output
    /// (sessions::MODE_RESET) really leaves what a full-screen program set
    /// up: the alternate screen (so output reaches the scrollback again and
    /// the wheel scrolls), mouse tracking, bracketed paste, insert mode and
    /// the cursor shape.
    #[test]
    fn the_session_mode_reset_returns_to_the_normal_screen() {
        use alacritty_terminal::term::TermMode;

        let (handle, _woken) = handle(20, 4, 100);
        handle.feed_blocking(b"before\r\n").unwrap();
        // htop or vim: alternate screen, SGR mouse, bracketed paste,
        // insert mode, a bar cursor.
        handle
            .feed_blocking(
                b"\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[4h\x1b[5 qfull screen",
            )
            .unwrap();
        handle.flush_blocking();
        assert!(handle.term().lock().mode().contains(TermMode::ALT_SCREEN));

        handle.feed_blocking(crate::sessions::MODE_RESET).unwrap();
        for line in 0..10 {
            handle
                .feed_blocking(format!("line {line}\r\n").as_bytes())
                .unwrap();
        }
        handle.flush_blocking();
        let term = handle.term().lock();
        let mode = *term.mode();
        for left in [
            TermMode::ALT_SCREEN,
            TermMode::MOUSE_MODE,
            TermMode::SGR_MOUSE,
            TermMode::BRACKETED_PASTE,
            TermMode::INSERT,
        ] {
            assert!(!mode.intersects(left), "{left:?} is still set");
        }
        assert_eq!(term.cursor_style().shape, CursorShape::Block);
        drop(term);
        assert!(handle.history_size() >= 7, "output reaches the scrollback");
        assert!(
            !handle
                .screen_text()
                .iter()
                .any(|row| row.contains("full screen"))
        );
    }

    #[test]
    fn try_feed_reports_a_full_queue() {
        let (handle, _woken) = handle(20, 4, 100);
        // Larger than the whole queue can ever hold.
        let big = vec![b'x'; CHUNK_BYTES * (QUEUE_CHUNKS + 1)];
        assert_eq!(handle.try_feed(&big), Err(FeedError::Full));
        assert_eq!(handle.try_feed(b"ok"), Ok(()));
    }

    #[test]
    fn close_drops_the_sink_and_refuses_more_output() {
        let (handle, _woken) = handle(20, 4, 100);
        let sink = Arc::new(RecordingSink::default());
        handle.set_sink(Some(sink.clone()));
        handle.feed_blocking(b"before").unwrap();
        handle.close();
        assert!(handle.is_closed());
        assert_eq!(handle.feed_blocking(b"after"), Err(FeedError::Closed));
        assert_eq!(handle.try_feed(b"after"), Err(FeedError::Closed));
        // Flushing a closed terminal returns at once.
        handle.flush_blocking();
        handle.send_input(b"typed".to_vec());
        assert!(sink.sent.lock().unwrap().is_empty(), "no sink after close");
        assert_eq!(Arc::strong_count(&sink), 1, "the sink was released");
    }

    /// DECCOLM (D90): only after DECSET 40, as in xterm; the grid takes 80
    /// or 132 columns at once (the text after it is laid out in them) and
    /// the sink is told; the width stays while the view's size does not
    /// change, and a new view size or RIS ends it.
    #[test]
    fn deccolm_switches_columns_only_when_allowed() {
        let (handle, _woken) = handle(100, 6, 100);
        let sink = Arc::new(RecordingSink::default());
        handle.set_sink(Some(sink.clone()));
        let natural = TerminalSize::cells(100, 6);
        assert!(!handle.resize(natural));
        // `tput reset` sends DECRST 3 without DECSET 40: the width stays.
        handle.feed_blocking(b"\x1b[?3;4l").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 100);
        // vttest: DECSET 40, then DECRST 3 and a line as wide as the screen.
        let line = "*".repeat(80);
        handle
            .feed_blocking(format!("\x1b[?40h\x1b[?3l{line}x").as_bytes())
            .unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size(), TerminalSize::cells(80, 6));
        assert_eq!(
            sink.sizes.lock().unwrap().last(),
            Some(&TerminalSize::cells(80, 6))
        );
        // The 81st character wrapped at column 80.
        assert_eq!(handle.screen_text()[..2], [line.clone(), "x".to_owned()]);
        // DECSET 3: 132 columns.
        handle.feed_blocking(b"\x1b[?3h").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 132);
        // The view's size again (every frame): the width stays.
        assert!(!handle.resize(natural));
        assert_eq!(handle.size().cols, 132);
        // A new view size ends it.
        assert!(handle.resize(TerminalSize::cells(110, 6)));
        assert_eq!(handle.size().cols, 110);
        // RIS ends a forced width and the permission.
        handle.feed_blocking(b"\x1b[?40h\x1b[?3l").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 80);
        handle.feed_blocking(b"\x1bc").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 110);
        handle.feed_blocking(b"\x1b[?3l").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 110, "not allowed after RIS");
        // DECRST 40 ends a forced width too (a reconnect sends it).
        handle.feed_blocking(b"\x1b[?40h\x1b[?3l").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 80);
        handle.feed_blocking(b"\x1b[?40l").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 110);
        // DECCOLM and RIS in one write: the grid is back at the view's width
        // (review, D90).
        handle.feed_blocking(b"\x1b[?40h\x1b[?3h\x1bc").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 110);
        assert_eq!(handle.term().lock().columns(), 110);
    }

    /// A DECCOLM keeps the scrollback: a wider reflow joins wrapped rows,
    /// which is no clear of the journal (review, D90).
    #[test]
    fn deccolm_keeps_the_journal() {
        let (handle, _woken) = handle(100, 6, 2000);
        handle.set_sink(Some(Arc::new(RecordingSink::default())));
        assert!(!handle.resize(TerminalSize::cells(100, 6)));
        // 800 lines, then a line that wraps at 100 columns among the grid's
        // own scrollback (the newest 500 lines), then a few more.
        let mut output = String::new();
        for line in 0..800 {
            output.push_str(&format!("line {line}\r\n"));
        }
        output.push_str(&format!("{}\r\n", "w".repeat(120)));
        for line in 0..20 {
            output.push_str(&format!("after {line}\r\n"));
        }
        handle.feed_blocking(output.as_bytes()).unwrap();
        handle.flush_blocking();
        let journal = handle.outbox().journal().len();
        assert!(journal > 0, "the old lines are in the journal");
        let before = handle.history_size();
        handle.feed_blocking(b"\x1b[?40h\x1b[?3h").unwrap();
        handle.flush_blocking();
        assert_eq!(handle.size().cols, 132);
        assert_eq!(handle.outbox().journal().len(), journal);
        assert!(handle.history_size() + 1 >= before, "no line was lost");
    }

    #[test]
    fn resize_changes_the_grid_and_tells_the_sink() {
        let (handle, _woken) = handle(20, 4, 100);
        let sink = Arc::new(RecordingSink::default());
        handle.set_sink(Some(sink.clone()));
        let size = TerminalSize {
            cols: 30,
            rows: 6,
            pixel_width: 252,
            pixel_height: 102,
        };
        assert!(handle.resize(size));
        assert!(!handle.resize(size));
        assert_eq!(handle.size(), size);
        assert_eq!(handle.screen_text().len(), 6);
        assert_eq!(sink.sizes.lock().unwrap().last(), Some(&size));
    }
}
