//! View models of the records the UI shows, built from the core's
//! `RecordView`s (docs/data-model.md, sections 7.1 and 7.2).
//!
//! The raw `data` object stays available for the panels: an edit starts from
//! it and changes only the edited members (`record_save` rule). `Debug` never
//! prints host data (docs/security.md, section 9.1).

use serde_json::{Map, Value};
use shellpp_core::service::dto::{GroupView, HostView, RecordView, TagColor};
use uuid::Uuid;

/// A host as the screens use it.
#[derive(Clone, PartialEq)]
pub struct HostItem {
    pub record_id: Uuid,
    pub vault_id: Uuid,
    pub revision: u64,
    /// `label`; the title falls back to the address (UI 2.3).
    pub label: Option<String>,
    pub address: String,
    pub tags: Vec<String>,
    /// `--tag-graphite` unless the user picked another color.
    pub color: TagColor,
    /// `ssh.username` of the host, else the inherited one.
    pub username: Option<String>,
    /// `ssh.port` of the host, else the inherited one.
    pub port: Option<u16>,
    /// `os.family` when detected (`linux`, `windows`, ...): picks the tile icon.
    pub os_family: Option<String>,
    /// The resolved group (last element of the core's `path`), `None` at top level.
    pub group_id: Option<Uuid>,
    /// Ancestor groups, top level first, ending with the host's own group.
    pub path: Vec<Uuid>,
    pub created_at: String,
    pub modified_at: String,
    /// The full `host` data object without secret members.
    pub data: Value,
    /// Dotted paths of secret members that hold a value (`ssh.password`).
    pub secrets_set: Vec<String>,
}

impl std::fmt::Debug for HostItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostItem")
            .field("record_id", &self.record_id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// A group as the screens use it.
#[derive(Clone, PartialEq)]
pub struct GroupItem {
    pub record_id: Uuid,
    pub vault_id: Uuid,
    pub revision: u64,
    pub name: String,
    pub color: TagColor,
    /// The resolved parent (last element of the core's `path`), `None` at top level.
    pub parent_id: Option<Uuid>,
    /// Ancestor groups, top level first, ending with the parent.
    pub path: Vec<Uuid>,
    /// Live hosts in the whole subtree (UI 2.3 group card).
    pub host_count: u32,
    /// Direct children: subgroups plus hosts.
    pub child_count: u32,
    pub created_at: String,
    pub modified_at: String,
    /// The full `group` data object.
    pub data: Value,
}

impl std::fmt::Debug for GroupItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupItem")
            .field("record_id", &self.record_id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

fn str_member(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn color_member(data: &Value) -> TagColor {
    data.get("color")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

impl HostItem {
    pub fn from_view(view: &HostView) -> Self {
        let data = &view.data;
        let ssh = data.get("ssh");
        let username = ssh
            .and_then(|ssh| str_member(ssh, "username"))
            .or_else(|| view.inherited.username.as_ref().map(|v| v.value.clone()));
        let port = ssh
            .and_then(|ssh| ssh.get("port"))
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .or_else(|| view.inherited.port.as_ref().map(|v| v.value));
        let tags = data
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            record_id: view.record_id,
            vault_id: view.vault_id,
            revision: view.revision,
            label: str_member(data, "label"),
            address: str_member(data, "address").unwrap_or_default(),
            tags,
            color: color_member(data),
            username,
            port,
            os_family: data.get("os").and_then(|os| str_member(os, "family")),
            group_id: view.path.last().copied(),
            path: view.path.clone(),
            created_at: view.created_at.clone(),
            modified_at: view.modified_at.clone(),
            data: view.data.clone(),
            secrets_set: view.secrets_set.clone(),
        }
    }

    /// The card title: the label, else the address (UI 2.3).
    pub fn title(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.address)
    }

    /// The data object to start an edit from.
    pub fn data_map(&self) -> Map<String, Value> {
        self.data.as_object().cloned().unwrap_or_default()
    }
}

impl GroupItem {
    pub fn from_view(view: &GroupView) -> Self {
        Self {
            record_id: view.record_id,
            vault_id: view.vault_id,
            revision: view.revision,
            name: str_member(&view.data, "name").unwrap_or_default(),
            color: color_member(&view.data),
            parent_id: view.path.last().copied(),
            path: view.path.clone(),
            host_count: view.host_count,
            child_count: view.child_count,
            created_at: view.created_at.clone(),
            modified_at: view.modified_at.clone(),
            data: view.data.clone(),
        }
    }

    /// The data object to start an edit from.
    pub fn data_map(&self) -> Map<String, Value> {
        self.data.as_object().cloned().unwrap_or_default()
    }
}

/// Splits query results into hosts and groups, keeping their order.
pub fn split_views(views: &[RecordView]) -> (Vec<HostItem>, Vec<GroupItem>) {
    let mut hosts = Vec::new();
    let mut groups = Vec::new();
    for view in views {
        match view {
            RecordView::Host(host) => hosts.push(HostItem::from_view(host)),
            RecordView::Group(group) => groups.push(GroupItem::from_view(group)),
        }
    }
    (hosts, groups)
}
