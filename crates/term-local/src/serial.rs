//! Serial ports (UI 1.4, 2.2; D88): the ports of this device ([`ports`])
//! and a session on one of them ([`open`]). One thread owns the port: it
//! writes what [`SerialWriter`] queued, then reads with a short timeout,
//! and sends the output on a tokio channel.
//!
//! One handle, one thread: Windows serializes the I/O of a synchronous
//! handle, so a second handle for the reader would hold every write behind
//! a pending read, and the duplicate would be inheritable by programs the
//! app starts (which would keep the port busy). The thread looks at the
//! close flag between reads and between written chunks, so a closed
//! session lets go of the port within about [`READ_TIMEOUT`] (a port opens
//! once: Reconnect needs it free) even in the middle of a long paste; the
//! session is closed by [`SerialWriter::close`] or by dropping both its
//! output and its writers. A read or write error (the device was
//! unplugged) or end of file ends the output channel. Nothing here logs
//! port names, output or input.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::pty::{MAX_CHUNK, OUTPUT_CAPACITY};

/// How long a read waits before the thread writes what is queued and looks
/// at the close flag again; also the timeout of one write.
pub const READ_TIMEOUT: Duration = Duration::from_millis(20);
/// Queued input is written in chunks of this size, the close flag looked
/// at between them.
const WRITE_CHUNK: usize = 64;
/// How long the thread writes before it reads again (a long paste does not
/// hold the output back).
const WRITE_BUDGET: Duration = Duration::from_millis(100);
/// Chunks the writers may queue before [`SerialWriter::write`] waits.
const WRITE_QUEUE: usize = 64;

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

/// Whether two port names name the same port: on Windows the names are not
/// case-sensitive and may carry the device prefix (`\\.\COM12` is `COM12`).
pub fn same_port(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        let bare = |name: &str| {
            let name = name.trim();
            name.strip_prefix(r"\\.\").unwrap_or(name).to_owned()
        };
        bare(a).eq_ignore_ascii_case(&bare(b))
    } else {
        a == b
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
                if ports()
                    .iter()
                    .any(|port| same_port(&port.name, &config.port)) =>
            {
                SerialError::Busy
            }
            serialport::ErrorKind::NoDevice => SerialError::Missing,
            serialport::ErrorKind::InvalidInput => SerialError::Settings,
            serialport::ErrorKind::Io(ErrorKind::PermissionDenied) => SerialError::Busy,
            serialport::ErrorKind::Io(ErrorKind::NotFound) => SerialError::Missing,
            _ => SerialError::Io,
        })?;
    Ok(start(port))
}

/// What the session's thread reads and writes: the real port, a test's
/// fake.
pub trait Port: std::io::Read + std::io::Write + Send {}

impl<T: std::io::Read + std::io::Write + Send> Port for T {}

/// Starts the thread of an open port.
pub fn start(port: impl Port + 'static) -> SerialSession {
    let closed = Arc::new(AtomicBool::new(false));
    let (output_tx, output_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let (input_tx, input_rx) = std_mpsc::sync_channel(WRITE_QUEUE);
    let thread_closed = Arc::clone(&closed);
    let started = std::thread::Builder::new()
        .name("shellpp-serial".to_owned())
        .spawn(move || run(port, &thread_closed, &input_rx, &output_tx));
    if started.is_err() {
        // No thread: the output ends at once, the port was dropped.
        closed.store(true, Ordering::SeqCst);
    }
    SerialSession {
        output: output_rx,
        writer: SerialWriter {
            input: input_tx,
            closed,
        },
    }
}

/// The thread of an open port: queued input first (for at most
/// [`WRITE_BUDGET`]), then a read; until the session is closed or the
/// port fails. The port closes when the thread ends.
fn run(
    mut port: impl Port,
    closed: &AtomicBool,
    input: &std_mpsc::Receiver<Vec<u8>>,
    output: &mpsc::Sender<Vec<u8>>,
) {
    let mut buf = vec![0u8; MAX_CHUNK];
    let open = || !closed.load(Ordering::SeqCst) && !output.is_closed();
    'run: while open() {
        let started = Instant::now();
        while started.elapsed() < WRITE_BUDGET {
            let chunk = match input.try_recv() {
                Ok(chunk) => chunk,
                Err(std_mpsc::TryRecvError::Empty) => break,
                // Every writer is gone.
                Err(std_mpsc::TryRecvError::Disconnected) => break 'run,
            };
            if !open() {
                break 'run;
            }
            match port.write_all(&chunk).and_then(|()| port.flush()) {
                Ok(()) => {}
                // Flow control held it past the timeout: that chunk is lost,
                // not the session.
                Err(error) if is_timeout(&error) || error.kind() == ErrorKind::WriteZero => {}
                Err(_) => break 'run,
            }
        }
        match port.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if output.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(error) if is_timeout(&error) => {}
            // The device is gone (or another failure): the output ends.
            Err(_) => break,
        }
    }
    closed.store(true, Ordering::SeqCst);
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::TimedOut | ErrorKind::Interrupted | ErrorKind::WouldBlock
    )
}

