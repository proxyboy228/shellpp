//! Session tabs on the session layer (`crate::sessions`): [`Bridge`] is the
//! app's [`SessionLauncher`]. It opens a session with the tab's terminal
//! (`SessionManager::open`), applies every view the session reports to the
//! pane on the UI thread ([`apply`]), and turns the pane's buttons and
//! dialog answers into [`SessionHandle`] commands.
//!
//! The mapping is in one place, [`apply`]:
//!
//! | session view | tab |
//! |---|---|
//! | `Connecting` without `reconnect` | the connection screen at `step` |
//! | `Connecting` with `reconnect` | the "Reconnecting (attempt N)" overlay over the output |
//! | `Connected` | the terminal |
//! | `Ended` | "Session ended with code N" over the output |
//! | `Dropped` | the reason over the output; `reconnect`: the countdown or "after unlock" |
//! | `Failed` | the failure screen (the revoked key's warning when revoked) |
//! | `Closed` | the tab closes |
//! | `prompt` | the prompt form or the host key dialog |
//! | `title`, `os`, `theme` | the tab's name and OS icon, the terminal theme |
//! | `backspace` | what the terminal's Backspace key sends |

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, SharedString, Task};
use gpui_kit::assets::IconName;
use shellpp_core::service::dto::{Backspace, OsFamily};
use shellpp_ssh::FailureKind;

use super::model::{
    ConnectStep, ExitStatus, Failure, HostKeyAsk, HostKeyInfo, HostKeyReply, Keep, Phase, Prompt,
    PromptReply, ReconnectInfo, SessionControl,
};
use super::{SessionLauncher, SessionPane};
use crate::sessions::{
    self, HostKeyChoice, OpenRequest, PromptKind, SaveOption, SessionError, SessionHandle,
    SessionKind, SessionState,
};
use crate::terminal::TerminalHandle;
use crate::terminal::input::BackspaceKey;

/// The app's launcher: sessions through the installed session layer
/// (`crate::sessions::ui::install`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Bridge;

impl SessionLauncher for Bridge {
    fn launch(
        &self,
        request: OpenRequest,
        terminal: TerminalHandle,
        cx: &mut Context<SessionPane>,
    ) -> Result<Rc<dyn SessionControl>, Failure> {
        let Some(manager) = sessions::ui::manager(cx) else {
            tracing::warn!("the session layer is not installed");
            return Err(Failure::Other);
        };
        let size = terminal.size();
        let opened = manager
            .open(request, (size.cols, size.rows), Arc::new(terminal))
            .map_err(failure_of)?;
        let prompt = Rc::new(Cell::new(None));
        let shown = prompt.clone();
        let events = sessions::ui::forward_events(opened.events, cx, move |pane, view, cx| {
            apply(pane, &view, &shown, cx);
        });
        Ok(Rc::new(Control {
            handle: opened.handle,
            prompt,
            _events: events,
        }))
    }
}

/// The commands of one session tab.
struct Control {
    handle: SessionHandle,
    /// The id of the prompt on display (replies name it).
    prompt: Rc<Cell<Option<u64>>>,
    /// Applies the session's views to the pane while the tab lives.
    _events: Task<()>,
}

impl Control {
    fn reply(&self, reply: sessions::PromptReply) {
        let Some(id) = self.prompt.take() else {
            tracing::debug!(session = self.handle.id(), "answer without a prompt");
            return;
        };
        if let Err(error) = self.handle.reply(id, reply) {
            tracing::debug!(session = self.handle.id(), %error, "answer not taken");
        }
    }
}

impl SessionControl for Control {
    fn cancel(&self, _: &mut App) {
        self.handle.cancel();
    }

    fn reconnect(&self, _: &mut App) {
        self.handle.reconnect();
    }

    fn close(&self, _: &mut App) {
        self.handle.close();
    }

    fn reply_prompt(&self, reply: PromptReply, _: &mut App) {
        self.reply(match reply {
            PromptReply::Answer { value, keep } => {
                sessions::PromptReply::Text { value, save: keep }
            }
            PromptReply::Cancel => sessions::PromptReply::Cancel,
            PromptReply::Allow => sessions::PromptReply::Allow,
        });
    }

    fn reply_host_key(&self, reply: HostKeyReply, _: &mut App) {
        self.reply(sessions::PromptReply::HostKey(match reply {
            HostKeyReply::AcceptAndSave => HostKeyChoice::AcceptAndSave,
            HostKeyReply::ConnectOnce => HostKeyChoice::ConnectOnce,
            HostKeyReply::Replace => HostKeyChoice::Replace,
            HostKeyReply::Cancel => HostKeyChoice::Cancel,
        }));
    }
}

