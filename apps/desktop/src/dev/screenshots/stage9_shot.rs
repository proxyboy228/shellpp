//! The stage 9 states of the screenshot run (run 5, plan 9; D87, D88): the
//! host panel with a Telnet block and its warning, the New Serial
//! Connection dialog, and Telnet and serial session tabs.
//!
//! - The Telnet states use a host of their own, `edge-router (lab)` on
//!   127.0.0.1, saved for the state and removed by the reset
//!   ([`remove_telnet_host`]), so the demo hosts of the other states stay as
//!   they are. Its Telnet block points at a small Telnet server of this
//!   run ([`telnet_port`]: a login prompt, nothing more).
//! - The serial states use [`DemoPorts`], the serial backend of the run:
//!   two ports, and a port that answers like a network device's console.
//!
//! Nothing here needs the SSH test server, hardware or the network.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;
use shellpp_term_local::serial::{self, PortInfo, SerialConfig, SerialError, SerialSession};

use super::{ScreenshotError, ShotState, click, press, settle, shell_of};
use crate::app::RootView;
use crate::session_ui::{Phase, SessionPane};
use crate::sessions::SerialBackend;
use crate::shell::{Panel, SessionKind, SessionRequest, Shell, TopTab};
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 4] = [
    ShotState::HostPanelTelnet,
    ShotState::SerialDialog,
    ShotState::TelnetTab,
    ShotState::SerialTab,
];

/// The label and address of the Telnet host of these states.
const TELNET_HOST_LABEL: &str = "edge-router (lab)";
const TELNET_HOST_ADDRESS: &str = "127.0.0.1";
/// What the Telnet server and the demo port show, and what the states wait
/// for.
const TELNET_BANNER: &[u8] = b"\r\nUbuntu 24.04.1 LTS\r\n\r\nedge-router login: ";
const TELNET_MARKER: &str = "edge-router login:";
const CONSOLE_BANNER: &[u8] =
    b"\r\n\r\nedge-switch con0 is now available\r\n\r\nPress RETURN to get started.\r\n\r\nedge-switch>";
const CONSOLE_MARKER: &str = "edge-switch>";
const SESSION_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(100);

/// Ctrl+Alt+S: New Serial Connection.
const NEW_SERIAL: &str = "ctrl-alt-s";

fn failed(shot: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(shot, reason)
}

// -- The serial ports of the run -------------------------------------------

/// The serial ports of the screenshot run: COM3 (a USB adapter) and COM4.
/// An open port prints a console banner and echoes what is typed.
#[derive(Debug, Default)]
pub struct DemoPorts;

impl SerialBackend for DemoPorts {
    fn ports(&self) -> Vec<PortInfo> {
        vec![
            PortInfo {
                name: "COM3".into(),
                description: Some("USB-SERIAL CH340".into()),
            },
            PortInfo {
                name: "COM4".into(),
                description: None,
            },
        ]
    }

    fn open(&self, _config: &SerialConfig) -> Result<SerialSession, SerialError> {
        let (device, port) = mpsc::channel::<Vec<u8>>();
        device
            .send(CONSOLE_BANNER.to_vec())
            .map_err(|_| SerialError::Io)?;
        Ok(serial::start(DemoReader(port), DemoEcho(device)))
    }
}

/// The port's output: what the device sends; a timeout while it is quiet.
struct DemoReader(mpsc::Receiver<Vec<u8>>);

impl std::io::Read for DemoReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.recv_timeout(serial::READ_TIMEOUT) {
            Ok(bytes) => {
                let len = bytes.len().min(buf.len());
                buf[..len].copy_from_slice(&bytes[..len]);
                Ok(len)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(std::io::ErrorKind::TimedOut.into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }
}

/// The device echoes input; Enter gives a new prompt.
struct DemoEcho(mpsc::Sender<Vec<u8>>);

impl std::io::Write for DemoEcho {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut echo = Vec::new();
        for &byte in buf {
            if byte == b'\r' {
                echo.extend_from_slice(b"\r\nedge-switch>");
            } else {
                echo.push(byte);
            }
        }
        self.0
            .send(echo)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -- The Telnet server of the run ------------------------------------------

/// The port of the run's Telnet server on 127.0.0.1, started on first use.
/// Each connection gets ECHO and SUPPRESS-GO-AHEAD offered and a login
/// prompt; what the client sends is read and dropped.
fn telnet_port() -> Option<u16> {
    static PORT: OnceLock<Option<u16>> = OnceLock::new();
    *PORT.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        std::thread::Builder::new()
            .name("shellpp-screens-telnet".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let _ = std::thread::Builder::new()
                        .name("shellpp-screens-telnet-conn".into())
                        .spawn(move || serve(stream));
                }
            })
            .ok()?;
        Some(port)
    })
}

fn serve(mut stream: TcpStream) {
    // IAC WILL ECHO, IAC WILL SUPPRESS-GO-AHEAD.
    if stream.write_all(&[255, 251, 1, 255, 251, 3]).is_err()
        || stream.write_all(TELNET_BANNER).is_err()
    {
        return;
    }
    let mut buffer = [0u8; 1024];
    while matches!(stream.read(&mut buffer), Ok(read) if read > 0) {}
}

