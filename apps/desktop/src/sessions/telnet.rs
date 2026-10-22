//! Telnet sessions (UI 3.1 "Add Telnet", stage 9; run 5, plan 9; D87): a
//! TCP connection to the host's Telnet port and the same terminal as SSH.
//!
//! - RFC 854 with the options a terminal needs: ECHO and SUPPRESS-GO-AHEAD
//!   from the server, TERMINAL-TYPE (RFC 1091, `terminal.term`) and NAWS
//!   (RFC 1073, the window size, sent again on resize) from this side.
//!   Every other option is refused; an option is answered only when its
//!   state changes, so negotiation cannot loop.
//! - Output: `IAC IAC` is a 0xFF byte, `CR NUL` a CR; commands and
//!   subnegotiations never reach the terminal. Input: 0xFF is doubled and
//!   CR goes as `CR NUL`.
//! - Nothing is encrypted, the login included: the host panel says so.
//!   Login is interactive (the host's credentials are not sent).
//! - The host closing the connection is a drop "closed by the server", a
//!   network error one "connection lost": the output stays and Reconnect
//!   starts again (Telnet has no exit status).
//!
//! Nothing here logs terminal data.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use shellpp_ssh::FailureKind;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

use super::charset::OutputDecoder;
use super::transport::{BoxFuture, TermEnd, TermEvent, TermEvents, TermInput, TermParts};
use super::types::{ConnectStep, SessionError, SessionState};
use super::{Control, PumpEnd, Session, Shared, open_input, pump};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
/// Longest subnegotiation kept (a terminal type request is a few bytes).
const MAX_SUBNEGOTIATION: usize = 256;
/// How long a connection attempt may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Where the parser stands in the byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Data,
    /// After a CR: a NUL that follows is dropped.
    Cr,
    Iac,
    /// After WILL, WONT, DO or DONT: the option byte follows.
    Command(u8),
    Sb,
    SbIac,
}

/// The Telnet protocol without I/O: decodes output into terminal data and
/// replies, encodes input.
#[derive(Debug)]
pub struct Codec {
    state: State,
    sb: Vec<u8>,
    terminal: String,
    size: (u16, u16),
    /// Options this side has agreed to perform (WILL sent).
    local: [bool; 256],
    /// Options the server performs (DO sent).
    remote: [bool; 256],
}

impl Codec {
    /// A codec that reports `terminal` as its type and `size` (columns,
    /// rows) as its window.
    pub fn new(terminal: &str, size: (u16, u16)) -> Self {
        Self {
            state: State::Data,
            sb: Vec::new(),
            terminal: terminal.to_ascii_uppercase(),
            size,
            local: [false; 256],
            remote: [false; 256],
        }
    }

    /// Decodes `input`: terminal data goes to `data`, protocol replies to
    /// `reply`.
    pub fn decode(&mut self, input: &[u8], data: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &byte in input {
            match self.state {
                State::Data => self.data(byte, data),
                State::Cr => {
                    self.state = State::Data;
                    if byte != 0 {
                        self.data(byte, data);
                    }
                }
                State::Iac => {
                    self.state = State::Data;
                    match byte {
                        IAC => data.push(IAC),
                        WILL | WONT | DO | DONT => self.state = State::Command(byte),
                        SB => {
                            self.sb.clear();
                            self.state = State::Sb;
                        }
                        // NOP, GA, AYT and the others: nothing to show.
                        _ => {}
                    }
                }
                State::Command(command) => {
                    self.state = State::Data;
                    self.negotiate(command, byte, reply);
                }
                State::Sb => {
                    if byte == IAC {
                        self.state = State::SbIac;
                    } else if self.sb.len() < MAX_SUBNEGOTIATION {
                        self.sb.push(byte);
                    }
                }
                State::SbIac => match byte {
                    SE => {
                        self.state = State::Data;
                        self.subnegotiation(reply);
                    }
                    IAC => {
                        self.state = State::Sb;
                        if self.sb.len() < MAX_SUBNEGOTIATION {
                            self.sb.push(IAC);
                        }
                    }
                    _ => self.state = State::Data,
                },
            }
        }
    }

