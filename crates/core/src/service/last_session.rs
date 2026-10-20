//! The last session of Restore (UI 6, 4.7; docs/data-model.md, section
//! 16.3): the windows, tabs and pane trees open when the app last exited,
//! with the custom tab titles of Rename (UI 4.4) and the session targets
//! (saved host links, quick-connect `user@host:port`, local shell ids).
//! Never secrets or terminal content.
//!
//! It holds host data (quick-connect addresses and usernames), so it is not
//! a `local_settings` value (plaintext, docs/storage-format.md, section 4.3)
//! but an encrypted record of the personal vault: type `last_session`
//! (0x0A02), at most one, device-local (sync never pushes it and ignores
//! pulled copies). The core checks every value again before writing.
//! Nothing here logs addresses, usernames or titles.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Core;
use super::dto::RecordRef;
use super::records::parts;
use crate::error::{CoreError, ValidationReason};
use crate::model::time::format_timestamp;
use crate::model::types::RecordKind;

/// Windows kept at most.
pub const MAX_WINDOWS: usize = 16;
/// Tabs kept at most per window (docs/data-model.md, section 12).
pub const MAX_TABS: usize = crate::settings::SESSION_LAST_MAX_TABS;
/// Panes kept at most per tab (section 12).
pub const MAX_PANES_PER_TAB: usize = crate::settings::SESSION_LAST_MAX_LEAVES;
/// Split nesting kept at most per tab (section 12; a leaf has depth 0).
pub const MAX_DEPTH: usize = crate::settings::SESSION_LAST_MAX_DEPTH;
/// Panes kept at most in all (keeps the record far below the payload limit).
pub const MAX_PANES: usize = 512;
/// Longest custom tab title (characters).
pub const MAX_TITLE: usize = 256;

/// The saved session of the last exit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastSession {
    /// When it was saved (RFC 3339, set by the core).
    pub saved_at: String,
    /// The windows, the first one first (Restore puts it into the current
    /// window and opens the others as new windows).
    pub windows: Vec<SavedWindow>,
}

impl LastSession {
    /// Whether anything can be restored.
    pub fn has_tabs(&self) -> bool {
        self.windows.iter().any(|window| !window.tabs.is_empty())
    }
}

/// The session tabs of one window, left to right.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedWindow {
    pub tabs: Vec<SavedTab>,
}

/// One session tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedTab {
    /// The title set with Rename (UI 4.4); `None`: the host's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub root: SavedPane,
}

/// How a split lays out its panes (docs/data-model.md, section 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    /// Side by side ("Split vertically").
    Row,
    /// Stacked ("Split horizontally").
    Column,
}

/// A pane tree in the shape of docs/data-model.md, section 12.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum SavedPane {
    Leaf {
        session: SavedSession,
    },
    Split {
        direction: SplitDirection,
        /// The share of each child, summing to 1.
        sizes: Vec<f64>,
        /// 2 to 4 panes.
        children: Vec<SavedPane>,
    },
}

impl SavedPane {
    /// The sessions of the tree, depth first.
    pub fn sessions(&self) -> Vec<&SavedSession> {
        match self {
            SavedPane::Leaf { session } => vec![session],
            SavedPane::Split { children, .. } => {
                children.iter().flat_map(SavedPane::sessions).collect()
            }
        }
    }
}

/// What a pane connects to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSession", into = "RawSession")]
pub enum SavedSession {
    /// SSH to a saved host.
    Host(RecordRef),
    /// SSH quick connect (UI 2.1) to `username@address:port`.
    Quick {
        address: String,
        port: Option<u16>,
        username: Option<String>,
    },
    /// A local terminal; `shell` is a detected shell's id, `None` the
    /// default shell of this device.
    Local { shell: Option<String> },
}

/// The JSON form of [`SavedSession`]: `{"kind": "ssh", "host": host_link}`,
/// `{"kind": "ssh", "address", "port"?, "username"?}` or
/// `{"kind": "local", "shell"?}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSession {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<RecordRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shell: Option<String>,
}

impl TryFrom<RawSession> for SavedSession {
    type Error = &'static str;

