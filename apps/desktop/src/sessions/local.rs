//! Local terminals (UI 4.1, 12.4; lead decision 9): the shell in a
//! pseudo-terminal (`shellpp-term-local`: ConPTY on Windows, a pty on macOS
//! and Linux). The default shell is PowerShell 7 (`pwsh`) when it is
//! installed, else Windows PowerShell, on Windows, and the user's login
//! shell elsewhere; the `terminal.local_shell` setting or the requested id
//! choose another. The view reports the resolved shell's id, so that
//! Duplicate, a new split pane and Restore open the same shell. Exit code 0 closes the tab, another code
//! shows "Session ended with code N"; a shell that cannot start is
//! `Failed`. Reconnect starts the same shell again. No autoreconnect, no
//! connection log.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use shellpp_term_local::{
    ExitInfo, LocalError, LocalSession, PtyConfig, PtyWriter, ShellInfo, ShellSpec,
};
use tokio::sync::{mpsc, oneshot};

use super::backend::SessionSettings;
use super::charset::OutputDecoder;
use super::transport::{BoxFuture, TermEnd, TermEvent, TermEvents, TermInput, TermParts};
use super::types::{SessionError, SessionState};
use super::{Control, InputReceiver, PumpEnd, Session, Shared, open_input, pump};

/// Shell detection and PTY start (real: `shellpp-term-local`). Every method
/// may block.
pub trait LocalBackend: Send + Sync + 'static {
    /// The detected shells; cached unless `refresh` (WSL detection is slow).
    fn shells(&self, refresh: bool) -> Vec<ShellInfo>;
    fn default_id(&self, shells: &[ShellInfo]) -> Option<String>;
    fn resolve(
        &self,
        requested: Option<&str>,
        setting: Option<&str>,
        shells: &[ShellInfo],
    ) -> Result<ShellSpec, LocalError>;
    fn spawn(
        &self,
        spec: &ShellSpec,
        cols: u16,
        rows: u16,
        term: &str,
    ) -> Result<TermParts, LocalError>;
}

/// The real backend.
#[derive(Debug, Default)]
pub struct PtyBackend {
    cache: Mutex<Option<Vec<ShellInfo>>>,
}

impl PtyBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LocalBackend for PtyBackend {
    fn shells(&self, refresh: bool) -> Vec<ShellInfo> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if refresh || cache.is_none() {
            *cache = Some(shellpp_term_local::detect_shells());
        }
        cache.clone().unwrap_or_default()
    }

    fn default_id(&self, shells: &[ShellInfo]) -> Option<String> {
        shellpp_term_local::default_shell_id(shells)
    }

    fn resolve(
        &self,
        requested: Option<&str>,
        setting: Option<&str>,
        shells: &[ShellInfo],
    ) -> Result<ShellSpec, LocalError> {
        shellpp_term_local::resolve_shell(requested, setting, shells)
    }

    fn spawn(
        &self,
        spec: &ShellSpec,
        cols: u16,
        rows: u16,
        term: &str,
    ) -> Result<TermParts, LocalError> {
        let session = shellpp_term_local::spawn(PtyConfig {
            shell: spec.clone(),
            cols,
            rows,
            term: term.to_owned(),
            cwd: None,
        })?;
        let writer = session.writer();
        Ok(parts(session, writer))
    }
}

fn parts(session: LocalSession, writer: PtyWriter) -> TermParts {
    let LocalSession { output, exit, .. } = session;
    TermParts {
        events: Box::new(LocalEvents {
            output,
            exit: Some(exit),
            exited: None,
        }),
        input: Arc::new(LocalInput(writer)),
    }
}

struct LocalEvents {
    output: mpsc::Receiver<Vec<u8>>,
    exit: Option<oneshot::Receiver<ExitInfo>>,
    exited: Option<TermEnd>,
}

/// Output still arriving after the process exited is read for this long.
const DRAIN_AFTER_EXIT: Duration = Duration::from_millis(150);

