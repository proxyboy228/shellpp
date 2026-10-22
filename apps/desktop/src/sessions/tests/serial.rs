//! Serial sessions (D88) on a fake port behind [`SerialBackend`]: the
//! port's output in the terminal, typed input on the port, a device that
//! goes away, Reconnect, the reasons a port does not open, and a Cancel
//! while a slow port opens.

use std::io::ErrorKind;
use std::sync::mpsc as std_mpsc;

use shellpp_term_local::serial::{self, PortInfo, SerialConfig, SerialError, SerialSession};

use super::*;

/// The far end of an open fake port.
struct Wire {
    /// Bytes the device sends (`Err` unplugs it).
    to_port: std_mpsc::Sender<std::io::Result<Vec<u8>>>,
    /// What the session wrote.
    written: Arc<Mutex<Vec<u8>>>,
}

/// A port: reads time out until the device sends something; what is
/// written is kept; dropping it lets go of the port (`open_ports`).
struct Port {
    from_device: std_mpsc::Receiver<std::io::Result<Vec<u8>>>,
    written: Arc<Mutex<Vec<u8>>>,
    open_ports: Arc<AtomicUsize>,
}

impl Drop for Port {
    fn drop(&mut self) {
        self.open_ports.fetch_sub(1, Ordering::SeqCst);
    }
}

impl std::io::Read for Port {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.from_device.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(bytes)) => {
                buf[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            Ok(Err(error)) => Err(error),
            Err(std_mpsc::RecvTimeoutError::Timeout) => Err(ErrorKind::TimedOut.into()),
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(ErrorKind::BrokenPipe.into()),
        }
    }
}

impl std::io::Write for Port {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written.lock().expect("written").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Lists `COM3`; each open (after `open_delay`) hands its far end to the
/// test, or fails with the next queued error.
#[derive(Default)]
struct FakePorts {
    opened: Mutex<Vec<SerialConfig>>,
    wires: Mutex<Option<std_mpsc::Sender<Wire>>>,
    failures: Mutex<Vec<SerialError>>,
    open_delay: Duration,
    /// Ports open now, and opens that succeeded.
    open_ports: Arc<AtomicUsize>,
    opens: AtomicUsize,
    /// The far ends nobody asked for: kept, so a quiet device stays
    /// plugged in (a dropped end reads as unplugged).
    idle: Mutex<Vec<Wire>>,
}

impl SerialBackend for FakePorts {
    fn ports(&self) -> Vec<PortInfo> {
        vec![PortInfo {
            name: "COM3".into(),
            description: Some("USB Serial".into()),
        }]
    }

