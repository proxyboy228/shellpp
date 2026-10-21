//! What a file panel shows of a folder (UI 5.3, 5.5; night decision D48),
//! for the local and the remote panel alike, without I/O:
//!
//! - [`sorted`]: folders (and links to folders) first in both directions,
//!   then by the column; names in natural order (`file2` before `file10`),
//!   ignoring case; equal values by name;
//! - [`visible`]: the filter by name (case-insensitive) and Show Hidden
//!   Files (off by default);
//! - [`History`]: back and forward;
//! - [`parent`] (the ".." row) and [`breadcrumbs`], POSIX and Windows paths
//!   (a Windows path starts with its drive, whose crumb opens the drive
//!   selection);
//! - the texts of a row: [`Entry::size_text`] (bytes, KB, MB, GB; a dash for
//!   folders), [`Entry::kind_text`] (the extension, "file" or "folder") and
//!   [`mode_text`] (`-rw-r--r--`, remote panel only).

use std::cmp::Ordering;

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    /// A symbolic link; a link to a folder sorts and opens as a folder.
    Link {
        to_folder: bool,
    },
    Other,
}

/// One row of a panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// Modification time, Unix seconds.
    pub modified: Option<u64>,
    /// Permission bits (remote panel).
    pub permissions: Option<u32>,
    /// Hidden: a dot-file, or on Windows the hidden or system attribute.
    pub hidden: bool,
}

impl Entry {
    /// A folder, or a link to one.
    pub fn is_folder_like(&self) -> bool {
        matches!(self.kind, Kind::Folder | Kind::Link { to_folder: true })
    }

    /// The Size column: a dash for folders.
    pub fn size_text(&self) -> String {
        if self.is_folder_like() {
            "—".to_owned()
        } else {
            human_size(self.size)
        }
    }

    /// The Kind column: "folder", the extension (lowercase), or "file".
    pub fn kind_text(&self) -> String {
        if self.is_folder_like() {
            return "folder".to_owned();
        }
        match self.name.rfind('.') {
            Some(at) if at > 0 && at + 1 < self.name.len() => self.name[at + 1..].to_lowercase(),
            _ => "file".to_owned(),
        }
    }
}

/// Bytes in the units of UI 5.3: `512 bytes`, `1.5 KB`, `12 MB`, `1.2 GB`
/// (1024-based; one decimal below 10).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return if bytes == 1 {
            "1 byte".to_owned()
        } else {
            format!("{bytes} bytes")
        };
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// `-rw-r--r--` for a mode (`d` for a folder, `l` for a link).
pub fn mode_text(mode: u32, kind: Kind) -> String {
    let mut out = String::with_capacity(10);
    out.push(match kind {
        Kind::Folder => 'd',
        Kind::Link { .. } => 'l',
        _ => '-',
    });
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        out.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    out
}

/// Names in natural order: case-insensitive, runs of digits by their value
/// (`file2` < `file10`); equal ones by their exact text.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        chars.next();
                    }
                    digits
                };
                let (x, y) = (take(&mut left), take(&mut right));
                let (x_trim, y_trim) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = x_trim
                    .len()
                    .cmp(&y_trim.len())
                    .then_with(|| x_trim.cmp(y_trim));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left.next();
                right.next();
            }
        }
    }
}

/// The column a panel sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Name,
    Modified,
    Size,
    Kind,
}

/// The sort of a panel: a column and its direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            column: Column::Name,
            descending: false,
        }
    }
}

impl Sort {
    /// A click on a column header: the same column flips the direction,
    /// another one sorts by it ascending.
    pub fn clicked(self, column: Column) -> Self {
        if self.column == column {
            Self {
                column,
                descending: !self.descending,
            }
        } else {
            Self {
                column,
                descending: false,
            }
        }
    }
}

/// `entries` in the order of `sort`, folders first.
pub fn sorted(entries: &[Entry], sort: Sort) -> Vec<Entry> {
    let mut out = entries.to_vec();
    out.sort_by(|a, b| {
        let folders = b.is_folder_like().cmp(&a.is_folder_like());
        if folders != Ordering::Equal {
            return folders;
        }
        let by_column = match sort.column {
            Column::Name => natural_cmp(&a.name, &b.name),
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Size => a.size.cmp(&b.size),
            Column::Kind => a.kind_text().cmp(&b.kind_text()),
        };
        let by_column = if sort.descending {
            by_column.reverse()
        } else {
            by_column
        };
        by_column.then_with(|| natural_cmp(&a.name, &b.name))
    });
    out
}

/// The entries the panel shows: those whose name holds `filter` (ignoring
/// case; empty: all), the hidden ones only with `show_hidden`.
pub fn visible<'a>(entries: &'a [Entry], filter: &str, show_hidden: bool) -> Vec<&'a Entry> {
    let filter = filter.trim().to_lowercase();
    entries
        .iter()
        .filter(|entry| show_hidden || !entry.hidden)
        .filter(|entry| filter.is_empty() || entry.name.to_lowercase().contains(&filter))
        .collect()
}

