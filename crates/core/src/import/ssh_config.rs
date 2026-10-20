//! The OpenSSH client configuration (`ssh_config(5)`) for the import of
//! `~/.ssh/config` (SPEC "Import"; UI 2.5; night plan, stage 3 item 10;
//! night decision D35): the parser and the evaluation of the options of each
//! host, as `ssh` evaluates them.
//!
//! - Lines are `Keyword value` or `Keyword=value`, keywords case-insensitive,
//!   values with double quotes, `#` comments.
//! - `Include` (files and `*`/`?` patterns in the file name, relative paths
//!   from the folder of the main file, `~` for the home folder) is expanded in
//!   place, at most 16 levels deep and never twice in one chain.
//! - `Host` patterns (`*`, `?`, `!` negation, case-insensitive) and a basic
//!   `Match`: `all`, `host`, `originalhost` and `user` with their patterns and
//!   `!`; a `Match` with another criterion (`exec`, `localuser`, `canonical`,
//!   `final`, ...) never applies and is reported.
//! - For each concrete alias (a `Host` pattern without wildcards or
//!   negation) every matching block applies in file order; the first value
//!   of a keyword wins, except the keywords that add up (`IdentityFile`,
//!   `CertificateFile`, the forwardings, `SendEnv`, `SetEnv`).
//! - Patterns (`Host *`, `Host *.example.com`) are templates: their options
//!   apply to the hosts they match and they never become hosts.
//!
//! Nothing here logs paths or values.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Most `Include` levels.
pub const MAX_INCLUDE_DEPTH: usize = 16;
/// The largest config file read (each file).
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Most lines over all files.
pub const MAX_LINES: usize = 200_000;

/// Keywords whose values add up instead of the first one winning.
const CUMULATIVE: &[&str] = &[
    "identityfile",
    "certificatefile",
    "localforward",
    "remoteforward",
    "dynamicforward",
    "sendenv",
    "setenv",
];

/// One option line (a directive): lowercase keyword, its arguments, where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    pub keyword: String,
    pub args: Vec<String>,
    pub line: Location,
}

/// A line of a file (1-based), for the problems list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    pub line: usize,
}

/// What a block applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// Lines before the first `Host` or `Match`: every host.
    Global,
    /// `Host` patterns.
    Host(Vec<String>),
    /// `Match` criteria: (negated, criterion, patterns); `None` when the
    /// block uses a criterion that is not supported.
    Match(Option<Vec<MatchCriterion>>),
}

/// One `Match` criterion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchCriterion {
    pub negated: bool,
    pub kind: MatchKind,
    pub patterns: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    All,
    Host,
    OriginalHost,
    User,
}

/// A `Host` or `Match` block and its options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub condition: Condition,
    pub options: Vec<Directive>,
    pub line: Location,
}

/// A problem found while reading (shown in the import's list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub line: Location,
    pub kind: ProblemKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// A keyword without a value, or an unterminated quote.
    Malformed,
    /// An `Include` that matched no readable file.
    IncludeMissing,
    /// `Include` deeper than [`MAX_INCLUDE_DEPTH`] or in a loop.
    IncludeLoop,
    /// An `Include` of a network path ([`is_network_path`]), not followed.
    IncludeNetwork,
    /// A file over [`MAX_FILE_BYTES`], or too many lines.
    TooLarge,
    /// A `Match` criterion this import does not evaluate; the block never applies.
    MatchUnsupported(String),
}

/// A whole configuration: its blocks in file order (includes expanded).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub blocks: Vec<Block>,
    pub problems: Vec<Problem>,
}

