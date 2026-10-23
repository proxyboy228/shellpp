//! The local terminal (UI 4.1, lead decision 9) with this machine's default
//! shell (PowerShell 7 when it is installed, else Windows PowerShell, on
//! Windows; the login shell elsewhere): exit codes, Reconnect, and a shell
//! that cannot start.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_default_shell_runs_and_its_exit_codes_follow_the_rules() {
    let harness = Harness::new();
    let terminal = Arc::new(TestTerminal::default());
    let mut session = harness
        .open_with(OpenRequest::Local { shell_id: None }, terminal)
        .expect("opens");
    assert_eq!(session.view.kind, SessionKind::Local);
    let view = session.state(SessionState::Connected).await;
    // The resolved default shell is reported (Duplicate opens it again).
    let default = shellpp_term_local::default_shell_id(&shellpp_term_local::detect_shells());
    assert_eq!(view.shell_id, default);
    if cfg!(windows) {
        let expected = match view.shell_id.as_deref() {
            Some("pwsh") => "PowerShell 7",
            _ => "PowerShell",
        };
        assert_eq!(view.title, expected);
    }
    assert!(view.host.is_none() && view.address.is_none());

    // A non-zero code ends the session; the output stays.
    session.any_output().await;
    session.type_line("exit 3");
    let view = session.state(SessionState::Ended).await;
    assert_eq!(view.exit_status, Some(3));
    assert_eq!(view.message().as_deref(), Some("Session ended with code 3"));
    assert!(
        harness.calls.starts().is_empty(),
        "no connection log for local"
    );

    // Reconnect starts the same shell again; exit 0 closes the tab.
    session.handle.reconnect();
    session.state(SessionState::Connected).await;
    session.type_line("exit");
    session.state(SessionState::Closed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_shell_fails_with_a_message() {
    let harness = Harness::new();
    let mut session = harness
        .open_with(
            OpenRequest::Local {
                shell_id: Some("no-such-shell".to_owned()),
            },
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::ShellUnavailable));
    assert_eq!(
        view.message().as_deref(),
        Some("The local shell could not be started.")
    );
    session.handle.close();
    session.state(SessionState::Closed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_and_resize_reach_the_shell() {
    let harness = Harness::new();
    let terminal = Arc::new(TestTerminal::default());
    let mut session = harness
        .open_with(OpenRequest::Local { shell_id: None }, terminal)
        .expect("opens");
    session.state(SessionState::Connected).await;
    let marker = random_secret("local");
    let (head, tail) = marker.split_at(6);
    // The echo of the typed command never holds the marker in one piece.
    let command = if cfg!(windows) {
        format!("Write-Output ('{head}' + '{tail}')")
    } else {
        format!("echo {head}''{tail}")
    };
    session.handle.resize(90, 20);
    session.type_line(&command);
    session.output(&marker).await;
    session.type_line("exit");
    session.state(SessionState::Closed).await;
}

/// Counts shell detections; starts nothing.
#[derive(Default)]
struct CountingLocal {
    detections: AtomicUsize,
}

impl LocalBackend for CountingLocal {
    fn shells(&self, _refresh: bool) -> Vec<shellpp_term_local::ShellInfo> {
        self.detections.fetch_add(1, Ordering::SeqCst);
        Vec::new()
    }
    fn default_id(&self, _shells: &[shellpp_term_local::ShellInfo]) -> Option<String> {
        None
    }
    fn resolve(
        &self,
        _requested: Option<&str>,
        _setting: Option<&str>,
        _shells: &[shellpp_term_local::ShellInfo],
    ) -> Result<shellpp_term_local::ShellSpec, shellpp_term_local::LocalError> {
        Err(shellpp_term_local::LocalError::NoShell)
    }
    fn spawn(
        &self,
        _spec: &shellpp_term_local::ShellSpec,
        _cols: u16,
        _rows: u16,
        _term: &str,
    ) -> Result<crate::sessions::transport::TermParts, shellpp_term_local::LocalError> {
        Err(shellpp_term_local::LocalError::NoShell)
    }
}

/// D100, item 5: the app start detects no shells (WSL is slow and can hang);
/// the first local terminal of the run does, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shells_are_detected_only_when_a_local_terminal_opens() {
    let local = Arc::new(CountingLocal::default());
    let harness = Harness::with(|options| options.local = local.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        local.detections.load(Ordering::SeqCst),
        0,
        "detected at start"
    );
    let mut session = harness
        .open_with(
            OpenRequest::Local { shell_id: None },
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    session.state(SessionState::Failed).await;
    assert_eq!(local.detections.load(Ordering::SeqCst), 1);
}