/// Saves the Telnet host of these states; returns its record id.
fn save_telnet_host(
    state: &Entity<AppState>,
    cx: &mut AsyncApp,
    shot: ShotState,
) -> Result<uuid::Uuid, ScreenshotError> {
    let port = telnet_port().ok_or_else(|| failed(shot, "the Telnet server did not start"))?;
    let data = json!({
        "label": TELNET_HOST_LABEL,
        "address": TELNET_HOST_ADDRESS,
        "primary_protocol": "telnet",
        "telnet": {"port": port},
    });
    let data = data.as_object().cloned().unwrap_or_default();
    cx.update(|cx| state.update(cx, |state, cx| state.save_host(None, data, cx)))
        .map(|host| host.record_id)
        .map_err(|_| failed(shot, "the Telnet host was not saved"))
}

/// Removes the Telnet host of these states, if a state saved it (the
/// reset of the demo window calls it).
pub fn remove_telnet_host(state: &Entity<AppState>, cx: &mut gpui::App) {
    let ids: Vec<uuid::Uuid> = state
        .read(cx)
        .hosts()
        .iter()
        .filter(|host| host.title() == TELNET_HOST_LABEL && host.address == TELNET_HOST_ADDRESS)
        .map(|host| host.record_id)
        .collect();
    if !ids.is_empty() {
        let _ = state.update(cx, |state, cx| state.delete_records(ids, cx));
    }
}

// -- Reaching and checking the states --------------------------------------

/// The active session pane of the window.
fn active_pane(shell: &Entity<Shell>, cx: &gpui::App) -> Option<Entity<SessionPane>> {
    match shell.read(cx).top_tab() {
        TopTab::Session(id) => shell.read(cx).pane(id).cloned(),
        TopTab::Vaults | TopTab::Sftp => None,
    }
}

fn shows(pane: &Entity<SessionPane>, text: &str, cx: &gpui::App) -> bool {
    pane.read(cx)
        .terminal()
        .map(|terminal| terminal.read(cx).handle().screen_text())
        .unwrap_or_default()
        .iter()
        .any(|row| row.contains(text))
}

/// Waits until the active session runs and shows `marker`.
async fn wait_for(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    marker: &str,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + SESSION_TIMEOUT;
    loop {
        settle(cx, handle).await?;
        let ready = cx.update(|cx| {
            active_pane(shell, cx).is_some_and(|pane| {
                *pane.read(cx).phase() == Phase::Running && shows(&pane, marker, cx)
            })
        });
        if ready {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(failed(shot, "timeout: the session's prompt"));
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    match shot {
        ShotState::HostPanelTelnet => {
            let id = save_telnet_host(state, cx, shot)?;
            cx.update_window(handle, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.open_host_panel(Some(id), window, cx));
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await?;
            cx.update(|cx| {
                if let Some(Panel::Host(panel)) = shell.read(cx).panel() {
                    panel.read(cx).reveal_telnet();
                }
            });
            settle(cx, handle).await
        }
        ShotState::SerialDialog => {
            press(cx, handle, NEW_SERIAL)?;
            settle(cx, handle).await?;
            // The ports are listed off the UI thread.
            settle(cx, handle).await
        }
        ShotState::TelnetTab => {
            let id = save_telnet_host(state, cx, shot)?;
            cx.update(|cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_session(
                        SessionRequest::new(SessionKind::Telnet)
                            .host(id)
                            .title(TELNET_HOST_LABEL),
                        cx,
                    );
                })
            });
            wait_for(cx, handle, &shell, TELNET_MARKER, shot).await
        }
        ShotState::SerialTab => {
            press(cx, handle, NEW_SERIAL)?;
            settle(cx, handle).await?;
            settle(cx, handle).await?;
            click(cx, handle, "serial-connect")?;
            wait_for(cx, handle, &shell, CONSOLE_MARKER, shot).await
        }
        _ => Err(failed(shot, "not a stage 9 state")),
    }
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    let reason = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let shown = |id: &'static str| window.try_find(id).is_some_and(|found| found.visible());
            let session = |kind: SessionKind, marker: &str| {
                let id = match shell.read(cx).top_tab() {
                    TopTab::Session(id) => id,
                    _ => return Some("no session tab is active"),
                };
                if shell.read(cx).sessions().get(id).map(|tab| tab.kind()) != Some(kind) {
                    return Some("another kind of tab");
                }
                let running = active_pane(&shell, cx).is_some_and(|pane| {
                    *pane.read(cx).phase() == Phase::Running && shows(&pane, marker, cx)
                });
                (!running).then_some("the session does not show its prompt")
            };
            match shot {
                ShotState::HostPanelTelnet => (!shown("host-telnet-warning"))
                    .then_some("the Telnet block and its warning are not on screen"),
                ShotState::SerialDialog => (!shown("serial-dialog")).then_some("no Serial dialog"),
                ShotState::TelnetTab => session(SessionKind::Telnet, TELNET_MARKER),
                ShotState::SerialTab => session(SessionKind::Serial, CONSOLE_MARKER),
                _ => Some("not a stage 9 state"),
            }
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    match reason {
        Some(reason) => Err(failed(shot, reason)),
        None => Ok(()),
    }
}
