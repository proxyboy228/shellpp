//! Command history: capture rules and the secret detector (D16;
//! docs/security.md, sections 10.1 and 10.2; docs/data-model.md, section 13.1;
//! docs/storage-format.md, section 7).
//!
//! The UI captures candidates at Enter (rules 1-3 of section 10.1); the core
//! applies rule 4 ([`crate::service::host_api::HistoryRecordInput`]) and
//! writes a `history_entry` only when every check passes. A dropped command is
//! never written, not even encrypted, and never logged.
//!
//! Owner: S4. Signatures written by the lead.

use std::sync::LazyLock;

use regex::Regex;

use crate::service::dto::HistoryCandidate;
use crate::service::host_api::HistoryDropReason;

/// Longest command kept, in characters (rule 4 of section 10.1).
pub const MAX_COMMAND_CHARS: usize = 4096;

/// Characters a terminal shows instead of typed characters when it masks
/// input (the echo-off rule of section 10.1).
pub const MASK_CHARS: &[char] = &['*', '\u{2022}', '\u{25cf}'];

/// The rule of docs/security.md, section 10.2 that matched a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretRule {
    /// `-p<value>`: a token `^-p\S+$` (case-sensitive).
    S1,
    /// Substring `--password`.
    S2,
    /// `[a-z0-9_]*password=`.
    S3,
    /// `[a-z0-9_]*token=`.
    S4,
    /// Substring `secret`.
    S5,
    /// `api[_-]?key`.
    S6,
    /// `user:pass@`: `[^\s:@/]+:[^\s@/]+@`.
    S7,
    /// Substring `authorization:`.
    S8,
    /// `\bbearer\b`.
    S9,
    /// Known token prefixes (case-sensitive).
    S10,
    /// A high-entropy string of 20 or more characters.
    S11,
}

/// The password prompt pattern of section 10.1 (stage 0 decision; the list
/// lives here, with tests in every listed language).
static PASSWORD_PROMPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)",
        r"\b(password|passwd|passphrase|passcode|pin|otp|one[- ]time|verification code|token|2fa|authenticator|secret)\b",
        r"|(passwort|kennwort|mot de passe|contraseña|senha|parola|wachtwoord|hasło|heslo|пароль|密码|パスワード|비밀번호)",
    ))
    .expect("static regex")
});

/// Rules S2-S9 (case-insensitive), in rule order.
static CASE_INSENSITIVE_RULES: LazyLock<Vec<(SecretRule, Regex)>> = LazyLock::new(|| {
    [
        (SecretRule::S2, r"(?i)--password"),
        (SecretRule::S3, r"(?i)[a-z0-9_]*password="),
        (SecretRule::S4, r"(?i)[a-z0-9_]*token="),
        (SecretRule::S5, r"(?i)secret"),
        (SecretRule::S6, r"(?i)api[_-]?key"),
        (SecretRule::S7, r"(?i)[^\s:@/]+:[^\s@/]+@"),
        (SecretRule::S8, r"(?i)authorization:"),
        (SecretRule::S9, r"(?i)\bbearer\b"),
    ]
    .into_iter()
    .map(|(rule, pattern)| (rule, Regex::new(pattern).expect("static regex")))
    .collect()
});

/// Rule S10: public token formats (case-sensitive; stage 0 decision).
static TOKEN_PREFIXES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}\b",
        r"|\bgithub_pat_[A-Za-z0-9_]{22,}",
        r"|\bglpat-[A-Za-z0-9_-]{20,}",
        r"|\bxox[abprs]-[A-Za-z0-9-]{10,}",
        r"|\b(AKIA|ASIA)[A-Z0-9]{16}\b",
        r"|\bAIza[A-Za-z0-9_-]{35}",
        r"|\b(sk|rk)_live_[A-Za-z0-9]{16,}",
        r"|\bnpm_[A-Za-z0-9]{36}\b",
        r"|\bpypi-[A-Za-z0-9_-]{50,}",
        r"|-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----",
        r"|\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.",
        r"|\bshellpp_(at|rt|lt|rc)_[A-Za-z0-9_-]{20,}",
        r"|\bRK1-[0-9A-Za-z]{5}-",
    ))
    .expect("static regex")
});

/// S11 thresholds (docs/security.md, section 10.2).
pub const S11_MIN_LEN: usize = 20;
/// Mixed-class candidates: lowercase, uppercase and digits with at least this entropy.
pub const S11_MIXED_ENTROPY: f64 = 3.5;
/// Hex candidates: at least this long with digits and letters.
pub const S11_HEX_MIN_LEN: usize = 32;
/// Long candidates: at least this long with at least [`S11_LONG_ENTROPY`].
pub const S11_LONG_MIN_LEN: usize = 32;
pub const S11_LONG_ENTROPY: f64 = 4.2;

/// Rules S1-S11 of docs/security.md, section 10.2. `true` = drop the command.
pub fn looks_like_secret(command: &str) -> bool {
    secret_rule(command).is_some()
}

/// The first rule of S1-S11 that matches the command, in rule order.
pub fn secret_rule(command: &str) -> Option<SecretRule> {
    if command.split_whitespace().any(s1_token) {
        return Some(SecretRule::S1);
    }
    if let Some((rule, _)) = CASE_INSENSITIVE_RULES
        .iter()
        .find(|(_, regex)| regex.is_match(command))
    {
        return Some(*rule);
    }
    if TOKEN_PREFIXES.is_match(command) {
        return Some(SecretRule::S10);
    }
    if high_entropy(command) {
        return Some(SecretRule::S11);
    }
    None
}

