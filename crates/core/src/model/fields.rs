//! Typed member readers with error collection, shared by the host and group
//! validators (docs/data-model.md, sections 1 and 3.5; stage 1 plan 5.5).
//!
//! Conventions: a missing member and `null` mean "absent"; a value of the
//! wrong JSON type is `invalid`; every refused member adds one
//! [`FieldError`] with its dotted path and nothing is written.

use std::net::Ipv6Addr;

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::{FieldError, ValidationReason};
use crate::model::text;

/// Collected validation errors.
#[derive(Debug, Default)]
pub struct Errors {
    list: Vec<FieldError>,
}

impl Errors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, field: impl Into<String>, reason: ValidationReason) {
        self.list.push(FieldError::new(field, reason));
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn extend(&mut self, other: Errors) {
        self.list.extend(other.list);
    }

    pub fn into_vec(self) -> Vec<FieldError> {
        self.list
    }

    /// `Ok(value)` when no error was collected.
    pub fn finish<T>(self, value: T) -> Result<T, Vec<FieldError>> {
        if self.list.is_empty() {
            Ok(value)
        } else {
            Err(self.list)
        }
    }
}

/// Joins a parent path and a member name.
pub fn path(parent: &str, member: &str) -> String {
    if parent.is_empty() {
        member.to_owned()
    } else {
        format!("{parent}.{member}")
    }
}

/// The value of a member, `None` when missing or `null`.
pub fn present<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    map.get(key).filter(|value| !value.is_null())
}

/// `string(n)?`
pub fn opt_string(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    max_chars: usize,
    errors: &mut Errors,
) -> Option<String> {
    let value = present(map, key)?;
    let Some(raw) = value.as_str() else {
        errors.push(field, ValidationReason::Invalid);
        return None;
    };
    match text::string(raw, max_chars) {
        Ok(value) => value,
        Err(reason) => {
            errors.push(field, reason);
            None
        }
    }
}

/// `text(n)?`
pub fn opt_text(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    max_bytes: usize,
    errors: &mut Errors,
) -> Option<String> {
    let value = present(map, key)?;
    let Some(raw) = value.as_str() else {
        errors.push(field, ValidationReason::Invalid);
        return None;
    };
    match text::text(raw, max_bytes) {
        Ok(value) => value,
        Err(reason) => {
            errors.push(field, reason);
            None
        }
    }
}

/// `uuid?` (lowercase hyphenated on output).
pub fn opt_uuid(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    errors: &mut Errors,
) -> Option<Uuid> {
    let value = present(map, key)?;
    match value
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw.trim()).ok())
    {
        Some(id) => Some(id),
        None => {
            errors.push(field, ValidationReason::Invalid);
            None
        }
    }
}

/// `int[min..max]?`: a non-integer is `invalid`, an integer outside the range `out_of_range`.
pub fn opt_int(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    min: i64,
    max: i64,
    errors: &mut Errors,
) -> Option<i64> {
    let value = present(map, key)?;
    let number = value.as_i64().or_else(|| value.as_u64().map(|_| i64::MAX));
    match number {
        Some(n) if (min..=max).contains(&n) => Some(n),
        Some(_) => {
            errors.push(field, ValidationReason::OutOfRange);
            None
        }
        None => {
            errors.push(field, ValidationReason::Invalid);
            None
        }
    }
}

/// `bool?`
pub fn opt_bool(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    errors: &mut Errors,
) -> Option<bool> {
    let value = present(map, key)?;
    match value.as_bool() {
        Some(flag) => Some(flag),
        None => {
            errors.push(field, ValidationReason::Invalid);
            None
        }
    }
}

/// `enum?`: one of `allowed`, compared exactly.
pub fn opt_enum(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    allowed: &[&'static str],
    errors: &mut Errors,
) -> Option<&'static str> {
    let value = present(map, key)?;
    match value
        .as_str()
        .and_then(|raw| allowed.iter().copied().find(|item| *item == raw))
    {
        Some(item) => Some(item),
        None => {
            errors.push(field, ValidationReason::Invalid);
            None
        }
    }
}

/// `tag_color` values (docs/data-model.md, section 3.5).
pub const TAG_COLORS: &[&str] = &[
    "graphite", "jade", "amber", "coral", "plum", "moss", "ocean", "rose",
];

