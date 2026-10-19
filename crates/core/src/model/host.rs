//! The `host` record (docs/data-model.md, sections 7.2-7.5; stage 1 plan 5.5).
//!
//! [`build`] turns the webview's data object into the data that is stored:
//! secret paths inside `data` are refused, every known member is validated
//! and normalized (all errors together), unknown members of the input and of
//! the stored data are kept, the managed members `order_key`, `import_key`
//! and `os` keep their stored values, and `secrets` are applied.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{
    self, Errors, TAG_COLORS, opt_bool, opt_enum, opt_int, opt_object, opt_string, opt_text,
    opt_uuid, opt_uuid_list, present,
};
use crate::model::merge::{keep_from, merge_level, object};
use crate::model::text;
use crate::model::wipe::WipeOnDrop;

/// Dotted paths of the secret members of a host.
pub const SECRET_PATHS: &[&str] = &["ssh.password", "ssh.proxy.password"];

/// Members only the core writes; input values are ignored.
pub const MANAGED: &[&str] = &["order_key", "import_key", "os"];

const TOP_KNOWN: &[&str] = &[
    "address",
    "label",
    "group_id",
    "tags",
    "notes",
    "color",
    "backspace",
    "charset",
    "terminal_theme",
    "record_history",
    "record_output",
    "primary_protocol",
    "ssh",
    "telnet",
    "order_key",
    "import_key",
    "os",
];

const SSH_KNOWN: &[&str] = &[
    "port",
    "username",
    "password",
    "identity_id",
    "key_id",
    "certificate_key_id",
    "fido2_key_id",
    "jump_chain",
    "proxy",
    "agent_forwarding",
    "startup_snippet_id",
    "env",
    "keepalive_s",
    "mosh",
];

const PROXY_KNOWN: &[&str] = &["type", "address", "port", "username", "password"];

/// Limits (docs/data-model.md, sections 3.5 and 7).
pub const MAX_TAGS: usize = 32;
pub const MAX_JUMP_HOSTS: usize = 8;
pub const MAX_ENV_VARS: usize = 64;
pub const MAX_SECRET_BYTES: usize = 4096;
pub const MAX_ENV_VALUE_BYTES: usize = 1024;
pub const MAX_NOTES_BYTES: usize = 32_768;

/// Context of a host save.
#[derive(Clone, Copy, Debug)]
pub struct HostRules {
    /// The record id when updating (a host may not be its own jump host).
    pub record_id: Option<Uuid>,
    /// `terminal_theme.record_id` is allowed only in the personal vault.
    pub personal_vault: bool,
}

/// Validates `input` and builds the data to store (without `order_key` for a
/// new record: the caller assigns it).
pub fn build(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
    secrets: &BTreeMap<String, Option<SecretString>>,
    rules: &HostRules,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    refuse_secret_paths(input, &mut errors);
    let normalized = normalize(input, rules, &mut errors);
    check_secrets(secrets, &mut errors);
    let Some(normalized) = normalized.filter(|_| errors.is_empty()) else {
        return Err(errors.into_vec());
    };

    // Holds the stored secrets: wiped when the secrets are refused.
    let mut data = WipeOnDrop::new(merge(normalized, input, stored));
    let mut errors = Errors::new();
    apply_secrets(&mut data, secrets, &mut errors);
    errors.finish(()).map(|()| data.into_inner())
}

/// `true` when `data` alone (a draft) would be accepted as a new host.
pub fn is_valid_draft(input: &Map<String, Value>, rules: &HostRules) -> bool {
    let mut errors = Errors::new();
    refuse_secret_paths(input, &mut errors);
    normalize(input, rules, &mut errors).is_some() && errors.is_empty()
}

fn refuse_secret_paths(input: &Map<String, Value>, errors: &mut Errors) {
    let ssh = object(Some(input), "ssh");
    if ssh.and_then(|ssh| present(ssh, "password")).is_some() {
        errors.push("ssh.password", ValidationReason::NotAllowed);
    }
    if object(ssh, "proxy")
        .and_then(|proxy| present(proxy, "password"))
        .is_some()
    {
        errors.push("ssh.proxy.password", ValidationReason::NotAllowed);
    }
}

