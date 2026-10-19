//! Terminal output of one session (stage 2 plan, decision 3; docs/architecture.md,
//! section 3.5): a replay ring with the last [`REPLAY_BYTES`] of output and at
//! most one subscriber (the Tauri `Channel` of the pane that shows the
//! session).
//!
//! - Output produced before `session_attach` or during a window reload goes
//!   to the ring only; `session_attach` replays the ring first, then streams
//!   live output, so nothing is lost or duplicated.
//! - A new attach replaces the previous subscriber.
//! - Live output is coalesced: a frame is sent at once when the previous one
//!   left at least [`COALESCE_INTERVAL`] ago (interactive echo has no added
//!   latency), otherwise the caller flushes after the remaining time; a
//!   frame never exceeds [`MAX_FRAME_BYTES`].
//! - Flow control: the pane acknowledges the bytes the terminal has
//!   processed (`session_ack`). While more than [`PAUSE_BYTES`] sent to the
//!   subscriber are unacknowledged, [`OutputHub::paused`] is true and the
//!   session stops reading output, so the SSH channel or the PTY pushes back
//!   on the program that writes; reading resumes below [`RESUME_BYTES`].
//!   Without a subscriber output is never paused (it goes to the ring).
//! - The replay never makes the terminal answer again: queries an
//!   application sent earlier (cursor position, device attributes, colors)
//!   that a pane already received are removed from the replayed bytes
//!   ([`replay_bytes`]), so a reloaded pane does not send stale replies to
//!   whatever runs now. Output no pane has received yet is replayed as it
//!   is: its queries still wait for an answer (a local shell in a
//!   pseudo-console asks for the cursor position before its first prompt).
//!
//! Output bytes are terminal data: never logged.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// Bytes of output kept for a replay.
pub const REPLAY_BYTES: usize = 256 * 1024;
/// Largest frame sent through the channel.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Shortest interval between two frames (unless a frame is full).
pub const COALESCE_INTERVAL: Duration = Duration::from_millis(8);
/// Unacknowledged bytes above which the session stops reading output.
pub const PAUSE_BYTES: u64 = 2 * 1024 * 1024;
/// Unacknowledged bytes at or below which reading resumes.
pub const RESUME_BYTES: u64 = 512 * 1024;

/// Where output frames go: the Tauri channel of the attached pane, or a test
/// collector.
pub trait OutputSink: Send + Sync + 'static {
    /// Sends one frame (raw bytes). `false` when the receiver is gone.
    fn send(&self, frame: Vec<u8>) -> bool;
}

/// Replay ring, subscriber, coalescing and flow control state.
pub struct OutputHub {
    inner: Mutex<HubInner>,
    /// Woken when reading may resume.
    resume: Notify,
}

struct HubInner {
    ring: VecDeque<u8>,
    sink: Option<Box<dyn OutputSink>>,
    pending: Vec<u8>,
    last_frame: Option<Instant>,
    /// Bytes sent to the subscriber and not acknowledged yet.
    unacked: u64,
    paused: bool,
    /// Bytes at the end of the ring that no subscriber has received.
    undelivered: usize,
}

impl HubInner {
    /// Forgets the flow control state (no subscriber, or a new one).
    fn reset_credit(&mut self) -> bool {
        self.unacked = 0;
        std::mem::replace(&mut self.paused, false)
    }

    fn sent(&mut self, bytes: usize) {
        self.unacked = self
            .unacked
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        if self.unacked > PAUSE_BYTES {
            self.paused = true;
        }
    }
}

impl std::fmt::Debug for OutputHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("OutputHub")
            .field("buffered", &inner.ring.len())
            .field("attached", &inner.sink.is_some())
            .field("paused", &inner.paused)
            .finish()
    }
}

