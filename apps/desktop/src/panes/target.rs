//! What a pane connects to ([`PaneTarget`]): the target of its session, used
//! to open a pane (the compact picker, Duplicate, Restore) and saved in the
//! Restore snapshot (lead decision L5: host record ids, quick-connect
//! `user@host:port`, local shell ids; never secrets or terminal content).
//!
//! On disk a target is the `session` object of docs/data-model.md, section
//! 12: `{"kind": "ssh", "host": {"vault_id", "record_id"}}` for a saved
//! host, `{"kind": "ssh", "address", "port"?, "username"?}` for a quick
//! connect and `{"kind": "local", "shell"?}` for a local terminal (no
//! `shell`: the default shell of the device that restores it).

use gpui::SharedString;
use serde::{Deserialize, Serialize};
use shellpp_core::service::dto::SessionTarget;
use uuid::Uuid;

use crate::hosts::QuickTarget;
use crate::session_ui::TabInfo;
use crate::sessions::OpenRequest;
use crate::shell::{SessionKind, SessionRequest, display_address};
use crate::state::AppState;

/// The target of a pane's session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "WireSession", into = "WireSession")]
pub enum PaneTarget {
    /// A saved host.
    Host { vault_id: Uuid, record_id: Uuid },
    /// A quick connect without a saved host (UI 2.1).
    Quick(QuickTarget),
    /// A local terminal; `None` = the default shell (`terminal.local_shell`
    /// or the detected default, UI 12.4).
    Local { shell_id: Option<String> },
}

impl PaneTarget {
    /// The target of an open request (what a session pane was opened with).
    pub fn from_open(request: &OpenRequest) -> Self {
        match request {
            OpenRequest::Ssh(SessionTarget::Host {
                vault_id,
                record_id,
            }) => PaneTarget::Host {
                vault_id: *vault_id,
                record_id: *record_id,
            },
            OpenRequest::Ssh(SessionTarget::Quick {
                address,
                port,
                username,
            }) => PaneTarget::Quick(QuickTarget {
                username: username.clone(),
                host: address.clone(),
                port: *port,
            }),
            OpenRequest::Local { shell_id } => PaneTarget::Local {
                shell_id: shell_id.clone(),
            },
        }
    }

    /// The target of a session tab request (SSH to a saved host or a quick
    /// connect, a local terminal); `None` for the kinds without panes (SFTP,
    /// serial, the new tab page) and an SSH request without a target.
    pub fn from_request(request: &SessionRequest, state: &AppState) -> Option<Self> {
        match request.kind() {
            SessionKind::Ssh => {
                if let Some(record_id) = request.host_id() {
                    let vault_id = state
                        .host(record_id)
                        .map(|host| host.vault_id)
                        .or_else(|| state.vault_id())?;
                    return Some(PaneTarget::Host {
                        vault_id,
                        record_id,
                    });
                }
                request.quick_target().cloned().map(PaneTarget::Quick)
            }
            SessionKind::Local => Some(PaneTarget::Local { shell_id: None }),
            SessionKind::Sftp | SessionKind::Serial | SessionKind::NewTab => None,
        }
    }

    /// The saved host, if any.
    pub fn host_id(&self) -> Option<Uuid> {
        match self {
            PaneTarget::Host { record_id, .. } => Some(*record_id),
            PaneTarget::Quick(_) | PaneTarget::Local { .. } => None,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, PaneTarget::Local { .. })
    }

    /// The request that opens a session to this target.
    pub fn open_request(&self) -> OpenRequest {
        match self {
            PaneTarget::Host {
                vault_id,
                record_id,
            } => OpenRequest::Ssh(SessionTarget::Host {
                vault_id: *vault_id,
                record_id: *record_id,
            }),
            PaneTarget::Quick(quick) => OpenRequest::Ssh(SessionTarget::Quick {
                address: quick.host.clone(),
                port: quick.port,
                username: quick.username.clone(),
            }),
            PaneTarget::Local { shell_id } => OpenRequest::Local {
                shell_id: shell_id.clone(),
            },
        }
    }

    /// What the pane shows before the session layer knows more (title,
    /// address, OS icon) and what it opens. A saved host that is gone
    /// still opens: the session layer reports it (UI 4.8).
    pub fn session(&self, state: &AppState) -> (TabInfo, OpenRequest) {
        use crate::session_ui::bridge::initial_icon;
        let info = match self {
            PaneTarget::Host { record_id, .. } => match state.host(*record_id) {
                Some(host) => TabInfo::remote(
                    host.title().to_owned(),
                    display_address(host.username.as_deref(), &host.address, host.port),
                    initial_icon(host.os_family.as_deref(), false),
                ),
                None => TabInfo::remote(
                    SharedString::from(SessionKind::Ssh.default_title()),
                    "",
                    initial_icon(None, false),
                ),
            },
            PaneTarget::Quick(quick) => TabInfo::remote(
                quick.title(),
                display_address(quick.username.as_deref(), &quick.host, quick.port),
                initial_icon(None, false),
            ),
            PaneTarget::Local { .. } => TabInfo::local(
                SessionKind::Local.default_title(),
                SessionKind::Local.default_icon(),
            ),
        };
        (info, self.open_request())
    }
}