/// Applies a view of the session to its pane. `prompt` keeps the id of the
/// prompt on display for the replies.
pub fn apply(
    pane: &mut SessionPane,
    view: &sessions::SessionView,
    prompt: &Cell<Option<u64>>,
    cx: &mut Context<SessionPane>,
) {
    if view.state == SessionState::Closed {
        prompt.set(None);
        pane.set_phase(Phase::Closed, None, cx);
        return;
    }
    // The title (a local terminal's shell name) and the host's OS icon.
    let title = if view.title.trim().is_empty() {
        pane.info().title().clone()
    } else {
        SharedString::from(view.title.clone())
    };
    let icon = match (view.kind, &view.os) {
        (SessionKind::Ssh, Some(os)) => crate::hosts::layout::os_icon(Some(os_family(os.family))),
        _ => pane.info().icon(),
    };
    pane.set_info(title, icon, cx);
    // The shell a default local terminal resolved to (UI 4.4: Duplicate
    // opens the same shell).
    if let Some(shell_id) = &view.shell_id {
        pane.pin_local_shell(shell_id);
    }
    // The host's terminal theme, or the default one (UI 4.1).
    pane.set_theme_slug(&view.theme, cx);
    // The host's Backspace setting (UI 3.1).
    pane.set_backspace(backspace_of(view.backspace), cx);

    let step = view.step.map_or(ConnectStep::Resolving, step_of);
    let reconnect = view.reconnect.map(|reconnect| ReconnectInfo {
        attempt: reconnect.attempt,
        next_at: reconnect.next_at,
        waiting_for_unlock: reconnect.waiting_for_unlock,
    });
    let (phase, reconnect) = match view.state {
        SessionState::Connecting if view.reconnect.is_some() => {
            (Phase::Reconnecting(step), reconnect)
        }
        SessionState::Connecting => (Phase::Connecting(step), None),
        SessionState::Connected => (Phase::Running, None),
        SessionState::Ended => (
            Phase::Ended(match &view.exit_signal {
                Some(signal) => ExitStatus::Signal(SharedString::from(signal.clone())),
                None => ExitStatus::Code(view.exit_status.unwrap_or(255)),
            }),
            None,
        ),
        SessionState::Dropped => (
            Phase::Dropped(failure_of(
                view.error.unwrap_or(SessionError::ClosedByServer),
            )),
            reconnect.filter(|_| view.reconnect.is_some_and(|r| !r.in_progress)),
        ),
        SessionState::Failed => (
            Phase::Failed(failure_of(view.error.unwrap_or(SessionError::Internal))),
            None,
        ),
        SessionState::Closed => unreachable!("handled above"),
    };
    let revoked = (view.error == Some(SessionError::HostKeyRevoked))
        .then(|| view.host_key.as_ref().map(host_key_of))
        .flatten();
    pane.set_phase(phase, reconnect, cx);
    pane.set_startup_ask(view.startup_ask.as_ref(), cx);
    pane.set_place(
        matches!(view.state, SessionState::Failed | SessionState::Dropped)
            .then(|| view.error_place.as_ref().map(place_of))
            .flatten(),
        cx,
    );
    if view.state == SessionState::Failed {
        pane.set_revoked_key(revoked, cx);
    }

    match &view.prompt {
        None => {
            prompt.set(None);
            pane.clear_ask(cx);
        }
        Some(shown) => {
            prompt.set(Some(shown.id));
            match &shown.kind {
                PromptKind::HostKeyUnknown { key } => {
                    pane.ask_host_key(HostKeyAsk::Unknown(host_key_of(key)), cx)
                }
                PromptKind::HostKeyChanged {
                    key,
                    old_key_type,
                    old_fingerprint,
                } => pane.ask_host_key(
                    HostKeyAsk::Changed {
                        key: host_key_of(key),
                        old_key_type: old_key_type.clone(),
                        old_fingerprint: old_fingerprint.clone(),
                    },
                    cx,
                ),
                PromptKind::AgentSignature { fingerprint } => {
                    pane.ask_signature(fingerprint.clone(), cx)
                }
                kind => pane.ask_prompt(prompt_of(kind), cx),
            }
        }
    }
}

/// What the terminal's Backspace key sends for the host setting: DEL for
/// "Default" and "^?", BS for "^H".
fn backspace_of(backspace: Backspace) -> BackspaceKey {
    match backspace {
        Backspace::Default | Backspace::Del => BackspaceKey::Del,
        Backspace::CtrlH => BackspaceKey::CtrlH,
    }
}

/// The OS family name the host icons use.
pub(crate) fn os_family(family: OsFamily) -> &'static str {
    match family {
        OsFamily::Linux => "linux",
        OsFamily::Windows => "windows",
        OsFamily::Macos => "macos",
        OsFamily::Bsd => "bsd",
        OsFamily::Other => "other",
    }
}

