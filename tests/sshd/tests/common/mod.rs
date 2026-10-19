//! Helpers shared by the integration tests: a russh client that accepts any
//! host key (and records it), a terminal wrapper over a session channel, a
//! fake forwarded agent and generated secrets. Not every test crate uses
//! every helper.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, ChannelOpenHandle, Msg, Session};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, Pty};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How long a test waits for anything the server should do promptly.
pub const WAIT: Duration = Duration::from_secs(15);

/// A random secret for one test run (never a fixed password).
pub fn random_secret() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

/// What the client handler observed.
#[derive(Clone, Default)]
pub struct Observed {
    pub server_key: Arc<Mutex<Option<PublicKey>>>,
    pub banner: Arc<Mutex<Option<String>>>,
}

impl Observed {
    pub fn server_key(&self) -> PublicKey {
        self.server_key
            .lock()
            .expect("lock")
            .clone()
            .expect("server key seen")
    }

    pub fn banner(&self) -> Option<String> {
        self.banner.lock().expect("lock").clone()
    }
}

/// The client handler: accepts every host key; serves a fake agent with
/// `agent_keys` on forwarded agent channels when set.
pub struct TestClient {
    observed: Observed,
    agent_keys: Option<Arc<Vec<PublicKey>>>,
}

impl client::Handler for TestClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        if let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key {
            *self.observed.server_key.lock().expect("lock") = Some(key.clone());
        }
        Ok(true)
    }

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        *self.observed.banner.lock().expect("lock") = Some(banner.to_owned());
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(keys) = self.agent_keys.clone() {
            reply.accept().await;
            tokio::spawn(fake_agent(channel, keys));
        }
        Ok(())
    }
}

pub type ClientHandle = client::Handle<TestClient>;

/// Client options of [`connect_with`].
#[derive(Default)]
pub struct ClientOptions {
    pub agent_keys: Option<Vec<PublicKey>>,
    pub keepalive: Option<Duration>,
}

pub async fn connect(port: u16) -> (ClientHandle, Observed) {
    connect_with(port, ClientOptions::default()).await
}

pub async fn connect_with(port: u16, options: ClientOptions) -> (ClientHandle, Observed) {
    let observed = Observed::default();
    let config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        keepalive_interval: options.keepalive,
        keepalive_max: 2,
        ..Default::default()
    };
    let handler = TestClient {
        observed: observed.clone(),
        agent_keys: options.agent_keys.map(Arc::new),
    };
    let handle = tokio::time::timeout(
        WAIT,
        client::connect(Arc::new(config), ("127.0.0.1", port), handler),
    )
    .await
    .expect("connect in time")
    .expect("connect");
    (handle, observed)
}

pub async fn password_login(port: u16, user: &str, password: &str) -> ClientHandle {
    let (mut handle, _) = connect(port).await;
    let result = handle
        .authenticate_password(user, password)
        .await
        .expect("auth request");
    assert!(result.success(), "password accepted");
    handle
}

pub async fn publickey_auth(handle: &mut ClientHandle, user: &str, key: PrivateKey) -> bool {
    let hash = if key.algorithm().is_rsa() {
        handle
            .best_supported_rsa_hash()
            .await
            .expect("rsa hash query")
            .flatten()
    } else {
        None
    };
    handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .expect("auth request")
        .success()
}

/// How a session channel ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    ExitStatus(u32),
    WithoutStatus,
}

/// Options of [`Terminal::open_with`].
pub struct TerminalOptions<'a> {
    pub term: &'a str,
    pub cols: u32,
    pub rows: u32,
    pub env: &'a [(&'a str, &'a str)],
    pub agent_forwarding: bool,
}

impl Default for TerminalOptions<'_> {
    fn default() -> Self {
        Self {
            term: "xterm-256color",
            cols: 80,
            rows: 24,
            env: &[],
            agent_forwarding: false,
        }
    }
}

/// A shell session channel with its collected output.
pub struct Terminal {
    channel: Channel<Msg>,
    output: Vec<u8>,
    mark: usize,
    exit_status: Option<u32>,
    ended: bool,
}

impl Terminal {
    pub async fn open(handle: &ClientHandle) -> Self {
        Self::open_with(handle, TerminalOptions::default()).await
    }

    pub async fn open_with(handle: &ClientHandle, options: TerminalOptions<'_>) -> Self {
        let channel = handle
            .channel_open_session()
            .await
            .expect("session channel");
        channel
            .request_pty(
                true,
                options.term,
                options.cols,
                options.rows,
                0,
                0,
                &[(Pty::ECHO, 1)],
            )
            .await
            .expect("pty request");
        for (name, value) in options.env {
            channel
                .set_env(true, *name, *value)
                .await
                .expect("env request");
        }
        if options.agent_forwarding {
            channel.agent_forward(true).await.expect("agent request");
        }
        channel.request_shell(true).await.expect("shell request");
        Self {
            channel,
            output: Vec::new(),
            mark: 0,
            exit_status: None,
            ended: false,
        }
    }