    fn open(&self, config: &SerialConfig) -> Result<SerialSession, SerialError> {
        self.opened.lock().expect("opened").push(config.clone());
        std::thread::sleep(self.open_delay);
        if let Some(error) = self.failures.lock().expect("failures").pop() {
            return Err(error);
        }
        let (to_port, from_device) = std_mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        self.open_ports.fetch_add(1, Ordering::SeqCst);
        self.opens.fetch_add(1, Ordering::SeqCst);
        let session = serial::start(Port {
            from_device,
            written: written.clone(),
            open_ports: Arc::clone(&self.open_ports),
        });
        let wire = Wire { to_port, written };
        match self.wires.lock().expect("wires").as_ref() {
            Some(wires) => {
                let _ = wires.send(wire);
            }
            None => self.idle.lock().expect("idle").push(wire),
        }
        Ok(session)
    }
}

/// Waits for the next open of the fake.
async fn next_wire(wires: &mut std_mpsc::Receiver<Wire>) -> Wire {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(wire) = wires.try_recv() {
            return wire;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the port was not opened"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn written_until(wire: &Wire, wanted: &[u8]) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let written = wire.written.lock().expect("written").clone();
        if written.windows(wanted.len()).any(|window| window == wanted) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{wanted:?} not written; got {written:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn harness(ports: Arc<FakePorts>) -> Harness {
    Harness::with(move |options| options.serial = ports)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_serial_port_runs_drops_and_reconnects() {
    let ports = Arc::new(FakePorts::default());
    let (wires_tx, mut wires) = std_mpsc::channel();
    *ports.wires.lock().expect("wires") = Some(wires_tx);
    let harness = harness(ports.clone());
    let mut config = SerialConfig::new("COM3");
    config.baud = 115_200;
    let mut session = harness
        .open_with(
            OpenRequest::Serial(config.clone()),
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    assert_eq!(session.view.kind, SessionKind::Serial);
    assert_eq!(session.view.title, "COM3");
    let wire = next_wire(&mut wires).await;
    session.state(SessionState::Connected).await;
    assert_eq!(*ports.opened.lock().expect("opened"), [config.clone()]);

    // The device's output in the terminal, typed input on the port.
    wire.to_port
        .send(Ok(b"\r\nRouter>".to_vec()))
        .expect("send");
    session.output_from(0, "Router>").await;
    session.type_line("enable");
    written_until(&wire, b"enable\r").await;
    // A resize is nothing to a serial line.
    session.handle.resize(100, 30);

    // Unplugged: dropped, the output kept.
    wire.to_port
        .send(Err(ErrorKind::PermissionDenied.into()))
        .expect("send");
    let view = session.state(SessionState::Dropped).await;
    assert_eq!(view.error, Some(SessionError::SerialLost));
    assert!(session.terminal.text().contains("Router>"));

    // Reconnect opens the port again with the same settings.
    session.handle.reconnect();
    let wire = next_wire(&mut wires).await;
    session.state(SessionState::Connected).await;
    assert_eq!(ports.opened.lock().expect("opened").len(), 2);
    assert_eq!(ports.opened.lock().expect("opened")[1], config);
    session.type_line("show version");
    written_until(&wire, b"show version\r").await;

    // Closing lets go of the port.
    session.handle.close();
    session.state(SessionState::Closed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_port_that_does_not_open_says_why_and_reconnect_tries_again() {
    let ports = Arc::new(FakePorts::default());
    // Popped from the end: busy first, then missing.
    *ports.failures.lock().expect("failures") = vec![SerialError::Missing, SerialError::Busy];
    let harness = harness(ports.clone());
    let mut session = harness
        .open_with(
            OpenRequest::Serial(SerialConfig::new("COM3")),
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::Serial(SerialError::Busy)));
    assert_eq!(
        view.error.map(SessionError::message),
        Some("The serial port is in use by another program, or access to it is denied.")
    );
    session.handle.reconnect();
    session
        .until("missing", |view| {
            view.error == Some(SessionError::Serial(SerialError::Missing))
        })
        .await;
    // The third attempt opens.
    session.handle.reconnect();
    session.state(SessionState::Connected).await;
    assert_eq!(ports.opened.lock().expect("opened").len(), 3);
}

/// Cancel while a slow port opens (a Bluetooth port can take seconds):
/// the tab fails as cancelled at once, and the port the opening still
/// opens is let go, so the next attempt finds it free (review, D88).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_the_port_opens_lets_go_of_it() {
    let ports = Arc::new(FakePorts {
        open_delay: Duration::from_millis(300),
        ..FakePorts::default()
    });
    let harness = harness(ports.clone());
    let mut session = harness
        .open_with(
            OpenRequest::Serial(SerialConfig::new("COM3")),
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    // The port is opening (its blocking thread is in the open).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while ports.opened.lock().expect("opened").is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the open did not start"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Reconnect while it opens starts no second open.
    session.handle.reconnect();
    session.handle.cancel();
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::Cancelled));
    assert_eq!(ports.opened.lock().expect("opened").len(), 1);
    // The cancelled opening still opens the port, which is let go at once.
    while ports.opens.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while ports.open_ports.load(Ordering::SeqCst) != 0 {
        assert!(std::time::Instant::now() < deadline, "the port stayed open");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Reconnect opens it again.
    session.handle.reconnect();
    session.state(SessionState::Connected).await;
    assert_eq!(ports.open_ports.load(Ordering::SeqCst), 1);
    session.handle.close();
    session.state(SessionState::Closed).await;
}