/// The normalized known members, or `None` when a member was refused.
fn normalize(
    input: &Map<String, Value>,
    rules: &HostRules,
    errors: &mut Errors,
) -> Option<Map<String, Value>> {
    let before = errors.is_empty();
    let mut out = Map::new();

    match present(input, "address") {
        None => errors.push("address", ValidationReason::Required),
        Some(value) => match value.as_str().map(fields::address) {
            None => errors.push("address", ValidationReason::Invalid),
            Some(Ok(Some(address))) => {
                out.insert("address".into(), Value::String(address));
            }
            Some(Ok(None)) => errors.push("address", ValidationReason::Required),
            Some(Err(reason)) => errors.push("address", reason),
        },
    }
    if let Some(label) = opt_string(input, "label", "label", 256, errors) {
        out.insert("label".into(), Value::String(label));
    }
    if let Some(group_id) = opt_uuid(input, "group_id", "group_id", errors) {
        out.insert("group_id".into(), json!(group_id));
    }
    if let Some(tags) = tags(input, errors) {
        out.insert("tags".into(), tags);
    }
    if let Some(notes) = opt_text(input, "notes", "notes", MAX_NOTES_BYTES, errors) {
        out.insert("notes".into(), Value::String(notes));
    }
    if let Some(color) = opt_enum(input, "color", "color", TAG_COLORS, errors) {
        out.insert("color".into(), json!(color));
    }
    if let Some(backspace) = opt_enum(
        input,
        "backspace",
        "backspace",
        &["default", "del", "ctrl_h"],
        errors,
    ) {
        out.insert("backspace".into(), json!(backspace));
    }
    if let Some(charset) = charset(input, errors) {
        out.insert("charset".into(), Value::String(charset));
    }
    if let Some(theme) = present(input, "terminal_theme") {
        match fields::theme_ref(theme, rules.personal_vault) {
            Ok(theme) => {
                out.insert("terminal_theme".into(), theme);
            }
            Err(reason) => errors.push("terminal_theme", reason),
        }
    }
    for member in ["record_history", "record_output"] {
        if let Some(mode) = opt_enum(input, member, member, &["default", "on", "off"], errors) {
            out.insert(member.into(), json!(mode));
        }
    }
    let telnet = opt_object(input, "telnet", "telnet", errors).map(|telnet| {
        let mut telnet = telnet.clone();
        telnet.retain(|_, value| !value.is_null());
        if let Some(port) = opt_int(&telnet, "port", "telnet.port", 1, 65535, errors) {
            telnet.insert("port".into(), json!(port));
        }
        // The login name typed at the Telnet prompt (UI 3.1 "Add Telnet"):
        // `string(256)?`, not inherited.
        match opt_string(&telnet, "username", "telnet.username", 256, errors) {
            Some(username) => {
                telnet.insert("username".into(), Value::String(username));
            }
            None => {
                telnet.remove("username");
            }
        }
        telnet
    });
    if let Some(protocol) = opt_enum(
        input,
        "primary_protocol",
        "primary_protocol",
        &["ssh", "telnet"],
        errors,
    ) {
        if protocol == "telnet" && telnet.is_none() {
            errors.push("primary_protocol", ValidationReason::Invalid);
        } else {
            out.insert("primary_protocol".into(), json!(protocol));
        }
    }
    if let Some(telnet) = telnet {
        out.insert("telnet".into(), Value::Object(telnet));
    }
    let ssh_input = opt_object(input, "ssh", "ssh", errors);
    let ssh = normalize_ssh(ssh_input.unwrap_or(&Map::new()), rules, errors);
    out.insert("ssh".into(), Value::Object(ssh));

    (before && errors.is_empty()).then_some(out)
}

fn tags(input: &Map<String, Value>, errors: &mut Errors) -> Option<Value> {
    let value = present(input, "tags")?;
    let Some(items) = value.as_array() else {
        errors.push("tags", ValidationReason::Invalid);
        return None;
    };
    let mut seen: Vec<String> = Vec::new();
    let mut tags: Vec<Value> = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let field = format!("tags.{index}");
        match item.as_str().map(text::tag) {
            Some(Ok(tag)) => {
                let folded = text::fold(&tag);
                if !seen.contains(&folded) {
                    seen.push(folded);
                    tags.push(Value::String(tag));
                }
            }
            Some(Err(reason)) => {
                errors.push(field, reason);
                ok = false;
            }
            None => {
                errors.push(field, ValidationReason::Invalid);
                ok = false;
            }
        }
    }
    if tags.len() > MAX_TAGS {
        errors.push("tags", ValidationReason::TooMany);
        return None;
    }
    ok.then_some(Value::Array(tags))
}