    fn try_from(raw: RawSession) -> Result<Self, Self::Error> {
        let RawSession {
            kind,
            host,
            address,
            port,
            username,
            shell,
        } = raw;
        let plain = port.is_none() && username.is_none() && shell.is_none();
        match (kind.as_str(), host, address) {
            ("ssh", Some(host), None) if plain => Ok(SavedSession::Host(host)),
            ("ssh", None, Some(address)) if shell.is_none() => Ok(SavedSession::Quick {
                address,
                port,
                username,
            }),
            ("local", None, None) if port.is_none() && username.is_none() => {
                Ok(SavedSession::Local { shell })
            }
            _ => Err("invalid session"),
        }
    }
}

impl From<SavedSession> for RawSession {
    fn from(session: SavedSession) -> Self {
        let mut raw = RawSession {
            kind: "ssh".to_owned(),
            host: None,
            address: None,
            port: None,
            username: None,
            shell: None,
        };
        match session {
            SavedSession::Host(host) => raw.host = Some(host),
            SavedSession::Quick {
                address,
                port,
                username,
            } => {
                raw.address = Some(address);
                raw.port = port;
                raw.username = username;
            }
            SavedSession::Local { shell } => {
                raw.kind = "local".to_owned();
                raw.shell = shell;
            }
        }
        raw
    }
}

/// A text of 1 to `max` characters without control characters.
fn text_ok(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control)
}

/// Checks a pane tree at nesting `depth`; returns its pane count.
fn check_pane(pane: &SavedPane, depth: usize, field: &str) -> Result<usize, CoreError> {
    use ValidationReason::{Invalid, TooDeep, TooMany};
    match pane {
        SavedPane::Leaf { session } => {
            let ok = match session {
                SavedSession::Host(_) => true,
                SavedSession::Quick {
                    address,
                    port,
                    username,
                } => {
                    text_ok(address, 255)
                        && !address.chars().any(char::is_whitespace)
                        && *port != Some(0)
                        && username.as_deref().is_none_or(|user| text_ok(user, 255))
                }
                SavedSession::Local { shell } => {
                    shell.as_deref().is_none_or(|shell| text_ok(shell, 128))
                }
            };
            if ok {
                Ok(1)
            } else {
                Err(CoreError::field(format!("{field}.session"), Invalid))
            }
        }
        SavedPane::Split {
            sizes, children, ..
        } => {
            if !(2..=4).contains(&children.len()) || sizes.len() != children.len() {
                return Err(CoreError::field(field.to_owned(), Invalid));
            }
            if depth >= MAX_DEPTH {
                return Err(CoreError::field(field.to_owned(), TooDeep));
            }
            let total: f64 = sizes.iter().sum();
            if sizes.iter().any(|size| !size.is_finite() || *size <= 0.0)
                || (total - 1.0).abs() > 0.001
            {
                return Err(CoreError::field(format!("{field}.sizes"), Invalid));
            }
            let mut panes = 0;
            for (index, child) in children.iter().enumerate() {
                panes += check_pane(child, depth + 1, &format!("{field}.children.{index}"))?;
                if panes > MAX_PANES_PER_TAB {
                    return Err(CoreError::field(field.to_owned(), TooMany));
                }
            }
            Ok(panes)
        }
    }
}

/// Checks one tab of a save at `field`: its title and its pane tree;
/// returns its pane count.
fn check_tab_at(tab: &SavedTab, field: &str) -> Result<usize, CoreError> {
    if tab
        .title
        .as_deref()
        .is_some_and(|title| !text_ok(title, MAX_TITLE))
    {
        return Err(CoreError::field(
            format!("{field}.title"),
            ValidationReason::Invalid,
        ));
    }
    check_pane(&tab.root, 0, &format!("{field}.root"))
}

/// Checks one tab by the rules of [`check_windows`] (its title and pane
/// tree, not the limits of the whole save); returns its pane count. The app
/// leaves out a tab that fails, so one bad tab does not lose the save.
pub fn check_tab(tab: &SavedTab) -> Result<usize, CoreError> {
    check_tab_at(tab, "tab")
}

/// Checks the windows of a save (the limits of this module, the shapes of
/// docs/data-model.md, section 16.3).
pub fn check_windows(windows: &[SavedWindow]) -> Result<(), CoreError> {
    use ValidationReason::TooMany;
    if windows.len() > MAX_WINDOWS {
        return Err(CoreError::field("windows", TooMany));
    }
    let mut panes = 0;
    for (w, window) in windows.iter().enumerate() {
        if window.tabs.len() > MAX_TABS {
            return Err(CoreError::field(format!("windows.{w}.tabs"), TooMany));
        }
        for (t, tab) in window.tabs.iter().enumerate() {
            panes += check_tab_at(tab, &format!("windows.{w}.tabs.{t}"))?;
            if panes > MAX_PANES {
                return Err(CoreError::field("windows", TooMany));
            }
        }
    }
    Ok(())
}

