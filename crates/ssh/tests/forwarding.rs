//! Port forwarding (stage 5) against the russh test server: Local, Remote
//! and Dynamic rules to a TCP echo service on loopback, how a forwarded
//! connection ends (half-closed, closed by the server, reset), a busy
//! local port, a refused remote listener, stopping, and a host's rules on
//! one shared connection that come up again after the connection drops.

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use shellpp_ssh::{
    Bound, Connection, ForwardEnd, ForwardError, ForwardKind, ForwardSpec, HostForwarder,
    RuleStatus, connect,
};
use shellpp_test_sshd::{ServerConfig, ServerEvent, TestServer};
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A TCP service that answers every line with `echo:` and the line.
struct Echo {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Echo {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Echo {
    async fn start() -> Echo {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => {
                                let mut reply = b"echo:".to_vec();
                                reply.extend_from_slice(&buf[..n]);
                                if socket.write_all(&reply).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
        Echo { addr, task }
    }
}

/// Sends `text` and reads the echo service's answer.
async fn ask(stream: &mut TcpStream, text: &str) -> String {
    stream.write_all(text.as_bytes()).await.expect("write");
    let expected = text.len() + 5;
    let mut answer = Vec::new();
    let mut buf = [0u8; 256];
    while answer.len() < expected {
        let n = tokio::time::timeout(SHORT, stream.read(&mut buf))
            .await
            .expect("an answer in time")
            .expect("read");
        assert!(n > 0, "closed early: {answer:?}");
        answer.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(answer).expect("text")
}

async fn forwarding_server(password: &str, forwarding: bool) -> TestServer {
    let mut config = ServerConfig::new(vec![password_user("alice", password)]);
    config.forwarding = forwarding;
    start_with(config).await
}

async fn connection(server: &TestServer, password: &str) -> Connection {
    connect(
        spec(server.addr(), Some("alice"), password_plan(password)),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects")
}

fn local(host: &str, port: u16, bind_port: u16) -> ForwardSpec {
    ForwardSpec {
        kind: ForwardKind::Local {
            host: host.to_owned(),
            port,
        },
        bind_address: String::new(),
        bind_port,
    }
}

fn local_addr(bound: Bound) -> SocketAddr {
    match bound {
        Bound::Local(addr) => addr,
        Bound::Remote(_) => panic!("a local listener"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_rule_reaches_the_destination_through_the_ssh_host() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(local("127.0.0.1", echo.addr.port(), 0))
        .await
        .expect("starts");
    let addr = local_addr(forward.bound());
    // An empty bind address is loopback, never all interfaces.
    assert!(addr.ip().is_loopback());

    // Two connections at once through the one rule.
    let mut first = TcpStream::connect(addr).await.expect("connect");
    let mut second = TcpStream::connect(addr).await.expect("connect");
    assert_eq!(ask(&mut first, "one").await, "echo:one");
    assert_eq!(ask(&mut second, "two").await, "echo:two");
    assert!(eventually(SHORT, || forward.active_connections() == 2).await);
    assert!(server.events().contains(&ServerEvent::DirectTcpip {
        host: "127.0.0.1".to_owned(),
        port: u32::from(echo.addr.port()),
    }));

    // Stopping closes the listener and the open connections.
    forward.stop().await;
    let mut buf = [0u8; 8];
    let closed = tokio::time::timeout(SHORT, first.read(&mut buf)).await;
    assert!(matches!(closed, Ok(Ok(0)) | Ok(Err(_))), "{closed:?}");
    assert!(
        eventually(SHORT, || std::net::TcpStream::connect(addr).is_err()).await,
        "the port is free again"
    );
}

/// A destination that takes one connection and hands it to `serve`.
async fn destination<F, Fut>(serve: F) -> (SocketAddr, tokio::task::JoinHandle<()>)
where
    F: FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await {
            serve(socket).await;
        }
    });
    (addr, task)
}

/// Reads `stream` to its end.
async fn read_to_end(stream: &mut TcpStream) -> Vec<u8> {
    let mut all = Vec::new();
    tokio::time::timeout(SHORT, stream.read_to_end(&mut all))
        .await
        .expect("the end in time")
        .expect("read");
    all
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_goes_on_after_the_destination_s_end() {
    const UPLOAD: usize = 4 << 20;
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let (received, count) = tokio::sync::oneshot::channel();
    let (addr, _task) = destination(|mut socket| async move {
        socket.write_all(b"ready").await.expect("write");
        socket.shutdown().await.expect("shutdown");
        let mut all = Vec::new();
        let _ = socket.read_to_end(&mut all).await;
        let _ = received.send(all.len());
    })
    .await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(local("127.0.0.1", addr.port(), 0))
        .await
        .expect("starts");
    let mut stream = TcpStream::connect(local_addr(forward.bound()))
        .await
        .expect("connect");
    assert_eq!(read_to_end(&mut stream).await, b"ready");

    // The destination's end is the end of one direction only.
    stream.write_all(&vec![7u8; UPLOAD]).await.expect("upload");
    stream.shutdown().await.expect("shutdown");
    let count = tokio::time::timeout(SHORT, count)
        .await
        .expect("the destination read it all in time")
        .expect("a count");
    assert_eq!(count, UPLOAD);
    assert!(eventually(SHORT, || forward.active_connections() == 0).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_s_close_ends_a_connection_the_local_side_keeps_open() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    // A reset destination: the test server closes the channel at once.
    let (addr, _task) = destination(|mut socket| async move {
        let mut buf = [0u8; 16];
        let _ = socket.read(&mut buf).await;
        socket.set_zero_linger().expect("linger");
    })
    .await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(local("127.0.0.1", addr.port(), 0))
        .await
        .expect("starts");
    let mut stream = TcpStream::connect(local_addr(forward.bound()))
        .await
        .expect("connect");
    stream.write_all(b"hi").await.expect("write");
    assert!(read_to_end(&mut stream).await.is_empty());

    // The local side neither sends nor closes.
    assert!(eventually(SHORT, || forward.active_connections() == 0).await);
    drop(stream);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reset_local_connection_ends_while_the_destination_stays_open() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    // Reads to the end and keeps its side open: the server never closes.
    let (addr, _task) = destination(|mut socket| async move {
        let mut buf = [0u8; 1024];
        while let Ok(n) = socket.read(&mut buf).await {
            if n == 0 {
                break;
            }
        }
        std::future::pending::<()>().await;
    })
    .await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(local("127.0.0.1", addr.port(), 0))
        .await
        .expect("starts");
    let mut stream = TcpStream::connect(local_addr(forward.bound()))
        .await
        .expect("connect");
    stream.write_all(b"hi").await.expect("write");
    assert!(eventually(SHORT, || forward.active_connections() == 1).await);
    stream.set_zero_linger().expect("linger");
    drop(stream);

    // Our EOF, then our CLOSE: the server has not closed within 5 s.
    assert!(eventually(SHORT * 3, || forward.active_connections() == 0).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reset_local_connection_that_stopped_reading_ends() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&sent);
    // Sends until the connection goes; never closes by itself.
    let (addr, _task) = destination(|mut socket| async move {
        let chunk = vec![b'x'; 64 << 10];
        while socket.write_all(&chunk).await.is_ok() {
            counted.fetch_add(chunk.len(), std::sync::atomic::Ordering::SeqCst);
        }
        std::future::pending::<()>().await;
    })
    .await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(local("127.0.0.1", addr.port(), 0))
        .await
        .expect("starts");
    let stream = TcpStream::connect(local_addr(forward.bound()))
        .await
        .expect("connect");
    // Nothing is read here: the data piles up on the way.
    assert!(
        eventually(SHORT, || sent.load(std::sync::atomic::Ordering::SeqCst)
            >= 2 << 20)
        .await
    );
    stream.set_zero_linger().expect("linger");
    drop(stream);

    // The writes to the socket fail; our EOF, then our CLOSE.
    assert!(eventually(SHORT * 3, || forward.active_connections() == 0).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_connection_to_a_closed_port_here_ends() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let closed = free_port().await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Remote {
                host: "127.0.0.1".to_owned(),
                port: closed,
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("starts");
    let Bound::Remote(port) = forward.bound() else {
        panic!("a remote listener");
    };
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the server listens");
    // Our EOF: nothing here took the connection.
    assert!(read_to_end(&mut stream).await.is_empty());
    drop(stream);
    assert!(eventually(SHORT, || forward.active_connections() == 0).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_local_port_is_named_in_the_error() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let taken = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = taken.local_addr().expect("address").port();
    let connection = connection(&server, &password).await;
    let error = connection
        .forward(local("127.0.0.1", 9, port))
        .await
        .expect_err("in use");
    assert_eq!(error, ForwardError::PortBusy(port));
    assert_eq!(error.to_string(), format!("port {port} is already in use"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dynamic_rule_is_a_socks5_proxy_with_names_resolved_remotely() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Dynamic,
            bind_address: "127.0.0.1".to_owned(),
            bind_port: 0,
        })
        .await
        .expect("starts");
    let addr = local_addr(forward.bound());
    let mut socks = TcpStream::connect(addr).await.expect("connect");
    socks.write_all(&[5, 1, 0]).await.unwrap();
    let mut choice = [0u8; 2];
    socks.read_exact(&mut choice).await.unwrap();
    assert_eq!(choice, [5, 0]);
    // A name: the SSH host resolves it.
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend_from_slice(b"localhost");
    request.extend_from_slice(&echo.addr.port().to_be_bytes());
    socks.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    socks.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "succeeded");
    assert_eq!(ask(&mut socks, "via socks").await, "echo:via socks");
    assert!(server.events().contains(&ServerEvent::DirectTcpip {
        host: "localhost".to_owned(),
        port: u32::from(echo.addr.port()),
    }));

    // A destination the SSH host cannot reach: "connection refused".
    let closed = support::net::closed_port().await;
    let mut socks = TcpStream::connect(addr).await.expect("connect");
    socks.write_all(&[5, 1, 0]).await.unwrap();
    socks.read_exact(&mut choice).await.unwrap();
    let mut request = vec![5, 1, 0, 1, 127, 0, 0, 1];
    request.extend_from_slice(&closed.port().to_be_bytes());
    socks.write_all(&request).await.unwrap();
    socks.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 5, "connection refused");
    forward.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_rule_listens_on_the_ssh_host_and_connects_here() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Remote {
                host: "127.0.0.1".to_owned(),
                port: echo.addr.port(),
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("starts");
    let Bound::Remote(port) = forward.bound() else {
        panic!("a remote listener");
    };
    assert_ne!(port, 0, "the server chose the port");
    // The test server listens on loopback: connect to "the SSH host's" port.
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the server listens");
    assert_eq!(ask(&mut stream, "from far").await, "echo:from far");
    // The empty bind address went out as 127.0.0.1.
    assert!(server.events().contains(&ServerEvent::TcpipForward {
        address: "127.0.0.1".to_owned(),
        port: 0,
    }));
    forward.stop().await;
    assert!(server.events().contains(&ServerEvent::CancelTcpipForward {
        address: "127.0.0.1".to_owned(),
        port: u32::from(port),
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_remote_listener_is_an_error() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, false).await;
    let connection = connection(&server, &password).await;
    let error = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Remote {
                host: "127.0.0.1".to_owned(),
                port: 9,
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect_err("refused");
    assert_eq!(error, ForwardError::Refused);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_connection_ends_the_rules() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let connection = connection(&server, &password).await;
    let forward = connection
        .forward(local("127.0.0.1", echo.addr.port(), 0))
        .await
        .expect("starts");
    assert_eq!(server.drop_connections(), 1);
    let end = tokio::time::timeout(SHORT, forward.ended())
        .await
        .expect("ends in time");
    assert_eq!(end, ForwardEnd::ConnectionLost);
}

/// A free local port (bound and released).
async fn free_port() -> u16 {
    support::net::closed_port().await.port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hosts_rules_share_one_connection_and_come_back_after_a_drop() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let addr = server.addr();
    let connect_fn: shellpp_ssh::forward::ConnectFn = Arc::new(move || {
        let password = password.clone();
        Box::pin(async move {
            connect(
                spec(addr, Some("alice"), password_plan(&password)),
                TestInteraction::accepting().as_dyn(),
            )
            .await
            .map(Arc::new)
        })
    });
    let forwarder = HostForwarder::new(connect_fn);
    let (port1, port2) = (free_port().await, free_port().await);
    let first = forwarder
        .start(1, local("127.0.0.1", echo.addr.port(), port1))
        .await;
    let second = forwarder
        .start(
            2,
            ForwardSpec {
                kind: ForwardKind::Dynamic,
                bind_address: String::new(),
                bind_port: port2,
            },
        )
        .await;
    assert!(
        matches!(first, RuleStatus::Running(Bound::Local(_))),
        "{first:?}"
    );
    assert!(
        matches!(second, RuleStatus::Running(Bound::Local(_))),
        "{second:?}"
    );
    assert_eq!(
        server.connection_count(),
        1,
        "one connection for both rules"
    );

    // The connection drops: both rules wait, then come back on a new one.
    let mut statuses = forwarder.subscribe();
    assert_eq!(server.drop_connections(), 1);
    let reconnecting = tokio::time::timeout(
        SHORT,
        statuses.wait_for(|statuses| {
            statuses
                .values()
                .all(|status| matches!(status, RuleStatus::Reconnecting { .. }))
        }),
    )
    .await
    .is_ok_and(|seen| seen.is_ok());
    assert!(reconnecting, "the rules wait for the reconnect");
    let back = tokio::time::timeout(
        Duration::from_secs(15),
        statuses.wait_for(|statuses| {
            statuses.len() == 2
                && statuses
                    .values()
                    .all(|status| matches!(status, RuleStatus::Running(_)))
        }),
    )
    .await
    .is_ok_and(|seen| seen.is_ok());
    assert!(
        back,
        "the rules run again: {:?}",
        forwarder.subscribe().borrow().clone()
    );
    assert!(eventually(SHORT, || server.connection_count() == 1).await);
    // The same local port answers again.
    let mut stream = TcpStream::connect(("127.0.0.1", port1))
        .await
        .expect("connect");
    assert_eq!(ask(&mut stream, "again").await, "echo:again");

    // Stopping the last rule closes the connection.
    forwarder.stop(1).await;
    assert_eq!(server.connection_count(), 1, "rule 2 still runs");
    forwarder.stop(2).await;
    assert!(eventually(SHORT, || server.connection_count() == 0).await);
    forwarder.shutdown().await;
}

/// A reconnect that fails for another reason than the transport (here the
/// password) ends the attempts with that failure instead of retrying
/// forever; a rule started while the supervisor reconnects waits for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_login_ends_the_reconnects_and_new_rules_wait_for_one() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let addr = server.addr();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wrong_after = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let connect_fn: shellpp_ssh::forward::ConnectFn = {
        let calls = Arc::clone(&calls);
        let wrong_after = Arc::clone(&wrong_after);
        Arc::new(move || {
            let password = if wrong_after.load(std::sync::atomic::Ordering::SeqCst) {
                "wrong".to_owned()
            } else {
                password.clone()
            };
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                let mut spec = spec(addr, Some("alice"), password_plan(&password));
                spec.auth.keyboard_interactive = false;
                connect(spec, TestInteraction::accepting().as_dyn())
                    .await
                    .map(Arc::new)
            })
        })
    };
    let forwarder = HostForwarder::new(connect_fn);
    let first = forwarder
        .start(1, local("127.0.0.1", echo.addr.port(), 0))
        .await;
    assert!(matches!(first, RuleStatus::Running(_)), "{first:?}");
    let mut statuses = forwarder.subscribe();
    wrong_after.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(server.drop_connections(), 1);
    let waiting = tokio::time::timeout(
        SHORT,
        statuses
            .wait_for(|statuses| matches!(statuses.get(&1), Some(RuleStatus::Reconnecting { .. }))),
    )
    .await
    .is_ok_and(|seen| seen.is_ok());
    assert!(waiting);
    // Started during the reconnect: it waits, it does not connect itself.
    let second = forwarder
        .start(2, local("127.0.0.1", echo.addr.port(), 0))
        .await;
    assert!(
        matches!(second, RuleStatus::Reconnecting { .. }),
        "{second:?}"
    );
    let failed = tokio::time::timeout(
        Duration::from_secs(15),
        statuses.wait_for(|statuses| {
            statuses.len() == 2
                && statuses
                    .values()
                    .all(|status| matches!(status, RuleStatus::ConnectFailed(_)))
        }),
    )
    .await
    .is_ok_and(|seen| seen.is_ok());
    assert!(failed, "{:?}", forwarder.subscribe().borrow().clone());
    // One login attempt after the drop, not one every minute.
    let attempts = calls.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), attempts);
    assert_eq!(attempts, 2, "the first connect and one reconnect");
    forwarder.shutdown().await;
}

/// Restarting a rule on the same port works at once: `Forward::stop`
/// returns once the listener is closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rule_restarts_on_its_own_port() {
    let password = random_secret("pw");
    let server = forwarding_server(&password, true).await;
    let echo = Echo::start().await;
    let connection = connection(&server, &password).await;
    let port = free_port().await;
    for _ in 0..5 {
        let forward = connection
            .forward(local("127.0.0.1", echo.addr.port(), port))
            .await
            .expect("the port is free again");
        forward.stop().await;
    }
}