fn charset(input: &Map<String, Value>, errors: &mut Errors) -> Option<String> {
    let value = present(input, "charset")?;
    let Some(raw) = value.as_str() else {
        errors.push("charset", ValidationReason::Invalid);
        return None;
    };
    match text::string(raw, 64) {
        Ok(Some(label)) => {
            let label = label.to_lowercase();
            if label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:-".contains(&b))
            {
                Some(label)
            } else {
                errors.push("charset", ValidationReason::Invalid);
                None
            }
        }
        Ok(None) => None,
        Err(reason) => {
            errors.push("charset", reason);
            None
        }
    }
}

fn normalize_ssh(
    ssh: &Map<String, Value>,
    rules: &HostRules,
    errors: &mut Errors,
) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(port) = opt_int(ssh, "port", "ssh.port", 1, 65535, errors) {
        out.insert("port".into(), json!(port));
    }
    if let Some(username) = opt_string(ssh, "username", "ssh.username", 256, errors) {
        out.insert("username".into(), Value::String(username));
    }
    for member in [
        "identity_id",
        "key_id",
        "certificate_key_id",
        "fido2_key_id",
        "startup_snippet_id",
    ] {
        let field = format!("ssh.{member}");
        if let Some(id) = opt_uuid(ssh, member, &field, errors) {
            out.insert(member.into(), json!(id));
        }
    }
    if let Some(chain) = opt_uuid_list(
        ssh,
        "jump_chain",
        "ssh.jump_chain",
        MAX_JUMP_HOSTS,
        rules.record_id,
        errors,
    ) {
        out.insert("jump_chain".into(), json!(chain));
    }
    if let Some(proxy) = opt_object(ssh, "proxy", "ssh.proxy", errors)
        && let Some(proxy) = normalize_proxy(proxy, errors)
    {
        out.insert("proxy".into(), Value::Object(proxy));
    }
    for member in ["agent_forwarding", "mosh"] {
        let field = format!("ssh.{member}");
        if let Some(flag) = opt_bool(ssh, member, &field, errors) {
            out.insert(member.into(), Value::Bool(flag));
        }
    }
    if let Some(env) = env(ssh, errors) {
        out.insert("env".into(), env);
    }
    if let Some(keepalive) = opt_int(ssh, "keepalive_s", "ssh.keepalive_s", 0, 3600, errors) {
        out.insert("keepalive_s".into(), json!(keepalive));
    }
    out
}

/// `ssh.proxy` (stage 5 shape; validated if present). Every problem is `invalid`.
fn normalize_proxy(proxy: &Map<String, Value>, errors: &mut Errors) -> Option<Map<String, Value>> {
    let mut out = Map::new();
    let mut ok = true;
    match present(proxy, "type").and_then(Value::as_str) {
        Some(kind @ ("http" | "socks5")) => {
            out.insert("type".into(), json!(kind));
        }
        _ => {
            errors.push("ssh.proxy.type", ValidationReason::Invalid);
            ok = false;
        }
    }
    match present(proxy, "address")
        .and_then(Value::as_str)
        .map(fields::address)
    {
        Some(Ok(Some(address))) => {
            out.insert("address".into(), Value::String(address));
        }
        _ => {
            errors.push("ssh.proxy.address", ValidationReason::Invalid);
            ok = false;
        }
    }
    match present(proxy, "port").and_then(Value::as_i64) {
        Some(port) if (1..=65535).contains(&port) => {
            out.insert("port".into(), json!(port));
        }
        _ => {
            errors.push("ssh.proxy.port", ValidationReason::Invalid);
            ok = false;
        }
    }
    if let Some(username) = opt_string(proxy, "username", "ssh.proxy.username", 256, errors) {
        out.insert("username".into(), Value::String(username));
    }
    ok.then_some(out)
}