/// Splits a line into words: whitespace or one `=` after the keyword
/// separate, double quotes group. `None` for an unterminated quote.
fn words(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    // One `=` may separate the keyword from its value, with or without
    // spaces around it (`Port=22`, `Port = 22`).
    let mut equals_used = false;
    for c in line.chars() {
        let keyword_separator =
            c == '=' && !equals_used && (out.is_empty() || (out.len() == 1 && !started));
        if keyword_separator && !quoted {
            equals_used = true;
        }
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if !quoted && (c.is_whitespace() || keyword_separator) => {
                if started {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started {
        out.push(current);
    }
    Some(out)
}

/// `~/` and `~` at the start become the home folder.
/// A path that names another machine: `\\server\share`, `//server/share`
/// (and the `\\?\UNC\` and `\\.\` forms, which start the same way). An
/// import never opens one: on Windows that alone connects to the server
/// and offers it the user's NTLM credentials (security.md 13.5, "Include
/// follows only local files").
pub fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let mut start = text.chars().take(2);
    matches!(
        (start.next(), start.next()),
        (Some('\\' | '/'), Some('\\' | '/'))
    )
}

pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// OpenSSH wildcard matching (`*`, `?`), case-insensitive.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// A pattern list (`a,b,!c` or words): matches when a positive pattern
/// matches and no negated one does.
pub fn pattern_list_matches(patterns: &[String], text: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.iter().flat_map(|item| item.split(',')) {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        if let Some(negated) = pattern.strip_prefix('!') {
            if wildcard(negated, text) {
                return false;
            }
        } else if wildcard(pattern, text) {
            matched = true;
        }
    }
    matched
}

/// `true` for a pattern that names one host (no wildcard, no negation).
pub fn is_concrete(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.contains(['*', '?', '!', ','])
}

fn parse_match(args: &[String]) -> Result<Vec<MatchCriterion>, String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let raw = args[index].to_lowercase();
        let (negated, name) = match raw.strip_prefix('!') {
            Some(name) => (true, name.to_owned()),
            None => (false, raw),
        };
        let kind = match name.as_str() {
            "all" => {
                out.push(MatchCriterion {
                    negated,
                    kind: MatchKind::All,
                    patterns: Vec::new(),
                });
                index += 1;
                continue;
            }
            "host" => MatchKind::Host,
            "originalhost" => MatchKind::OriginalHost,
            "user" => MatchKind::User,
            other => return Err(other.chars().take(32).collect()),
        };
        let patterns = args
            .get(index + 1)
            .map(|value| vec![value.clone()])
            .ok_or_else(|| name.clone())?;
        out.push(MatchCriterion {
            negated,
            kind,
            patterns,
        });
        index += 2;
    }
    Ok(out)
}

/// Lists the files of an `Include` argument: a plain path, or a `*`/`?`
/// pattern in the file name (sorted, as `glob(3)` returns them).
fn include_files(argument: &str, base: &Path, home: &Path) -> Vec<PathBuf> {
    let path = expand_home(argument, home);
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return if path.is_file() {
            vec![path]
        } else {
            Vec::new()
        };
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|file| wildcard(name, file))
        })
        .collect();
    files.sort();
    files
}

struct Reader<'a> {
    base: PathBuf,
    home: &'a Path,
    config: Config,
    lines: usize,
}

impl Reader<'_> {
    fn current(&mut self, file: &Path, line: usize) -> &mut Block {
        if self.config.blocks.is_empty() {
            self.config.blocks.push(Block {
                condition: Condition::Global,
                options: Vec::new(),
                line: Location {
                    file: file.to_path_buf(),
                    line,
                },
            });
        }
        self.config.blocks.last_mut().expect("a block")
    }

    fn read(&mut self, file: &Path, chain: &mut Vec<PathBuf>, at: Option<Location>) {
        let location = |line| Location {
            file: file.to_path_buf(),
            line,
        };
        let problem = |kind, line: Option<Location>| Problem {
            line: line.unwrap_or_else(|| location(0)),
            kind,
        };
        if chain.len() >= MAX_INCLUDE_DEPTH || chain.iter().any(|seen| seen == file) {
            self.config
                .problems
                .push(problem(ProblemKind::IncludeLoop, at));
            return;
        }
        let too_large = std::fs::metadata(file).is_ok_and(|meta| meta.len() > MAX_FILE_BYTES);
        let text = match (too_large, std::fs::read(file)) {
            (false, Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            (true, _) => {
                self.config
                    .problems
                    .push(problem(ProblemKind::TooLarge, at));
                return;
            }
            (false, Err(_)) => {
                self.config
                    .problems
                    .push(problem(ProblemKind::IncludeMissing, at));
                return;
            }
        };
        chain.push(file.to_path_buf());
        for (index, raw) in text.lines().enumerate() {
            let number = index + 1;
            self.lines += 1;
            if self.lines > MAX_LINES {
                self.config
                    .problems
                    .push(problem(ProblemKind::TooLarge, Some(location(number))));
                break;
            }
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(mut parts) = words(line) else {
                self.config
                    .problems
                    .push(problem(ProblemKind::Malformed, Some(location(number))));
                continue;
            };
            if parts.is_empty() {
                continue;
            }
            let keyword = parts.remove(0).to_lowercase();
            if parts.is_empty() {
                self.config
                    .problems
                    .push(problem(ProblemKind::Malformed, Some(location(number))));
                continue;
            }
            match keyword.as_str() {
                "host" => self.config.blocks.push(Block {
                    condition: Condition::Host(parts),
                    options: Vec::new(),
                    line: location(number),
                }),
                "match" => {
                    let criteria = match parse_match(&parts) {
                        Ok(criteria) => Some(criteria),
                        Err(name) => {
                            self.config.problems.push(problem(
                                ProblemKind::MatchUnsupported(name),
                                Some(location(number)),
                            ));
                            None
                        }
                    };
                    self.config.blocks.push(Block {
                        condition: Condition::Match(criteria),
                        options: Vec::new(),
                        line: location(number),
                    });
                }
                "include" => {
                    for argument in &parts {
                        if is_network_path(&expand_home(argument, self.home)) {
                            self.config
                                .problems
                                .push(problem(ProblemKind::IncludeNetwork, Some(location(number))));
                            continue;
                        }
                        let files = include_files(argument, &self.base, self.home);
                        if files.is_empty() && !argument.contains(['*', '?']) {
                            self.config
                                .problems
                                .push(problem(ProblemKind::IncludeMissing, Some(location(number))));
                        }
                        for included in files {
                            self.read(&included, chain, Some(location(number)));
                        }
                    }
                }
                _ => {
                    let option = Directive {
                        keyword,
                        args: parts,
                        line: location(number),
                    };
                    self.current(file, number).options.push(option);
                }
            }
        }
        chain.pop();
    }
}