/// S1: a token `^-p\S+$`. A token inside quotes (`'-pX'`) is checked
/// without its leading quotes, which only fails closed.
fn s1_token(token: &str) -> bool {
    let token = token.trim_start_matches(['\'', '"']);
    token
        .strip_prefix("-p")
        .is_some_and(|value| !value.is_empty())
}

/// Shannon entropy of a string over its own characters, in bits per character.
pub fn shannon_entropy(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0.0;
    }
    let mut counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for c in &chars {
        *counts.entry(*c).or_default() += 1;
    }
    let len = chars.len() as f64;
    counts
        .values()
        .map(|count| {
            let p = *count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// S11 step 2: a candidate token.
fn s11_candidate(token: &str) -> bool {
    token.chars().count() >= S11_MIN_LEN
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '_' | '-'))
        && (!token.contains('/') || token.contains('+'))
}

/// S11 step 3 for one candidate.
fn s11_matches(token: &str) -> bool {
    let len = token.chars().count();
    let lower = token.chars().any(|c| c.is_ascii_lowercase());
    let upper = token.chars().any(|c| c.is_ascii_uppercase());
    let digit = token.chars().any(|c| c.is_ascii_digit());
    let letter = token.chars().any(|c| c.is_ascii_alphabetic());
    let hex = token.chars().all(|c| c.is_ascii_hexdigit());
    let entropy = shannon_entropy(token);
    (lower && upper && digit && entropy >= S11_MIXED_ENTROPY)
        || (hex && len >= S11_HEX_MIN_LEN && digit && letter)
        || (len >= S11_LONG_MIN_LEN && entropy >= S11_LONG_ENTROPY)
}

/// S11: split on whitespace, quotes and `= : , ; @`; any matching candidate.
fn high_entropy(command: &str) -> bool {
    command
        .split(|c: char| {
            c.is_whitespace() || matches!(c, '\'' | '"' | '`' | '=' | ':' | ',' | ';' | '@')
        })
        .filter(|token| s11_candidate(token))
        .any(s11_matches)
}

/// The password prompt pattern of docs/security.md, section 10.1
/// (case-insensitive, with the listed non-English words).
pub fn prompt_looks_like_password(prompt_text: &str) -> bool {
    PASSWORD_PROMPT.is_match(prompt_text)
}

/// Skips the rest of a control string (OSC, DCS, SOS, PM, APC) up to its
/// terminator: BEL, ST (`ESC \` or U+009C).
fn skip_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(c) = chars.next() {
        match c {
            '\u{7}' | '\u{9c}' => return,
            '\u{1b}' => {
                if chars.peek() == Some(&'\\') {
                    chars.next();
                }
                return;
            }
            _ => {}
        }
    }
}

/// Skips the parameters, intermediates and final byte of a CSI sequence.
fn skip_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for c in chars.by_ref() {
        if ('\u{40}'..='\u{7e}').contains(&c) {
            return;
        }
        if !('\u{20}'..='\u{3f}').contains(&c) {
            // Not a CSI byte: the sequence was cut off.
            return;
        }
    }
}

/// Rule 3 of section 10.1: removes C0 and C1 control characters, leftover
/// escape sequences and bracketed paste markers.
///
/// Escape sequences (CSI, OSC, DCS and the other control strings, and
/// two-character escapes) are removed whole, in their 7-bit and 8-bit forms.
/// Bracketed paste markers are removed also in the caret notation `^[[200~`
/// and `^[[201~` that a shell without bracketed paste support echoes. A TAB
/// becomes a space; every other control character is dropped. Trailing
/// whitespace is trimmed.
pub fn clean_command(raw: &str) -> String {
    let raw = raw.replace("^[[200~", "").replace("^[[201~", "");
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => skip_csi(&mut chars),
                Some(']' | 'P' | 'X' | '^' | '_') => skip_string(&mut chars),
                Some(next) if ('\u{20}'..='\u{2f}').contains(&next) => {
                    // Intermediate bytes, then one final byte.
                    while let Some(&peek) = chars.peek() {
                        chars.next();
                        if !('\u{20}'..='\u{2f}').contains(&peek) {
                            break;
                        }
                    }
                }
                // A two-character escape (or ESC at the end).
                Some(_) | None => {}
            },
            '\u{9b}' => skip_csi(&mut chars),
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => skip_string(&mut chars),
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    out
}

/// Rule 4 of section 10.1 and the secret detector, in the order of the stage
/// 2 plan (section 4.4), without the checks only the facade can make (lock
/// state and the per-session duplicate rule). Returns the command to store.
pub fn evaluate(
    candidate: &HistoryCandidate,
    injected: bool,
    record_history: bool,
) -> Result<String, HistoryDropReason> {
    if !record_history {
        return Err(HistoryDropReason::Off);
    }
    if injected {
        return Err(HistoryDropReason::Injected);
    }
    if candidate.alt_screen {
        return Err(HistoryDropReason::AltScreen);
    }
    let command = clean_command(&candidate.command);
    let masked = command.trim().chars().all(|c| MASK_CHARS.contains(&c));
    if candidate.typed_printable && masked {
        return Err(HistoryDropReason::EchoOff);
    }
    if prompt_looks_like_password(&clean_command(&candidate.prompt_text)) {
        return Err(HistoryDropReason::PasswordPrompt);
    }
    if command.is_empty() {
        return Err(HistoryDropReason::Empty);
    }
    if command.starts_with(' ') {
        return Err(HistoryDropReason::LeadingSpace);
    }
    if command.chars().count() > MAX_COMMAND_CHARS {
        return Err(HistoryDropReason::TooLong);
    }
    if looks_like_secret(&command) {
        return Err(HistoryDropReason::Secret);
    }
    Ok(command)
}

#[cfg(test)]
mod tests;