fn exit_end(info: Option<ExitInfo>) -> TermEnd {
    match info.and_then(|info| info.code) {
        Some(code) => TermEnd::ExitStatus(code),
        None => TermEnd::ExitUnknown,
    }
}

impl TermEvents for LocalEvents {
    fn next(&mut self) -> BoxFuture<'_, TermEvent> {
        Box::pin(async move {
            loop {
                if let Some(end) = self.exited.clone() {
                    return match tokio::time::timeout(DRAIN_AFTER_EXIT, self.output.recv()).await {
                        Ok(Some(chunk)) => TermEvent::Data(chunk),
                        Ok(None) | Err(_) => TermEvent::End(end),
                    };
                }
                let Some(exit) = self.exit.as_mut() else {
                    return TermEvent::End(TermEnd::ExitUnknown);
                };
                let got = tokio::select! {
                    biased;
                    chunk = self.output.recv() => Got::Chunk(chunk),
                    info = exit => Got::Exit(info.ok()),
                };
                match got {
                    Got::Chunk(Some(chunk)) => return TermEvent::Data(chunk),
                    Got::Chunk(None) => {
                        let info = match self.exit.take() {
                            Some(exit) => exit.await.ok(),
                            None => None,
                        };
                        return TermEvent::End(exit_end(info));
                    }
                    Got::Exit(info) => {
                        self.exit = None;
                        self.exited = Some(exit_end(info));
                    }
                }
            }
        })
    }
}

enum Got {
    Chunk(Option<Vec<u8>>),
    Exit(Option<ExitInfo>),
}

struct LocalInput(PtyWriter);

/// Whether the PTY is still there after a write or resize: only
/// [`LocalError::Closed`] means it is gone. A failed resize or a failed
/// write (`Io`) loses that one operation, not the keyboard input of the
/// rest of the session.
fn still_open(result: Result<(), LocalError>) -> bool {
    !matches!(result, Err(LocalError::Closed))
}

impl TermInput for LocalInput {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        let writer = self.0.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || still_open(writer.write(&data)))
                .await
                .unwrap_or(false)
        })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, bool> {
        let writer = self.0.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || still_open(writer.resize(cols, rows)))
                .await
                .unwrap_or(false)
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        let writer = self.0.clone();
        Box::pin(async move {
            let _ = tokio::task::spawn_blocking(move || writer.kill()).await;
        })
    }
}

/// Resolves the shell and starts it (blocking).
fn start(
    shared: &Shared,
    requested: Option<&str>,
    size: (u16, u16),
) -> Result<(ShellSpec, String, TermParts), SessionError> {
    let settings = SessionSettings::read(&*shared.core);
    let shells = shared.local.shells(false);
    let spec = shared
        .local
        .resolve(requested, settings.local_shell.as_deref(), &shells)
        .map_err(|_| SessionError::ShellUnavailable)?;
    let parts = shared
        .local
        .spawn(&spec, size.0, size.1, &settings.term)
        .map_err(|_| SessionError::ShellUnavailable)?;
    Ok((spec, settings.term, parts))
}

/// [`start`] on a blocking thread.
async fn start_blocking(
    shared: &Arc<Shared>,
    requested: Option<String>,
    size: (u16, u16),
) -> Result<(ShellSpec, String, TermParts), SessionError> {
    let shared = shared.clone();
    tokio::task::spawn_blocking(move || start(&shared, requested.as_deref(), size))
        .await
        .unwrap_or(Err(SessionError::Internal))
}

