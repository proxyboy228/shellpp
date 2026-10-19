//! Local terminals (desktop only; stage 2 plan, section 4.5): the shell of
//! UI 12.4 in a pseudo-terminal (`shellpp-term-local`). State `connected` at
//! once; exit code 0 closes the tab, another code shows "Session ended with
//! code N"; a shell that cannot start is `failed` with `unavailable`.
//! Reconnect starts the same shell again. No autoreconnect, no connection log.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use shellpp_core::error::ValidationReason;
use shellpp_core::service::dto::{
    Backspace, LocalOpenInput, LocalShell, LocalShellsResult, SessionInfo, SessionKind,
    SessionState,
};
use shellpp_term_local::{
    ExitInfo, LocalError, LocalSession, PtyConfig, PtyWriter, ShellInfo, ShellSpec,
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::backend::SessionSettings;
use super::charset::OutputDecoder;
use super::transport::{BoxFuture, TermEnd, TermEvent, TermEvents, TermInput, TermParts};
use super::{Control, PumpEnd, Session, SessionManager, Shared, error_code, pump};
use crate::error::CmdError;

/// Shell detection and PTY start (real: `shellpp-term-local`; tests: fakes).
/// Every method may block.
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
                if let Some(end) = self.exited {
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

impl TermInput for LocalInput {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        let writer = self.0.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || writer.write(&data).is_ok())
                .await
                .unwrap_or(false)
        })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, bool> {
        let writer = self.0.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || writer.resize(cols, rows).is_ok())
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

/// The shells for `local_shells`.
pub fn shells_result(backend: &dyn LocalBackend, shells: &[ShellInfo]) -> LocalShellsResult {
    LocalShellsResult {
        shells: shells
            .iter()
            .map(|shell| LocalShell {
                id: shell.id.clone(),
                label: shell.label.clone(),
            })
            .collect(),
        default_id: backend.default_id(shells),
    }
}

/// Maps a resolve error: an unknown requested id is the caller's mistake
/// (`validation_failed`), anything else means no usable shell (`unavailable`).
fn resolve_error(error: LocalError, requested: bool) -> CmdError {
    match error {
        LocalError::NoShell if requested => {
            CmdError::validation("shell_id", ValidationReason::Invalid)
        }
        _ => CmdError::unavailable(),
    }
}

/// `local_open`.
pub(super) async fn open(
    manager: &SessionManager,
    window: &str,
    input: LocalOpenInput,
) -> Result<SessionInfo, CmdError> {
    let shared = manager.shared.clone();
    let backend = shared.local.clone();
    let core = shared.core.clone();
    let requested = input.shell_id.clone().filter(|id| !id.is_empty());
    let (cols, rows) = (input.cols, input.rows);
    let resolved = shared
        .rt
        .spawn_blocking(move || {
            if !core.is_unlocked() {
                return Err(CmdError::locked());
            }
            let settings = SessionSettings::read(&*core);
            let shells = backend.shells(false);
            let spec = backend
                .resolve(
                    requested.as_deref(),
                    settings.local_shell.as_deref(),
                    &shells,
                )
                .map_err(|error| resolve_error(error, requested.is_some()))?;
            let spawned = backend.spawn(&spec, cols, rows, &settings.term);
            Ok((settings, spec, spawned))
        })
        .await
        .map_err(|_| CmdError::internal())?;
    let (settings, spec, spawned) = resolved?;
    let (state, error, parts) = match spawned {
        Ok(parts) => (SessionState::Connected, None, Some(parts)),
        Err(_) => {
            tracing::warn!("local shell did not start");
            (SessionState::Failed, Some(error_code("unavailable")), None)
        }
    };
    let info = SessionInfo {
        session_id: Uuid::now_v7(),
        rev: 1,
        kind: SessionKind::Local,
        window: window.to_owned(),
        state,
        step: None,
        title: spec.label.clone(),
        target: None,
        shell_id: spec.id.clone(),
        address: None,
        port: None,
        username: None,
        os: None,
        terminal_theme: settings.default_theme.clone(),
        backspace: Backspace::Default,
        record_history: settings.record_history,
        prompt: None,
        error,
        host_key: None,
        exit_status: None,
        reconnect: None,
    };
    let (session, ctrl) = Session::new(info.clone(), (cols, rows), None);
    // A running shell takes input from the first moment.
    let input = parts
        .as_ref()
        .map(|_| super::open_input(&mut session.lock(), (cols, rows)));
    if let Err(error) = manager.insert(&session) {
        // The window closed while the shell started: end it at once.
        if let Some(parts) = parts {
            let _ = tokio::time::timeout(super::CLOSE_TIMEOUT, parts.input.close()).await;
        }
        return Err(error);
    }
    shared.events.session_changed(&info);
    tracing::info!(session = %session.id, kind = "local", "session opened");
    let running = parts.zip(input);
    drop(shared.rt.spawn(run(
        shared.clone(),
        session,
        ctrl,
        running,
        spec,
        settings.term,
    )));
    Ok(info)
}

/// The task of one local terminal.
async fn run(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut ctrl: mpsc::UnboundedReceiver<Control>,
    mut running: Option<(TermParts, super::InputReceiver)>,
    spec: ShellSpec,
    term: String,
) {
    loop {
        if let Some((parts, input)) = running.take() {
            let decoder = OutputDecoder::new(encoding_rs::UTF_8);
            let end = pump(&shared, &session, &mut ctrl, parts, decoder, input).await;
            tracing::info!(session = %session.id, ?end, "local shell ended");
            match end {
                PumpEnd::Closed => return,
                PumpEnd::Ended(TermEnd::ExitStatus(0)) => {
                    shared.close_session(&session);
                    return;
                }
                PumpEnd::Ended(end) => {
                    let status = match end {
                        TermEnd::ExitStatus(status) => status,
                        TermEnd::ExitSignal | TermEnd::Dropped(_) | TermEnd::ExitUnknown => 255,
                    };
                    session.update(&*shared.events, |st| {
                        st.info.state = SessionState::Ended;
                        st.info.exit_status = Some(status);
                        st.info.error = None;
                    });
                }
            }
        }
        match ctrl.recv().await {
            Some(Control::Reconnect) => {
                let backend = shared.local.clone();
                let (cols, rows) = session.lock().size;
                let shell = spec.clone();
                let term = term.clone();
                let spawned = shared
                    .rt
                    .spawn_blocking(move || backend.spawn(&shell, cols, rows, &term))
                    .await
                    .ok()
                    .and_then(Result::ok);
                match spawned {
                    Some(parts) => {
                        // The pane still shows the previous shell's screen.
                        super::reset_terminal_modes(&session);
                        let mut input = None;
                        session.update(&*shared.events, |st| {
                            input = Some(super::open_input(st, (cols, rows)));
                            st.info.state = SessionState::Connected;
                            st.info.exit_status = None;
                            st.info.error = None;
                        });
                        match input {
                            Some(input) => running = Some((parts, input)),
                            None => return,
                        }
                    }
                    None => {
                        session.update(&*shared.events, |st| {
                            st.info.state = SessionState::Failed;
                            st.info.exit_status = None;
                            st.info.error = Some(error_code("unavailable"));
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
    fn resolve_errors() {
        assert_eq!(
            resolve_error(LocalError::NoShell, true),
            CmdError::validation("shell_id", ValidationReason::Invalid)
        );
        assert_eq!(
            resolve_error(LocalError::NoShell, false),
            CmdError::unavailable()
        );
        assert_eq!(
            resolve_error(LocalError::InvalidShell, false),
            CmdError::unavailable()
        );
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