impl Default for OutputHub {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HubInner {
                ring: VecDeque::new(),
                sink: None,
                pending: Vec::new(),
                last_frame: None,
                unacked: 0,
                paused: false,
                undelivered: 0,
            }),
            resume: Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// New output. Returns the time after which the caller must call
    /// [`OutputHub::flush`] when part of the output is held back.
    pub fn push(&self, data: &[u8]) -> Option<Duration> {
        self.push_at(data, Instant::now())
    }

    fn push_at(&self, data: &[u8], now: Instant) -> Option<Duration> {
        if data.is_empty() {
            return None;
        }
        let mut inner = self.lock();
        append_ring(&mut inner.ring, data);
        if inner.sink.is_none() {
            inner.undelivered = (inner.undelivered + data.len()).min(inner.ring.len());
            return None;
        }
        inner.pending.extend_from_slice(data);
        let due = match inner.last_frame {
            Some(last) => now.saturating_duration_since(last) >= COALESCE_INTERVAL,
            None => true,
        };
        if due || inner.pending.len() >= MAX_FRAME_BYTES {
            send_pending(&mut inner, now);
            return None;
        }
        let elapsed = inner.last_frame.map_or(COALESCE_INTERVAL, |last| {
            now.saturating_duration_since(last)
        });
        Some(COALESCE_INTERVAL.saturating_sub(elapsed))
    }

    /// Sends output held back by [`OutputHub::push`].
    pub fn flush(&self) {
        let mut inner = self.lock();
        let was_paused = inner.paused;
        send_pending(&mut inner, Instant::now());
        if was_paused && !inner.paused {
            self.resume.notify_one();
        }
    }

    /// Replays the ring to `sink` and makes it the subscriber (replacing the
    /// previous one). Held-back output is part of the replay; the replay
    /// counts as sent for flow control.
    pub fn attach(&self, sink: Box<dyn OutputSink>) {
        let mut inner = self.lock();
        // Held-back output never reached the previous subscriber.
        let fresh = (inner.undelivered + inner.pending.len()).min(inner.ring.len());
        inner.pending.clear();
        inner.sink = None;
        let was_paused = inner.reset_credit();
        let replay = replay_bytes(&inner.ring, fresh);
        inner.undelivered = 0;
        for frame in replay.chunks(MAX_FRAME_BYTES) {
            if !sink.send(frame.to_vec()) {
                break;
            }
            inner.sent(frame.len());
        }
        inner.sink = Some(sink);
        inner.last_frame = Some(Instant::now());
        if was_paused && !inner.paused {
            self.resume.notify_one();
        }
    }

    /// Drops the subscriber (session closed, window reloading).
    pub fn detach(&self) {
        let mut inner = self.lock();
        inner.sink = None;
        inner.undelivered = (inner.undelivered + inner.pending.len()).min(inner.ring.len());
        inner.pending.clear();
        if inner.reset_credit() {
            self.resume.notify_one();
        }
    }

    /// The subscriber processed `bytes` more bytes of output.
    pub fn ack(&self, bytes: u64) {
        let mut inner = self.lock();
        inner.unacked = inner.unacked.saturating_sub(bytes);
        if inner.paused && inner.unacked <= RESUME_BYTES {
            inner.paused = false;
            self.resume.notify_one();
        }
    }

    /// Too much output is unacknowledged: stop reading until
    /// [`OutputHub::resumed`].
    pub fn paused(&self) -> bool {
        self.lock().paused
    }

    /// Completes when reading may resume (check [`OutputHub::paused`] again).
    pub async fn resumed(&self) {
        self.resume.notified().await;
    }

    /// Bytes in the replay ring.
    pub fn buffered(&self) -> usize {
        self.lock().ring.len()
    }
}

fn append_ring(ring: &mut VecDeque<u8>, data: &[u8]) {
    if data.len() >= REPLAY_BYTES {
        ring.clear();
        ring.extend(&data[data.len() - REPLAY_BYTES..]);
        return;
    }
    let overflow = (ring.len() + data.len()).saturating_sub(REPLAY_BYTES);
    ring.drain(..overflow);
    ring.extend(data);
}

/// The ring without a partial UTF-8 sequence at its start (the ring may
/// have cut one) and without the terminal queries ([`strip_queries`]) of
/// the part a subscriber received before; the last `fresh` bytes (never
/// received) are kept as they are.
fn replay_bytes(ring: &VecDeque<u8>, fresh: usize) -> Vec<u8> {
    let bytes: Vec<u8> = ring.iter().copied().collect();
    let skip = bytes
        .iter()
        .take(3)
        .take_while(|byte| (**byte & 0b1100_0000) == 0b1000_0000)
        .count();
    let split = bytes.len().saturating_sub(fresh).max(skip);
    let mut out = strip_queries(&bytes[skip..split]);
    out.extend_from_slice(&bytes[split..]);
    out
}

/// Longest escape sequence that is examined; longer ones are kept as they are.
const MAX_SEQUENCE: usize = 4096;

