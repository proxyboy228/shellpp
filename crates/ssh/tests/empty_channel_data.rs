//! The second russh patch (third_party/russh/PATCHES.md, change 2; D191): an
//! empty `CHANNEL_DATA` (or `CHANNEL_EXTENDED_DATA`) is valid SSH (RFC 4254,
//! section 5.2, puts no lower bound on the data), but russh 0.64.1's
//! `ChannelRx` handed it to the reader as a read of 0 bytes, which every
//! `AsyncRead` user takes for the end of the stream. An SFTP session
//! (`russh_sftp` over `Channel::into_stream`, the path of `shellpp_ssh::Sftp`)
//! ended on such a packet, and so did any stream read through the same type.
//!
//! russh never sends an empty data packet itself, so the server here is a
//! russh server behind a proxy that adds one: both ends negotiate the `none`
//! cipher and MAC (test only), so the proxy can frame the server's packets
//! and put an empty data packet for the same channel after a chosen one.

mod support;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::PublicKeyOrCertificate;
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId, Preferred, cipher, compression, mac};
use russh_sftp::protocol::{File, Name, StatusCode};
use secrecy::ExposeSecret;
use shellpp_test_sshd::{KeyType, generate_key};
use support::random_secret;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// How long one step may take.
const STEP: Duration = Duration::from_secs(10);

/// `SSH_MSG_CHANNEL_DATA` and `SSH_MSG_CHANNEL_EXTENDED_DATA` (RFC 4254).
const CHANNEL_DATA: u8 = 94;
const CHANNEL_EXTENDED_DATA: u8 = 95;

/// `future`, or a panic naming `what` after [`STEP`].
async fn in_time<T>(what: &str, future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(STEP, future)
        .await
        .unwrap_or_else(|_| panic!("{what}: no answer in time"))
}

/// Cleartext transport, so that the proxy can read and add packets.
fn cleartext() -> Preferred {
    Preferred {
        cipher: Cow::Borrowed(&[cipher::NONE]),
        mac: Cow::Borrowed(&[mac::NONE]),
        compression: Cow::Borrowed(&[compression::NONE]),
        ..Preferred::default()
    }
}

/// The server: one password, SFTP on the `sftp` subsystem, and an exec
/// that writes two lines to stdout and two to stderr, then ends.
struct Server {
    password: String,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == self.password {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        self.channels.insert(channel.id(), channel);
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match self.channels.remove(&channel) {
            Some(open) if name == "sftp" => {
                session.channel_success(channel)?;
                russh_sftp::server::run(open.into_stream(), Sftp).await;
            }
            _ => session.channel_failure(channel)?,
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, &b"first\n"[..])?;
        session.extended_data(channel, 1, &b"err first\n"[..])?;
        session.data(channel, &b"second\n"[..])?;
        session.extended_data(channel, 1, &b"err second\n"[..])?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)
    }
}

/// SFTP that knows one path: every `realpath` answers the home folder.
struct Sftp;

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn realpath(&mut self, id: u32, _path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![File::dummy("/home/alice")],
        })
    }
}

/// Starts the server and a proxy in front of it. The proxy passes the
/// server's packets on and, after the `after`-th packet of type `kind`, adds
/// an empty one of that type for the same channel (and stream). Returns
/// the proxy's address and the flag it raises once it added the packet.
async fn start(password: &str, kind: u8, after: usize) -> (std::net::SocketAddr, Arc<AtomicBool>) {
    let generated = generate_key(KeyType::Ed25519, "empty-data-host", None).expect("host key");
    let host_key = russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
        .expect("decodes");
    let config = Arc::new(russh::server::Config {
        keys: vec![host_key],
        preferred: cleartext(),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let server = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = server.local_addr().expect("address");
    let password = password.to_owned();
    tokio::spawn(async move {
        while let Ok((stream, _)) = server.accept().await {
            let handler = Server {
                password: password.clone(),
                channels: HashMap::new(),
            };
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, stream, handler).await {
                    let _ = session.await;
                }
            });
        }
    });

    let proxy = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let proxy_addr = proxy.local_addr().expect("address");
    let added = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&added);
    tokio::spawn(async move {
        let (client, _) = proxy.accept().await.expect("the client connects");
        let upstream = TcpStream::connect(server_addr)
            .await
            .expect("to the server");
        let (mut client_read, mut client_write) = client.into_split();
        let (server_read, mut server_write) = upstream.into_split();
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
        });
        let _ = relay(
            BufReader::new(server_read),
            &mut client_write,
            kind,
            after,
            &flag,
        )
        .await;
    });
    (proxy_addr, added)
}