impl std::fmt::Display for PaneTarget {
    /// Only the kind: targets name hosts and users, which never go to logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PaneTarget::Host { .. } => "host",
            PaneTarget::Quick(_) => "quick connect",
            PaneTarget::Local { .. } => "local terminal",
        })
    }
}

/// `host_link` of docs/data-model.md, section 3.4.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostLink {
    vault_id: Uuid,
    record_id: Uuid,
}

/// The `session` object on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSession {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<HostLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shell: Option<String>,
}

impl From<PaneTarget> for WireSession {
    fn from(target: PaneTarget) -> Self {
        match target {
            PaneTarget::Host {
                vault_id,
                record_id,
            } => WireSession {
                kind: "ssh".into(),
                host: Some(HostLink {
                    vault_id,
                    record_id,
                }),
                ..Default::default()
            },
            PaneTarget::Quick(quick) => WireSession {
                kind: "ssh".into(),
                address: Some(quick.host),
                port: quick.port,
                username: quick.username,
                ..Default::default()
            },
            PaneTarget::Local { shell_id } => WireSession {
                kind: "local".into(),
                shell: shell_id,
                ..Default::default()
            },
        }
    }
}

impl TryFrom<WireSession> for PaneTarget {
    type Error = &'static str;

    fn try_from(wire: WireSession) -> Result<Self, Self::Error> {
        let nonempty = |value: Option<String>| value.filter(|value| !value.is_empty());
        match wire.kind.as_str() {
            "ssh" => match (wire.host, nonempty(wire.address)) {
                (Some(host), None) if wire.port.is_none() && wire.username.is_none() => {
                    Ok(PaneTarget::Host {
                        vault_id: host.vault_id,
                        record_id: host.record_id,
                    })
                }
                (None, Some(address)) => Ok(PaneTarget::Quick(QuickTarget {
                    username: nonempty(wire.username),
                    host: address,
                    port: wire.port,
                })),
                _ => Err("an SSH session needs a host link or an address"),
            },
            "local" if wire.host.is_none() && wire.address.is_none() => Ok(PaneTarget::Local {
                shell_id: nonempty(wire.shell),
            }),
            _ => Err("unknown session kind"),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn targets_round_trip_on_disk() {
        let vault_id = Uuid::new_v4();
        let record_id = Uuid::new_v4();
        let cases = [
            (
                PaneTarget::Host {
                    vault_id,
                    record_id,
                },
                json!({"kind": "ssh", "host": {"vault_id": vault_id, "record_id": record_id}}),
            ),
            (
                PaneTarget::Quick(QuickTarget {
                    username: Some("tester".into()),
                    host: "192.0.2.7".into(),
                    port: Some(2222),
                }),
                json!({"kind": "ssh", "address": "192.0.2.7", "port": 2222, "username": "tester"}),
            ),
            (
                PaneTarget::Quick(QuickTarget {
                    username: None,
                    host: "2001:db8::1".into(),
                    port: None,
                }),
                json!({"kind": "ssh", "address": "2001:db8::1"}),
            ),
            (
                PaneTarget::Local {
                    shell_id: Some("pwsh".into()),
                },
                json!({"kind": "local", "shell": "pwsh"}),
            ),
            (
                PaneTarget::Local { shell_id: None },
                json!({"kind": "local"}),
            ),
        ];
        for (target, wire) in cases {
            assert_eq!(serde_json::to_value(&target).unwrap(), wire);
            assert_eq!(serde_json::from_value::<PaneTarget>(wire).unwrap(), target);
            // The open request leads back to the same target.
            assert_eq!(PaneTarget::from_open(&target.open_request()), target);
        }
        for bad in [
            json!({"kind": "ssh"}),
            json!({"kind": "ssh", "address": ""}),
            json!({"kind": "ssh", "host": {"vault_id": vault_id, "record_id": record_id}, "address": "a"}),
            json!({"kind": "sftp", "address": "a"}),
            json!({"kind": "local", "address": "a"}),
            json!({"kind": "local", "password": "x"}),
        ] {
            assert!(serde_json::from_value::<PaneTarget>(bad).is_err());
        }
    }

    #[test]
    fn display_names_only_the_kind() {
        let quick = PaneTarget::Quick(QuickTarget {
            username: Some("tester".into()),
            host: "192.0.2.7".into(),
            port: None,
        });
        assert_eq!(quick.to_string(), "quick connect");
        assert!(!format!("{quick}").contains("192.0.2.7"));
    }
}
