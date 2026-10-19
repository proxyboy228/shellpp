//! Per-device settings stored in `local_settings` as JSON values, with
//! defaults and validation (docs/data-model.md, section 16.1;
//! docs/storage-format.md, section 4.3). Readable while locked.
//!
//! A missing row means the default. A stored value that is no longer valid
//! (for example written by a newer app) is kept in the database, and the
//! default is used as the effective value. Settings never hold secrets, host
//! data or command text.
//!
//! Owner: W2.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::error::{CoreError, FieldError, ValidationReason};
use crate::model::fields::theme_ref;
use crate::platform::OsKind;
use crate::service::dto::{SettingsSetInput, SettingsValues};

/// Screens with `view.<screen>.layout` and `view.<screen>.sort`.
pub const VIEW_SCREENS: &[&str] = &["hosts", "keychain", "forwarding", "snippets", "known_hosts"];

/// The auto-lock setting key.
pub const AUTO_LOCK_KEY: &str = "security.auto_lock_minutes";

/// The last session setting key (stage 2; docs/data-model.md, section 16.3).
pub const SESSION_LAST_KEY: &str = "session.last";

/// Limits of `session.last` (docs/data-model.md, section 12).
pub const SESSION_LAST_MAX_TABS: usize = 32;
pub const SESSION_LAST_MAX_LEAVES: usize = 16;
/// Split nesting: a leaf has depth 0, a split of leaves depth 1.
pub const SESSION_LAST_MAX_DEPTH: usize = 4;

/// Value types of settings.
#[derive(Clone, Debug)]
enum Kind {
    Enum(&'static [&'static str]),
    Bool,
    /// Finite number in a closed range.
    Number(f64, f64),
    /// Integer in a closed range.
    Int(i64, i64),
    /// A string of 1 to n characters without control characters.
    Text(usize),
    /// A string of at most n bytes, or `null`.
    NullableText(usize),
    /// `https://` URL of at most 2048 bytes, or `null`.
    NullableHttpsUrl,
    /// 1, 5, 15, 30, 60 or `null` (never). The app offers Never, 5, 15, 30
    /// and 60 (decision D29); 1 stays valid for tests and older values.
    AutoLock,
    ThemeRef,
    /// Object: action id -> shortcut string or `null`.
    ShortcutOverrides,
    /// List of hint ids.
    HintList,
    /// `session.last` (docs/data-model.md, section 16.3): `null` or the tabs
    /// of the last exit (stage 2).
    SessionLast,
}

#[derive(Clone, Debug)]
struct Def {
    kind: Kind,
    default: Value,
}

/// The settings registry and the stored rows.
#[derive(Clone, Debug)]
pub struct Settings {
    defs: BTreeMap<String, Def>,
    stored: BTreeMap<String, Value>,
}

/// A validated `settings_set`: rows to write and rows to delete.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsUpdate {
    pub puts: Vec<(String, Value)>,
    pub deletes: Vec<String>,
}

impl SettingsUpdate {
    /// `(key, compact JSON)` pairs for `local_settings`.
    pub fn put_rows(&self) -> Vec<(String, String)> {
        self.puts
            .iter()
            .map(|(key, value)| (key.clone(), value.to_string()))
            .collect()
    }
}