/// Back and forward of a panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    current: String,
    back: Vec<String>,
    forward: Vec<String>,
}

impl History {
    pub fn new(start: &str) -> Self {
        Self {
            current: start.to_owned(),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    /// Opens `path`: the current folder goes to Back, Forward is cleared
    /// (nothing happens for the folder that is open).
    pub fn go(&mut self, path: &str) {
        if path == self.current {
            return;
        }
        self.back
            .push(std::mem::replace(&mut self.current, path.to_owned()));
        self.forward.clear();
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Back: the folder to open, if any.
    pub fn back(&mut self) -> Option<&str> {
        let previous = self.back.pop()?;
        self.forward
            .push(std::mem::replace(&mut self.current, previous));
        Some(&self.current)
    }

    /// Forward: the folder to open, if any.
    pub fn forward(&mut self) -> Option<&str> {
        let next = self.forward.pop()?;
        self.back.push(std::mem::replace(&mut self.current, next));
        Some(&self.current)
    }
}

/// The path style of a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    /// `/home/user` (every remote panel; the local one on Unix).
    Posix,
    /// `C:\Users\user` (the local panel on Windows).
    Windows,
}

fn separator(style: PathStyle) -> char {
    match style {
        PathStyle::Posix => '/',
        PathStyle::Windows => '\\',
    }
}

/// The root of `path` (`/`, or `C:\`), and the rest.
fn split_root(path: &str, style: PathStyle) -> (String, &str) {
    match style {
        PathStyle::Posix => match path.strip_prefix('/') {
            Some(rest) => ("/".to_owned(), rest),
            None => (String::new(), path),
        },
        PathStyle::Windows => {
            let bytes = path.as_bytes();
            if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
                let drive = path[..2].to_uppercase();
                (
                    format!("{drive}\\"),
                    path[2..].trim_start_matches(['\\', '/']),
                )
            } else {
                (String::new(), path)
            }
        }
    }
}

/// The folder above `path` (the ".." row); `None` at the root.
pub fn parent(path: &str, style: PathStyle) -> Option<String> {
    let (root, rest) = split_root(path, style);
    let separators: &[char] = match style {
        PathStyle::Posix => &['/'],
        PathStyle::Windows => &['\\', '/'],
    };
    let rest = rest.trim_end_matches(separators);
    if rest.is_empty() {
        return None;
    }
    Some(match rest.rfind(separators) {
        Some(at) => format!("{root}{}", &rest[..at]),
        None if root.is_empty() => ".".to_owned(),
        None => root,
    })
}

/// One crumb of the path bar: its label and the folder it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub path: String,
    /// The drive of a Windows path: its click opens the drive selection.
    pub drive: bool,
}