/// Removes the sequences a terminal answers (through `onData`) from replayed
/// output: device status and cursor position reports (`CSI n`, `CSI ? n`),
/// device attributes (`CSI c`, `CSI > c`, `CSI = c`), version and mode
/// requests (`CSI > q`, `CSI ? u`, DECRQM `CSI ... $ p`), window reports
/// (`CSI t`), DECRQSS and other DCS requests, and OSC color queries (OSC 4,
/// 10-19, 104... with `?`). Everything else, including incomplete
/// sequences, is kept.
pub fn strip_queries(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b || index + 1 >= bytes.len() {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        let rest = &bytes[index..];
        let (length, query) = match rest[1] {
            b'[' => csi_sequence(rest),
            b']' => string_sequence(rest, true),
            b'P' => string_sequence(rest, false),
            _ => (0, false),
        };
        if length == 0 {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        if !query {
            out.extend_from_slice(&rest[..length]);
        }
        index += length;
    }
    out
}

/// Length of the CSI sequence at the start of `seq` (0 when incomplete or
/// too long) and whether the terminal answers it.
fn csi_sequence(seq: &[u8]) -> (usize, bool) {
    let mut end = 2;
    while end < seq.len() && end < MAX_SEQUENCE {
        let byte = seq[end];
        if (0x40..=0x7e).contains(&byte) {
            let body = &seq[2..end];
            let prefix = body.first().copied();
            let intermediate = body.last().copied();
            let query = match byte {
                // DSR / cursor position report; device attributes.
                b'n' | b'c' => true,
                // Window and text area reports (xterm `CSI 1x t`, `CSI 2x t`).
                b't' => body
                    .split(|b| *b == b';')
                    .next()
                    .is_some_and(|first| first.len() == 2 && matches!(first[0], b'1' | b'2')),
                // XTVERSION `CSI > q`.
                b'q' => prefix == Some(b'>'),
                // Keyboard protocol query `CSI ? u`.
                b'u' => prefix == Some(b'?'),
                // DECRQM `CSI Ps $ p` / `CSI ? Ps $ p`.
                b'p' => intermediate == Some(b'$'),
                _ => false,
            };
            return (end + 1, query);
        }
        if !(0x20..=0x3f).contains(&byte) {
            // Not a CSI sequence after all (a control character inside).
            return (0, false);
        }
        end += 1;
    }
    (0, false)
}

/// Length of the OSC (`osc`) or DCS sequence at the start of `seq`,
/// terminated by BEL (OSC only) or ST (`ESC \`); 0 when incomplete or too
/// long. OSC color queries and every DCS request (DECRQSS, XTGETTCAP) are
/// answered by the terminal.
fn string_sequence(seq: &[u8], osc: bool) -> (usize, bool) {
    let mut end = 2;
    while end < seq.len() && end < MAX_SEQUENCE {
        let byte = seq[end];
        let terminator = if osc && byte == 0x07 {
            Some(end + 1)
        } else if byte == 0x1b && seq.get(end + 1) == Some(&b'\\') {
            Some(end + 2)
        } else {
            None
        };
        if let Some(length) = terminator {
            let body = &seq[2..end];
            let query = if osc {
                osc_query(body)
            } else {
                body.starts_with(b"$q") || body.starts_with(b"+q")
            };
            return (length, query);
        }
        end += 1;
    }
    (0, false)
}

/// OSC 4 (palette), 10-19 (dynamic colors) and 5 (special colors) with a
/// `?` argument ask for a color.
fn osc_query(body: &[u8]) -> bool {
    let mut parts = body.split(|b| *b == b';');
    let Some(number) = parts.next() else {
        return false;
    };
    let colors = matches!(number, b"4" | b"5")
        || (number.len() == 2 && number[0] == b'1' && number[1].is_ascii_digit());
    colors && parts.any(|part| part == b"?")
}

fn send_pending(inner: &mut HubInner, now: Instant) {
    if inner.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut inner.pending);
    inner.last_frame = Some(now);
    let Some(sink) = inner.sink.take() else {
        return;
    };
    let mut alive = true;
    for frame in pending.chunks(MAX_FRAME_BYTES) {
        if !sink.send(frame.to_vec()) {
            alive = false;
            break;
        }
        inner.sent(frame.len());
    }
    if alive {
        inner.sink = Some(sink);
    } else {
        // The pane is gone (window reload, closed tab): keep buffering in the
        // ring until the next attach; nothing waits for its acknowledgements.
        inner.undelivered = (inner.undelivered + pending.len()).min(inner.ring.len());
        inner.reset_credit();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;

    /// Collects frames; `alive` false simulates a closed webview.
    #[derive(Clone, Default)]
    pub(crate) struct Collector {
        pub frames: Arc<Mutex<Vec<Vec<u8>>>>,
        pub dead: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Collector {
        pub(crate) fn bytes(&self) -> Vec<u8> {
            self.frames.lock().unwrap().concat()
        }
        pub(crate) fn frame_count(&self) -> usize {
            self.frames.lock().unwrap().len()
        }
    }

    impl OutputSink for Collector {
        fn send(&self, frame: Vec<u8>) -> bool {
            if self.dead.load(std::sync::atomic::Ordering::SeqCst) {
                return false;
            }
            self.frames.lock().unwrap().push(frame);
            true
        }
    }

    #[test]
    fn output_before_attach_is_replayed() {
        let hub = OutputHub::new();
        assert_eq!(hub.push(b"motd\r\n"), None);
        assert_eq!(hub.push(b"$ "), None);
        let sink = Collector::default();
        hub.attach(Box::new(sink.clone()));
        assert_eq!(sink.bytes(), b"motd\r\n$ ");
        // Live output follows the replay without duplicates.
        let now = Instant::now() + COALESCE_INTERVAL;
        assert_eq!(hub.push_at(b"x", now), None);
        assert_eq!(sink.bytes(), b"motd\r\n$ x");
    }

    #[test]
    fn a_new_attach_replaces_the_subscriber() {
        let hub = OutputHub::new();
        let first = Collector::default();
        hub.attach(Box::new(first.clone()));
        let t0 = Instant::now() + COALESCE_INTERVAL;
        hub.push_at(b"one ", t0);
        let second = Collector::default();
        hub.attach(Box::new(second.clone()));
        hub.push_at(b"two", t0 + COALESCE_INTERVAL * 4);
        assert_eq!(first.bytes(), b"one ");
        assert_eq!(second.bytes(), b"one two");
    }

    #[test]
    fn ring_keeps_the_last_bytes_on_a_utf8_boundary() {
        let hub = OutputHub::new();
        // A 3-byte character, then enough output to push its first byte out.
        hub.push("\u{65e5}".as_bytes());
        hub.push(&vec![b'a'; REPLAY_BYTES - 2]);
        assert_eq!(hub.buffered(), REPLAY_BYTES);
        let sink = Collector::default();
        hub.attach(Box::new(sink.clone()));
        // The two continuation bytes left in the ring are not replayed.
        assert_eq!(sink.bytes(), vec![b'a'; REPLAY_BYTES - 2]);
        assert_eq!(sink.frame_count(), REPLAY_BYTES / MAX_FRAME_BYTES);
        assert!(
            sink.frames
                .lock()
                .unwrap()
                .iter()
                .all(|f| f.len() <= MAX_FRAME_BYTES)
        );

        // A chunk larger than the ring keeps only its tail.
        let hub = OutputHub::new();
        let mut big = vec![b'x'; 10];
        big.extend(vec![b'y'; REPLAY_BYTES]);
        hub.push(&big);
        let sink = Collector::default();
        hub.attach(Box::new(sink.clone()));
        assert_eq!(sink.bytes(), vec![b'y'; REPLAY_BYTES]);
    }

    #[test]
    fn coalescing_holds_back_bursts() {
        let hub = OutputHub::new();
        let sink = Collector::default();
        hub.attach(Box::new(sink.clone()));
        let t0 = Instant::now() + COALESCE_INTERVAL;
        // The first chunk after a quiet period leaves at once.
        assert_eq!(hub.push_at(b"a", t0), None);
        assert_eq!(sink.frame_count(), 1);
        // Within the interval output is held back.
        let wait = hub
            .push_at(b"b", t0 + Duration::from_millis(2))
            .expect("held back");
        assert_eq!(wait, Duration::from_millis(6));
        assert!(hub.push_at(b"c", t0 + Duration::from_millis(3)).is_some());
        assert_eq!(sink.frame_count(), 1);
        hub.flush();
        assert_eq!(sink.frame_count(), 2);
        assert_eq!(sink.bytes(), b"abc");
        // A full frame leaves at once.
        let t1 = Instant::now();
        hub.push_at(b"d", t1);
        assert_eq!(hub.push_at(&vec![b'e'; MAX_FRAME_BYTES], t1), None);
        assert!(
            sink.frames
                .lock()
                .unwrap()
                .iter()
                .all(|f| f.len() <= MAX_FRAME_BYTES)
        );
        assert_eq!(sink.bytes().len(), 4 + MAX_FRAME_BYTES);
    }

    #[test]
    fn a_dead_subscriber_is_dropped_and_output_stays_in_the_ring() {
        let hub = OutputHub::new();
        let sink = Collector::default();
        hub.attach(Box::new(sink.clone()));
        sink.dead.store(true, std::sync::atomic::Ordering::SeqCst);
        let t0 = Instant::now() + COALESCE_INTERVAL;
        hub.push_at(b"lost?", t0);
        assert!(format!("{hub:?}").contains("attached: false"));
        let next = Collector::default();
        hub.attach(Box::new(next.clone()));
        assert_eq!(next.bytes(), b"lost?");
        hub.detach();
        assert!(format!("{hub:?}").contains("attached: false"));
    }

    #[test]
    fn unacknowledged_output_pauses_and_acks_resume() {
        let hub = OutputHub::new();
        // Without a subscriber nothing pauses.
        hub.push(&vec![b'a'; 3 * MAX_FRAME_BYTES]);
        assert!(!hub.paused());
        let sink = Collector::default();
        hub.attach(Box::new(sink.clone()));
        assert!(!hub.paused());
        let chunk = vec![b'b'; MAX_FRAME_BYTES];
        let mut sent = 0u64;
        let mut now = Instant::now() + COALESCE_INTERVAL;
        while !hub.paused() {
            hub.push_at(&chunk, now);
            now += COALESCE_INTERVAL;
            sent += MAX_FRAME_BYTES as u64;
            assert!(sent <= PAUSE_BYTES + 2 * MAX_FRAME_BYTES as u64);
        }
        // The replay counted too.
        assert!(sent < PAUSE_BYTES);
        // Acknowledging down to the resume mark resumes; above it does not.
        hub.ack(PAUSE_BYTES / 2);
        assert!(hub.paused());
        hub.ack(PAUSE_BYTES);
        assert!(!hub.paused());
        // A pending wake-up is kept for the reader.
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(1), hub.resumed())
                    .await
                    .expect("resume notified");
            });
        // A new subscriber or a detach clears the debt.
        while !hub.paused() {
            hub.push_at(&chunk, now);
            now += COALESCE_INTERVAL;
        }
        hub.detach();
        assert!(!hub.paused());
        hub.attach(Box::new(Collector::default()));
        assert!(!hub.paused());
        // Acks never underflow.
        hub.ack(u64::MAX);
        assert!(!hub.paused());
    }

    #[test]
    fn replay_drops_terminal_queries_a_pane_received_before() {
        let hub = OutputHub::new();
        let queries: &[u8] =
            b"$ \x1b[6n\x1b[c\x1b[>c\x1b[>0q\x1b[?u\x1b[?2004$p\x1b[18t\x1b]11;?\x07\x1b]10;?\x1b\\\x1b]4;1;?\x07\x1bP$qm\x1b\\";
        let others: &[u8] =
            b"\x1b[31mred\x1b[0m \x1b]0;title\x07\x1b[?2004h\x1b[2J\x1b]11;#000000\x07";
        hub.push(queries);
        hub.push(others);
        // Nobody received this yet (a shell that asks before its first
        // prompt): the first pane gets it as it is and answers.
        let first = Collector::default();
        hub.attach(Box::new(first.clone()));
        assert_eq!(first.bytes(), [queries, others].concat());
        // A reload attaches again: the queries were answered already.
        let second = Collector::default();
        hub.attach(Box::new(second.clone()));
        assert_eq!(second.bytes(), [b"$ " as &[u8], others].concat());
        // Output while nobody is attached stays unfiltered, after the
        // filtered part; an unfinished sequence at the end is kept.
        hub.detach();
        hub.push(b"\x1b[6n\x1b[6");
        let third = Collector::default();
        hub.attach(Box::new(third.clone()));
        assert_eq!(
            third.bytes(),
            [b"$ " as &[u8], others, b"\x1b[6n\x1b[6"].concat()
        );
        // Live output is never filtered.
        let now = Instant::now() + COALESCE_INTERVAL;
        hub.push_at(b"n\x1b[5n", now);
        assert!(third.bytes().ends_with(b"\x1b[6n\x1b[6n\x1b[5n"));
        assert_eq!(strip_queries(b"a\x1b"), b"a\x1b".to_vec());
        assert_eq!(strip_queries(b"\x1b[5n\x1b[?6n"), Vec::<u8>::new());
    }
}
