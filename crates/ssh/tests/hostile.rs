//! A deliberately hostile russh server (written here, not the test server):
//! once the shell starts it tries to open channels to the client
//! (docs/security.md, section 11.4: only `auth-agent@openssh.com`, and only
//! after forwarding was requested, may be accepted), then writes stderr and
//! stdout and ends the shell with an exit signal. In its flooding mode it
//! streams output instead of answering the pty request.

mod support;

use std::sync::{Arc, Mutex};

use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId, Pty, Sig};
use secrecy::ExposeSecret;
use shellpp_ssh::{FailureKind, SessionEnd, ShellEvent, ShellRequest, SystemAgent, connect};
use shellpp_test_sshd::{KeyType, generate_key};
use support::agent::TestAgent;
use support::*;

type Outcomes = Arc<Mutex<Vec<(&'static str, bool)>>>;

/// Output the flooding server streams before (never) answering pty-req.
const FLOOD_BYTES: usize = 16 * 1024 * 1024;

struct Hostile {
    outcomes: Outcomes,
    /// Stream output instead of answering the pty request.
    flood: bool,
}

impl russh::server::Handler for Hostile {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        mut channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        tokio::spawn(async move { while channel.wait().await.is_some() {} });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if !self.flood {
            session.channel_success(channel)?;
            return Ok(());
        }
        let handle = session.handle();
        let outcomes = Arc::clone(&self.outcomes);
        tokio::spawn(async move {
            let chunk = vec![b'x'; 32 * 1024];
            let mut sent = 0;
            while sent < FLOOD_BYTES {
                if handle.data(channel, chunk.clone()).await.is_err() {
                    break;
                }
                sent += chunk.len();
            }
            outcomes
                .lock()
                .unwrap()
                .push(("flood finished", sent >= FLOOD_BYTES));
        });
        Ok(())
    }

    async fn agent_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let handle = session.handle();
        let outcomes = Arc::clone(&self.outcomes);
        tokio::spawn(async move {
            let mut results = vec![
                ("session", handle.channel_open_session().await.is_ok()),
                (
                    "direct-tcpip",
                    handle
                        .channel_open_direct_tcpip("127.0.0.1", 22, "127.0.0.1", 50000)
                        .await
                        .is_ok(),
                ),
                (
                    "forwarded-tcpip",
                    handle
                        .channel_open_forwarded_tcpip("127.0.0.1", 8080, "127.0.0.1", 50000)
                        .await
                        .is_ok(),
                ),
                (
                    "x11",
                    handle.channel_open_x11("127.0.0.1", 6000).await.is_ok(),
                ),
            ];
            results.push(("auth-agent", handle.channel_open_agent().await.is_ok()));
            outcomes.lock().unwrap().extend(results);
            let _ = handle.extended_data(channel, 1, b"err-text".to_vec()).await;
            let _ = handle.data(channel, b"out-text".to_vec()).await;
            let _ = handle
                .exit_signal_request(channel, Sig::TERM, false, String::new(), String::new())
                .await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
        Ok(())
    }
}

/// Starts the hostile server; returns its address and the channel outcomes.
async fn start_hostile() -> (std::net::SocketAddr, Outcomes, tokio::task::JoinHandle<()>) {
    start_hostile_with(false).await
}

async fn start_hostile_with(
    flood: bool,
) -> (std::net::SocketAddr, Outcomes, tokio::task::JoinHandle<()>) {
    let generated = generate_key(KeyType::Ed25519, "hostile-host", None).expect("host key");
    let host_key = russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
        .expect("decodes");
    let config = Arc::new(russh::server::Config {
        keys: vec![host_key],
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    let outcomes: Outcomes = Arc::new(Mutex::new(Vec::new()));
    let task = {
        let outcomes = Arc::clone(&outcomes);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handler = Hostile {
                    outcomes: Arc::clone(&outcomes),
                    flood,
                };
                let config = Arc::clone(&config);
                tokio::spawn(async move {
                    if let Ok(session) = russh::server::run_stream(config, stream, handler).await {
                        let _ = session.await;
                    }
                });
            }
        })
    };
    (addr, outcomes, task)
}

async fn collect(shell: &mut shellpp_ssh::Shell) -> (String, SessionEnd) {
    let mut text = String::new();
    loop {
        match tokio::time::timeout(SHORT, shell.next())
            .await
            .expect("shell ends in time")
        {
            ShellEvent::Data(data) => text.push_str(&String::from_utf8_lossy(&data)),
            ShellEvent::End(end) => return (text, end),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_opened_channels_are_rejected() {
    let (addr, outcomes, task) = start_hostile().await;
    let connection = connect(
        spec(addr, Some("mallory"), password_plan("anything")),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    let (text, end) = collect(&mut shell).await;
    // stderr and stdout in arrival order, then the signal.
    assert_eq!(text, "err-textout-text");
    assert_eq!(end, SessionEnd::ExitSignal("TERM".to_owned()));
    let outcomes = outcomes.lock().unwrap().clone();
    assert_eq!(
        outcomes,
        vec![
            ("session", false),
            ("direct-tcpip", false),
            ("forwarded-tcpip", false),
            ("x11", false),
            ("auth-agent", false),
        ]
    );
    connection.disconnect().await;
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_agent_channel_is_accepted_only_after_forwarding_was_requested() {
    let generated = generate_key(KeyType::Ed25519, "agent", None).expect("key");
    let key = russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
        .expect("decodes");
    let agent = TestAgent::start(vec![key]).await;
    let (addr, outcomes, task) = start_hostile().await;
    let mut spec = spec(addr, Some("mallory"), password_plan("anything"));
    spec.agent = Some(SystemAgent::at(agent.location()));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection
        .open_shell(ShellRequest {
            agent_forwarding: true,
            ..shell_request()
        })
        .await
        .expect("shell");
    let (_, end) = collect(&mut shell).await;
    assert_eq!(end, SessionEnd::ExitSignal("TERM".to_owned()));
    let outcomes = outcomes.lock().unwrap().clone();
    assert_eq!(
        outcomes,
        vec![
            ("session", false),
            ("direct-tcpip", false),
            ("forwarded-tcpip", false),
            ("x11", false),
            ("auth-agent", true),
        ]
    );
    connection.disconnect().await;
    task.abort();
}

/// A server that streams output before it answers the pty request: the
/// client keeps at most a bounded amount of it and gives up with a protocol
/// error long before the server's flood (or the request timeout) ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_before_the_pty_reply_is_bounded() {
    let (addr, outcomes, task) = start_hostile_with(true).await;
    let connection = connect(
        spec(addr, Some("mallory"), password_plan("anything")),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    let started = std::time::Instant::now();
    let error = connection
        .open_shell(shell_request())
        .await
        .expect_err("no shell");
    assert_eq!(error.kind, FailureKind::Protocol);
    assert!(
        started.elapsed() < SHORT,
        "failed before the request timeout"
    );
    assert!(
        !outcomes.lock().unwrap().contains(&("flood finished", true)),
        "the client stopped reading before the whole flood arrived"
    );
    connection.disconnect().await;
    task.abort();
}