fn env_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && name.len() <= 256
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn env(ssh: &Map<String, Value>, errors: &mut Errors) -> Option<Value> {
    let value = present(ssh, "env")?;
    let Some(items) = value.as_array() else {
        errors.push("ssh.env", ValidationReason::Invalid);
        return None;
    };
    if items.len() > MAX_ENV_VARS {
        errors.push("ssh.env", ValidationReason::TooMany);
        return None;
    }
    let mut ok = true;
    let mut names: Vec<&str> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(item) = item.as_object() else {
            errors.push(format!("ssh.env.{index}"), ValidationReason::Invalid);
            ok = false;
            continue;
        };
        let name = item.get("name").and_then(Value::as_str);
        match name {
            Some(name) if env_name_ok(name) => {
                if names.contains(&name) {
                    errors.push(format!("ssh.env.{index}.name"), ValidationReason::Duplicate);
                    ok = false;
                } else {
                    names.push(name);
                }
            }
            _ => {
                errors.push(format!("ssh.env.{index}.name"), ValidationReason::Invalid);
                ok = false;
            }
        }
        let value = item.get("value").and_then(Value::as_str);
        match value.map(|value| text::secret(value, MAX_ENV_VALUE_BYTES)) {
            Some(Ok(())) => {}
            Some(Err(reason)) => {
                errors.push(format!("ssh.env.{index}.value"), reason);
                ok = false;
            }
            None => {
                errors.push(format!("ssh.env.{index}.value"), ValidationReason::Invalid);
                ok = false;
            }
        }
        if let (Some(name), Some(value)) = (name, value) {
            out.push(json!({ "name": name, "value": value }));
        }
    }
    ok.then_some(Value::Array(out))
}

fn check_secrets(secrets: &BTreeMap<String, Option<SecretString>>, errors: &mut Errors) {
    for (path, value) in secrets {
        if !SECRET_PATHS.contains(&path.as_str()) {
            errors.push(path.clone(), ValidationReason::NotAllowed);
            continue;
        }
        if let Some(value) = value
            && let Err(reason) = text::secret(value.expose_secret(), MAX_SECRET_BYTES)
        {
            errors.push(path.clone(), reason);
        }
    }
}

/// Assembles the stored data: normalized members, unknown members, managed
/// members and stored secrets.
fn merge(
    mut normalized: Map<String, Value>,
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let input_ssh = object(Some(input), "ssh");
    let stored_ssh = object(stored, "ssh");
    let ssh_normalized = match normalized.remove("ssh") {
        Some(Value::Object(ssh)) => ssh,
        _ => Map::new(),
    };
    let mut ssh_normalized = ssh_normalized;
    let proxy_normalized = match ssh_normalized.remove("proxy") {
        Some(Value::Object(proxy)) => Some(proxy),
        _ => None,
    };

    let mut ssh = merge_level(SSH_KNOWN, ssh_normalized, input_ssh, stored_ssh);
    keep_from(&mut ssh, stored_ssh, &["password"]);
    if let Some(proxy_normalized) = proxy_normalized {
        let input_proxy = object(input_ssh, "proxy");
        let stored_proxy = object(stored_ssh, "proxy");
        let mut proxy = merge_level(PROXY_KNOWN, proxy_normalized, input_proxy, stored_proxy);
        keep_from(&mut proxy, stored_proxy, &["password"]);
        ssh.insert("proxy".into(), Value::Object(proxy));
    }

    let mut data = merge_level(TOP_KNOWN, normalized, Some(input), stored);
    keep_from(&mut data, stored, MANAGED);
    data.insert("ssh".into(), Value::Object(ssh));
    data
}

/// Applies `secrets`: a string sets the value (an empty string clears it),
/// `null` clears it, a missing path keeps the stored value.
fn apply_secrets(
    data: &mut Map<String, Value>,
    secrets: &BTreeMap<String, Option<SecretString>>,
    errors: &mut Errors,
) {
    for (path, value) in secrets {
        let value = value
            .as_ref()
            .map(|value| value.expose_secret())
            .filter(|value| !value.is_empty());
        let target = match path.as_str() {
            "ssh.password" => data
                .entry("ssh")
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut(),
            "ssh.proxy.password" => {
                let proxy = data
                    .get_mut("ssh")
                    .and_then(Value::as_object_mut)
                    .and_then(|ssh| ssh.get_mut("proxy"))
                    .and_then(Value::as_object_mut);
                if proxy.is_none() && value.is_some() {
                    errors.push(path.clone(), ValidationReason::NotAllowed);
                }
                proxy
            }
            _ => None,
        };
        let Some(target) = target else {
            continue;
        };
        match value {
            Some(value) => {
                target.insert("password".into(), Value::String(value.to_owned()));
            }
            None => {
                if let Some(mut old) = target.remove("password") {
                    crate::model::wipe::wipe_value(&mut old);
                }
            }
        }
    }
}

