//! Snippet variables (docs/data-model.md, section 10.1; UI 9.3): every
//! `{{ name }}` of a script, with optional spaces inside the braces and
//! `name` matching `[A-Za-z_][A-Za-z0-9_]{0,63}`. The same name is asked
//! once per run; there is no escape syntax. Anything else between braces
//! (`{{ 1x }}`, `{{a-b}}`, an unclosed `{{`) is script text and stays as it
//! is.
//!
//! Values typed at run time are never stored ([`fill`] returns the text to
//! send; the caller keeps it no longer than the run), and a command line a
//! snippet produced is never recorded in history (docs/security.md, 10.1).

use std::collections::HashMap;
use std::ops::Range;

/// Longest variable name.
pub const MAX_NAME_LEN: usize = 64;

/// One `{{ name }}` occurrence: the byte range of the whole placeholder
/// (braces included) and the name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placeholder {
    range: Range<usize>,
    name: String,
}

fn name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= MAX_NAME_LEN
        && (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Every placeholder of `script`, in order.
fn placeholders(script: &str) -> Vec<Placeholder> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = script[from..].find("{{") {
        let start = from + open;
        let inner_start = start + 2;
        let Some(close) = script[inner_start..].find("}}") else {
            break;
        };
        let inner_end = inner_start + close;
        let name = script[inner_start..inner_end].trim_matches(|c| c == ' ' || c == '\t');
        if name_ok(name) {
            found.push(Placeholder {
                range: start..inner_end + 2,
                name: name.to_owned(),
            });
            from = inner_end + 2;
        } else {
            // Not a variable: look again from the next brace, so `{{{x}}`
            // still finds `{{x}}`.
            from = start + 1;
        }
    }
    found
}

/// The variable names of `script`, each once, in the order they first
/// appear (the order of the form at run time).
pub fn variables(script: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for placeholder in placeholders(script) {
        if !names.contains(&placeholder.name) {
            names.push(placeholder.name);
        }
    }
    names
}

/// Why a script could not be filled in.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FillError {
    /// A variable has no value (the form was not complete).
    #[error("variable {0} has no value")]
    Missing(String),
}

/// `script` with every placeholder replaced by its value. Values are
/// inserted as they are (the user typed them for this run); a value never
/// expands further, so a value containing `{{x}}` stays literal.
pub fn fill(script: &str, values: &HashMap<String, String>) -> Result<String, FillError> {
    let mut out = String::with_capacity(script.len());
    let mut last = 0;
    for placeholder in placeholders(script) {
        let value = values
            .get(&placeholder.name)
            .ok_or_else(|| FillError::Missing(placeholder.name.clone()))?;
        out.push_str(&script[last..placeholder.range.start]);
        out.push_str(value);
        last = placeholder.range.end;
    }
    out.push_str(&script[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn names_are_found_once_in_order() {
        assert_eq!(
            variables("tar czf {{ archive }} {{dir}} && ls {{archive}}"),
            ["archive", "dir"]
        );
        assert_eq!(variables("echo {{\tUSER_1 }}"), ["USER_1"]);
        assert!(variables("no variables here").is_empty());
    }

    #[test]
    fn other_brace_text_is_script_text() {
        // Not names: a leading digit, a dash, empty, an unclosed pair.
        assert!(variables("{{ 1x }} {{a-b}} {{}} {{ }} {{ open").is_empty());
        // awk and shell braces are left alone.
        assert!(variables("awk '{print $1}' ${HOME} {a,b}").is_empty());
        // A third brace before a variable.
        assert_eq!(variables("{{{x}}"), ["x"]);
        let long = "a".repeat(MAX_NAME_LEN + 1);
        assert!(variables(&format!("{{{{{long}}}}}")).is_empty());
        let longest = "a".repeat(MAX_NAME_LEN);
        assert_eq!(variables(&format!("{{{{{longest}}}}}")), [longest]);
    }

    #[test]
    fn fill_replaces_every_occurrence_and_values_stay_literal() {
        let script = "cp {{ src }} {{dst}}; echo {{src}} {{ 1x }}";
        assert_eq!(
            fill(script, &values(&[("src", "a.txt"), ("dst", "{{src}}")])).unwrap(),
            "cp a.txt {{src}}; echo a.txt {{ 1x }}"
        );
        assert_eq!(
            fill(script, &values(&[("src", "a")])),
            Err(FillError::Missing("dst".to_owned()))
        );
        assert_eq!(fill("plain", &HashMap::new()).unwrap(), "plain");
        // Multi-byte text around placeholders.
        assert_eq!(
            fill("echo «{{ word }}»", &values(&[("word", "привет")])).unwrap(),
            "echo «привет»"
        );
    }
}