fn definitions(os: OsKind) -> BTreeMap<String, Def> {
    let mut defs: BTreeMap<String, Def> = BTreeMap::new();
    let mut add = |key: &str, kind: Kind, default: Value| {
        defs.insert(key.to_owned(), Def { kind, default });
    };
    add(
        "appearance.ui_theme",
        Kind::Enum(&["auto", "light", "dark"]),
        json!("auto"),
    );
    add("appearance.ui_zoom", Kind::Number(0.5, 2.0), json!(1.0));
    // Never by default (decision D29).
    add(AUTO_LOCK_KEY, Kind::AutoLock, Value::Null);
    add("terminal.autocomplete", Kind::Enum(&["off"]), json!("off"));
    add("terminal.autoreconnect", Kind::Bool, json!(true));
    add("terminal.record_history", Kind::Bool, json!(true));
    add("terminal.import_shell_history", Kind::Bool, json!(false));
    add("terminal.copy_on_select", Kind::Bool, json!(false));
    add("terminal.bell_sound", Kind::Bool, json!(false));
    add("terminal.bold_is_bright", Kind::Bool, json!(true));
    add(
        "terminal.cursor_style",
        Kind::Enum(&["block", "bar", "underline"]),
        json!("block"),
    );
    add("terminal.cursor_blink", Kind::Bool, json!(false));
    add(
        "terminal.term",
        Kind::Enum(&["xterm", "xterm-256color", "vt100", "linux"]),
        json!("xterm-256color"),
    );
    add(
        "terminal.font_family",
        Kind::Text(256),
        json!("JetBrains Mono"),
    );
    add("terminal.font_size", Kind::Int(6, 72), json!(14));
    add("terminal.line_height", Kind::Number(1.0, 2.0), json!(1.2));
    add("terminal.confirm_multiline_paste", Kind::Bool, json!(true));
    add(
        "terminal.default_theme",
        Kind::ThemeRef,
        json!({"builtin": "graphite"}),
    );
    add("terminal.highlighting_enabled", Kind::Bool, json!(true));
    add("terminal.os_detection_enabled", Kind::Bool, json!(true));
    add(
        "terminal.os_detection_script",
        Kind::NullableText(65_536),
        Value::Null,
    );
    add(
        "terminal.local_shell",
        Kind::NullableText(4096),
        Value::Null,
    );
    add("terminal.keepalive_s", Kind::Int(0, 3600), json!(30));
    add("sftp.show_hidden", Kind::Bool, json!(false));
    add("sftp.confirm_delete", Kind::Bool, json!(true));
    add(
        "sftp.name_conflict",
        Kind::Enum(&["ask", "replace", "skip", "rename"]),
        json!("ask"),
    );
    add("sftp.parallel_transfers", Kind::Int(1, 10), json!(3));
    add("sftp.open_program", Kind::NullableText(4096), Value::Null);
    add("logs.retention_days", Kind::Int(1, 3650), json!(90));
    add("logs.record_output", Kind::Bool, json!(false));
    add(
        "shortcuts.preset",
        Kind::Enum(&["pc", "mac"]),
        json!(if os == OsKind::Macos { "mac" } else { "pc" }),
    );
    add("shortcuts.overrides", Kind::ShortcutOverrides, json!({}));
    add("updates.auto_check", Kind::Bool, json!(false));
    add("updates.manifest_url", Kind::NullableHttpsUrl, Value::Null);
    for screen in VIEW_SCREENS {
        add(
            &format!("view.{screen}.layout"),
            Kind::Enum(&["grid", "list"]),
            json!("grid"),
        );
        add(
            &format!("view.{screen}.sort"),
            Kind::Enum(&["manual", "name", "created", "modified"]),
            json!(if *screen == "hosts" { "manual" } else { "name" }),
        );
    }
    add("view.details_panel_width", Kind::Int(320, 560), json!(380));
    add("hints.dismissed", Kind::HintList, json!([]));
    add(SESSION_LAST_KEY, Kind::SessionLast, Value::Null);
    defs
}

fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

