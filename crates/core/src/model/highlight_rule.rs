//! The `highlight_rule` record (docs/data-model.md, section 14.2; UI 12.4;
//! D25, D77): a custom rule (a regular expression with its colors) or the
//! color override of a built-in category.

use serde_json::{Map, Value};

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{Errors, opt_bool, opt_enum, present};
use crate::model::merge::{keep_from, merge_level};

/// Longest pattern, in characters (`string(512)`).
pub const MAX_PATTERN_CHARS: usize = 512;
/// Most custom rules of a vault (D77).
pub const MAX_CUSTOM_RULES: usize = 64;
/// The compiled size a pattern may take (the `regex` crate's limit).
const COMPILED_LIMIT: usize = 256 * 1024;

pub const KINDS: &[&str] = &["custom", "category"];
pub const CATEGORIES: &[&str] = &["error", "warning", "ok", "info", "debug", "ip_mac"];

const KNOWN: &[&str] = &[
    "kind",
    "category",
    "pattern",
    "case_insensitive",
    "color_dark",
    "color_light",
    "enabled",
];

/// A `#RRGGBB` color.
pub fn is_color(text: &str) -> bool {
    text.len() == 7
        && text.starts_with('#')
        && text[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The regular expression of a custom rule, as the terminal compiles it:
/// it must compile within the size limit and must not match the empty
/// string (it would mark nothing, everywhere).
pub fn compile(pattern: &str, case_insensitive: bool) -> Result<regex::Regex, ValidationReason> {
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .size_limit(COMPILED_LIMIT)
        .dfa_size_limit(COMPILED_LIMIT)
        .build()
        .map_err(|_| ValidationReason::Invalid)?;
    if regex.is_match("") {
        return Err(ValidationReason::Invalid);
    }
    Ok(regex)
}

fn color(input: &Map<String, Value>, key: &str, errors: &mut Errors) -> Option<String> {
    match present(input, key).map(Value::as_str) {
        None => None,
        Some(Some(text)) if is_color(text) => Some(text.to_ascii_uppercase()),
        Some(_) => {
            errors.push(key, ValidationReason::Invalid);
            None
        }
    }
}

/// Validates a rule and merges it over the stored one.
pub fn build(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    let mut normalized = Map::new();
    let kind = opt_enum(input, "kind", "kind", KINDS, &mut errors);
    match kind {
        Some(kind) => {
            normalized.insert("kind".into(), Value::String(kind.into()));
        }
        None if present(input, "kind").is_none() => errors.push("kind", ValidationReason::Required),
        None => {}
    }
    let case_insensitive =
        opt_bool(input, "case_insensitive", "case_insensitive", &mut errors).unwrap_or(false);
    match kind {
        Some("custom") => match present(input, "pattern").map(Value::as_str) {
            Some(Some(pattern)) if pattern.chars().count() > MAX_PATTERN_CHARS => {
                errors.push("pattern", ValidationReason::TooLong);
            }
            Some(Some(pattern)) if !pattern.is_empty() => {
                match compile(pattern, case_insensitive) {
                    Ok(_) => {
                        normalized.insert("pattern".into(), Value::String(pattern.to_owned()));
                    }
                    Err(reason) => errors.push("pattern", reason),
                }
            }
            Some(Some(_)) | None => errors.push("pattern", ValidationReason::Required),
            Some(None) => errors.push("pattern", ValidationReason::Invalid),
        },
        Some("category") => {
            match opt_enum(input, "category", "category", CATEGORIES, &mut errors) {
                Some(category) => {
                    normalized.insert("category".into(), Value::String(category.into()));
                }
                None if present(input, "category").is_none() => {
                    errors.push("category", ValidationReason::Required)
                }
                None => {}
            }
        }
        _ => {}
    }
    normalized.insert("case_insensitive".into(), Value::Bool(case_insensitive));
    match color(input, "color_dark", &mut errors) {
        Some(dark) => {
            normalized.insert("color_dark".into(), Value::String(dark));
        }
        None if present(input, "color_dark").is_none() => {
            errors.push("color_dark", ValidationReason::Required)
        }
        None => {}
    }
    if let Some(light) = color(input, "color_light", &mut errors) {
        normalized.insert("color_light".into(), Value::String(light));
    }
    if let Some(enabled) = opt_bool(input, "enabled", "enabled", &mut errors) {
        normalized.insert("enabled".into(), Value::Bool(enabled));
    }
    if !errors.is_empty() {
        return Err(errors.into_vec());
    }
    let mut data = merge_level(KNOWN, normalized, Some(input), stored);
    keep_from(&mut data, stored, &[]);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reasons(value: Value) -> Vec<(String, ValidationReason)> {
        build(value.as_object().unwrap(), None)
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|error| (error.field, error.reason))
            .collect()
    }

    #[test]
    fn rules_are_checked() {
        let ok = build(
            json!({"kind": "custom", "pattern": "timeout|refused", "case_insensitive": true,
                   "color_dark": "#ff8800"})
            .as_object()
            .unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(ok["color_dark"], json!("#FF8800"));
        assert!(
            build(
                json!({"kind": "category", "category": "error", "color_dark": "#112233",
                       "color_light": "#445566", "enabled": false})
                .as_object()
                .unwrap(),
                None,
            )
            .is_ok()
        );
        let invalid = ValidationReason::Invalid;
        assert_eq!(
            reasons(json!({"kind": "custom", "pattern": "(", "color_dark": "#000000"})),
            [("pattern".to_owned(), invalid)]
        );
        // A pattern that matches nothing at all marks every position.
        assert_eq!(
            reasons(json!({"kind": "custom", "pattern": "x*", "color_dark": "#000000"})),
            [("pattern".to_owned(), invalid)]
        );
        assert_eq!(
            reasons(json!({"kind": "custom", "pattern": "a".repeat(513), "color_dark": "#000000"})),
            [("pattern".to_owned(), ValidationReason::TooLong)]
        );
        assert_eq!(
            reasons(json!({"kind": "custom", "pattern": "ok", "color_dark": "red"})),
            [("color_dark".to_owned(), invalid)]
        );
        assert!(
            reasons(json!({"kind": "category", "category": "nope", "color_dark": "#000000"}))
                .iter()
                .any(|(field, _)| field == "category")
        );
        assert!(
            reasons(json!({"pattern": "x"}))
                .iter()
                .any(|(field, reason)| field == "kind" && *reason == ValidationReason::Required)
        );
        // A huge compiled pattern is refused, not built.
        assert_eq!(
            reasons(json!({"kind": "custom", "pattern": "(\\w{50}){50}", "color_dark": "#000000"})),
            [("pattern".to_owned(), invalid)]
        );
    }
}