/// Built-in terminal theme slugs (`apps/desktop/src/theme/terminal/*.json`;
/// the registry of `apps/desktop/src/theme/terminal.rs` lists the same ids in
/// the same order, a desktop test checks it).
pub const BUILTIN_THEMES: &[&str] = &[
    "graphite",
    "paper",
    "dracula",
    "nord",
    "solarized-dark",
    "solarized-light",
    "gruvbox-dark",
    "tokyo-night",
    "catppuccin-mocha",
    "catppuccin-latte",
];

/// An object member, `invalid` when it is not an object.
pub fn opt_object<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    field: &str,
    errors: &mut Errors,
) -> Option<&'a Map<String, Value>> {
    let value = present(map, key)?;
    match value.as_object() {
        Some(object) => Some(object),
        None => {
            errors.push(field, ValidationReason::Invalid);
            None
        }
    }
}

/// A list of record ids: at most `max` (`too_many`), each a UUID (`invalid`),
/// not `self_id` (`invalid`), no duplicates (`duplicate`). All errors use `field`.
pub fn opt_uuid_list(
    map: &Map<String, Value>,
    key: &str,
    field: &str,
    max: usize,
    self_id: Option<Uuid>,
    errors: &mut Errors,
) -> Option<Vec<Uuid>> {
    let value = present(map, key)?;
    let Some(items) = value.as_array() else {
        errors.push(field, ValidationReason::Invalid);
        return None;
    };
    if items.len() > max {
        errors.push(field, ValidationReason::TooMany);
        return None;
    }
    let mut ids: Vec<Uuid> = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item
            .as_str()
            .and_then(|raw| Uuid::parse_str(raw.trim()).ok())
        else {
            errors.push(field, ValidationReason::Invalid);
            return None;
        };
        if Some(id) == self_id {
            errors.push(field, ValidationReason::Invalid);
            return None;
        }
        if ids.contains(&id) {
            errors.push(field, ValidationReason::Duplicate);
            return None;
        }
        ids.push(id);
    }
    Some(ids)
}

/// A host address (docs/data-model.md, section 7.2): `string(255)`, surrounding
/// brackets removed, no whitespace, `@`, `/`, `\`, `[` or `]`, and a `:` only
/// inside an IPv6 literal with an optional `%zone`. `Ok(None)` when empty.
pub fn address(raw: &str) -> Result<Option<String>, ValidationReason> {
    let Some(value) = text::string(raw, 255)? else {
        return Ok(None);
    };
    let inner = match value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        Some(inner) => inner.to_owned(),
        None => value,
    };
    if inner.is_empty() {
        return Ok(None);
    }
    if inner
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '@' | '/' | '\\' | '[' | ']'))
    {
        return Err(ValidationReason::Invalid);
    }
    if inner.contains(':') {
        let (literal, zone) = match inner.split_once('%') {
            Some((literal, zone)) => (literal, Some(zone)),
            None => (inner.as_str(), None),
        };
        if literal.parse::<Ipv6Addr>().is_err() {
            return Err(ValidationReason::Invalid);
        }
        if let Some(zone) = zone
            && (zone.is_empty() || zone.contains(':') || zone.contains('%'))
        {
            return Err(ValidationReason::Invalid);
        }
    }
    Ok(Some(inner))
}