    fn data(&mut self, byte: u8, data: &mut Vec<u8>) {
        match byte {
            IAC => self.state = State::Iac,
            b'\r' => {
                data.push(byte);
                self.state = State::Cr;
            }
            _ => data.push(byte),
        }
    }

    fn negotiate(&mut self, command: u8, option: u8, reply: &mut Vec<u8>) {
        let index = usize::from(option);
        match command {
            WILL => {
                let wanted = matches!(option, OPT_ECHO | OPT_SGA);
                if wanted && !self.remote[index] {
                    self.remote[index] = true;
                    reply.extend_from_slice(&[IAC, DO, option]);
                } else if !wanted {
                    reply.extend_from_slice(&[IAC, DONT, option]);
                }
            }
            WONT if self.remote[index] => {
                self.remote[index] = false;
                reply.extend_from_slice(&[IAC, DONT, option]);
            }
            DO => {
                let offered = matches!(option, OPT_TTYPE | OPT_NAWS | OPT_SGA);
                if offered && !self.local[index] {
                    self.local[index] = true;
                    reply.extend_from_slice(&[IAC, WILL, option]);
                    if option == OPT_NAWS {
                        reply.extend_from_slice(&naws(self.size));
                    }
                } else if !offered {
                    reply.extend_from_slice(&[IAC, WONT, option]);
                }
            }
            DONT if self.local[index] => {
                self.local[index] = false;
                reply.extend_from_slice(&[IAC, WONT, option]);
            }
            _ => {}
        }
    }

    fn subnegotiation(&mut self, reply: &mut Vec<u8>) {
        if self.sb.first() == Some(&OPT_TTYPE)
            && self.sb.get(1) == Some(&TTYPE_SEND)
            && self.local[usize::from(OPT_TTYPE)]
        {
            reply.extend_from_slice(&[IAC, SB, OPT_TTYPE, TTYPE_IS]);
            for &byte in self.terminal.as_bytes() {
                if byte != IAC {
                    reply.push(byte);
                }
            }
            reply.extend_from_slice(&[IAC, SE]);
        }
    }

    /// The window's new size: the NAWS report to send, once agreed.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.size = (cols, rows);
        self.local[usize::from(OPT_NAWS)].then(|| naws(self.size))
    }

    /// Input for the wire: 0xFF doubled, CR as `CR NUL`.
    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len() + 4);
        for &byte in input {
            match byte {
                IAC => out.extend_from_slice(&[IAC, IAC]),
                b'\r' => out.extend_from_slice(&[b'\r', 0]),
                _ => out.push(byte),
            }
        }
        out
    }
}

/// A NAWS subnegotiation: width and height as 16-bit values, 0xFF doubled.
fn naws((cols, rows): (u16, u16)) -> Vec<u8> {
    let mut out = vec![IAC, SB, OPT_NAWS];
    for byte in cols.to_be_bytes().into_iter().chain(rows.to_be_bytes()) {
        out.push(byte);
        if byte == IAC {
            out.push(IAC);
        }
    }
    out.extend_from_slice(&[IAC, SE]);
    out
}

/// What the writer task sends.
enum Write {
    Bytes(Vec<u8>),
    Close,
}

/// The output side: reads the socket, gives the terminal its data and the
/// writer the replies. Cancel-safe: a read that is dropped loses nothing.
struct Events {
    read: OwnedReadHalf,
    codec: Arc<Mutex<Codec>>,
    writer: mpsc::UnboundedSender<Write>,
    buffer: Vec<u8>,
}

impl TermEvents for Events {
    fn next(&mut self) -> BoxFuture<'_, TermEvent> {
        Box::pin(async move {
            loop {
                self.buffer.resize(16 * 1024, 0);
                let read = match self.read.read(&mut self.buffer).await {
                    Ok(0) => return TermEvent::End(TermEnd::Dropped(FailureKind::Disconnected)),
                    Ok(read) => read,
                    Err(_) => return TermEvent::End(TermEnd::Dropped(FailureKind::Network)),
                };
                let mut data = Vec::with_capacity(read);
                let mut reply = Vec::new();
                if let Ok(mut codec) = self.codec.lock() {
                    codec.decode(&self.buffer[..read], &mut data, &mut reply);
                }
                if !reply.is_empty() {
                    let _ = self.writer.send(Write::Bytes(reply));
                }
                if !data.is_empty() {
                    return TermEvent::Data(data);
                }
            }
        })
    }
}