/// Copies the server's side to the client packet by packet (cleartext:
/// length, padding length, payload, padding; no MAC) and adds the empty one.
async fn relay(
    mut from: BufReader<tokio::net::tcp::OwnedReadHalf>,
    to: &mut tokio::net::tcp::OwnedWriteHalf,
    kind: u8,
    after: usize,
    added: &AtomicBool,
) -> std::io::Result<()> {
    let mut version = Vec::new();
    from.read_until(b'\n', &mut version).await?;
    to.write_all(&version).await?;
    let mut seen = 0;
    loop {
        let length = from.read_u32().await?;
        let mut rest = vec![0; length as usize];
        from.read_exact(&mut rest).await?;
        to.write_all(&length.to_be_bytes()).await?;
        to.write_all(&rest).await?;
        let payload = &rest[1..rest.len() - usize::from(rest[0])];
        if payload.first() != Some(&kind) || added.load(Ordering::SeqCst) {
            continue;
        }
        seen += 1;
        if seen == after {
            // The recipient channel, plus the stream code for extended data.
            let header = if kind == CHANNEL_DATA { 5 } else { 9 };
            let mut empty = payload[..header].to_vec();
            empty.extend_from_slice(&0u32.to_be_bytes());
            to.write_all(&packet(&empty)).await?;
            added.store(true, Ordering::SeqCst);
        }
    }
}

/// A cleartext binary packet (RFC 4253, section 6) around `payload`.
fn packet(payload: &[u8]) -> Vec<u8> {
    let mut padding = 8 - (5 + payload.len()) % 8;
    if padding < 4 {
        padding += 8;
    }
    let length = 1 + payload.len() + padding;
    let mut packet = Vec::with_capacity(4 + length);
    packet.extend_from_slice(&u32::try_from(length).unwrap().to_be_bytes());
    packet.push(u8::try_from(padding).unwrap());
    packet.extend_from_slice(payload);
    packet.resize(4 + length, 0);
    packet
}

struct Client;

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

async fn connect(addr: std::net::SocketAddr, password: &str) -> Handle<Client> {
    let config = Arc::new(client::Config {
        preferred: cleartext(),
        ..Default::default()
    });
    let mut handle = client::connect(config, addr, Client)
        .await
        .expect("connects");
    let auth = handle
        .authenticate_password("alice", password)
        .await
        .expect("authenticates");
    assert!(auth.success());
    handle
}

#[tokio::test]
async fn an_empty_data_packet_does_not_end_an_sftp_session() {
    let password = random_secret("pw");
    // The first data packet is the server's VERSION, the second the answer
    // to the first request: the empty one comes mid-session, between two.
    let (addr, added) = start(&password, CHANNEL_DATA, 2).await;
    let handle = in_time("connect", connect(addr, &password)).await;
    let channel = in_time("open", handle.channel_open_session())
        .await
        .expect("opens");
    in_time("subsystem", channel.request_subsystem(true, "sftp"))
        .await
        .expect("subsystem requested");
    let sftp = tokio::time::timeout(
        STEP,
        russh_sftp::client::SftpSession::new(channel.into_stream()),
    )
    .await
    .expect("in time")
    .expect("SFTP starts");
    let first = tokio::time::timeout(STEP, sftp.canonicalize("."))
        .await
        .expect("in time");
    assert_eq!(first.expect("first request"), "/home/alice");
    for round in 0..3 {
        let path = tokio::time::timeout(STEP, sftp.canonicalize("."))
            .await
            .unwrap_or_else(|_| panic!("round {round}: no answer"))
            .unwrap_or_else(|error| panic!("round {round}: the session ended: {error}"));
        assert_eq!(path, "/home/alice");
    }
    assert!(added.load(Ordering::SeqCst), "the proxy added the packet");
}

#[tokio::test]
async fn an_empty_data_packet_does_not_end_a_channel_stream() {
    let password = random_secret("pw");
    let (addr, added) = start(&password, CHANNEL_DATA, 1).await;
    let handle = in_time("connect", connect(addr, &password)).await;
    let channel = in_time("open", handle.channel_open_session())
        .await
        .expect("opens");
    in_time("exec", channel.exec(true, "lines"))
        .await
        .expect("exec sent");
    let mut stream = channel.into_stream();
    let mut output = Vec::new();
    tokio::time::timeout(STEP, stream.read_to_end(&mut output))
        .await
        .expect("in time")
        .expect("reads");
    assert!(added.load(Ordering::SeqCst), "the proxy added the packet");
    assert_eq!(String::from_utf8_lossy(&output), "first\nsecond\n");
}

#[tokio::test]
async fn an_empty_extended_data_packet_does_not_end_a_stderr_reader() {
    let password = random_secret("pw");
    let (addr, added) = start(&password, CHANNEL_EXTENDED_DATA, 1).await;
    let handle = in_time("connect", connect(addr, &password)).await;
    let mut channel = in_time("open", handle.channel_open_session())
        .await
        .expect("opens");
    in_time("exec", channel.exec(true, "lines"))
        .await
        .expect("exec sent");
    let mut output = Vec::new();
    tokio::time::timeout(
        STEP,
        channel.make_reader_ext(Some(1)).read_to_end(&mut output),
    )
    .await
    .expect("in time")
    .expect("reads");
    assert!(added.load(Ordering::SeqCst), "the proxy added the packet");
    assert_eq!(String::from_utf8_lossy(&output), "err first\nerr second\n");
}