pub(crate) fn step_of(step: sessions::ConnectStep) -> ConnectStep {
    match step {
        sessions::ConnectStep::Resolving => ConnectStep::Resolving,
        sessions::ConnectStep::Connecting => ConnectStep::Connecting,
        sessions::ConnectStep::HostKey => ConnectStep::HostKey,
        sessions::ConnectStep::Authenticating => ConnectStep::Authenticating,
        sessions::ConnectStep::OpeningSession => ConnectStep::OpeningSession,
    }
}

/// The text of where on the route a session failed.
pub(crate) fn place_of(place: &crate::sessions::types::ErrorPlace) -> SharedString {
    use super::strings;
    match place {
        crate::sessions::types::ErrorPlace::JumpHost {
            index,
            count,
            title,
        } => strings::place_jump_host(*index, *count, title).into(),
        crate::sessions::types::ErrorPlace::Proxy => strings::PLACE_PROXY.into(),
        crate::sessions::types::ErrorPlace::Chain { .. } => strings::PLACE_CHAIN.into(),
    }
}

pub(crate) fn host_key_of(key: &sessions::HostKeyInfo) -> HostKeyInfo {
    HostKeyInfo {
        address: HostKeyInfo::openssh_address(&key.address, key.port),
        key_type: key.key_type.clone(),
        fingerprint: key.fingerprint.clone(),
    }
}

fn keep_of(save: SaveOption) -> Keep {
    match save {
        SaveOption::None => Keep::None,
        SaveOption::Vault => Keep::Save,
        SaveOption::ThisSession => Keep::ThisSession,
    }
}

/// A text prompt (the host key kinds are dialogs of their own).
pub(crate) fn prompt_of(kind: &PromptKind) -> Prompt {
    match kind {
        // Dialogs of their own (`apply` routes them before this).
        PromptKind::Username
        | PromptKind::HostKeyUnknown { .. }
        | PromptKind::HostKeyChanged { .. }
        | PromptKind::AgentSignature { .. } => Prompt::Username,
        PromptKind::Password {
            username,
            error,
            save,
        } => Prompt::Password {
            username: username.clone(),
            error: *error,
            keep: keep_of(*save),
        },
        PromptKind::Passphrase {
            key_label,
            error,
            save,
        } => Prompt::Passphrase {
            key_label: key_label.clone(),
            error: *error,
            keep: keep_of(*save),
        },
        PromptKind::KeyboardInteractive {
            name,
            instruction,
            prompt,
            echo,
            index,
            count,
            error,
        } => Prompt::KeyboardInteractive {
            name: name.clone(),
            instruction: instruction.clone(),
            prompt: prompt.clone(),
            echo: *echo,
            index: *index,
            count: *count,
            error: *error,
        },
    }
}

/// The reason shown for a session error.
pub fn failure_of(error: SessionError) -> Failure {
    match error {
        SessionError::Ssh(kind) => match kind {
            FailureKind::NotFound => Failure::NotFound,
            FailureKind::Refused => Failure::Refused,
            FailureKind::Timeout => Failure::Timeout,
            FailureKind::Network => Failure::Network,
            FailureKind::HostKeyRejected => Failure::HostKeyRejected,
            FailureKind::AuthFailed => Failure::AuthFailed,
            FailureKind::Cancelled => Failure::Cancelled,
            FailureKind::Protocol => Failure::Protocol,
            FailureKind::ChannelFailed => Failure::ChannelFailed,
            FailureKind::Disconnected => Failure::Disconnected,
            FailureKind::KeyUnusable => Failure::KeyUnusable,
            FailureKind::ProxyFailed => Failure::ProxyFailed,
            FailureKind::ProxyAuthFailed => Failure::ProxyAuthFailed,
            FailureKind::ForwardingRefused => Failure::ForwardingRefused,
        },
        SessionError::ConnectionLost => Failure::ConnectionLost,
        SessionError::KeepaliveTimeout => Failure::KeepaliveTimeout,
        SessionError::ClosedByServer => Failure::ClosedByServer,
        SessionError::HostKeyRevoked => Failure::HostKeyRevoked,
        SessionError::Cancelled => Failure::Cancelled,
        SessionError::Locked => Failure::Locked,
        SessionError::HostMissing => Failure::HostMissing,
        SessionError::InvalidTarget => Failure::InvalidTarget,
        SessionError::RouteInvalid => Failure::JumpHosts,
        SessionError::ShellUnavailable => Failure::LocalShell,
        SessionError::Internal => Failure::Other,
    }
}

/// The icon of a tab before the session reports: the host's OS (UI 1.1),
/// else the server icon; a local terminal's monitor.
pub fn initial_icon(os_family: Option<&str>, local: bool) -> IconName {
    if local {
        IconName::Monitor
    } else {
        crate::hosts::layout::os_icon(os_family)
    }
}