/// The data object without secret members, and the paths of secret members
/// that hold a value (`HostView.data`, `HostView.secrets_set`).
pub fn public_view(data: &Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    let mut public = data.clone();
    let mut set = Vec::new();
    if let Some(ssh) = public.get_mut("ssh").and_then(Value::as_object_mut) {
        if let Some(mut password) = ssh.remove("password") {
            if password.as_str().is_some_and(|p| !p.is_empty()) {
                set.push("ssh.password".to_owned());
            }
            crate::model::wipe::wipe_value(&mut password);
        }
        if let Some(proxy) = ssh.get_mut("proxy").and_then(Value::as_object_mut)
            && let Some(mut password) = proxy.remove("password")
        {
            if password.as_str().is_some_and(|p| !p.is_empty()) {
                set.push("ssh.proxy.password".to_owned());
            }
            crate::model::wipe::wipe_value(&mut password);
        }
    }
    (public, set)
}

/// The value of a secret path, if set.
pub fn secret_value<'a>(data: &'a Map<String, Value>, path: &str) -> Option<&'a str> {
    let ssh = object(Some(data), "ssh");
    let value = match path {
        "ssh.password" => ssh.and_then(|ssh| ssh.get("password")),
        "ssh.proxy.password" => object(ssh, "proxy").and_then(|proxy| proxy.get("password")),
        _ => None,
    };
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// The title shown on cards and used by the Name sort: label, else address.
pub fn title(data: &Map<String, Value>) -> &str {
    data.get("label")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
        .or_else(|| data.get("address").and_then(Value::as_str))
        .unwrap_or("")
}