fn check(kind: &Kind, value: &Value) -> Result<Value, ValidationReason> {
    use ValidationReason::{Invalid, OutOfRange, TooLong};
    match kind {
        Kind::Enum(allowed) => match value.as_str() {
            Some(text) if allowed.contains(&text) => Ok(value.clone()),
            _ => Err(Invalid),
        },
        Kind::Bool => value.as_bool().map(Value::Bool).ok_or(Invalid),
        Kind::Number(min, max) => {
            let number = value.as_f64().filter(|n| n.is_finite()).ok_or(Invalid)?;
            if number < *min || number > *max {
                return Err(OutOfRange);
            }
            Ok(value.clone())
        }
        Kind::Int(min, max) => {
            if !(value.is_i64() || value.is_u64()) {
                return Err(Invalid);
            }
            match value.as_i64() {
                Some(number) if (*min..=*max).contains(&number) => Ok(value.clone()),
                _ => Err(OutOfRange),
            }
        }
        Kind::Text(max) => {
            let text = value.as_str().ok_or(Invalid)?;
            if text.trim().is_empty() || has_control(text) {
                return Err(Invalid);
            }
            if text.chars().count() > *max {
                return Err(TooLong);
            }
            Ok(value.clone())
        }
        Kind::NullableText(max) => match value {
            Value::Null => Ok(Value::Null),
            Value::String(text) => {
                if text.contains('\0') {
                    Err(Invalid)
                } else if text.len() > *max {
                    Err(TooLong)
                } else {
                    Ok(value.clone())
                }
            }
            _ => Err(Invalid),
        },
        Kind::NullableHttpsUrl => match value {
            Value::Null => Ok(Value::Null),
            Value::String(text) => {
                if text.len() > 2048 {
                    Err(TooLong)
                } else if !text.starts_with("https://") || text.chars().any(char::is_whitespace) {
                    Err(Invalid)
                } else {
                    Ok(value.clone())
                }
            }
            _ => Err(Invalid),
        },
        Kind::AutoLock => match value {
            Value::Null => Ok(Value::Null),
            _ => match value.as_i64() {
                Some(1 | 5 | 15 | 30 | 60) if value.is_i64() || value.is_u64() => Ok(value.clone()),
                Some(_) => Err(OutOfRange),
                None => Err(Invalid),
            },
        },
        Kind::ThemeRef => theme_ref(value, true),
        Kind::ShortcutOverrides => {
            let object = value.as_object().ok_or(Invalid)?;
            if object.len() > 512 {
                return Err(TooLong);
            }
            let mut out = Map::new();
            for (action, shortcut) in object {
                if action.is_empty() || action.len() > 128 || has_control(action) {
                    return Err(Invalid);
                }
                match shortcut {
                    Value::Null => {
                        out.insert(action.clone(), Value::Null);
                    }
                    Value::String(text) if text.len() <= 64 && !has_control(text) => {
                        out.insert(action.clone(), shortcut.clone());
                    }
                    _ => return Err(Invalid),
                }
            }
            Ok(Value::Object(out))
        }
        Kind::HintList => {
            let items = value.as_array().ok_or(Invalid)?;
            if items.len() > 512 {
                return Err(TooLong);
            }
            for item in items {
                match item.as_str() {
                    Some(id) if !id.is_empty() && id.len() <= 128 && !has_control(id) => {}
                    _ => return Err(Invalid),
                }
            }
            Ok(value.clone())
        }
        Kind::SessionLast => session_last(value),
    }
}

/// `true` for an object whose members are all in `allowed`.
fn only_members(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

/// A `host_link`: `{"vault_id": uuid, "record_id": uuid}`.
fn host_link_ok(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let uuid = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|raw| uuid::Uuid::parse_str(raw).is_ok())
    };
    only_members(object, &["vault_id", "record_id"]) && uuid("vault_id") && uuid("record_id")
}

/// Checks a pane; returns its leaf count and split depth.
fn session_pane(value: &Value) -> Result<(usize, usize), ValidationReason> {
    use ValidationReason::Invalid;
    let object = value.as_object().ok_or(Invalid)?;
    if let Some(session) = object.get("session") {
        let session = session.as_object().ok_or(Invalid)?;
        if !only_members(object, &["session"]) || !only_members(session, &["kind", "host"]) {
            return Err(Invalid);
        }
        let host = session.get("host").filter(|host| !host.is_null());
        if host.is_some_and(|host| !host_link_ok(host)) {
            return Err(Invalid);
        }
        return match session.get("kind").and_then(Value::as_str) {
            Some("ssh") if host.is_some() => Ok((1, 0)),
            Some("local") => Ok((1, 0)),
            _ => Err(Invalid),
        };
    }
    if !only_members(object, &["direction", "sizes", "children"]) {
        return Err(Invalid);
    }
    match object.get("direction").and_then(Value::as_str) {
        Some("row" | "column") => {}
        _ => return Err(Invalid),
    }
    let sizes = object
        .get("sizes")
        .and_then(Value::as_array)
        .ok_or(Invalid)?;
    let children = object
        .get("children")
        .and_then(Value::as_array)
        .ok_or(Invalid)?;
    if !(2..=4).contains(&children.len()) || sizes.len() != children.len() {
        return Err(Invalid);
    }
    let mut total = 0.0;
    for size in sizes {
        match size.as_f64() {
            Some(size) if size.is_finite() && size > 0.0 => total += size,
            _ => return Err(Invalid),
        }
    }
    if (total - 1.0).abs() > 0.001 {
        return Err(Invalid);
    }
    let mut leaves = 0usize;
    let mut depth = 0usize;
    for child in children {
        let (child_leaves, child_depth) = session_pane(child)?;
        leaves += child_leaves;
        depth = depth.max(child_depth);
    }
    Ok((leaves, depth + 1))
}