    pub async fn send(&self, text: &str) {
        self.channel
            .data_bytes(text.as_bytes().to_vec())
            .await
            .expect("send data");
    }

    pub async fn resize(&self, cols: u32, rows: u32) {
        self.channel
            .window_change(cols, rows, 0, 0)
            .await
            .expect("window change");
    }

    /// Everything received so far.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }

    /// Waits until `needle` appears in the output after the previous match;
    /// returns the output from the previous match to the end of this one.
    pub async fn wait_for(&mut self, needle: &str) -> String {
        let deadline = tokio::time::Instant::now() + WAIT;
        let needle_bytes = needle.as_bytes();
        loop {
            let unseen = &self.output[self.mark..];
            let found = unseen
                .windows(needle_bytes.len().max(1))
                .position(|window| window == needle_bytes);
            if let Some(position) = found {
                let end = position + needle_bytes.len();
                let text = String::from_utf8_lossy(&unseen[..end]).into_owned();
                self.mark += end;
                return text;
            }
            assert!(
                !self.ended,
                "session ended before {needle:?}; output: {:?}",
                self.text()
            );
            match tokio::time::timeout_at(deadline, self.next()).await {
                Ok(true) => {}
                Ok(false) => self.ended = true,
                Err(_) => panic!(
                    "timed out waiting for {needle:?}; output: {:?}",
                    self.text()
                ),
            }
        }
    }

    /// Waits for the end of the session.
    pub async fn wait_end(&mut self) -> Ending {
        let deadline = tokio::time::Instant::now() + WAIT;
        while !self.ended {
            match tokio::time::timeout_at(deadline, self.next()).await {
                Ok(true) => {}
                Ok(false) => self.ended = true,
                Err(_) => panic!("timed out waiting for the end; output: {:?}", self.text()),
            }
        }
        match self.exit_status {
            Some(status) => Ending::ExitStatus(status),
            None => Ending::WithoutStatus,
        }
    }

    /// Reads one channel message; `false` when the channel is gone.
    async fn next(&mut self) -> bool {
        match self.channel.wait().await {
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                self.output.extend_from_slice(&data);
                true
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                self.exit_status = Some(exit_status);
                true
            }
            Some(ChannelMsg::Close) | None => false,
            Some(_) => true,
        }
    }
}

/// Result of an exec request.
#[derive(Debug)]
pub struct ExecResult {
    pub accepted: bool,
    pub stdout: String,
    pub exit_status: Option<u32>,
}

pub async fn exec(handle: &ClientHandle, command: &str) -> ExecResult {
    let mut channel = handle
        .channel_open_session()
        .await
        .expect("session channel");
    channel.exec(true, command).await.expect("exec request");
    let mut result = ExecResult {
        accepted: false,
        stdout: String::new(),
        exit_status: None,
    };
    let mut stdout = Vec::new();
    loop {
        let message = tokio::time::timeout(WAIT, channel.wait())
            .await
            .expect("exec answer in time");
        match message {
            Some(ChannelMsg::Success) => result.accepted = true,
            Some(ChannelMsg::Failure) => break,
            Some(ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
            Some(ChannelMsg::ExitStatus { exit_status }) => result.exit_status = Some(exit_status),
            Some(ChannelMsg::Close) | None => break,
            Some(_) => {}
        }
    }
    result.stdout = String::from_utf8_lossy(&stdout).into_owned();
    result
}

/// A minimal agent on a forwarded channel: answers identity requests with
/// `keys` (blobs encoded by ssh-key) and fails every other request.
async fn fake_agent(channel: Channel<Msg>, keys: Arc<Vec<PublicKey>>) {
    const REQUEST_IDENTITIES: u8 = 11;
    const IDENTITIES_ANSWER: u8 = 12;
    const FAILURE: u8 = 5;
    let mut stream = channel.into_stream();
    loop {
        let mut length = [0_u8; 4];
        if stream.read_exact(&mut length).await.is_err() {
            return;
        }
        let mut request = vec![0_u8; u32::from_be_bytes(length) as usize];
        if stream.read_exact(&mut request).await.is_err() {
            return;
        }
        let mut body = Vec::new();
        if request.first() == Some(&REQUEST_IDENTITIES) {
            body.push(IDENTITIES_ANSWER);
            body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
            for key in keys.iter() {
                let blob = key.to_bytes().expect("key blob");
                body.extend_from_slice(&(blob.len() as u32).to_be_bytes());
                body.extend_from_slice(&blob);
                let comment = b"fake-agent";
                body.extend_from_slice(&(comment.len() as u32).to_be_bytes());
                body.extend_from_slice(comment);
            }
        } else {
            body.push(FAILURE);
        }
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        if stream.write_all(&frame).await.is_err() || stream.flush().await.is_err() {
            return;
        }
    }
}

/// Polls `condition` until it holds or [`WAIT`] passes.
pub async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !condition() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