/// The raw `group_id` member.
pub fn group_id(data: &Map<String, Value>) -> Option<Uuid> {
    data.get("group_id")
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationReason::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn rules() -> HostRules {
        HostRules {
            record_id: None,
            personal_vault: true,
        }
    }

    fn errors_of(
        result: Result<Map<String, Value>, Vec<FieldError>>,
    ) -> Vec<(String, ValidationReason)> {
        result
            .unwrap_err()
            .into_iter()
            .map(|error| (error.field, error.reason))
            .collect()
    }

    fn no_secrets() -> BTreeMap<String, Option<SecretString>> {
        BTreeMap::new()
    }

    #[test]
    fn minimal_host_is_normalized() {
        let data = build(
            &map(json!({"address": " [2001:db8::5] "})),
            None,
            &no_secrets(),
            &rules(),
        )
        .unwrap();
        assert_eq!(
            Value::Object(data),
            json!({"address": "2001:db8::5", "ssh": {}})
        );
    }

    #[test]
    fn draft_rule_requires_a_valid_address() {
        assert!(!is_valid_draft(&map(json!({})), &rules()));
        assert!(!is_valid_draft(&map(json!({"address": "  "})), &rules()));
        assert!(!is_valid_draft(&map(json!({"address": "a b"})), &rules()));
        assert!(!is_valid_draft(
            &map(json!({"address": "ok.example.com", "ssh": {"port": 0}})),
            &rules()
        ));
        assert!(is_valid_draft(
            &map(json!({"address": "ok.example.com"})),
            &rules()
        ));
        assert_eq!(
            errors_of(build(
                &map(json!({"label": "x"})),
                None,
                &no_secrets(),
                &rules()
            )),
            vec![("address".into(), Required)]
        );
    }

    #[test]
    fn validation_table() {
        let id = Uuid::now_v7();
        let other = Uuid::now_v7();
        let input = map(json!({
            "address": "bad host",
            "label": "x".repeat(257),
            "group_id": "not-a-uuid",
            "tags": ["ok", "a,b", "", 5],
            "notes": "a\u{1}b",
            "color": "purple",
            "backspace": "bs",
            "charset": "UTF 8",
            "terminal_theme": {"builtin": "neon"},
            "record_history": "maybe",
            "record_output": true,
            "primary_protocol": "telnet",
            "ssh": {
                "port": 70000,
                "username": "u\u{7}",
                "identity_id": 5,
                "jump_chain": [id.to_string()],
                "proxy": {"type": "ftp", "address": "proxy.example.com", "port": 8080},
                "agent_forwarding": "yes",
                "env": [
                    {"name": "LANG", "value": "C"},
                    {"name": "LANG", "value": "C"},
                    {"name": "1BAD", "value": "x"},
                    {"name": "BIG", "value": "v".repeat(1025)},
                    "nope"
                ],
                "keepalive_s": -1
            }
        }));
        let rules = HostRules {
            record_id: Some(id),
            personal_vault: true,
        };
        let errors = errors_of(build(&input, None, &no_secrets(), &rules));
        let expected: Vec<(String, ValidationReason)> = vec![
            ("address".into(), Invalid),
            ("label".into(), TooLong),
            ("group_id".into(), Invalid),
            ("tags.1".into(), Invalid),
            ("tags.2".into(), Invalid),
            ("tags.3".into(), Invalid),
            ("notes".into(), Invalid),
            ("color".into(), Invalid),
            ("backspace".into(), Invalid),
            ("charset".into(), Invalid),
            ("terminal_theme".into(), Invalid),
            ("record_history".into(), Invalid),
            ("record_output".into(), Invalid),
            ("primary_protocol".into(), Invalid),
            ("ssh.port".into(), OutOfRange),
            ("ssh.username".into(), Invalid),
            ("ssh.identity_id".into(), Invalid),
            ("ssh.jump_chain".into(), Invalid),
            ("ssh.proxy.type".into(), Invalid),
            ("ssh.agent_forwarding".into(), Invalid),
            ("ssh.env.1.name".into(), Duplicate),
            ("ssh.env.2.name".into(), Invalid),
            ("ssh.env.3.value".into(), TooLong),
            ("ssh.env.4".into(), Invalid),
            ("ssh.keepalive_s".into(), OutOfRange),
        ];
        assert_eq!(errors, expected);
        let _ = other;
    }

    #[test]
    fn tags_are_deduplicated_case_insensitively_and_limited() {
        let data = build(
            &map(json!({"address": "h.example.com", "tags": ["Prod", "db", "prod", " DB "]})),
            None,
            &no_secrets(),
            &rules(),
        )
        .unwrap();
        assert_eq!(data["tags"], json!(["Prod", "db"]));
        let many: Vec<String> = (0..33).map(|i| format!("t{i}")).collect();
        assert_eq!(
            errors_of(build(
                &map(json!({"address": "h.example.com", "tags": many})),
                None,
                &no_secrets(),
                &rules()
            )),
            vec![("tags".into(), TooMany)]
        );
        let dup_many: Vec<String> = (0..40).map(|i| format!("T{}", i % 32)).collect();
        assert!(
            build(
                &map(json!({"address": "h.example.com", "tags": dup_many})),
                None,
                &no_secrets(),
                &rules()
            )
            .is_ok()
        );
    }

    #[test]
    fn telnet_protocol_needs_a_telnet_member() {
        let data = build(
            &map(json!({"address": "h.example.com", "primary_protocol": "telnet", "telnet": {"port": 23}})),
            None,
            &no_secrets(),
            &rules(),
        )
        .unwrap();
        assert_eq!(data["primary_protocol"], json!("telnet"));
        assert_eq!(data["telnet"], json!({"port": 23}));
    }

    #[test]
    fn telnet_username_is_validated_and_trimmed() {
        let data = build(
            &map(json!({"address": "h.example.com", "telnet": {"port": 2323, "username": " admin "}})),
            None,
            &no_secrets(),
            &rules(),
        )
        .unwrap();
        assert_eq!(data["telnet"], json!({"port": 2323, "username": "admin"}));
        let blank = build(
            &map(json!({"address": "h.example.com", "telnet": {"username": "  "}})),
            None,
            &no_secrets(),
            &rules(),
        )
        .unwrap();
        assert_eq!(blank["telnet"], json!({}));
        assert_eq!(
            errors_of(build(
                &map(
                    json!({"address": "h.example.com", "telnet": {"port": 0, "username": "a\u{7}"}})
                ),
                None,
                &no_secrets(),
                &rules()
            )),
            vec![
                ("telnet.port".into(), OutOfRange),
                ("telnet.username".into(), Invalid)
            ]
        );
    }

    #[test]
    fn charset_is_lowercased() {
        let data = build(
            &map(json!({"address": "h.example.com", "charset": "UTF-8"})),
            None,
            &no_secrets(),
            &rules(),
        )
        .unwrap();
        assert_eq!(data["charset"], json!("utf-8"));
    }

    #[test]
    fn secret_paths_inside_data_are_refused() {
        let input = map(
            json!({"address": "h.example.com", "ssh": {"password": "x", "proxy": {"password": "y"}}}),
        );
        assert_eq!(
            errors_of(build(&input, None, &no_secrets(), &rules())),
            vec![
                ("ssh.password".into(), NotAllowed),
                ("ssh.proxy.password".into(), NotAllowed),
                ("ssh.proxy.type".into(), Invalid),
                ("ssh.proxy.address".into(), Invalid),
                ("ssh.proxy.port".into(), Invalid),
            ]
        );
    }

    #[test]
    fn secrets_are_set_kept_and_cleared() {
        let mut secrets = BTreeMap::new();
        secrets.insert(
            "ssh.password".to_owned(),
            Some(SecretString::from(" p4ss ")),
        );
        let input = map(json!({"address": "h.example.com", "ssh": {"username": "admin"}}));
        let first = build(&input, None, &secrets, &rules()).unwrap();
        assert_eq!(secret_value(&first, "ssh.password"), Some(" p4ss "));
        let (public, set) = public_view(&first);
        assert_eq!(set, vec!["ssh.password".to_owned()]);
        assert_eq!(
            Value::Object(public),
            json!({"address": "h.example.com", "ssh": {"username": "admin"}})
        );

        // Missing path keeps the stored value.
        let kept = build(&input, Some(&first), &no_secrets(), &rules()).unwrap();
        assert_eq!(secret_value(&kept, "ssh.password"), Some(" p4ss "));

        // null clears.
        let mut clear = BTreeMap::new();
        clear.insert("ssh.password".to_owned(), None);
        let cleared = build(&input, Some(&first), &clear, &rules()).unwrap();
        assert_eq!(secret_value(&cleared, "ssh.password"), None);

        // Unknown path and oversize values are refused.
        let mut bad = BTreeMap::new();
        bad.insert("ssh.username".to_owned(), Some(SecretString::from("x")));
        bad.insert(
            "ssh.password".to_owned(),
            Some(SecretString::from("x".repeat(4097))),
        );
        assert_eq!(
            errors_of(build(&input, None, &bad, &rules())),
            vec![
                ("ssh.password".into(), TooLong),
                ("ssh.username".into(), NotAllowed)
            ]
        );

        // A proxy password needs a proxy.
        let mut proxy_secret = BTreeMap::new();
        proxy_secret.insert(
            "ssh.proxy.password".to_owned(),
            Some(SecretString::from("x")),
        );
        assert_eq!(
            errors_of(build(&input, None, &proxy_secret, &rules())),
            vec![("ssh.proxy.password".into(), NotAllowed)]
        );
    }

    #[test]
    fn unknown_and_managed_members_are_kept() {
        let stored = map(json!({
            "address": "old.example.com",
            "order_key": "V",
            "import_key": "ssh_config:old",
            "os": {"family": "linux"},
            "x_future": {"a": 1},
            "ssh": {"password": "secret", "x_ssh_future": true}
        }));
        let input = map(json!({
            "address": "new.example.com",
            "order_key": "ZZZ",
            "os": {"family": "windows"},
            "x_client": 1,
            "ssh": {"port": 2222}
        }));
        let data = build(&input, Some(&stored), &no_secrets(), &rules()).unwrap();
        assert_eq!(
            Value::Object(data),
            json!({
                "address": "new.example.com",
                "order_key": "V",
                "import_key": "ssh_config:old",
                "os": {"family": "linux"},
                "x_future": {"a": 1},
                "x_client": 1,
                "ssh": {"port": 2222, "password": "secret", "x_ssh_future": true}
            })
        );
    }

    #[test]
    fn title_falls_back_to_address() {
        assert_eq!(
            title(&map(json!({"address": "a.example.com"}))),
            "a.example.com"
        );
        assert_eq!(
            title(&map(json!({"address": "a.example.com", "label": "A"}))),
            "A"
        );
    }
}