/// `session.last`: `null`, or `{"saved_at": timestamp, "tabs": [{"root": pane}]}`
/// with at most 32 tabs, 16 leaves and split depth 4 per tab. An empty tab
/// list is allowed: it means nothing to restore (docs/data-model.md, 16.3).
fn session_last(value: &Value) -> Result<Value, ValidationReason> {
    use ValidationReason::{Invalid, TooMany};
    if value.is_null() {
        return Ok(Value::Null);
    }
    let object = value.as_object().ok_or(Invalid)?;
    if !only_members(object, &["saved_at", "tabs"]) {
        return Err(Invalid);
    }
    let saved_at = object
        .get("saved_at")
        .and_then(Value::as_str)
        .ok_or(Invalid)?;
    if time::OffsetDateTime::parse(saved_at, &time::format_description::well_known::Rfc3339)
        .is_err()
    {
        return Err(Invalid);
    }
    let tabs = object
        .get("tabs")
        .and_then(Value::as_array)
        .ok_or(Invalid)?;
    if tabs.len() > SESSION_LAST_MAX_TABS {
        return Err(TooMany);
    }
    for tab in tabs {
        let tab = tab.as_object().ok_or(Invalid)?;
        if !only_members(tab, &["root"]) {
            return Err(Invalid);
        }
        let (leaves, depth) = session_pane(tab.get("root").ok_or(Invalid)?)?;
        if leaves > SESSION_LAST_MAX_LEAVES || depth > SESSION_LAST_MAX_DEPTH {
            return Err(Invalid);
        }
    }
    Ok(value.clone())
}

impl Settings {
    /// The registry for `os` with the stored `(key, JSON)` rows.
    pub fn new(os: OsKind, rows: &[(String, String)]) -> Self {
        let stored = rows
            .iter()
            .filter_map(|(key, json)| {
                serde_json::from_str::<Value>(json)
                    .ok()
                    .map(|value| (key.clone(), value))
            })
            .collect();
        Self {
            defs: definitions(os),
            stored,
        }
    }

