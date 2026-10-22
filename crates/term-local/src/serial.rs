//! Serial ports (UI 1.4, 2.2; D88): the ports of this device ([`ports`])
//! and a session on one of them ([`open`]). Output arrives on a tokio
//! channel from a reader thread; input goes through [`SerialWriter`].
//!
//! The reader thread reads with a short timeout, so that it notices a
//! [`SerialWriter::close`] within [`READ_TIMEOUT`] and lets go of the port
//! (a port opens once: Reconnect needs it free). A read error (the device
//! was unplugged) or end of file ends the output channel. Nothing here logs
//! port names, output or input.

use std::io::{ErrorKind, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::pty::{MAX_CHUNK, OUTPUT_CAPACITY};

/// How long a read waits before the reader thread looks at the close flag
/// again; also the write timeout.
pub const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// Data bits per character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DataBits {
    Five,
    Six,
    Seven,
    #[default]
    Eight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StopBits {
    #[default]
    One,
    Two,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FlowControl {
    #[default]
    None,
    /// XON/XOFF.
    Software,
    /// RTS/CTS.
    Hardware,
}

/// How to open a port: 9600 8N1 without flow control by default (the
/// console of most network devices).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SerialConfig {
    /// `COM3`, `/dev/ttyUSB0`.
    pub port: String,
    pub baud: u32,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow: FlowControl,
}

/// The baud rate of a new connection.
pub const DEFAULT_BAUD: u32 = 9600;

impl SerialConfig {
    /// `port` at the defaults.
    pub fn new(port: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            baud: DEFAULT_BAUD,
            data_bits: DataBits::default(),
            parity: Parity::default(),
            stop_bits: StopBits::default(),
            flow: FlowControl::default(),
        }
    }

    /// The short form of the settings: `9600 8N1`.
    pub fn summary(&self) -> String {
        let bits = match self.data_bits {
            DataBits::Five => '5',
            DataBits::Six => '6',
            DataBits::Seven => '7',
            DataBits::Eight => '8',
        };
        let parity = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        let stop = match self.stop_bits {
            StopBits::One => '1',
            StopBits::Two => '2',
        };
        format!("{} {bits}{parity}{stop}", self.baud)
    }
}

/// A port of this device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    /// What [`SerialConfig::port`] takes.
    pub name: String,
    /// The USB product or manufacturer, when the system knows it.
    pub description: Option<String>,
}

/// Why a port did not open or a write failed. No port names in the
/// messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SerialError {
    /// The system does not list the port.
    #[error("no such port")]
    Missing,
    /// The port is listed but cannot be opened: another program has it, or
    /// access is denied.
    #[error("port busy")]
    Busy,
    /// The port refuses the baud rate or the other settings.
    #[error("settings refused")]
    Settings,
    /// Any other failure.
    #[error("i/o error")]
    Io,
    /// The session was closed.
    #[error("closed")]
    Closed,
}

/// The serial ports of this device, sorted by name (blocking; a failed
/// enumeration is an empty list).
pub fn ports() -> Vec<PortInfo> {
    let mut ports: Vec<PortInfo> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|port| PortInfo {
            description: match port.port_type {
                serialport::SerialPortType::UsbPort(usb) => usb.product.or(usb.manufacturer),
                _ => None,
            },
            name: port.port_name,
        })
        .collect();
    sort_ports(&mut ports);
    ports
}

/// Sorts ports by name, numbers by value (`COM9` before `COM10`), and
/// drops a name listed twice.
pub fn sort_ports(ports: &mut Vec<PortInfo>) {
    ports.sort_by_key(|port| natural_key(&port.name));
    ports.dedup_by(|a, b| a.name == b.name);
}

/// `COM10` after `COM9`.
fn natural_key(name: &str) -> (String, u64) {
    let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (stem, number) = name.split_at(name.len() - digits);
    (stem.to_ascii_lowercase(), number.parse().unwrap_or(0))
}

/// An open port: its output and the writer.
#[derive(Debug)]
pub struct SerialSession {
    /// Chunks as read; closed when the port fails, reaches end of file or
    /// the session is closed.
    pub output: mpsc::Receiver<Vec<u8>>,
    pub writer: SerialWriter,
}