/// `theme_ref` (docs/data-model.md, section 3.5): `{"builtin": slug}` or
/// `{"record_id": uuid}`, exactly one member. A `record_id` is allowed only in
/// the personal vault (`not_allowed`).
pub fn theme_ref(value: &Value, personal_vault: bool) -> Result<Value, ValidationReason> {
    let Some(object) = value.as_object() else {
        return Err(ValidationReason::Invalid);
    };
    if object.len() != 1 {
        return Err(ValidationReason::Invalid);
    }
    if let Some(slug) = object.get("builtin") {
        return match slug.as_str() {
            Some(slug) if BUILTIN_THEMES.contains(&slug) => Ok(json!({ "builtin": slug })),
            _ => Err(ValidationReason::Invalid),
        };
    }
    if let Some(id) = object.get("record_id") {
        let Some(id) = id.as_str().and_then(|raw| Uuid::parse_str(raw).ok()) else {
            return Err(ValidationReason::Invalid);
        };
        if !personal_vault {
            return Err(ValidationReason::NotAllowed);
        }
        return Ok(json!({ "record_id": id }));
    }
    Err(ValidationReason::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationReason::*;

    #[test]
    fn address_rules() {
        assert_eq!(
            address("db1.example.com"),
            Ok(Some("db1.example.com".into()))
        );
        assert_eq!(address("  192.0.2.10 "), Ok(Some("192.0.2.10".into())));
        assert_eq!(address("[2001:db8::1]"), Ok(Some("2001:db8::1".into())));
        assert_eq!(address("2001:db8::1"), Ok(Some("2001:db8::1".into())));
        assert_eq!(address("fe80::1%eth0"), Ok(Some("fe80::1%eth0".into())));
        assert_eq!(address("myserver"), Ok(Some("myserver".into())));
        assert_eq!(address(""), Ok(None));
        assert_eq!(address("[]"), Ok(None));
        assert_eq!(address("user@example.com"), Err(Invalid));
        assert_eq!(address("example.com/path"), Err(Invalid));
        assert_eq!(address("a b.example.com"), Err(Invalid));
        assert_eq!(address("back\\slash"), Err(Invalid));
        assert_eq!(address("example.com:22"), Err(Invalid));
        assert_eq!(address("[2001:db8::1]:22"), Err(Invalid));
        assert_eq!(address("fe80::1%"), Err(Invalid));
        assert_eq!(address("a[b"), Err(Invalid));
        assert_eq!(address(&"a".repeat(256)), Err(TooLong));
        assert_eq!(address(&"a".repeat(255)), Ok(Some("a".repeat(255))));
    }

    #[test]
    fn theme_ref_rules() {
        assert_eq!(
            theme_ref(&json!({"builtin": "paper"}), true),
            Ok(json!({"builtin": "paper"}))
        );
        assert_eq!(theme_ref(&json!({"builtin": "neon"}), true), Err(Invalid));
        let id = "0199f6a1-0000-7000-8000-000000000001";
        assert_eq!(
            theme_ref(&json!({"record_id": id}), true),
            Ok(json!({"record_id": id}))
        );
        assert_eq!(theme_ref(&json!({"record_id": id}), false), Err(NotAllowed));
        assert_eq!(
            theme_ref(&json!({"builtin": "paper", "record_id": id}), true),
            Err(Invalid)
        );
        assert_eq!(theme_ref(&json!("paper"), true), Err(Invalid));
    }

    #[test]
    fn ints_and_lists() {
        let map = json!({"a": 70000, "b": "22", "c": 22, "d": 1.5, "e": u64::MAX})
            .as_object()
            .unwrap()
            .clone();
        let mut errors = Errors::new();
        assert_eq!(opt_int(&map, "a", "a", 1, 65535, &mut errors), None);
        assert_eq!(opt_int(&map, "b", "b", 1, 65535, &mut errors), None);
        assert_eq!(opt_int(&map, "c", "c", 1, 65535, &mut errors), Some(22));
        assert_eq!(opt_int(&map, "d", "d", 1, 65535, &mut errors), None);
        assert_eq!(opt_int(&map, "e", "e", 1, 65535, &mut errors), None);
        assert_eq!(opt_int(&map, "missing", "m", 1, 65535, &mut errors), None);
        let list = errors.into_vec();
        assert_eq!(
            list,
            vec![
                FieldError::new("a", OutOfRange),
                FieldError::new("b", Invalid),
                FieldError::new("d", Invalid),
                FieldError::new("e", OutOfRange),
            ]
        );

        let id = Uuid::now_v7();
        let other = Uuid::now_v7();
        let map = json!({
            "ok": [other.to_string()],
            "dup": [other.to_string(), other.to_string()],
            "selfref": [id.to_string()],
            "many": vec![Uuid::nil().to_string(); 9],
        })
        .as_object()
        .unwrap()
        .clone();
        let mut errors = Errors::new();
        assert_eq!(
            opt_uuid_list(&map, "ok", "ok", 8, Some(id), &mut errors),
            Some(vec![other])
        );
        assert_eq!(
            opt_uuid_list(&map, "dup", "dup", 8, Some(id), &mut errors),
            None
        );
        assert_eq!(
            opt_uuid_list(&map, "selfref", "selfref", 8, Some(id), &mut errors),
            None
        );
        assert_eq!(
            opt_uuid_list(&map, "many", "many", 8, Some(id), &mut errors),
            None
        );
        assert_eq!(
            errors.into_vec(),
            vec![
                FieldError::new("dup", Duplicate),
                FieldError::new("selfref", Invalid),
                FieldError::new("many", TooMany),
            ]
        );
    }
}
