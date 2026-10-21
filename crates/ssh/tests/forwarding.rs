//! Port forwarding (stage 5) against the russh test server: Local, Remote
//! and Dynamic rules to a TCP echo service on loopback, a busy local port,
//! a refused remote listener, stopping, and a host's rules on one shared
//! connection that come up again after the connection drops.

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
