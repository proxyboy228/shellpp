//! TCP transport: name resolution and connection with the failure kinds of
//! [`FailureKind`], and a stream wrapper that can be cut from outside (used
//! when keepalives go unanswered, so the russh session ends at once instead
//! of waiting for the operating system's TCP timeout).

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::{Duration, Instant};

use crate::error::FailureKind;

/// Resolves `address` (a host name or an IP literal, IPv6 without brackets).
/// Any resolution failure, and an empty answer, is `NotFound`.
pub async fn resolve(address: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, FailureKind> {
    let address = address.trim();
    if address.is_empty() {
        return Err(FailureKind::NotFound);
    }
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((address, port))
        .await
        .map_err(|_| FailureKind::NotFound)?
        .collect();
    if addrs.is_empty() {
        return Err(FailureKind::NotFound);
    }
    Ok(addrs)
}

/// Shortest time one address gets before the next one is tried.
const MIN_ADDRESS_TIMEOUT: Duration = Duration::from_secs(2);

/// Connects to the first address that accepts, before `deadline`. Each
/// address gets an equal share of the remaining time (at least
/// [`MIN_ADDRESS_TIMEOUT`]), so an address that never answers (an IPv6 route
/// that black-holes packets) does not use up the time of the others. The
/// error of the last address decides the kind: refused, timed out, or any
/// other network error.
pub async fn connect_any(
    addrs: &[std::net::SocketAddr],
    deadline: Instant,
) -> Result<TcpStream, FailureKind> {
    let stream = connect_each(addrs, deadline, TcpStream::connect).await?;
    // Interactive traffic: no Nagle delay on keystrokes.
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// [`connect_any`] with the connect step given (tests pass one whose first
/// address never answers, whatever the network does with silent addresses).
async fn connect_each<S, F, Fut>(
    addrs: &[std::net::SocketAddr],
    deadline: Instant,
    mut connect: F,
) -> Result<S, FailureKind>
where
    F: FnMut(std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = io::Result<S>>,
{
    let mut last = FailureKind::Network;
    for (index, addr) in addrs.iter().enumerate() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(FailureKind::Timeout);
        }
        let left = u32::try_from(addrs.len() - index)
            .unwrap_or(u32::MAX)
            .max(1);
        let share = (remaining / left).max(MIN_ADDRESS_TIMEOUT).min(remaining);
        match tokio::time::timeout(share, connect(*addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => {
                last = io_failure(&error);
                tracing::debug!(kind = last.reason(), "tcp connect attempt failed");
            }
            Err(_) => {
                last = FailureKind::Timeout;
                tracing::debug!("tcp connect attempt timed out");
            }
        }
    }
    Err(last)
}

pub(crate) fn io_failure(error: &io::Error) -> FailureKind {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => FailureKind::Refused,
        io::ErrorKind::TimedOut => FailureKind::Timeout,
        _ => FailureKind::Network,
    }
}

/// Cuts a [`KillableStream`] from another task.
#[derive(Debug, Default)]
pub(crate) struct KillSwitch {
    killed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl KillSwitch {
    pub(crate) fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        let waker = self.waker.lock().map(|mut w| w.take()).unwrap_or_default();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(crate) fn is_killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }

    fn register(&self, waker: &Waker) {
        if let Ok(mut slot) = self.waker.lock() {
            match slot.as_ref() {
                Some(current) if current.will_wake(waker) => {}
                _ => *slot = Some(waker.clone()),
            }
        }
    }
}

/// A stream that fails every read and write once its [`KillSwitch`] fired.
/// The russh session task polls reads and writes from one task, so waking
/// the last registered waker re-polls whichever operation is pending.
#[derive(Debug)]
pub(crate) struct KillableStream<S> {
    inner: S,
    switch: Arc<KillSwitch>,
}

impl<S> KillableStream<S> {
    pub(crate) fn new(inner: S, switch: Arc<KillSwitch>) -> Self {
        Self { inner, switch }
    }
}

fn killed_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "connection closed locally",
    )
}

impl<S: AsyncRead + Unpin> AsyncRead for KillableStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.switch.is_killed() {
            return Poll::Ready(Err(killed_error()));
        }
        self.switch.register(cx.waker());
        if self.switch.is_killed() {
            return Poll::Ready(Err(killed_error()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for KillableStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.switch.is_killed() {
            return Poll::Ready(Err(killed_error()));
        }
        self.switch.register(cx.waker());
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.switch.is_killed() {
            return Poll::Ready(Err(killed_error()));
        }
        self.switch.register(cx.waker());
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.switch.is_killed() {
            // Nothing more goes out; the socket closes when the stream drops.
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn kill_wakes_a_pending_read_and_fails_writes() {
        let (client, _server) = tokio::io::duplex(64);
        let switch = Arc::new(KillSwitch::default());
        let mut stream = KillableStream::new(client, switch.clone());
        let killer = {
            let switch = switch.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                switch.kill();
            })
        };
        let mut buf = [0u8; 8];
        let error = stream
            .read(&mut buf)
            .await
            .expect_err("read must fail after kill");
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert!(stream.write_all(b"x").await.is_err());
        killer.await.expect("killer task");
    }

    #[tokio::test]
    async fn empty_or_unresolvable_names_are_not_found() {
        assert_eq!(resolve("", 22).await, Err(FailureKind::NotFound));
        assert_eq!(
            resolve("shellpp..invalid", 22).await,
            Err(FailureKind::NotFound)
        );
    }

    /// An address that never answers does not use up the time of the next one.
    #[tokio::test]
    async fn a_silent_address_leaves_time_for_the_next_one() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let reachable = listener.local_addr().expect("address");
        // 192.0.2.0/24 is never routed (RFC 5737), but some networks answer
        // for it anyway (a transparent proxy accepted it on a dev server):
        // its connect hangs here like a black hole, whatever the network.
        let silent = std::net::SocketAddr::from(([192, 0, 2, 1], 22));
        let connect = move |addr: std::net::SocketAddr| async move {
            if addr == silent {
                std::future::pending::<()>().await;
            }
            TcpStream::connect(addr).await
        };
        let started = Instant::now();
        let deadline = started + Duration::from_secs(6);
        let stream = connect_each(&[silent, reachable], deadline, connect)
            .await
            .expect("the reachable address connects");
        assert_eq!(stream.peer_addr().expect("peer"), reachable);
        assert!(started.elapsed() < Duration::from_secs(5));
        // Every address timing out ends with Timeout at the deadline.
        let deadline = Instant::now() + Duration::from_millis(300);
        let result = connect_each(&[silent, silent], deadline, connect).await;
        assert!(matches!(result, Err(FailureKind::Timeout)));
    }

    #[tokio::test]
    async fn ip_literals_resolve_without_dns() {
        let addrs = resolve("127.0.0.1", 2222).await.expect("literal");
        assert_eq!(
            addrs,
            vec![std::net::SocketAddr::from(([127, 0, 0, 1], 2222))]
        );
        let addrs = resolve("::1", 22).await.expect("ipv6 literal");
        assert_eq!(addrs[0].port(), 22);
    }
}
