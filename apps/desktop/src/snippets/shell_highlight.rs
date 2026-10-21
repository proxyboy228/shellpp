//! Highlighting of a snippet's script (UI 9.3; D74): the app's own reading
//! of a shell script through GPUI Kit's parser-independent highlighter
//! seam ([`InputHighlighter`]), with no grammar crate: comments, quoted
//! strings, `$variables`, the snippet's `{{name}}` variables (also inside
//! double quotes), keywords, numbers and operators. It only colors the
//! text: the script is stored and run as typed.

use std::ops::Range;

use gpui::{Context, HighlightStyle, SharedString, Window};
use gpui_kit::component::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope,
};

/// The language name of the script editor.
pub const LANGUAGE: &str = "shell";

/// What a piece of the script is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Comment,
    String,
    Variable,
    /// A `{{name}}` variable of the snippet.
    Placeholder,
    Keyword,
    Number,
    Operator,
}

impl Kind {
    /// The style of the theme's syntax colors.
    fn style_name(self) -> &'static str {
        match self {
            Kind::Comment => "comment",
            Kind::String => "string",
            Kind::Variable => "variable",
            Kind::Placeholder => "embedded",
            Kind::Keyword => "keyword",
            Kind::Number => "number",
            Kind::Operator => "operator",
        }
    }
}

const KEYWORDS: [&str; 19] = [
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "select", "time", "return", "local", "export",
];

