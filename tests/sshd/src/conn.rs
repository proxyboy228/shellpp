//! TCP stream wrapper of one connection:
//!
//! - a [`Switch`], so the server can drop the connection at the TCP level
//!   (no SSH disconnect message, no exit status) or freeze it (the socket
//!   stays open but nothing is read or written any more, which lets client
//!   keepalive timeouts be tested). russh owns the stream inside its session
//!   task; the switch is the only way to reach it from outside. Killing
//!   wakes the pending read and write, both fail with `ConnectionAborted`,
//!   the session task ends and drops the socket.
//! - a [`Trace`], which detects replies russh sends without telling the
//!   handler (see [`Trace::burst_finished`]); the server records them as
//!   keepalives.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

const RUNNING: u8 = 0;
const FROZEN: u8 = 1;
const KILLED: u8 = 2;

/// The switch of one connection, shared between the server registry, the
/// session tasks (for the shell's `drop` command) and the stream.
#[derive(Debug, Default)]
pub(crate) struct Switch {
    state: AtomicU8,
    read_waker: Mutex<Option<Waker>>,
    write_waker: Mutex<Option<Waker>>,
}

impl Switch {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Drops the connection: pending and future I/O fails. Returns `false`
    /// when it was already killed.
    pub(crate) fn kill(&self) -> bool {
        let previous = self.state.swap(KILLED, Ordering::SeqCst);
        self.wake_all();
        previous != KILLED
    }

    /// Stops all I/O without closing the socket. Returns `false` when the
    /// connection was not running.
    pub(crate) fn freeze(&self) -> bool {
        let frozen = self
            .state
            .compare_exchange(RUNNING, FROZEN, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        self.wake_all();
        frozen
    }

    fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }

    fn wake_all(&self) {
        for slot in [&self.read_waker, &self.write_waker] {
            let waker = slot.lock().map(|mut guard| guard.take()).unwrap_or(None);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    fn register(slot: &Mutex<Option<Waker>>, cx: &Context<'_>) {
        if let Ok(mut guard) = slot.lock() {
            match guard.as_ref() {
                Some(existing) if existing.will_wake(cx.waker()) => {}
                _ => *guard = Some(cx.waker().clone()),
            }
        }
    }
}

/// Activity flags of one connection, used to observe requests that russh
/// answers on its own.
///
/// russh 0.64 answers unknown global requests such as
/// `keepalive@openssh.com` with a failure reply (as OpenSSH does) without
/// calling the handler. The session loop handles one inbound packet (or
/// queued outbound messages) per iteration and writes what that iteration
/// produced as one burst (`write_all` then `flush`). So a burst on an
/// authenticated connection that follows inbound bytes, while no handler
/// callback ran and no session task queued output since the previous burst,
/// is a reply to such a request. Other rare sources (a client-initiated
/// rekey, unknown channel types or requests with a reply) are counted the
/// same way.
#[derive(Debug, Default)]
pub(crate) struct Trace {
    authenticated: AtomicBool,
    callback: AtomicBool,
    output: AtomicBool,
    inbound: AtomicBool,
    burst: AtomicBool,
}

impl Trace {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The connection authenticated.
    pub(crate) fn authenticated(&self) {
        self.authenticated.store(true, Ordering::SeqCst);
    }

    /// A handler callback ran (call it first thing in every callback).
    pub(crate) fn callback(&self) {
        self.callback.store(true, Ordering::SeqCst);
    }

    /// A session task is about to queue outbound messages.
    pub(crate) fn output(&self) {
        self.output.store(true, Ordering::SeqCst);
    }

    fn inbound(&self) {
        self.inbound.store(true, Ordering::SeqCst);
    }

    fn write_started(&self) {
        self.burst.store(true, Ordering::SeqCst);
    }

    /// End of a write burst; `true` when it answered a request the handler
    /// never saw. Resets the flags for the next burst.
    fn burst_finished(&self) -> bool {
        if !self.burst.swap(false, Ordering::SeqCst) {
            return false;
        }
        let inbound = self.inbound.swap(false, Ordering::SeqCst);
        let callback = self.callback.swap(false, Ordering::SeqCst);
        let output = self.output.swap(false, Ordering::SeqCst);
        self.authenticated.load(Ordering::SeqCst) && inbound && !callback && !output
    }
}

fn aborted() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "connection dropped by the test server",
    )
}

/// Called for every burst detected by [`Trace::burst_finished`].
pub(crate) type SilentReply = Box<dyn Fn() + Send + Sync>;

/// The stream handed to russh.
pub(crate) struct SwitchedStream {
    inner: TcpStream,
    switch: Arc<Switch>,
    trace: Arc<Trace>,
    on_silent_reply: SilentReply,
}

impl SwitchedStream {
    pub(crate) fn new(
        inner: TcpStream,
        switch: Arc<Switch>,
        trace: Arc<Trace>,
        on_silent_reply: SilentReply,
    ) -> Self {
        Self {
            inner,
            switch,
            trace,
            on_silent_reply,
        }
    }
}

impl AsyncRead for SwitchedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Switch::register(&self.switch.read_waker, cx);
        match self.switch.state() {
            KILLED => Poll::Ready(Err(aborted())),
            FROZEN => Poll::Pending,
            _ => {
                let before = buf.filled().len();
                let result = Pin::new(&mut self.inner).poll_read(cx, buf);
                if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
                    self.trace.inbound();
                }
                result
            }
        }
    }
}

impl AsyncWrite for SwitchedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Switch::register(&self.switch.write_waker, cx);
        match self.switch.state() {
            KILLED => Poll::Ready(Err(aborted())),
            FROZEN => Poll::Pending,
            _ => {
                let result = Pin::new(&mut self.inner).poll_write(cx, buf);
                if matches!(result, Poll::Ready(Ok(written)) if written > 0) {
                    self.trace.write_started();
                }
                result
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Switch::register(&self.switch.write_waker, cx);
        match self.switch.state() {
            KILLED => Poll::Ready(Err(aborted())),
            FROZEN => Poll::Pending,
            _ => {
                let result = Pin::new(&mut self.inner).poll_flush(cx);
                if matches!(result, Poll::Ready(Ok(()))) && self.trace.burst_finished() {
                    (self.on_silent_reply)();
                }
                result
            }
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Switch::register(&self.switch.write_waker, cx);
        match self.switch.state() {
            KILLED => Poll::Ready(Err(aborted())),
            FROZEN => Poll::Pending,
            _ => Pin::new(&mut self.inner).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_counts_only_after_silent_inbound_traffic() {
        let trace = Trace::default();
        // Before authentication nothing counts.
        trace.inbound();
        trace.write_started();
        assert!(!trace.burst_finished());

        trace.authenticated();
        // A flush without writes is not a burst.
        trace.inbound();
        assert!(!trace.burst_finished());
        // Inbound bytes, no callback, no output: a silent reply.
        trace.write_started();
        assert!(trace.burst_finished());
        // Flags were reset: a burst without inbound bytes does not count.
        trace.write_started();
        assert!(!trace.burst_finished());
        // A handler callback since the previous burst: not silent.
        trace.inbound();
        trace.callback();
        trace.write_started();
        assert!(!trace.burst_finished());
        // Output queued by a session task: not silent.
        trace.inbound();
        trace.output();
        trace.write_started();
        assert!(!trace.burst_finished());
    }

    #[test]
    fn kill_and_freeze_report_changes() {
        let switch = Switch::default();
        assert!(switch.freeze());
        assert!(!switch.freeze());
        assert!(switch.kill());
        assert!(!switch.kill());
        assert!(!switch.freeze());
    }
}