/// Reads the configuration at `path` with its includes. Relative includes
/// start in the folder of `path` (`~/.ssh` for `~/.ssh/config`).
pub fn read(path: &Path, home: &Path) -> Config {
    let base = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".ssh"));
    let mut reader = Reader {
        base,
        home,
        config: Config::default(),
        lines: 0,
    };
    let mut chain = Vec::new();
    reader.read(path, &mut chain, None);
    reader.config
}

/// The effective options of one host: keyword -> values (one for most
/// keywords, every value for the cumulative ones), in `ssh`'s order.
pub type Options = BTreeMap<String, Vec<Vec<String>>>;

impl Config {
    /// The concrete aliases of the `Host` lines, each with the line it was
    /// first named on: the first alias of each `Host` line that has one
    /// (later lines with the same first alias are left out).
    pub fn hosts(&self) -> Vec<(String, Location)> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for block in &self.blocks {
            if let Condition::Host(patterns) = &block.condition
                && let Some(first) = patterns.iter().find(|pattern| is_concrete(pattern))
                && seen.insert(first.to_lowercase())
            {
                out.push((first.clone(), block.line.clone()));
            }
        }
        out
    }

    /// The options that apply to `alias`, as `ssh alias` would use them.
    pub fn options_for(&self, alias: &str) -> Options {
        let mut options: Options = BTreeMap::new();
        for block in &self.blocks {
            let applies = match &block.condition {
                Condition::Global => true,
                Condition::Host(patterns) => pattern_list_matches(patterns, alias),
                Condition::Match(None) => false,
                Condition::Match(Some(criteria)) => criteria.iter().all(|criterion| {
                    let hit = match criterion.kind {
                        MatchKind::All => true,
                        MatchKind::OriginalHost => pattern_list_matches(&criterion.patterns, alias),
                        MatchKind::Host => {
                            let host = options
                                .get("hostname")
                                .and_then(|values| values.first())
                                .and_then(|args| args.first())
                                .map(|name| name.replace("%h", alias))
                                .unwrap_or_else(|| alias.to_owned());
                            pattern_list_matches(&criterion.patterns, &host)
                        }
                        MatchKind::User => options
                            .get("user")
                            .and_then(|values| values.first())
                            .and_then(|args| args.first())
                            .is_some_and(|user| pattern_list_matches(&criterion.patterns, user)),
                    };
                    hit != criterion.negated
                }),
            };
            if !applies {
                continue;
            }
            for option in &block.options {
                let cumulative = CUMULATIVE.contains(&option.keyword.as_str());
                let entry = options.entry(option.keyword.clone()).or_default();
                if cumulative || entry.is_empty() {
                    entry.push(option.args.clone());
                }
            }
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_quotes() {
        assert_eq!(words("Host a b").unwrap(), ["Host", "a", "b"]);
        assert_eq!(words("Port=2222").unwrap(), ["Port", "2222"]);
        assert_eq!(words("Port = 2222").unwrap(), ["Port", "2222"]);
        assert_eq!(
            words(r#"IdentityFile "C:\Users\a b\key""#).unwrap(),
            ["IdentityFile", r"C:\Users\a b\key"]
        );
        assert_eq!(words("SetEnv A=b").unwrap(), ["SetEnv", "A=b"]);
        assert!(words(r#"User "open"#).is_none());
    }

    #[test]
    fn wildcards_and_pattern_lists() {
        assert!(wildcard("*", "anything"));
        assert!(wildcard("*.example.com", "db.EXAMPLE.com"));
        assert!(!wildcard("*.example.com", "example.com"));
        assert!(wildcard("db?", "db1"));
        assert!(!wildcard("db?", "db12"));
        assert!(wildcard("a*b*c", "aXXbYYc"));
        let list = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(pattern_list_matches(
            &list(&["*.example.com", "!bad.example.com"]),
            "good.example.com"
        ));
        assert!(!pattern_list_matches(
            &list(&["*.example.com", "!bad.example.com"]),
            "bad.example.com"
        ));
        assert!(pattern_list_matches(&list(&["a,b"]), "b"));
        assert!(
            !pattern_list_matches(&list(&["!a"]), "b"),
            "only negations never match"
        );
        assert!(is_concrete("web-1"));
        assert!(!is_concrete("web-*"));
        assert!(!is_concrete("!web"));
    }
}