/// The crumbs of `path`, the root first.
pub fn breadcrumbs(path: &str, style: PathStyle) -> Vec<Crumb> {
    let (root, rest) = split_root(path, style);
    let sep = separator(style);
    let mut out = Vec::new();
    let mut current = root.clone();
    if !root.is_empty() {
        out.push(Crumb {
            label: match style {
                PathStyle::Posix => "/".to_owned(),
                PathStyle::Windows => root.trim_end_matches('\\').to_owned(),
            },
            path: root.clone(),
            drive: style == PathStyle::Windows,
        });
    }
    let separators: &[char] = match style {
        PathStyle::Posix => &['/'],
        PathStyle::Windows => &['\\', '/'],
    };
    for part in rest.split(separators).filter(|part| !part.is_empty()) {
        if !current.is_empty() && !current.ends_with(sep) {
            current.push(sep);
        }
        current.push_str(part);
        out.push(Crumb {
            label: part.to_owned(),
            path: current.clone(),
            drive: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: Kind, size: u64, modified: u64) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            size,
            modified: Some(modified),
            permissions: None,
            hidden: name.starts_with('.'),
        }
    }

    #[test]
    fn natural_order_ignores_case_and_reads_numbers() {
        let mut names = vec!["file10", "File2", "file1", "a", "B", "file02b"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["a", "B", "file1", "File2", "file02b", "file10"]);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
        assert_eq!(natural_cmp("отчёт 2", "отчёт 10"), Ordering::Less);
    }

    #[test]
    fn folders_come_first_in_both_directions() {
        let entries = vec![
            entry("b.txt", Kind::File, 10, 3),
            entry("src", Kind::Folder, 0, 1),
            entry("a.log", Kind::File, 30, 2),
            entry("link", Kind::Link { to_folder: true }, 0, 4),
            entry("file-link", Kind::Link { to_folder: false }, 5, 5),
        ];
        let names = |sort: Sort| -> Vec<String> {
            sorted(&entries, sort)
                .into_iter()
                .map(|entry| entry.name)
                .collect()
        };
        assert_eq!(
            names(Sort::default()),
            ["link", "src", "a.log", "b.txt", "file-link"]
        );
        let by_size_down = Sort::default().clicked(Column::Size).clicked(Column::Size);
        assert!(by_size_down.descending);
        assert_eq!(
            names(by_size_down),
            ["link", "src", "a.log", "b.txt", "file-link"]
        );
        assert_eq!(
            names(Sort {
                column: Column::Modified,
                descending: false
            }),
            ["src", "link", "a.log", "b.txt", "file-link"]
        );
        assert_eq!(
            names(Sort {
                column: Column::Kind,
                descending: false
            }),
            ["link", "src", "file-link", "a.log", "b.txt"]
        );
    }

    #[test]
    fn the_filter_and_hidden_files() {
        let entries = vec![
            entry(".bashrc", Kind::File, 1, 1),
            entry("Notes.txt", Kind::File, 1, 1),
            entry("notebook", Kind::Folder, 0, 1),
            entry("photo.png", Kind::File, 1, 1),
        ];
        let names = |filter: &str, hidden: bool| -> Vec<&str> {
            visible(&entries, filter, hidden)
                .into_iter()
                .map(|entry| entry.name.as_str())
                .collect()
        };
        assert_eq!(names("", false), ["Notes.txt", "notebook", "photo.png"]);
        assert_eq!(names("", true).len(), 4);
        assert_eq!(names(" NOTE ", false), ["Notes.txt", "notebook"]);
        assert_eq!(names("bash", false), Vec::<&str>::new());
        assert_eq!(names("bash", true), [".bashrc"]);
    }

    #[test]
    fn back_and_forward() {
        let mut history = History::new("/home/u");
        assert!(!history.can_back());
        history.go("/home/u/a");
        history.go("/home/u/a/b");
        history.go("/home/u/a/b");
        assert_eq!(history.back(), Some("/home/u/a"));
        assert_eq!(history.back(), Some("/home/u"));
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), Some("/home/u/a"));
        // A new folder clears Forward.
        history.go("/tmp");
        assert!(!history.can_forward());
        assert_eq!(history.current(), "/tmp");
        assert_eq!(history.back(), Some("/home/u/a"));
    }

    #[test]
    fn parents_and_crumbs_of_both_path_styles() {
        assert_eq!(
            parent("/home/u/a", PathStyle::Posix).as_deref(),
            Some("/home/u")
        );
        assert_eq!(parent("/home", PathStyle::Posix).as_deref(), Some("/"));
        assert_eq!(parent("/", PathStyle::Posix), None);
        assert_eq!(
            parent("/home/u/", PathStyle::Posix).as_deref(),
            Some("/home")
        );
        assert_eq!(
            parent(r"C:\Users\a", PathStyle::Windows).as_deref(),
            Some(r"C:\Users")
        );
        assert_eq!(
            parent(r"C:\Users", PathStyle::Windows).as_deref(),
            Some(r"C:\")
        );
        assert_eq!(parent(r"C:\", PathStyle::Windows), None);

        let crumbs = breadcrumbs("/home/u/a", PathStyle::Posix);
        let pairs: Vec<(&str, &str)> = crumbs
            .iter()
            .map(|crumb| (crumb.label.as_str(), crumb.path.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("/", "/"),
                ("home", "/home"),
                ("u", "/home/u"),
                ("a", "/home/u/a")
            ]
        );
        let crumbs = breadcrumbs(r"c:\Users\Отчёты", PathStyle::Windows);
        let pairs: Vec<(&str, &str, bool)> = crumbs
            .iter()
            .map(|crumb| (crumb.label.as_str(), crumb.path.as_str(), crumb.drive))
            .collect();
        assert_eq!(
            pairs,
            [
                ("C:", r"C:\", true),
                ("Users", r"C:\Users", false),
                ("Отчёты", r"C:\Users\Отчёты", false)
            ]
        );
    }

    #[test]
    fn row_texts() {
        assert_eq!(human_size(0), "0 bytes");
        assert_eq!(human_size(1), "1 byte");
        assert_eq!(human_size(512), "512 bytes");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(12 * 1024 * 1024), "12 MB");
        assert_eq!(human_size(1288490188), "1.2 GB");
        let folder = entry("src", Kind::Folder, 4096, 1);
        assert_eq!(folder.size_text(), "—");
        assert_eq!(folder.kind_text(), "folder");
        assert_eq!(entry("A.TAR.GZ", Kind::File, 1, 1).kind_text(), "gz");
        assert_eq!(entry("Makefile", Kind::File, 1, 1).kind_text(), "file");
        assert_eq!(entry(".bashrc", Kind::File, 1, 1).kind_text(), "file");
        assert_eq!(mode_text(0o644, Kind::File), "-rw-r--r--");
        assert_eq!(mode_text(0o755, Kind::Folder), "drwxr-xr-x");
        assert_eq!(
            mode_text(0o777, Kind::Link { to_folder: true }),
            "lrwxrwxrwx"
        );
    }
}
