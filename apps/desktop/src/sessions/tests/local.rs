//! The local terminal (UI 4.1, lead decision 9) with this machine's default
//! shell (Windows PowerShell on Windows, the login shell elsewhere): exit
//! codes, Reconnect, and a shell that cannot start.

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
    if cfg!(windows) {
        assert_eq!(view.title, "PowerShell");
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