/// Where a word may start: the text's start, after a blank or an operator.
fn word_boundary(bytes: &[u8], at: usize) -> bool {
    at == 0
        || matches!(
            bytes[at - 1],
            b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')'
        )
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The end of a `{{name}}` starting at `at`, on one line.
fn placeholder_end(text: &str, at: usize) -> Option<usize> {
    let rest = text.get(at..)?;
    if !rest.starts_with("{{") {
        return None;
    }
    let close = rest.find("}}")?;
    let inner = rest[2..close].trim();
    let mut chars = inner.chars();
    let named = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    (named && !rest[..close].contains('\n')).then_some(at + close + 2)
}

/// The end of a `$name`, `${...}`, `$1` or `$?` starting at `at`.
fn variable_end(bytes: &[u8], at: usize) -> Option<usize> {
    let next = *bytes.get(at + 1)?;
    if next == b'{' {
        let close = bytes[at + 2..].iter().position(|byte| *byte == b'}')?;
        return Some(at + 2 + close + 1);
    }
    if next.is_ascii_alphabetic() || next == b'_' {
        let length = bytes[at + 1..]
            .iter()
            .take_while(|byte| is_word(**byte))
            .count();
        return Some(at + 1 + length);
    }
    matches!(
        next,
        b'0'..=b'9' | b'?' | b'#' | b'@' | b'*' | b'$' | b'!' | b'-'
    )
    .then_some(at + 2)
}

/// The pieces of `script` worth a color, in order, as byte ranges.
pub fn tokens(script: &str) -> Vec<(Range<usize>, Kind)> {
    let bytes = script.as_bytes();
    let length = bytes.len();
    let mut out = Vec::new();
    let mut at = 0;
    while at < length {
        if let Some(end) = placeholder_end(script, at) {
            out.push((at..end, Kind::Placeholder));
            at = end;
            continue;
        }
        match bytes[at] {
            b'#' if word_boundary(bytes, at) => {
                let end = script[at..].find('\n').map_or(length, |line| at + line);
                out.push((at..end, Kind::Comment));
                at = end;
            }
            b'\'' => {
                let end = script[at + 1..]
                    .find('\'')
                    .map_or(length, |close| at + 1 + close + 1);
                out.push((at..end, Kind::String));
                at = end;
            }
            b'"' => at = double_quoted(script, at, &mut out),
            b'$' if bytes.get(at + 1) == Some(&b'(') => {
                out.push((at..at + 2, Kind::Operator));
                at += 2;
            }
            b'$' => match variable_end(bytes, at) {
                Some(end) => {
                    out.push((at..end, Kind::Variable));
                    at = end;
                }
                None => at += 1,
            },
            b'|' | b'&' | b';' | b'>' | b'<' | b'(' | b')' => {
                let run = bytes[at..]
                    .iter()
                    .take_while(|byte| matches!(byte, b'|' | b'&' | b';' | b'>' | b'<'))
                    .count()
                    .max(1);
                out.push((at..at + run, Kind::Operator));
                at += run;
            }
            byte if is_word(byte) => {
                let end = at
                    + bytes[at..]
                        .iter()
                        .take_while(|byte| is_word(**byte))
                        .count();
                let word = &script[at..end];
                let ends_word = bytes.get(end).is_none_or(|next| {
                    matches!(
                        next,
                        b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b')'
                    )
                });
                if word_boundary(bytes, at) && ends_word {
                    if KEYWORDS.contains(&word) {
                        out.push((at..end, Kind::Keyword));
                    } else if word.bytes().all(|byte| byte.is_ascii_digit()) {
                        out.push((at..end, Kind::Number));
                    }
                }
                at = end;
            }
            _ => at += 1,
        }
    }
    out
}

/// A double-quoted string from `start`: its `$variables` and `{{name}}`s
/// keep their own colors. Returns where it ends.
fn double_quoted(script: &str, start: usize, out: &mut Vec<(Range<usize>, Kind)>) -> usize {
    let bytes = script.as_bytes();
    let length = bytes.len();
    let mut piece = start;
    let mut at = start + 1;
    loop {
        if at >= length {
            out.push((piece..length, Kind::String));
            return length;
        }
        if let Some(end) = placeholder_end(script, at) {
            if piece < at {
                out.push((piece..at, Kind::String));
            }
            out.push((at..end, Kind::Placeholder));
            piece = end;
            at = end;
            continue;
        }
        match bytes[at] {
            b'\\' => at = (at + 2).min(length),
            b'"' => {
                out.push((piece..at + 1, Kind::String));
                return at + 1;
            }
            b'$' => match variable_end(bytes, at) {
                Some(end) => {
                    if piece < at {
                        out.push((piece..at, Kind::String));
                    }
                    out.push((at..end, Kind::Variable));
                    piece = end;
                    at = end;
                }
                None => at += 1,
            },
            _ => at += 1,
        }
    }
}

/// The script editor's highlighter: the pieces of the text as it stands.
#[derive(Debug, Default)]
pub struct ShellHighlighter {
    tokens: Vec<(Range<usize>, Kind)>,
}

impl InputHighlighter for ShellHighlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        // A script is short: read it whole after each change.
        self.tokens = tokens(&text.to_string());
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut runs = Vec::new();
        let mut at = range.start;
        for (piece, kind) in &self.tokens {
            if piece.end <= range.start || piece.start >= range.end {
                continue;
            }
            let start = piece.start.max(range.start);
            let end = piece.end.min(range.end);
            if at < start {
                runs.push((at..start, HighlightStyle::default()));
            }
            runs.push((
                start..end,
                resolver.style(kind.style_name()).unwrap_or_default(),
            ));
            at = end;
        }
        if at < range.end {
            runs.push((at..range.end, HighlightStyle::default()));
        }
        runs
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(script: &str) -> Vec<(&str, Kind)> {
        tokens(script)
            .into_iter()
            .map(|(range, kind)| (&script[range], kind))
            .collect()
    }

    #[test]
    fn a_script_reads_as_its_pieces() {
        assert_eq!(
            kinds("if [ -f \"$HOME/x\" ]; then echo 'ok' # done\nfi"),
            [
                ("if", Kind::Keyword),
                ("\"", Kind::String),
                ("$HOME", Kind::Variable),
                ("/x\"", Kind::String),
                (";", Kind::Operator),
                ("then", Kind::Keyword),
                ("'ok'", Kind::String),
                ("# done", Kind::Comment),
                ("fi", Kind::Keyword),
            ]
        );
        assert_eq!(
            kinds(
                "sudo systemctl restart {{service}} && tail -n 50 \"/var/{{log}}\" | grep ${X:-y}"
            ),
            [
                ("{{service}}", Kind::Placeholder),
                ("&&", Kind::Operator),
                ("50", Kind::Number),
                ("\"/var/", Kind::String),
                ("{{log}}", Kind::Placeholder),
                ("\"", Kind::String),
                ("|", Kind::Operator),
                ("${X:-y}", Kind::Variable),
            ]
        );
        // Not a keyword inside a word, not a comment inside one.
        assert_eq!(kinds("fifo=a#b"), []);
        assert_eq!(
            kinds("echo $(date) $1 $?"),
            [
                ("$(", Kind::Operator),
                (")", Kind::Operator),
                ("$1", Kind::Variable),
                ("$?", Kind::Variable),
            ]
        );
        // Unterminated quotes run to the end; text outside ASCII is kept.
        assert_eq!(kinds("echo \"привет"), [("\"привет", Kind::String)]);
        assert_eq!(kinds("echo 'a"), [("'a", Kind::String)]);
        assert_eq!(
            kinds("{{ not a name }} {{ok}}"),
            [("{{ok}}", Kind::Placeholder)]
        );
    }

    /// The runs cover the asked range, in order, without overlaps.
    #[test]
    fn styles_cover_the_range() {
        struct Names;
        impl HighlightStyleResolver for Names {
            fn style(&self, _: &str) -> Option<HighlightStyle> {
                Some(HighlightStyle {
                    font_weight: Some(gpui::FontWeight::BOLD),
                    ..HighlightStyle::default()
                })
            }
        }
        let script = "echo $HOME # x";
        let highlighter = ShellHighlighter {
            tokens: tokens(script),
        };
        let runs = highlighter.styles(&(2..script.len()), &Names);
        assert_eq!(runs.first().map(|run| run.0.start), Some(2));
        assert_eq!(runs.last().map(|run| run.0.end), Some(script.len()));
        for pair in runs.windows(2) {
            assert_eq!(pair[0].0.end, pair[1].0.start);
        }
        assert!(runs.iter().any(|(range, style)| {
            &script[range.clone()] == "$HOME" && style.font_weight.is_some()
        }));
    }
}