    /// Every known key.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.defs.keys().map(String::as_str)
    }

    /// The effective value of a known key.
    pub fn get(&self, key: &str) -> Option<Value> {
        let def = self.defs.get(key)?;
        let value = self
            .stored
            .get(key)
            .and_then(|value| check(&def.kind, value).ok())
            .unwrap_or_else(|| def.default.clone());
        Some(value)
    }

    /// Every known key with its effective value.
    pub fn values(&self) -> SettingsValues {
        self.defs
            .keys()
            .filter_map(|key| self.get(key).map(|value| (key.clone(), value)))
            .collect()
    }

    /// The auto-lock interval; `None` means never.
    pub fn auto_lock_interval(&self) -> Option<Duration> {
        self.get(AUTO_LOCK_KEY)
            .and_then(|value| value.as_u64())
            .map(|minutes| Duration::from_secs(minutes * 60))
    }

    /// Validates a `settings_set` call: every value first, nothing is written
    /// on error (`validation_failed`, field = the key).
    pub fn prepare(&self, input: &SettingsSetInput) -> Result<SettingsUpdate, CoreError> {
        let mut errors: Vec<FieldError> = Vec::new();
        let mut update = SettingsUpdate::default();
        for (key, value) in &input.values {
            if input.reset.contains(key) {
                continue;
            }
            match self.defs.get(key) {
                None => errors.push(FieldError::new(key.clone(), ValidationReason::Invalid)),
                Some(def) => match check(&def.kind, value) {
                    Ok(value) => update.puts.push((key.clone(), value)),
                    Err(reason) => errors.push(FieldError::new(key.clone(), reason)),
                },
            }
        }
        for key in &input.reset {
            if self.defs.contains_key(key) {
                if !update.deletes.contains(key) {
                    update.deletes.push(key.clone());
                }
            } else {
                errors.push(FieldError::new(key.clone(), ValidationReason::Invalid));
            }
        }
        if errors.is_empty() {
            Ok(update)
        } else {
            Err(CoreError::Validation(errors))
        }
    }

    /// Applies a committed update; returns the keys whose effective value changed.
    pub fn apply(&mut self, update: &SettingsUpdate) -> Vec<String> {
        let touched: Vec<String> = update
            .puts
            .iter()
            .map(|(key, _)| key.clone())
            .chain(update.deletes.iter().cloned())
            .collect();
        let before: Vec<Option<Value>> = touched.iter().map(|key| self.get(key)).collect();
        for (key, value) in &update.puts {
            self.stored.insert(key.clone(), value.clone());
        }
        for key in &update.deletes {
            self.stored.remove(key);
        }
        let mut changed: Vec<String> = touched
            .iter()
            .zip(before)
            .filter(|(key, before)| self.get(key) != *before)
            .map(|(key, _)| key.clone())
            .collect();
        changed.sort();
        changed.dedup();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(values: Value, reset: &[&str]) -> SettingsSetInput {
        SettingsSetInput {
            values: values
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            reset: reset.iter().map(|key| (*key).to_owned()).collect(),
        }
    }

    fn reasons(error: CoreError) -> Vec<(String, ValidationReason)> {
        match error {
            CoreError::Validation(errors) => {
                errors.into_iter().map(|e| (e.field, e.reason)).collect()
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn defaults() {
        let settings = Settings::new(OsKind::Windows, &[]);
        let values = settings.values();
        assert_eq!(values["appearance.ui_theme"], json!("auto"));
        assert_eq!(values["appearance.ui_zoom"], json!(1.0));
        assert_eq!(values[AUTO_LOCK_KEY], Value::Null);
        assert_eq!(values["view.hosts.sort"], json!("manual"));
        assert_eq!(values["view.keychain.sort"], json!("name"));
        assert_eq!(values["view.hosts.layout"], json!("grid"));
        assert_eq!(values["view.details_panel_width"], json!(380));
        assert_eq!(values["terminal.keepalive_s"], json!(30));
        assert_eq!(values["terminal.import_shell_history"], json!(false));
        assert_eq!(values["logs.record_output"], json!(false));
        assert_eq!(values["shortcuts.preset"], json!("pc"));
        assert_eq!(
            values["terminal.default_theme"],
            json!({"builtin": "graphite"})
        );
        assert_eq!(values["terminal.local_shell"], Value::Null);
        assert_eq!(
            Settings::new(OsKind::Macos, &[]).get("shortcuts.preset"),
            Some(json!("mac"))
        );
        assert_eq!(settings.auto_lock_interval(), None);
    }

    #[test]
    fn stored_rows_and_invalid_rows() {
        let rows = vec![
            (AUTO_LOCK_KEY.to_owned(), "30".to_owned()),
            ("appearance.ui_theme".to_owned(), "\"neon\"".to_owned()),
            ("view.hosts.layout".to_owned(), "\"list\"".to_owned()),
            ("unknown.key".to_owned(), "1".to_owned()),
        ];
        let settings = Settings::new(OsKind::Linux, &rows);
        assert_eq!(settings.get(AUTO_LOCK_KEY), Some(json!(30)));
        assert_eq!(
            settings.auto_lock_interval(),
            Some(Duration::from_secs(1800))
        );
        assert_eq!(settings.get("appearance.ui_theme"), Some(json!("auto")));
        assert_eq!(settings.get("view.hosts.layout"), Some(json!("list")));
        assert_eq!(settings.get("unknown.key"), None);
        assert!(!settings.values().contains_key("unknown.key"));
    }

    #[test]
    fn validation() {
        let settings = Settings::new(OsKind::Windows, &[]);
        let error = settings
            .prepare(&input(
                json!({
                    "no.such.key": 1,
                    AUTO_LOCK_KEY: 7,
                    "appearance.ui_zoom": 3.0,
                    "appearance.ui_theme": 1,
                    "view.details_panel_width": 380.5,
                    "terminal.font_size": 100,
                    "terminal.default_theme": {"builtin": "neon"},
                    "updates.manifest_url": "http://example.com/m.json",
                    "hints.dismissed": [1]
                }),
                &["also.unknown"],
            ))
            .unwrap_err();
        assert_eq!(
            reasons(error),
            vec![
                ("appearance.ui_theme".into(), ValidationReason::Invalid),
                ("appearance.ui_zoom".into(), ValidationReason::OutOfRange),
                ("hints.dismissed".into(), ValidationReason::Invalid),
                ("no.such.key".into(), ValidationReason::Invalid),
                (AUTO_LOCK_KEY.into(), ValidationReason::OutOfRange),
                ("terminal.default_theme".into(), ValidationReason::Invalid),
                ("terminal.font_size".into(), ValidationReason::OutOfRange),
                ("updates.manifest_url".into(), ValidationReason::Invalid),
                ("view.details_panel_width".into(), ValidationReason::Invalid),
                ("also.unknown".into(), ValidationReason::Invalid),
            ]
        );
    }

    #[test]
    fn apply_reports_effective_changes() {
        let mut settings = Settings::new(OsKind::Windows, &[]);
        let update = settings
            .prepare(&input(
                json!({AUTO_LOCK_KEY: 5, "view.hosts.layout": "grid", "appearance.ui_zoom": 1.1}),
                &[],
            ))
            .unwrap();
        assert_eq!(update.puts.len(), 3);
        let changed = settings.apply(&update);
        // "grid" is the default, so its effective value did not change.
        assert_eq!(
            changed,
            vec!["appearance.ui_zoom".to_owned(), AUTO_LOCK_KEY.to_owned()]
        );
        assert_eq!(
            settings.auto_lock_interval(),
            Some(Duration::from_secs(300))
        );

        let reset = settings
            .prepare(&input(json!({}), &[AUTO_LOCK_KEY]))
            .unwrap();
        assert_eq!(reset.deletes, vec![AUTO_LOCK_KEY.to_owned()]);
        assert_eq!(settings.apply(&reset), vec![AUTO_LOCK_KEY.to_owned()]);
        assert_eq!(settings.get(AUTO_LOCK_KEY), Some(Value::Null));
        let rows = update.put_rows();
        assert!(rows.contains(&(AUTO_LOCK_KEY.to_owned(), "5".to_owned())));
    }

    #[test]
    fn reset_wins_over_a_value_for_the_same_key() {
        let settings = Settings::new(OsKind::Windows, &[]);
        let update = settings
            .prepare(&input(json!({AUTO_LOCK_KEY: 5}), &[AUTO_LOCK_KEY]))
            .unwrap();
        assert!(update.puts.is_empty());
        assert_eq!(update.deletes, vec![AUTO_LOCK_KEY.to_owned()]);
    }

    #[test]
    fn session_last_values() {
        let settings = Settings::new(OsKind::Windows, &[]);
        assert_eq!(settings.get(SESSION_LAST_KEY), Some(Value::Null));
        let host = json!({"vault_id": "0199f6a1-0000-7000-8000-0000000000a1",
            "record_id": "0199f6a1-0000-7000-8000-0000000000b1"});
        let ssh = json!({"session": {"kind": "ssh", "host": host}});
        let local = json!({"session": {"kind": "local"}});
        let split = |children: Vec<Value>| {
            let n = children.len() as f64;
            json!({"direction": "row", "sizes": vec![1.0 / n; children.len()], "children": children})
        };
        let value = |tabs: Vec<Value>| {
            json!({"saved_at": "2026-10-18T22:31:05.123Z",
                "tabs": tabs.into_iter().map(|root| json!({"root": root})).collect::<Vec<_>>()})
        };
        let ok = |v: Value| {
            settings
                .prepare(&input(json!({ SESSION_LAST_KEY: v }), &[]))
                .map(|update| update.puts.len())
        };
        let refused = |v: Value| match settings.prepare(&input(json!({ SESSION_LAST_KEY: v }), &[]))
        {
            Err(error) => reasons(error)[0].1,
            Ok(_) => panic!("accepted"),
        };
        assert_eq!(ok(Value::Null).unwrap(), 1);
        assert_eq!(ok(value(vec![])).unwrap(), 1);
        assert_eq!(ok(value(vec![ssh.clone(), local.clone()])).unwrap(), 1);
        assert_eq!(
            ok(value(vec![split(vec![
                ssh.clone(),
                local.clone(),
                local.clone()
            ])]))
            .unwrap(),
            1
        );
        // Sizes within 0.001 of 1.
        let near = json!({"direction": "column", "sizes": [0.3333, 0.3333, 0.3333],
            "children": [local, local, local]});
        assert!(ok(value(vec![near])).is_ok());
        // 16 leaves at depth 2 are allowed; 17 are not.
        let four = split(vec![
            local.clone(),
            local.clone(),
            local.clone(),
            local.clone(),
        ]);
        let sixteen = split(vec![four.clone(), four.clone(), four.clone(), four.clone()]);
        assert!(ok(value(vec![sixteen.clone()])).is_ok());
        let seventeen = split(vec![sixteen.clone(), local.clone()]);
        assert_eq!(refused(value(vec![seventeen])), ValidationReason::Invalid);
        // Depth 4 is allowed, depth 5 is not.
        let mut deep = local.clone();
        for _ in 0..4 {
            deep = split(vec![deep, local.clone()]);
        }
        assert!(ok(value(vec![deep.clone()])).is_ok());
        assert_eq!(
            refused(value(vec![split(vec![deep, local.clone()])])),
            ValidationReason::Invalid
        );
        assert_eq!(
            refused(value(vec![local.clone(); 33])),
            ValidationReason::TooMany
        );
        for bad in [
            json!("x"),
            json!({"tabs": []}),
            json!({"saved_at": "yesterday", "tabs": []}),
            json!({"saved_at": "2026-10-18T22:31:05.123Z"}),
            json!({"saved_at": "2026-10-18T22:31:05.123Z", "tabs": [], "extra": 1}),
            value(vec![json!({"session": {"kind": "ssh"}})]),
            value(vec![json!({"session": {"kind": "sftp", "host": host}})]),
            value(vec![
                json!({"session": {"kind": "ssh", "host": {"vault_id": "x", "record_id": "y"}}}),
            ]),
            value(vec![json!({"session": {"kind": "local"}, "title": "db"})]),
            value(vec![
                json!({"session": {"kind": "local", "address": "203.0.113.1"}}),
            ]),
            value(vec![
                json!({"direction": "row", "sizes": [1.0], "children": [local]}),
            ]),
            value(vec![
                json!({"direction": "diagonal", "sizes": [0.5, 0.5], "children": [local, local]}),
            ]),
            value(vec![
                json!({"direction": "row", "sizes": [0.5, 0.6], "children": [local, local]}),
            ]),
            value(vec![
                json!({"direction": "row", "sizes": [1.0, 0.0], "children": [local, local]}),
            ]),
            value(vec![
                json!({"direction": "row", "sizes": [0.5], "children": [local, local]}),
            ]),
            json!({"saved_at": "2026-10-18T22:31:05.123Z", "tabs": [{"root": local, "title": "x"}]}),
        ] {
            assert_eq!(refused(bad.clone()), ValidationReason::Invalid, "{bad}");
        }
    }

    #[test]
    fn auto_lock_values() {
        let settings = Settings::new(OsKind::Windows, &[]);
        for minutes in [1, 5, 15, 30, 60] {
            assert!(
                settings
                    .prepare(&input(json!({AUTO_LOCK_KEY: minutes}), &[]))
                    .is_ok()
            );
        }
        assert!(
            settings
                .prepare(&input(json!({AUTO_LOCK_KEY: null}), &[]))
                .is_ok()
        );
        assert!(
            settings
                .prepare(&input(json!({AUTO_LOCK_KEY: "15"}), &[]))
                .is_err()
        );
        assert!(
            settings
                .prepare(&input(json!({AUTO_LOCK_KEY: 15.0}), &[]))
                .is_err()
        );
    }
}