/// Queues input for the port; clones share it.
#[derive(Clone)]
pub struct SerialWriter {
    input: std_mpsc::SyncSender<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

impl std::fmt::Debug for SerialWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialWriter")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl SerialWriter {
    /// Queues `data` in chunks; waits while the queue is full (blocking).
    /// A chunk that flow control holds past the write timeout is lost, not
    /// the session.
    pub fn write(&self, data: &[u8]) -> Result<(), SerialError> {
        for chunk in data.chunks(WRITE_CHUNK) {
            if self.is_closed() || self.input.send(chunk.to_vec()).is_err() {
                return Err(SerialError::Closed);
            }
        }
        Ok(())
    }

    /// Ends the session: the thread lets go of the port after the read or
    /// the chunk it is in. Never waits.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// D95: a typed name and a listed one name the same port; on Windows
    /// the case and the device prefix do not count.
    #[test]
    fn port_names_compare_as_the_system_does() {
        assert!(same_port("COM3", "COM3"));
        assert!(!same_port("COM3", "COM30"));
        assert!(same_port("/dev/ttyUSB0", "/dev/ttyUSB0"));
        assert_eq!(same_port("com3", "COM3"), cfg!(windows));
        assert_eq!(same_port(r"\\.\COM12", "COM12"), cfg!(windows));
        assert!(!same_port("/dev/ttyusb0", "/dev/ttyUSB0") || cfg!(windows));
    }

    /// A port that times out until the test sends it bytes, records what is
    /// written (slowly, when asked) and when it is dropped.
    struct Fake {
        incoming: std_mpsc::Receiver<std::io::Result<Vec<u8>>>,
        written: Arc<Mutex<Vec<u8>>>,
        write_delay: Duration,
        dropped: Arc<AtomicBool>,
    }

    struct Ends {
        device: std_mpsc::Sender<std::io::Result<Vec<u8>>>,
        written: Arc<Mutex<Vec<u8>>>,
        dropped: Arc<AtomicBool>,
    }

    fn fake(write_delay: Duration) -> (Fake, Ends) {
        let (device, incoming) = std_mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let port = Fake {
            incoming,
            written: Arc::clone(&written),
            write_delay,
            dropped: Arc::clone(&dropped),
        };
        (
            port,
            Ends {
                device,
                written,
                dropped,
            },
        )
    }

    impl std::io::Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.incoming.recv_timeout(READ_TIMEOUT) {
                Ok(Ok(bytes)) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err(ErrorKind::TimedOut.into()),
            }
        }
    }

    impl std::io::Write for Fake {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.write_delay);
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn wait_for(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn output_input_and_a_lost_device() {
        let (port, ends) = fake(Duration::ZERO);
        let mut session = start(port);
        ends.device.send(Ok(b"login: ".to_vec())).unwrap();
        assert_eq!(
            session.output.blocking_recv().as_deref(),
            Some(&b"login: "[..])
        );
        // Input goes out between reads, without waiting for output.
        let typed = Instant::now();
        session.writer.write(b"admin\r").unwrap();
        wait_for("the input written", || {
            *ends.written.lock().unwrap() == b"admin\r"
        });
        assert!(typed.elapsed() < Duration::from_millis(500));
        // Unplugged: the output ends, the port is let go, writes fail.
        ends.device.send(Err(ErrorKind::BrokenPipe.into())).unwrap();
        assert_eq!(session.output.blocking_recv(), None);
        wait_for("the port let go", || ends.dropped.load(Ordering::SeqCst));
        assert_eq!(session.writer.write(b"x"), Err(SerialError::Closed));
    }

    #[test]
    fn close_lets_go_of_the_port() {
        let (port, ends) = fake(Duration::ZERO);
        let mut session = start(port);
        session.writer.close();
        assert_eq!(session.output.blocking_recv(), None);
        wait_for("the port let go", || ends.dropped.load(Ordering::SeqCst));
    }

    /// A session dropped without a close (a tab closed while the port was
    /// opening) lets go of the port as well.
    #[test]
    fn a_dropped_session_lets_go_of_the_port() {
        let (port, ends) = fake(Duration::ZERO);
        drop(start(port));
        wait_for("the port let go", || ends.dropped.load(Ordering::SeqCst));
    }

    /// Closing in the middle of a long paste (9600 baud: about 70 ms per
    /// chunk) does not wait for the paste and stops it.
    #[test]
    fn close_stops_a_long_paste_at_once() {
        let (port, ends) = fake(Duration::from_millis(70));
        let session = start(port);
        let writer = session.writer.clone();
        let paste = std::thread::spawn(move || writer.write(&[b'x'; 20 * 1024]));
        wait_for("the paste started", || {
            !ends.written.lock().unwrap().is_empty()
        });
        let closing = Instant::now();
        session.writer.close();
        assert!(
            closing.elapsed() < Duration::from_millis(50),
            "close waited"
        );
        wait_for("the port let go", || ends.dropped.load(Ordering::SeqCst));
        // A chunk (70 ms) and a read (20 ms), not the 20 KB (about 22 s).
        assert!(closing.elapsed() < Duration::from_secs(1));
        assert_eq!(paste.join().unwrap(), Err(SerialError::Closed));
        assert!(ends.written.lock().unwrap().len() < 4 * 1024);
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