/// The task of one local terminal.
pub(crate) async fn run(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut ctrl: mpsc::UnboundedReceiver<Control>,
    shell_id: Option<String>,
) {
    let requested = shell_id.filter(|id| !id.is_empty());
    let size = session.lock().size;
    let mut shell: Option<(ShellSpec, String)> = None;
    let mut running = None;
    match start_blocking(&shared, requested.clone(), size).await {
        Ok((spec, term, parts)) => {
            let mut input = None;
            session.update(|st| {
                input = Some(open_input(st, size));
                st.view.state = SessionState::Connected;
                st.view.title = spec.label.clone();
                st.view.shell_id = spec.id.clone();
            });
            let input = input.unwrap_or_else(InputReceiver::closed);
            shell = Some((spec, term));
            running = Some((parts, input));
        }
        Err(error) => {
            tracing::warn!(session = session.id, "local shell did not start");
            session.update(|st| {
                st.view.state = SessionState::Failed;
                st.view.error = Some(error);
            });
        }
    }
    loop {
        if let Some((parts, input)) = running.take() {
            let decoder = OutputDecoder::new(encoding_rs::UTF_8);
            let end = pump(&shared, &session, &mut ctrl, parts, decoder, input).await;
            match end {
                PumpEnd::Closed => return,
                PumpEnd::Ended(TermEnd::ExitStatus(0)) => {
                    tracing::info!(session = session.id, "local shell exited with 0");
                    shared.close_session(&session);
                    return;
                }
                PumpEnd::Ended(end) => {
                    tracing::info!(session = session.id, "local shell ended");
                    let (status, signal) = match end {
                        TermEnd::ExitStatus(status) => (status, None),
                        TermEnd::ExitSignal(signal) => (255, Some(signal)),
                        TermEnd::Dropped(_) | TermEnd::ExitUnknown => (255, None),
                    };
                    session.update(|st| {
                        st.view.state = SessionState::Ended;
                        st.view.exit_status = Some(status);
                        st.view.exit_signal = signal;
                        st.view.error = None;
                    });
                }
            }
        }
        match ctrl.recv().await {
            Some(Control::Reconnect) => {
                let size = session.lock().size;
                // The same shell again; a shell that never started is
                // resolved anew.
                let spawned = match shell.clone() {
                    Some((spec, term)) => {
                        let local = shared.local.clone();
                        tokio::task::spawn_blocking(move || {
                            local
                                .spawn(&spec, size.0, size.1, &term)
                                .map(|parts| (spec, term, parts))
                        })
                        .await
                        .ok()
                        .and_then(Result::ok)
                    }
                    None => start_blocking(&shared, requested.clone(), size).await.ok(),
                };
                match spawned {
                    Some((spec, term, parts)) => {
                        let title = spec.label.clone();
                        let shell_id = spec.id.clone();
                        shell = Some((spec, term));
                        // The terminal still shows the previous shell's screen.
                        super::reset_terminal_modes(&session).await;
                        let mut input = None;
                        session.update(|st| {
                            input = Some(open_input(st, size));
                            st.view.state = SessionState::Connected;
                            st.view.title = title;
                            st.view.shell_id = shell_id;
                            st.view.exit_status = None;
                            st.view.exit_signal = None;
                            st.view.error = None;
                        });
                        match input {
                            Some(input) => running = Some((parts, input)),
                            None => return,
                        }
                    }
                    None => {
                        session.update(|st| {
                            st.view.state = SessionState::Failed;
                            st.view.exit_status = None;
                            st.view.exit_signal = None;
                            st.view.error = Some(SessionError::ShellUnavailable);
                        });
                    }
                }
            }
            Some(Control::Cancel) => {}
            Some(Control::Close) | None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_closed_pty_ends_the_input() {
        assert!(still_open(Ok(())));
        assert!(still_open(Err(LocalError::Io)), "a failed resize or write");
        assert!(!still_open(Err(LocalError::Closed)));
    }

    #[test]
    fn exit_codes() {
        assert_eq!(
            exit_end(Some(ExitInfo { code: Some(0) })),
            TermEnd::ExitStatus(0)
        );
        assert_eq!(
            exit_end(Some(ExitInfo { code: Some(3) })),
            TermEnd::ExitStatus(3)
        );
        assert_eq!(
            exit_end(Some(ExitInfo { code: None })),
            TermEnd::ExitUnknown
        );
        assert_eq!(exit_end(None), TermEnd::ExitUnknown);
    }
}
