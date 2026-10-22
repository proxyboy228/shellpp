//! Snippets, snippet packages and forwarding rules (docs/data-model.md,
//! sections 10 and 11; stage 5 without its screens): save and list. Removal
//! goes through `record_delete` (a removed package moves its snippets out
//! of it, data-model 10.2), restore through `record_restore`.
//!
//! Every write goes through the record path of `records.rs`. Nothing here
//! logs scripts, addresses or labels.

use std::fmt;

use serde_json::{Map, Value};
use uuid::Uuid;

use super::Core;
use super::dto::RecordRef;
use super::records::{check_vault, parts};
use super::session::Session;
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::model::host;
use crate::model::snippet::{self, uuid_list, uuid_member};
use crate::model::text::fold;
use crate::model::types::RecordKind;

/// Input of `snippet_save`, `snippet_package_save` and
/// `forwarding_rule_save`: the record's data (members it does not name are
/// kept); `record_id` `None` creates one.
#[derive(Clone)]
pub struct RecordInput {
    pub vault_id: Uuid,
    pub record_id: Option<Uuid>,
    pub data: Map<String, Value>,
}

impl fmt::Debug for RecordInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordInput")
            .field("vault_id", &self.vault_id)
            .field("record_id", &self.record_id)
            .finish_non_exhaustive()
    }
}

/// A snippet as the screens and the run use it.
#[derive(Clone, PartialEq, Eq)]
pub struct SnippetView {
    pub record: RecordRef,
    pub label: Option<String>,
    pub script: String,
    pub package_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub target_host_ids: Vec<Uuid>,
    /// The `{{ name }}` variables of the script, once each, in order.
    pub variables: Vec<String>,
}

impl fmt::Debug for SnippetView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnippetView")
            .field("record", &self.record)
            .field("variables", &self.variables.len())
            .finish_non_exhaustive()
    }
}

/// A snippet package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetPackageView {
    pub record: RecordRef,
    pub name: String,
}

/// The kind of a forwarding rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardType {
    Local,
    Remote,
    Dynamic,
}

/// A forwarding rule as the screen and the forwarding service use it.
#[derive(Clone, PartialEq, Eq)]
pub struct ForwardingRuleView {
    pub record: RecordRef,
    pub kind: ForwardType,
    pub host_id: Uuid,
    pub label: Option<String>,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_address: Option<String>,
    pub dest_port: Option<u16>,
    /// Start when the host connects / when the app starts (after unlock).
    pub autostart_on_connect: bool,
    pub autostart_on_launch: bool,
    /// The bind address opens the port beyond loopback: the UI warns.
    pub wide_bind: bool,
    /// One line for the card (data-model 11): `127.0.0.1:8080 → db:5432 via
    /// web`, or `SOCKS 127.0.0.1:1080 via web`.
    pub summary: String,
}

impl fmt::Debug for ForwardingRuleView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForwardingRuleView")
            .field("record", &self.record)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// A workspace (UI 4.5): its name and tabs (host links and local
/// terminals; data-model 12).
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceView {
    pub record: RecordRef,
    pub name: String,
    pub tabs: Vec<crate::service::last_session::SavedTab>,
}

fn workspace_view(vault_id: Uuid, record_id: Uuid, data: &Map<String, Value>) -> WorkspaceView {
    WorkspaceView {
        record: RecordRef {
            vault_id,
            record_id,
        },
        name: string_member(data, "name").unwrap_or_default(),
        tabs: crate::model::workspace::tabs(data).unwrap_or_default(),
    }
}

