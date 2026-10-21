//! The texts of the Snippets screen (UI 9).

// Action bar (9.1).
pub const NEW_SNIPPET: &str = "New snippet";
pub const NEW_MENU: &str = "More to create";
pub const NEW_PACKAGE: &str = "New snippet package";
pub const SEARCH_PLACEHOLDER: &str = "Search snippets";
pub const ALL_SNIPPETS: &str = "Snippets";
pub const SHELL_HISTORY: &str = "Shell History";

// Shell History (9.1).
pub const HISTORY_SEARCH: &str = "Search commands";
pub const ALL_HOSTS: &str = "All hosts";
pub const NO_HOST: &str = "Removed host";
pub const SAVE_AS_SNIPPET: &str = "Save as snippet";
pub const HISTORY_NOTE: &str = "Commands typed in sessions to saved hosts, newest first. \
     Commands that look like secrets are never kept.";
pub const HISTORY_EMPTY: &str = "No commands here yet.";
pub const HISTORY_FAILED: &str = "The history could not be read.";

// Content (9.2).
pub const PACKAGES: &str = "Packages";
pub const SNIPPETS: &str = "Snippets";
pub const EMPTY_TITLE: &str = "No snippets";
pub const EMPTY_BODY: &str = "A snippet is a script you run on your hosts again and again. {{name}} in it is asked for at run time.";
pub const EMPTY_PACKAGE: &str = "This package has no snippets yet.";
pub const NO_MATCHES: &str = "No snippets match the search";
pub const NO_TARGETS: &str = "No targets";
pub const UNTITLED: &str = "Untitled snippet";

/// "3 targets".
pub fn targets(count: usize) -> String {
    match count {
        0 => NO_TARGETS.to_owned(),
        1 => "1 target".to_owned(),
        n => format!("{n} targets"),
    }
}

/// "4 snippets".
pub fn snippet_count(count: usize) -> String {
    match count {
        1 => "1 snippet".to_owned(),
        n => format!("{n} snippets"),
    }
}

// Menus.
pub const RUN: &str = "Run";
pub const EDIT: &str = "Edit";
pub const REMOVE: &str = "Remove";
pub const OPEN: &str = "Open";
pub const RENAME: &str = "Rename";
pub const CANCEL: &str = "Cancel";
pub const REMOVE_SNIPPET_TITLE: &str = "Remove snippet?";
pub const REMOVE_PACKAGE_TITLE: &str = "Remove package?";
pub const REMOVED: &str = "Snippet removed";
pub const PACKAGE_REMOVED: &str = "Package removed";
pub const REMOVE_FAILED: &str = "It could not be removed";
pub const UNDO_FAILED: &str = "It could not be brought back";
pub const SAVE_FAILED: &str = "The snippet could not be saved";

/// "Remove "Restart nginx"?"
pub fn remove_body(title: &str) -> String {
    format!("Remove \"{title}\"?")
}

/// "Remove "Deploy"? Its 3 snippets stay, outside any package."
pub fn remove_package_body(name: &str, snippets: usize) -> String {
    match snippets {
        0 => format!("Remove \"{name}\"?"),
        n => format!(
            "Remove \"{name}\"? Its {} stay, outside any package.",
            snippet_count(n)
        ),
    }
}

// Package dialog.
pub const PACKAGE_TITLE_NEW: &str = "New snippet package";
pub const PACKAGE_TITLE_RENAME: &str = "Rename package";
pub const PACKAGE_NAME: &str = "Name";
pub const PACKAGE_NAME_PLACEHOLDER: &str = "Deploy";
pub const CREATE: &str = "Create";
pub const SAVE: &str = "Save";

// Panel (9.3).
pub const PANEL_NEW: &str = "New snippet";
pub const PANEL_EDIT: &str = "Snippet";
pub const LABEL: &str = "Action description";
pub const LABEL_PLACEHOLDER: &str = "Restart nginx";
pub const PACKAGE: &str = "Package";
pub const NO_PACKAGE: &str = "No package";
pub const TAGS: &str = "Tags";
pub const TAGS_PLACEHOLDER: &str = "web, deploy";
pub const TAGS_NOTE: &str = "Separate tags with commas.";
pub const SCRIPT: &str = "Script";
pub const SCRIPT_NOTE: &str = "{{name}} is asked for each time the snippet runs.";
pub const TARGETS: &str = "Targets for execution";
pub const TARGETS_NOTE: &str =
    "Run executes the script on each of these hosts and shows what it printed.";
pub const ADD_TARGETS: &str = "Add targets";
pub const DONE: &str = "Done";
pub const MISSING_HOST: &str = "Removed host";

/// "Variables: name, port".
pub fn variables(names: &[String]) -> String {
    format!("Variables: {}", names.join(", "))
}

// Field errors.
pub const ERROR_SCRIPT: &str = "Write the script to run.";
pub const ERROR_TOO_LONG: &str = "This is too long.";
pub const ERROR_INVALID: &str = "This value is not valid.";
pub const ERROR_PACKAGE_NAME: &str = "Name the package.";

// Run (9.4).
pub const VARIABLES_TITLE: &str = "Values for the snippet";
pub const VARIABLES_NOTE: &str = "Each {{name}} of the script becomes the value typed here.";
pub const CONFIRM_TITLE: &str = "Run on these hosts?";
pub const RESULTS_TITLE: &str = "Run results";
pub const CLOSE: &str = "Close";
pub const WAITING: &str = "Waiting";
pub const RUNNING: &str = "Running";
pub const SUCCESS: &str = "Success";
pub const FAILED: &str = "Error";
pub const TIMED_OUT: &str = "Timed out";
pub const NO_OUTPUT: &str = "No output.";
pub const OUTPUT_CUT: &str = "The output was cut: only its beginning is kept.";
pub const NOT_CONNECTED: &str = "The run needs the session layer, which is not running.";

/// "Run "Restart nginx" on 6 hosts?"
pub fn confirm_body(title: &str, hosts: usize) -> String {
    format!("Run \"{title}\" on {hosts} hosts?")
}

/// "Exit code 1".
pub fn exit_code(code: u32) -> String {
    format!("Exit code {code}")
}

/// "4 of 6 succeeded".
pub fn summary(succeeded: usize, total: usize) -> String {
    format!("{succeeded} of {total} succeeded")
}
