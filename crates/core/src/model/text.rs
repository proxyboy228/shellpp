//! Text field types of docs/data-model.md, section 1.1.
//!
//! - `string(n)`: single line, NFC-normalized, trimmed, 1 to n Unicode scalar
//!   values, no control characters. An empty string means "absent".
//! - `text(n)`: multi-line, at most n UTF-8 bytes, LF line endings (CRLF and CR
//!   become LF), no control characters except LF and TAB. Empty means absent.
//! - `secret(n)`: stored exactly as entered, at most n UTF-8 bytes, no NUL.
//! - `tag`: `string(64)` without commas; compared case-insensitively.

use unicode_normalization::UnicodeNormalization;

use crate::error::ValidationReason;

/// `string(n)`: `Ok(None)` for an empty value.
pub fn string(value: &str, max_chars: usize) -> Result<Option<String>, ValidationReason> {
    let normalized: String = value.nfc().collect();
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ValidationReason::Invalid);
    }
    if trimmed.chars().count() > max_chars {
        return Err(ValidationReason::TooLong);
    }
    Ok(Some(trimmed.to_owned()))
}

/// `text(n)`: `Ok(None)` for an empty value.
pub fn text(value: &str, max_bytes: usize) -> Result<Option<String>, ValidationReason> {
    let unified = value.replace("\r\n", "\n").replace('\r', "\n");
    if unified.is_empty() {
        return Ok(None);
    }
    if unified
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ValidationReason::Invalid);
    }
    if unified.len() > max_bytes {
        return Err(ValidationReason::TooLong);
    }
    Ok(Some(unified))
}

/// `secret(n)`: checks only; the value is stored exactly as entered.
pub fn secret(value: &str, max_bytes: usize) -> Result<(), ValidationReason> {
    if value.len() > max_bytes {
        return Err(ValidationReason::TooLong);
    }
    if value.contains('\0') {
        return Err(ValidationReason::Invalid);
    }
    Ok(())
}

/// Maximum length of a tag in Unicode scalar values.
pub const TAG_MAX_CHARS: usize = 64;

/// A tag: `string(64)` without commas, required inside a tag list.
pub fn tag(value: &str) -> Result<String, ValidationReason> {
    let tag = string(value, TAG_MAX_CHARS)?.ok_or(ValidationReason::Invalid)?;
    if tag.contains(',') {
        return Err(ValidationReason::Invalid);
    }
    Ok(tag)
}

/// Case-insensitive comparison key (an approximation of Unicode simple case
/// folding through lowercase mapping) for tags, search and host names.
pub fn fold(value: &str) -> String {
    value.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationReason::*;

    #[test]
    fn string_rules() {
        assert_eq!(string("  web 1 ", 10), Ok(Some("web 1".to_owned())));
        assert_eq!(string("   ", 10), Ok(None));
        assert_eq!(string("", 10), Ok(None));
        assert_eq!(string("a\u{7}b", 10), Err(Invalid));
        assert_eq!(string("a\nb", 10), Err(Invalid));
        assert_eq!(string("abcdefghijk", 10), Err(TooLong));
        // Ten scalar values, more bytes.
        assert_eq!(string("ääääääääää", 10), Ok(Some("ääääääääää".to_owned())));
        // NFC: "e" + combining acute becomes one scalar value.
        assert_eq!(string("e\u{301}", 1), Ok(Some("\u{e9}".to_owned())));
    }

    #[test]
    fn text_rules() {
        assert_eq!(text("a\r\nb\rc", 100), Ok(Some("a\nb\nc".to_owned())));
        assert_eq!(text("a\tb", 100), Ok(Some("a\tb".to_owned())));
        assert_eq!(text("", 100), Ok(None));
        assert_eq!(text("  ", 100), Ok(Some("  ".to_owned())));
        assert_eq!(text("a\u{0}b", 100), Err(Invalid));
        assert_eq!(text("ä", 1), Err(TooLong));
    }

    #[test]
    fn secret_rules() {
        assert_eq!(secret(" padded ", 10), Ok(()));
        assert_eq!(secret("a\0", 10), Err(Invalid));
        assert_eq!(secret("ääääää", 10), Err(TooLong));
    }

    #[test]
    fn tag_rules() {
        assert_eq!(tag(" prod "), Ok("prod".to_owned()));
        assert_eq!(tag("a,b"), Err(Invalid));
        assert_eq!(tag(""), Err(Invalid));
        assert_eq!(tag(&"x".repeat(65)), Err(TooLong));
        assert_eq!(fold("PrOd"), "prod");
    }
}