fn string_member(data: &Map<String, Value>, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn port_member(data: &Map<String, Value>, key: &str) -> Option<u16> {
    data.get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
}

fn snippet_view(vault_id: Uuid, record_id: Uuid, data: &Map<String, Value>) -> SnippetView {
    let script = string_member(data, "script").unwrap_or_default();
    SnippetView {
        record: RecordRef {
            vault_id,
            record_id,
        },
        label: string_member(data, "label"),
        variables: crate::snippet::variables(&script),
        script,
        package_id: uuid_member(data, "package_id"),
        tags: data
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        target_host_ids: uuid_list(data, "target_host_ids"),
    }
}

/// `host:port` with an IPv6 literal in brackets.
fn endpoint(address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

fn rule_view(
    session: &Session,
    vault_id: Uuid,
    record_id: Uuid,
    data: &Map<String, Value>,
) -> Option<ForwardingRuleView> {
    let kind = match data.get("type")?.as_str()? {
        "local" => ForwardType::Local,
        "remote" => ForwardType::Remote,
        "dynamic" => ForwardType::Dynamic,
        _ => return None,
    };
    let host_id = uuid_member(data, "host_id")?;
    let bind_address = string_member(data, "bind_address").unwrap_or_else(|| "127.0.0.1".into());
    let bind_port = port_member(data, "bind_port")?;
    let dest_address = string_member(data, "dest_address");
    let dest_port = port_member(data, "dest_port");
    let host_title = session
        .index
        .get_kind(&host_id, RecordKind::Host)
        .map(|record| host::title(record.data()).to_owned())
        .unwrap_or_default();
    let summary = match (kind, &dest_address, dest_port) {
        (ForwardType::Dynamic, _, _) => {
            format!(
                "SOCKS {} via {host_title}",
                endpoint(&bind_address, bind_port)
            )
        }
        (_, Some(dest), Some(port)) => format!(
            "{} → {} via {host_title}",
            endpoint(&bind_address, bind_port),
            endpoint(dest, port)
        ),
        _ => return None,
    };
    Some(ForwardingRuleView {
        record: RecordRef {
            vault_id,
            record_id,
        },
        kind,
        host_id,
        label: string_member(data, "label"),
        wide_bind: snippet::is_wide_bind(&bind_address),
        bind_address,
        bind_port,
        dest_address,
        dest_port,
        autostart_on_connect: data
            .get("autostart_on_connect")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        autostart_on_launch: data
            .get("autostart_on_launch")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        summary,
    })
}

/// `missing_ref` for a reference that names no live record of `kind`.
fn check_ref(
    session: &Session,
    id: Option<Uuid>,
    kind: RecordKind,
    field: &str,
    errors: &mut Vec<FieldError>,
) {
    if let Some(id) = id
        && session.index.get_kind(&id, kind).is_none()
    {
        errors.push(FieldError::new(field, ValidationReason::MissingRef));
    }
}

impl Core {
    /// The stored data of `record_id` when it is a live record of `kind`.
    fn stored_of(
        session: &Session,
        record_id: Option<Uuid>,
        kind: RecordKind,
    ) -> Result<Option<Map<String, Value>>, CoreError> {
        match record_id {
            Some(id) => session
                .index
                .get_kind(&id, kind)
                .map(|record| Some(record.data().clone()))
                .ok_or(CoreError::NotFound),
            None => Ok(None),
        }
    }

    /// Writes one record of `kind` (validated data) and returns its id.
    fn save_kind(
        &self,
        input: &RecordInput,
        kind: RecordKind,
        build: impl FnOnce(
            &Session,
            Option<&Map<String, Value>>,
        ) -> Result<Map<String, Value>, Vec<FieldError>>,
    ) -> Result<Uuid, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        let stored = Self::stored_of(parts.session, input.record_id, kind)?;
        let data = build(parts.session, stored.as_ref()).map_err(CoreError::Validation)?;
        let record_id = input.record_id.unwrap_or_else(|| parts.ids.next(now));
        let write = parts
            .session
            .prepare_write(parts.store, kind, record_id, data, now)?;
        self.commit_records(&mut parts, vec![write], Vec::new(), Vec::new())?;
        tracing::info!(kind = kind.name(), "record saved");
        Ok(record_id)
    }

    /// Creates or updates a snippet (data-model 10.1): `package_id` must
    /// name a live package and every target a live host (`missing_ref`).
    pub fn snippet_save(&self, input: RecordInput) -> Result<SnippetView, CoreError> {
        let record_id = self.save_kind(&input, RecordKind::Snippet, |session, stored| {
            let mut data = snippet::build_snippet(&input.data, stored)?;
            let mut errors = Vec::new();
            check_ref(
                session,
                uuid_member(&data, "package_id"),
                RecordKind::SnippetPackage,
                "package_id",
                &mut errors,
            );
            for host in uuid_list(&data, "target_host_ids") {
                check_ref(
                    session,
                    Some(host),
                    RecordKind::Host,
                    "target_host_ids",
                    &mut errors,
                );
            }
            if errors.is_empty() {
                Ok(data)
            } else {
                crate::model::wipe::wipe_map(&mut data);
                errors.dedup();
                Err(errors)
            }
        })?;
        self.snippet_get(input.vault_id, record_id)
    }

    /// One snippet.
    pub fn snippet_get(&self, vault_id: Uuid, record_id: Uuid) -> Result<SnippetView, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let record = session
            .index
            .get_kind(&record_id, RecordKind::Snippet)
            .ok_or(CoreError::NotFound)?;
        Ok(snippet_view(vault_id, record_id, record.data()))
    }

    /// Every snippet of the vault, by label (the first script line when
    /// there is none), then id.
    pub fn snippets_list(&self, vault_id: Uuid) -> Result<Vec<SnippetView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let mut views: Vec<SnippetView> = session
            .index
            .of_kind(RecordKind::Snippet)
            .map(|record| snippet_view(vault_id, record.record_id, record.data()))
            .collect();
        let title = |view: &SnippetView| {
            fold(
                view.label
                    .as_deref()
                    .unwrap_or_else(|| view.script.lines().next().unwrap_or("")),
            )
        };
        views.sort_by(|a, b| {
            title(a)
                .cmp(&title(b))
                .then_with(|| a.record.record_id.cmp(&b.record.record_id))
        });
        Ok(views)
    }

    /// Creates or updates a snippet package (data-model 10.2).
    pub fn snippet_package_save(
        &self,
        input: RecordInput,
    ) -> Result<SnippetPackageView, CoreError> {
        let record_id = self.save_kind(&input, RecordKind::SnippetPackage, |_, stored| {
            snippet::build_package(&input.data, stored)
        })?;
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let record = session
            .index
            .get_kind(&record_id, RecordKind::SnippetPackage)
            .ok_or(CoreError::NotFound)?;
        Ok(SnippetPackageView {
            record: RecordRef {
                vault_id: input.vault_id,
                record_id,
            },
            name: string_member(record.data(), "name").unwrap_or_default(),
        })
    }

    /// Creates or updates a workspace (UI 4.5; data-model 12): a name and
    /// the tabs of a window, linking saved hosts and local terminals.
    pub fn workspace_save(&self, input: RecordInput) -> Result<WorkspaceView, CoreError> {
        let record_id = self.save_kind(&input, RecordKind::Workspace, |_, stored| {
            crate::model::workspace::build(&input.data, stored)
        })?;
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let record = session
            .index
            .get_kind(&record_id, RecordKind::Workspace)
            .ok_or(CoreError::NotFound)?;
        Ok(workspace_view(input.vault_id, record_id, record.data()))
    }

    /// Every workspace of the vault, by name.
    pub fn workspaces_list(&self, vault_id: Uuid) -> Result<Vec<WorkspaceView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let mut views: Vec<WorkspaceView> = session
            .index
            .of_kind(RecordKind::Workspace)
            .map(|record| workspace_view(vault_id, record.record_id, record.data()))
            .collect();
        views.sort_by(|a, b| {
            fold(&a.name)
                .cmp(&fold(&b.name))
                .then_with(|| a.record.record_id.cmp(&b.record.record_id))
        });
        Ok(views)
    }

    /// The workspaces' ids and names, by name: what the new tab page and the
    /// menus show, without reading the tabs.
    pub fn workspace_names(&self, vault_id: Uuid) -> Result<Vec<(Uuid, String)>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let mut names: Vec<(Uuid, String)> = session
            .index
            .of_kind(RecordKind::Workspace)
            .map(|record| {
                (
                    record.record_id,
                    string_member(record.data(), "name").unwrap_or_default(),
                )
            })
            .collect();
        names.sort_by(|a, b| fold(&a.1).cmp(&fold(&b.1)).then_with(|| a.0.cmp(&b.0)));
        Ok(names)
    }

    /// Every snippet package of the vault, by name.
    pub fn snippet_packages_list(
        &self,
        vault_id: Uuid,
    ) -> Result<Vec<SnippetPackageView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let mut views: Vec<SnippetPackageView> = session
            .index
            .of_kind(RecordKind::SnippetPackage)
            .map(|record| SnippetPackageView {
                record: RecordRef {
                    vault_id,
                    record_id: record.record_id,
                },
                name: string_member(record.data(), "name").unwrap_or_default(),
            })
            .collect();
        views.sort_by(|a, b| {
            fold(&a.name)
                .cmp(&fold(&b.name))
                .then_with(|| a.record.record_id.cmp(&b.record.record_id))
        });
        Ok(views)
    }

    /// Creates or updates a forwarding rule (data-model 11): `host_id` must
    /// name a live host (`missing_ref`).
    pub fn forwarding_rule_save(
        &self,
        input: RecordInput,
    ) -> Result<ForwardingRuleView, CoreError> {
        let record_id = self.save_kind(&input, RecordKind::ForwardingRule, |session, stored| {
            let data = snippet::build_rule(&input.data, stored)?;
            let mut errors = Vec::new();
            check_ref(
                session,
                uuid_member(&data, "host_id"),
                RecordKind::Host,
                "host_id",
                &mut errors,
            );
            if errors.is_empty() {
                Ok(data)
            } else {
                Err(errors)
            }
        })?;
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let record = session
            .index
            .get_kind(&record_id, RecordKind::ForwardingRule)
            .ok_or(CoreError::NotFound)?;
        rule_view(session, input.vault_id, record_id, record.data())
            .ok_or(CoreError::Internal("forwarding rule unreadable"))
    }

    /// The forwarding rules of the personal vault (the one open now): the
    /// only rules whose autostart flags count (data-model 11).
    pub fn forwarding_rules_personal(&self) -> Result<Vec<ForwardingRuleView>, CoreError> {
        let vault_id = {
            let inner = self.inner();
            Self::require_session(&inner)?.ring.vault_id()
        };
        self.forwarding_rules_list(vault_id)
    }

    /// Every forwarding rule of the vault, by label (the summary when
    /// there is none), then id. A rule whose data cannot be read is left out.
    pub fn forwarding_rules_list(
        &self,
        vault_id: Uuid,
    ) -> Result<Vec<ForwardingRuleView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let mut views: Vec<ForwardingRuleView> = session
            .index
            .of_kind(RecordKind::ForwardingRule)
            .filter_map(|record| rule_view(session, vault_id, record.record_id, record.data()))
            .collect();
        let title =
            |view: &ForwardingRuleView| fold(view.label.as_deref().unwrap_or(&view.summary));
        views.sort_by(|a, b| {
            title(a)
                .cmp(&title(b))
                .then_with(|| a.record.record_id.cmp(&b.record.record_id))
        });
        Ok(views)
    }
}