/// The input side: typed input and window sizes go to the writer task.
struct Input {
    codec: Arc<Mutex<Codec>>,
    writer: mpsc::UnboundedSender<Write>,
}

impl TermInput for Input {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        let sent = self.writer.send(Write::Bytes(Codec::encode(&data))).is_ok();
        Box::pin(async move { sent })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, bool> {
        let report = self
            .codec
            .lock()
            .ok()
            .and_then(|mut codec| codec.resize(cols, rows));
        let sent = match report {
            Some(report) => self.writer.send(Write::Bytes(report)).is_ok(),
            None => !self.writer.is_closed(),
        };
        Box::pin(async move { sent })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        let _ = self.writer.send(Write::Close);
        Box::pin(async {})
    }
}

/// Writes what it is given, in order, until the connection or the channel
/// closes.
async fn write_loop(mut write: OwnedWriteHalf, mut queue: mpsc::UnboundedReceiver<Write>) {
    while let Some(item) = queue.recv().await {
        match item {
            Write::Bytes(bytes) => {
                if write.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            Write::Close => break,
        }
    }
    let _ = write.shutdown().await;
}

/// Connects to `address`:`port` and returns the terminal over it.
pub async fn connect(
    address: &str,
    port: u16,
    terminal: &str,
    size: (u16, u16),
) -> Result<TermParts, FailureKind> {
    let addrs = shellpp_ssh::resolve(address, port).await?;
    let stream =
        shellpp_ssh::connect_any(&addrs, tokio::time::Instant::now() + CONNECT_TIMEOUT).await?;
    let (read, write) = stream.into_split();
    let codec = Arc::new(Mutex::new(Codec::new(terminal, size)));
    let (writer, queue) = mpsc::unbounded_channel();
    tokio::spawn(write_loop(write, queue));
    Ok(TermParts {
        events: Box::new(Events {
            read,
            codec: codec.clone(),
            writer: writer.clone(),
            buffer: Vec::new(),
        }),
        input: Arc::new(Input { codec, writer }),
        refused_env: Vec::new(),
    })
}

/// Where a Telnet session connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TelnetTarget {
    pub address: String,
    pub port: u16,
    /// What TERMINAL-TYPE reports (`terminal.term`).
    pub term: String,
}