impl Core {
    /// The session saved at the last exit, if any (Restore, UI 6). A value
    /// that no longer passes the checks counts as none.
    pub fn last_session(&self) -> Result<Option<LastSession>, CoreError> {
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        let Some(record) = parts.session.index.of_kind(RecordKind::LastSession).next() else {
            return Ok(None);
        };
        let value = Value::Object(record.data().clone());
        match serde_json::from_value::<LastSession>(value) {
            Ok(session) if check_windows(&session.windows).is_ok() => Ok(Some(session)),
            _ => {
                tracing::warn!("last session unreadable; ignored");
                Ok(None)
            }
        }
    }

    /// Saves the open windows for Restore, replacing the previous value
    /// (the app calls it at exit, when its last window closes and before it
    /// locks). Windows without tabs are dropped; nothing left removes the
    /// saved value, so Restore has nothing to offer.
    pub fn last_session_save(&self, windows: Vec<SavedWindow>) -> Result<(), CoreError> {
        let windows: Vec<SavedWindow> = windows
            .into_iter()
            .filter(|window| !window.tabs.is_empty())
            .collect();
        check_windows(&windows)?;
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let existing: Vec<uuid::Uuid> = parts
            .session
            .index
            .of_kind(RecordKind::LastSession)
            .map(|record| record.record_id)
            .collect();
        let class = RecordKind::LastSession.class();
        if windows.is_empty() {
            if existing.is_empty() {
                return Ok(());
            }
            // Device-local and never synced: no tombstone is needed.
            let deletions = existing.into_iter().map(|id| (id, class)).collect();
            self.commit_records(&mut parts, Vec::new(), deletions, Vec::new())?;
            tracing::info!("last session cleared");
            return Ok(());
        }
        let session = LastSession {
            saved_at: format_timestamp(now),
            windows,
        };
        let data = match serde_json::to_value(&session) {
            Ok(Value::Object(data)) => data,
            _ => return Err(CoreError::Internal("last session did not serialize")),
        };
        let record_id = existing
            .first()
            .copied()
            .unwrap_or_else(|| parts.ids.next(now));
        let write = parts.session.prepare_write(
            parts.store,
            RecordKind::LastSession,
            record_id,
            data,
            now,
        )?;
        // Extra copies (none are expected) go.
        let deletions = existing.into_iter().skip(1).map(|id| (id, class)).collect();
        self.commit_records(&mut parts, vec![write], deletions, Vec::new())?;
        tracing::info!(
            windows = session.windows.len(),
            tabs = session
                .windows
                .iter()
                .map(|window| window.tabs.len())
                .sum::<usize>(),
            "last session saved"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn host() -> SavedSession {
        SavedSession::Host(RecordRef {
            vault_id: Uuid::nil(),
            record_id: Uuid::max(),
        })
    }

    fn leaf(session: SavedSession) -> SavedPane {
        SavedPane::Leaf { session }
    }

    fn split(children: Vec<SavedPane>) -> SavedPane {
        let n = children.len() as f64;
        SavedPane::Split {
            direction: SplitDirection::Row,
            sizes: vec![1.0 / n; children.len()],
            children,
        }
    }

    #[test]
    fn the_json_shape_round_trips() {
        let session = LastSession {
            saved_at: "2026-10-20T10:00:00.000Z".to_owned(),
            windows: vec![SavedWindow {
                tabs: vec![
                    SavedTab {
                        title: Some("db".to_owned()),
                        root: split(vec![
                            leaf(host()),
                            leaf(SavedSession::Quick {
                                address: "192.0.2.7".to_owned(),
                                port: Some(2222),
                                username: Some("admin".to_owned()),
                            }),
                        ]),
                    },
                    SavedTab {
                        title: None,
                        root: leaf(SavedSession::Local {
                            shell: Some("pwsh".to_owned()),
                        }),
                    },
                ],
            }],
        };
        let value = serde_json::to_value(&session).unwrap();
        assert_eq!(value["windows"][0]["tabs"][0]["root"]["direction"], "row");
        assert_eq!(
            value["windows"][0]["tabs"][0]["root"]["children"][1]["session"],
            json!({"kind": "ssh", "address": "192.0.2.7", "port": 2222, "username": "admin"})
        );
        assert_eq!(
            value["windows"][0]["tabs"][1]["root"]["session"],
            json!({"kind": "local", "shell": "pwsh"})
        );
        assert!(value["windows"][0]["tabs"][1].get("title").is_none());
        let back: LastSession = serde_json::from_value(value).unwrap();
        assert_eq!(back, session);
        assert!(back.has_tabs());
        assert_eq!(back.windows[0].tabs[0].root.sessions().len(), 2);
    }

    #[test]
    fn malformed_sessions_are_refused() {
        let parse = |value: Value| serde_json::from_value::<SavedSession>(value).is_ok();
        assert!(parse(json!({"kind": "local"})));
        assert!(!parse(json!({"kind": "telnet", "address": "x"})));
        assert!(!parse(json!({"kind": "ssh"})));
        assert!(!parse(json!({"kind": "local", "address": "x"})));
        assert!(!parse(
            json!({"kind": "ssh", "address": "x", "shell": "pwsh"})
        ));
        assert!(!parse(
            json!({"kind": "ssh", "address": "x", "secret": "y"})
        ));
        // A leaf with extra members is not a leaf.
        let pane = |value: Value| serde_json::from_value::<SavedPane>(value).is_ok();
        assert!(!pane(
            json!({"session": {"kind": "local"}, "password": "x"})
        ));
    }

    #[test]
    fn limits_and_values_are_checked() {
        let tab = |root| SavedTab { title: None, root };
        let window = |tabs| SavedWindow { tabs };
        assert!(check_windows(&[window(vec![tab(leaf(host()))])]).is_ok());
        // Sizes must add up to 1, splits have 2 to 4 panes.
        let bad = SavedPane::Split {
            direction: SplitDirection::Column,
            sizes: vec![0.5, 0.6],
            children: vec![leaf(host()), leaf(host())],
        };
        assert!(check_windows(&[window(vec![tab(bad)])]).is_err());
        assert!(check_windows(&[window(vec![tab(split(vec![leaf(host())]))])]).is_err());
        // 16 panes at depth 2 are kept, 17 are not; depth 4 is kept, 5 is not.
        let four = || split(vec![leaf(host()), leaf(host()), leaf(host()), leaf(host())]);
        let sixteen = split(vec![four(), four(), four(), four()]);
        assert!(check_windows(&[window(vec![tab(sixteen.clone())])]).is_ok());
        let seventeen = split(vec![sixteen, leaf(host())]);
        assert!(check_windows(&[window(vec![tab(seventeen)])]).is_err());
        let mut deep = leaf(host());
        for _ in 0..MAX_DEPTH {
            deep = split(vec![deep, leaf(host())]);
        }
        assert!(check_windows(&[window(vec![tab(deep.clone())])]).is_ok());
        let deeper = split(vec![deep, leaf(host())]);
        assert!(check_windows(&[window(vec![tab(deeper)])]).is_err());
        // Titles: no control characters, not blank.
        let titled = |title: &str| SavedTab {
            title: Some(title.to_owned()),
            root: leaf(host()),
        };
        assert!(check_windows(&[window(vec![titled("prod")])]).is_ok());
        assert!(check_windows(&[window(vec![titled("a\u{7}b")])]).is_err());
        assert!(check_windows(&[window(vec![titled("  ")])]).is_err());
        // One tab on its own: the same rules, without the save's limits.
        assert_eq!(check_tab(&titled("prod")).unwrap(), 1);
        assert!(check_tab(&titled("a\tb")).is_err());
        assert!(check_tab(&titled(&"t".repeat(MAX_TITLE + 1))).is_err());
        assert_eq!(check_tab(&tab(four())).unwrap(), 4);
        assert!(check_tab(&tab(split(vec![leaf(host())]))).is_err());
        // Quick connect: an address without spaces, a port above 0.
        let quick = |address: &str, port| {
            tab(leaf(SavedSession::Quick {
                address: address.to_owned(),
                port,
                username: None,
            }))
        };
        assert!(check_windows(&[window(vec![quick("host.example", Some(22))])]).is_ok());
        assert!(check_windows(&[window(vec![quick("a b", None)])]).is_err());
        assert!(check_windows(&[window(vec![quick("h", Some(0))])]).is_err());
        let many: Vec<SavedWindow> = (0..=MAX_WINDOWS).map(|_| window(vec![])).collect();
        assert!(check_windows(&many).is_err());
    }
}