/// A keyword highlighting rule of the personal vault (data-model 14.2;
/// UI 12.4; D77): a custom rule or a category's colors.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HighlightRuleView {
    pub record: RecordRef,
    /// `custom` or `category`.
    pub kind: String,
    pub category: Option<String>,
    pub pattern: Option<String>,
    pub case_insensitive: bool,
    /// `#RRGGBB` on a dark terminal background.
    pub color_dark: String,
    /// `#RRGGBB` on a light background (`None`: the dark one).
    pub color_light: Option<String>,
    pub enabled: bool,
}

fn highlight_rule_view(
    vault_id: Uuid,
    record_id: Uuid,
    data: &Map<String, Value>,
) -> HighlightRuleView {
    let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
    HighlightRuleView {
        record: RecordRef {
            vault_id,
            record_id,
        },
        kind: text("kind").unwrap_or_default(),
        category: text("category"),
        pattern: text("pattern"),
        case_insensitive: data
            .get("case_insensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        color_dark: text("color_dark").unwrap_or_default(),
        color_light: text("color_light"),
        enabled: data.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    }
}

impl Core {
    /// The highlight rules of the vault, in creation order (data-model
    /// 14.2: custom rules are evaluated in it; of two records of one
    /// category, the newer counts).
    pub fn highlight_rules_list(
        &self,
        vault_id: Uuid,
    ) -> Result<Vec<HighlightRuleView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(vault_id))?;
        let mut views: Vec<HighlightRuleView> = session
            .index
            .of_kind(RecordKind::HighlightRule)
            .map(|record| highlight_rule_view(vault_id, record.record_id, record.data()))
            .collect();
        views.sort_by_key(|view| view.record.record_id);
        Ok(views)
    }

    /// Creates or updates a highlight rule: its checks (a pattern that
    /// compiles and does not match the empty string, `#RRGGBB` colors) and
    /// at most [`crate::model::highlight_rule::MAX_CUSTOM_RULES`] custom
    /// rules.
    pub fn highlight_rule_save(&self, input: RecordInput) -> Result<HighlightRuleView, CoreError> {
        let record_id = self.save_kind(&input, RecordKind::HighlightRule, |session, stored| {
            let data = crate::model::highlight_rule::build(&input.data, stored)?;
            let custom = data.get("kind").and_then(Value::as_str) == Some("custom");
            let others = session
                .index
                .of_kind(RecordKind::HighlightRule)
                .filter(|record| Some(record.record_id) != input.record_id)
                .filter(|record| {
                    record.data().get("kind").and_then(Value::as_str) == Some("custom")
                })
                .count();
            if custom && others >= crate::model::highlight_rule::MAX_CUSTOM_RULES {
                return Err(vec![FieldError::new("pattern", ValidationReason::TooMany)]);
            }
            Ok(data)
        })?;
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let record = session
            .index
            .get_kind(&record_id, RecordKind::HighlightRule)
            .ok_or(CoreError::NotFound)?;
        Ok(highlight_rule_view(
            input.vault_id,
            record_id,
            record.data(),
        ))
    }
}