/// Opens the port (blocking).
pub fn open(config: &SerialConfig) -> Result<SerialSession, SerialError> {
    let port = serialport::new(config.port.as_str(), config.baud)
        .data_bits(match config.data_bits {
            DataBits::Five => serialport::DataBits::Five,
            DataBits::Six => serialport::DataBits::Six,
            DataBits::Seven => serialport::DataBits::Seven,
            DataBits::Eight => serialport::DataBits::Eight,
        })
        .parity(match config.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        })
        .stop_bits(match config.stop_bits {
            StopBits::One => serialport::StopBits::One,
            StopBits::Two => serialport::StopBits::Two,
        })
        .flow_control(match config.flow {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Software => serialport::FlowControl::Software,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
        })
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|error| match error.kind() {
            // Windows reports a port in use as "no device" too.
            serialport::ErrorKind::NoDevice
                if ports().iter().any(|port| port.name == config.port) =>
            {
                SerialError::Busy
            }
            serialport::ErrorKind::NoDevice => SerialError::Missing,
            serialport::ErrorKind::InvalidInput => SerialError::Settings,
            serialport::ErrorKind::Io(ErrorKind::PermissionDenied) => SerialError::Busy,
            serialport::ErrorKind::Io(ErrorKind::NotFound) => SerialError::Missing,
            _ => SerialError::Io,
        })?;
    let reader = port.try_clone().map_err(|_| SerialError::Io)?;
    Ok(start(reader, port))
}

/// A port as two halves: what the reader thread reads and what the writer
/// writes (the real port and its clone; a test's pipe).
pub fn start(
    reader: impl std::io::Read + Send + 'static,
    writer: impl std::io::Write + Send + 'static,
) -> SerialSession {
    let shared = Arc::new(Shared {
        writer: Mutex::new(Some(Box::new(writer))),
        closed: AtomicBool::new(false),
    });
    let (output_tx, output_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let reader_shared = Arc::clone(&shared);
    let started = std::thread::Builder::new()
        .name("shellpp-serial-reader".to_owned())
        .spawn(move || read_loop(reader, &reader_shared, &output_tx));
    if started.is_err() {
        // No reader: the output ends at once and the port is let go.
        shared.close();
    }
    SerialSession {
        output: output_rx,
        writer: SerialWriter { shared },
    }
}

struct Shared {
    writer: Mutex<Option<Box<dyn std::io::Write + Send>>>,
    closed: AtomicBool,
}

impl Shared {
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        drop(lock(&self.writer).take());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_loop(mut reader: impl std::io::Read, shared: &Shared, output: &mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; MAX_CHUNK];
    while !shared.closed.load(Ordering::SeqCst) {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if output.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::TimedOut | ErrorKind::Interrupted | ErrorKind::WouldBlock
                ) => {}
            // The device is gone (or another failure): the output ends.
            Err(_) => break,
        }
    }
    // The reader's half closes with this thread; the writer's too.
    shared.close();
}

/// Writes to the port; clones share it.
#[derive(Clone)]
pub struct SerialWriter {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for SerialWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialWriter")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl SerialWriter {
    /// Writes all of `data` (blocking). A write that times out (flow
    /// control holds it) loses that write, not the session: `Io`.
    pub fn write(&self, data: &[u8]) -> Result<(), SerialError> {
        let mut writer = lock(&self.shared.writer);
        let Some(port) = writer.as_mut() else {
            return Err(SerialError::Closed);
        };
        port.write_all(data)
            .and_then(|()| port.flush())
            .map_err(|_| SerialError::Io)
    }

    /// Lets go of the port: the reader thread ends within [`READ_TIMEOUT`].
    pub fn close(&self) {
        self.shared.close();
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that times out until the test sends it bytes.
    struct Fake(std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>);

    impl std::io::Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(bytes)) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err(ErrorKind::TimedOut.into()),
            }
        }
    }

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn output_input_and_a_lost_device() {
        let (send, receive) = std::sync::mpsc::channel();
        let sink = Sink::default();
        let mut session = start(Fake(receive), sink.clone());
        send.send(Ok(b"login: ".to_vec())).unwrap();
        assert_eq!(
            session.output.blocking_recv().as_deref(),
            Some(&b"login: "[..])
        );
        session.writer.write(b"admin\r").unwrap();
        assert_eq!(*lock(&sink.0), b"admin\r");
        // Unplugged: the output ends and writes fail.
        send.send(Err(ErrorKind::BrokenPipe.into())).unwrap();
        assert_eq!(session.output.blocking_recv(), None);
        assert_eq!(session.writer.write(b"x"), Err(SerialError::Closed));
    }

    #[test]
    fn close_ends_the_reader() {
        let (_send, receive) = std::sync::mpsc::channel();
        let mut session = start(Fake(receive), Sink::default());
        session.writer.close();
        // The reader notices within its timeout and the channel closes.
        assert_eq!(session.output.blocking_recv(), None);
    }

    #[test]
    fn summaries_and_port_order() {
        let mut config = SerialConfig::new("COM3");
        assert_eq!(config.summary(), "9600 8N1");
        config.baud = 115_200;
        config.data_bits = DataBits::Seven;
        config.parity = Parity::Even;
        config.stop_bits = StopBits::Two;
        assert_eq!(config.summary(), "115200 7E2");
        let mut names = vec!["COM10", "COM9", "/dev/ttyUSB1", "/dev/ttyACM0", "COM1"];
        names.sort_by_key(|name| natural_key(name));
        assert_eq!(
            names,
            ["/dev/ttyACM0", "/dev/ttyUSB1", "COM1", "COM9", "COM10"]
        );
    }
}
