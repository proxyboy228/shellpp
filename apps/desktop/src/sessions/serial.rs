//! Serial sessions (UI 1.4, 2.2; D88): a port of this device in a session
//! tab with the same terminal. The port opens with the dialog's settings;
//! bytes go to the port as typed and its output goes to the terminal
//! (UTF-8). A port that fails to open is `Failed` with the reason; a device
//! that goes away is `Dropped` ("disconnected"), and Reconnect opens the
//! port again. No window size, no exit status, no connection log; serial
//! tabs are never saved (data-model 12).

use std::sync::Arc;

use shellpp_term_local::serial::{self, PortInfo, SerialConfig, SerialError, SerialSession};
use tokio::sync::mpsc;

use super::charset::OutputDecoder;
use super::transport::{BoxFuture, TermEnd, TermEvent, TermEvents, TermInput, TermParts};
use super::types::{SessionError, SessionState};
use super::{Control, PumpEnd, Session, Shared, open_input, pump};
use shellpp_ssh::FailureKind;

/// The ports of the device and opening one (real: `shellpp-term-local`).
/// Both may block.
pub trait SerialBackend: Send + Sync + 'static {
    fn ports(&self) -> Vec<PortInfo>;
    fn open(&self, config: &SerialConfig) -> Result<SerialSession, SerialError>;
}

/// The real backend.
#[derive(Debug, Default)]
pub struct PortBackend;

impl SerialBackend for PortBackend {
    fn ports(&self) -> Vec<PortInfo> {
        serial::ports()
    }

    fn open(&self, config: &SerialConfig) -> Result<SerialSession, SerialError> {
        serial::open(config)
    }
}

fn parts(session: SerialSession) -> TermParts {
    let SerialSession { output, writer } = session;
    TermParts {
        events: Box::new(Events(output)),
        input: Arc::new(Input(writer)),
        refused_env: Vec::new(),
    }
}

struct Events(mpsc::Receiver<Vec<u8>>);

impl TermEvents for Events {
    fn next(&mut self) -> BoxFuture<'_, TermEvent> {
        Box::pin(async move {
            match self.0.recv().await {
                Some(chunk) => TermEvent::Data(chunk),
                // The device went away (or the port failed).
                None => TermEvent::End(TermEnd::Dropped(FailureKind::Disconnected)),
            }
        })
    }
}

struct Input(serial::SerialWriter);

impl TermInput for Input {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        let writer = self.0.clone();
        Box::pin(async move {
            // Queued for the port's thread (waits while its queue is full);
            // only a closed port ends the input.
            tokio::task::spawn_blocking(move || {
                !matches!(writer.write(&data), Err(SerialError::Closed))
            })
            .await
            .unwrap_or(false)
        })
    }

    /// A serial line has no window size.
    fn resize(&self, _cols: u16, _rows: u16) -> BoxFuture<'_, bool> {
        Box::pin(async { true })
    }

    /// Never waits: the port's thread lets go of the port after the read
    /// or the chunk it is in.
    fn close(&self) -> BoxFuture<'_, ()> {
        self.0.close();
        Box::pin(async {})
    }
}

/// Opens the port on a blocking thread.
async fn open(shared: &Arc<Shared>, config: &SerialConfig) -> Result<TermParts, SerialError> {
    let backend = shared.serial.clone();
    let config = config.clone();
    tokio::task::spawn_blocking(move || backend.open(&config).map(parts))
        .await
        .unwrap_or(Err(SerialError::Io))
}

/// The task of one serial session: open the port, run the terminal until
/// the device goes away, and again on Reconnect.
pub(crate) async fn run(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut ctrl: mpsc::UnboundedReceiver<Control>,
    config: SerialConfig,
) {
    let mut first = true;
    loop {
        let size = session.lock().size;
        // Close or Cancel while the port opens: the opening goes on in its
        // blocking thread, and the port it opens is let go as soon as its
        // result is dropped (shellpp-term-local's thread ends with its
        // receiver). Reconnect while it opens changes nothing: a second
        // open would find the port busy.
        let opening = open(&shared, &config);
        tokio::pin!(opening);
        let opened = loop {
            tokio::select! {
                opened = &mut opening => break opened.map_err(Some),
                control = ctrl.recv() => match control {
                    Some(Control::Close) | None => return,
                    Some(Control::Cancel) => break Err(None),
                    Some(Control::Reconnect) => {}
                },
            }
        };
        match opened {
            Ok(parts) => {
                if !first {
                    // The terminal still shows the previous connection.
                    super::reset_terminal_modes(&session).await;
                }
                let mut input = None;
                session.update(|st| {
                    input = Some(open_input(st, size));
                    st.view.state = SessionState::Connected;
                    st.view.error = None;
                });
                let Some(input) = input else {
                    return;
                };
                tracing::info!(session = session.id, "serial port opened");
                let decoder = OutputDecoder::new(encoding_rs::UTF_8);
                match pump(&shared, &session, &mut ctrl, parts, decoder, input, None).await {
                    PumpEnd::Closed => return,
                    PumpEnd::Ended(_) => {
                        tracing::info!(session = session.id, "serial port went away");
                        session.update(|st| {
                            st.view.state = SessionState::Dropped;
                            st.view.error = Some(SessionError::SerialLost);
                        });
                    }
                }
            }
            Err(error) => {
                tracing::info!(session = session.id, "serial port did not open");
                session.update(|st| {
                    st.view.state = SessionState::Failed;
                    st.view.error =
                        Some(error.map_or(SessionError::Cancelled, SessionError::Serial));
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
        session.update(|st| {
            st.view.state = SessionState::Connecting;
            st.view.error = None;
        });
    }
}
