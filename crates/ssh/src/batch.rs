//! Running one command on many hosts (UI 9.4: a snippet on more than 4
//! targets fills a results table): every target gets its own connection
//! and an exec channel (never a shell), at most [`MAX_PARALLEL`] at a time,
//! each with its own timeout. A target that cannot be reached fails on its
//! own; the others go on.
//!
//! The command text may hold values typed for this run (snippet variables):
//! it is never logged, and the results keep only what the hosts printed.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use tokio::sync::mpsc;

use crate::client::{BoxFuture, Connection, ExecOutput};
use crate::error::{FailureKind, SshError};

/// Most targets running at the same time (UI 9.4).
pub const MAX_PARALLEL: usize = 8;

/// Opens the connection of one target (the desktop host resolves its spec,
/// route and interaction). The run owns it and closes it when the command
/// ended: never pass a connection shared with a session.
pub type ConnectTarget =
    Box<dyn FnOnce() -> BoxFuture<'static, Result<Connection, SshError>> + Send>;

/// How one target ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetOutcome {
    /// The command ran: its exit status (none when it ended by a signal or
    /// the channel closed without one) and its output (stdout and stderr).
    Finished(ExecOutput),
    /// The connection could not be opened (the error names its place on the
    /// route).
    ConnectFailed(SshError),
    /// The server refused the exec channel, or the connection dropped.
    ExecFailed(SshError),
    /// The timeout passed before the command ended (connecting included);
    /// the output so far.
    TimedOut(ExecOutput),
}

impl TargetOutcome {
    /// Success: the command ended with exit status 0.
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::Finished(output) if output.exit_status == Some(0))
    }
}

/// Progress of a run, as it happens (the table's states: waiting, running,
/// success, error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchEvent {
    /// Target `index` started connecting.
    Started(usize),
    /// Target `index` ended.
    Finished(usize, TargetOutcome),
}

/// Runs `command` on every target, at most [`MAX_PARALLEL`] at once; each
/// target has `timeout` for connecting and running together and keeps at
/// most `max_output` bytes. Progress goes to `events` (if any); the result
/// is in target order. Each connection is closed when its command ended.
pub async fn run(
    targets: Vec<ConnectTarget>,
    command: String,
    timeout: Duration,
    max_output: usize,
    events: Option<mpsc::UnboundedSender<BatchEvent>>,
) -> Vec<TargetOutcome> {
    let command = Arc::new(command);
    let jobs = targets.into_iter().enumerate().map(|(index, connect)| {
        let command = Arc::clone(&command);
        let events = events.clone();
        async move {
            if let Some(events) = &events {
                let _ = events.send(BatchEvent::Started(index));
            }
            let outcome = run_one(connect, &command, timeout, max_output).await;
            if let Some(events) = &events {
                let _ = events.send(BatchEvent::Finished(index, outcome.clone()));
            }
            (index, outcome)
        }
    });
    let mut results: Vec<(usize, TargetOutcome)> = futures::stream::iter(jobs)
        .buffer_unordered(MAX_PARALLEL)
        .collect()
        .await;
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, outcome)| outcome).collect()
}

async fn run_one(
    connect: ConnectTarget,
    command: &str,
    timeout: Duration,
    max_output: usize,
) -> TargetOutcome {
    let deadline = tokio::time::Instant::now() + timeout;
    let connection = match tokio::time::timeout_at(deadline, connect()).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => return TargetOutcome::ConnectFailed(error),
        Err(_) => return TargetOutcome::ConnectFailed(SshError::new(FailureKind::Timeout)),
    };
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let outcome = match connection
        .exec_with_stderr(command, remaining, max_output)
        .await
    {
        Ok(output) if output.timed_out => TargetOutcome::TimedOut(output),
        Ok(output) => TargetOutcome::Finished(output),
        Err(error) if error.kind == FailureKind::Timeout => {
            TargetOutcome::TimedOut(ExecOutput::default())
        }
        Err(error) => TargetOutcome::ExecFailed(error),
    };
    connection.disconnect().await;
    outcome
}

/// A boxed connect step from any future (for callers that build targets).
pub fn target<F>(connect: impl FnOnce() -> F + Send + 'static) -> ConnectTarget
where
    F: Future<Output = Result<Connection, SshError>> + Send + 'static,
{
    Box::new(move || Box::pin(connect()))
}