/// The task of one Telnet session: connect, run the terminal until it
/// drops, and again on Reconnect.
pub(crate) async fn run(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut ctrl: mpsc::UnboundedReceiver<Control>,
    target: TelnetTarget,
) {
    let mut first = true;
    loop {
        let size = session.lock().size;
        session.update(|st| {
            st.view.state = SessionState::Connecting;
            st.view.step = Some(ConnectStep::Connecting);
            st.view.error = None;
        });
        let connected = tokio::select! {
            connected = connect(&target.address, target.port, &target.term, size) => connected,
            control = ctrl.recv() => match control {
                Some(Control::Cancel) => Err(FailureKind::Cancelled),
                Some(Control::Reconnect) => continue,
                Some(Control::Close) | None => return,
            },
        };
        match connected {
            Ok(parts) => {
                if !first {
                    // The terminal still shows the previous connection.
                    super::reset_terminal_modes(&session).await;
                }
                let mut input = None;
                session.update(|st| {
                    input = Some(open_input(st, size));
                    st.view.state = SessionState::Connected;
                    st.view.step = None;
                });
                let Some(input) = input else {
                    return;
                };
                tracing::info!(session = session.id, "telnet session connected");
                let decoder = OutputDecoder::new(encoding_rs::UTF_8);
                match pump(&shared, &session, &mut ctrl, parts, decoder, input, None).await {
                    PumpEnd::Closed => return,
                    PumpEnd::Ended(end) => {
                        let kind = match end {
                            TermEnd::Dropped(kind) => kind,
                            _ => FailureKind::Disconnected,
                        };
                        tracing::info!(session = session.id, "telnet session dropped");
                        session.update(|st| {
                            st.view.state = SessionState::Dropped;
                            st.view.error = Some(SessionError::from_drop(kind));
                        });
                    }
                }
            }
            Err(FailureKind::Cancelled) => {
                session.update(|st| {
                    st.view.state = SessionState::Failed;
                    st.view.step = None;
                    st.view.error = Some(SessionError::Cancelled);
                });
            }
            Err(kind) => {
                tracing::info!(session = session.id, "telnet session did not connect");
                session.update(|st| {
                    st.view.state = SessionState::Failed;
                    st.view.step = None;
                    st.view.error = Some(SessionError::Ssh(kind));
                });
            }
        }
        first = false;
        // Waits for Reconnect (or the end).
        loop {
            match ctrl.recv().await {
                Some(Control::Reconnect) => break,
                Some(Control::Cancel) => {}
                Some(Control::Close) | None => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(codec: &mut Codec, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (mut data, mut reply) = (Vec::new(), Vec::new());
        codec.decode(input, &mut data, &mut reply);
        (data, reply)
    }

    #[test]
    fn options_are_agreed_once_and_others_refused() {
        let mut codec = Codec::new("xterm-256color", (80, 24));
        let (data, reply) = decode(
            &mut codec,
            &[
                IAC, WILL, OPT_ECHO, IAC, WILL, OPT_SGA, IAC, DO, OPT_TTYPE, IAC, DO, 39,
            ],
        );
        assert!(data.is_empty());
        assert_eq!(
            reply,
            [
                IAC, DO, OPT_ECHO, IAC, DO, OPT_SGA, IAC, WILL, OPT_TTYPE, IAC, WONT, 39
            ]
        );
        // The same offer again: no answer (no loop).
        let (_, reply) = decode(&mut codec, &[IAC, WILL, OPT_ECHO, IAC, DO, OPT_TTYPE]);
        assert!(reply.is_empty());
        // An unknown option the server offers is refused.
        let (_, reply) = decode(&mut codec, &[IAC, WILL, 42]);
        assert_eq!(reply, [IAC, DONT, 42]);
        // The terminal type when asked.
        let (_, reply) = decode(&mut codec, &[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        let mut expected = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice(b"XTERM-256COLOR");
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(reply, expected);
    }

    #[test]
    fn the_window_size_goes_on_agreement_and_on_resize() {
        let mut codec = Codec::new("xterm", (80, 24));
        assert_eq!(codec.resize(100, 30), None, "not agreed yet");
        let (_, reply) = decode(&mut codec, &[IAC, DO, OPT_NAWS]);
        assert_eq!(
            reply,
            [
                IAC, WILL, OPT_NAWS, IAC, SB, OPT_NAWS, 0, 100, 0, 30, IAC, SE
            ]
        );
        // 255 columns: the byte is doubled.
        assert_eq!(
            codec.resize(255, 40),
            Some(vec![IAC, SB, OPT_NAWS, 0, 255, 255, 0, 40, IAC, SE])
        );
    }

    #[test]
    fn data_keeps_its_bytes_and_drops_the_protocol() {
        let mut codec = Codec::new("xterm", (80, 24));
        let (data, _) = decode(
            &mut codec,
            &[
                b'a', IAC, IAC, b'b', b'\r', 0, b'c', b'\r', b'\n', IAC, 241, b'd',
            ],
        );
        assert_eq!(data, [b'a', IAC, b'b', b'\r', b'c', b'\r', b'\n', b'd']);
        // Split anywhere: the same result.
        let mut codec = Codec::new("xterm", (80, 24));
        let mut all = Vec::new();
        for byte in [
            b'x', IAC, WILL, OPT_ECHO, b'\r', 0, b'y', IAC, SB, 5, 6, IAC, SE,
        ] {
            let (data, _) = decode(&mut codec, &[byte]);
            all.extend(data);
        }
        assert_eq!(all, [b'x', b'\r', b'y']);
    }

    #[test]
    fn input_doubles_iac_and_sends_cr_nul() {
        assert_eq!(
            Codec::encode(&[b'l', b's', b'\r', IAC]),
            [b'l', b's', b'\r', 0, IAC, IAC]
        );
    }
}
