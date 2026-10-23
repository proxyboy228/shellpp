//! The russh patch of D100 (third_party/russh/PATCHES.md): a channel the
//! client closes first must not stay in the session's channel map after the
//! server's CLOSE. The map holds the channel's sender, so the channel's own
//! queue ends (`wait()` returns `None`) exactly when the entry is removed:
//! with russh 0.64.1 unpatched, the queue of such a channel never ends.
//!
//! Exec channels (OS detection, snippet runs) and forwarded agent channels
//! (one per use of the agent on the server) are both closed by the client
//! first in the app; these tests drive that order against the russh test
//! server through the raw russh client.

mod support;

use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, ChannelOpenHandle, Handle, Msg, Session};
use russh::keys::PublicKeyOrCertificate;
use russh::{Channel, ChannelMsg};
use shellpp_test_sshd::{ExecAnswer, ServerConfig, TestServer};
use support::{password_user, random_secret};
use tokio::sync::mpsc;

/// How long the channel's queue may take to end after the client's close.
const END: Duration = Duration::from_secs(10);

/// Accepts any host key; hands forwarded agent channels to the test.
struct Client {
    agent_channels: mpsc::UnboundedSender<Channel<Msg>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        let _ = self.agent_channels.send(channel);
        Ok(())
    }
}

async fn connect(
    server: &TestServer,
    password: &str,
) -> (Handle<Client>, mpsc::UnboundedReceiver<Channel<Msg>>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect(config, server.addr(), Client { agent_channels: tx })
        .await
        .expect("connects");
    let auth = handle
        .authenticate_password("alice", password)
        .await
        .expect("authenticates");
    assert!(auth.success());
    (handle, rx)
}

/// Reads the channel's queue until it ends; whether a `Close` came first.
async fn drain(channel: &mut Channel<Msg>) -> bool {
    let mut saw_close = false;
    while let Some(message) = channel.wait().await {
        if matches!(message, ChannelMsg::Close) {
            saw_close = true;
        }
    }
    saw_close
}

#[tokio::test]
async fn an_exec_channel_closed_first_by_the_client_leaves_the_session() {
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.exec = Some(ExecAnswer {
        stdout: "done\n".to_owned(),
        exit_status: 0,
    });
    let server = shellpp_test_sshd::TestServer::start(config)
        .await
        .expect("test server starts");
    let (handle, _agents) = connect(&server, &password).await;

    let mut client_first = 0;
    for _ in 0..10 {
        let mut channel = handle.channel_open_session().await.expect("opens");
        channel.exec(false, "uname -s").await.expect("exec sent");
        // The client's CLOSE goes out right behind the request, before the
        // server can answer: the server's CLOSE then finds the channel gone
        // from the client's table (the patched path).
        channel.close().await.expect("close sent");
        let saw_close = tokio::time::timeout(END, drain(&mut channel))
            .await
            .expect("the channel's queue ended: its entry left the session");
        if !saw_close {
            client_first += 1;
        }
    }
    assert!(client_first > 0, "the client never closed first");
}

#[tokio::test]
async fn an_agent_channel_closed_first_by_the_client_leaves_the_session() {
    let password = random_secret("pw");
    let server = shellpp_test_sshd::TestServer::start(ServerConfig::new(vec![password_user(
        "alice", &password,
    )]))
    .await
    .expect("test server starts");
    let (handle, mut agents) = connect(&server, &password).await;

    let shell = handle.channel_open_session().await.expect("opens");
    shell
        .agent_forward(true)
        .await
        .expect("agent forwarding requested");
    shell.request_shell(true).await.expect("shell requested");
    for round in 0..3 {
        // `agent-keys` makes the server open an agent channel to the client.
        shell
            .data(&b"agent-keys\n"[..])
            .await
            .expect("command sent");
        let mut channel = tokio::time::timeout(END, agents.recv())
            .await
            .expect("an agent channel arrived")
            .expect("open");
        // The client ends the channel before answering anything, as a
        // forwarded agent connection that the client's side ended does.
        channel.close().await.expect("close sent");
        let saw_close = tokio::time::timeout(END, drain(&mut channel))
            .await
            .unwrap_or_else(|_| panic!("round {round}: the agent channel's entry stayed"));
        assert!(!saw_close, "the server closed first");
    }
}
