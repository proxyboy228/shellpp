//! Autocomplete (beta; UI 4.2 and 12.4; run 8 decision D142): while the
//! user types a command line, the rest of the newest command of this
//! host's local history that starts with what is typed is drawn after the
//! cursor in a muted color; Tab or → types it.
//!
//! - Off by default (`terminal.autocomplete` = `off`; `history` turns it on).
//! - Only for sessions to saved hosts, whose history is kept (the session
//!   layer sets a [`HistorySource`]); the list is read once, when the first
//!   suggestion is wanted, and commands recorded during the session are
//!   added to it.
//! - The typed text is read from the screen like the history capture's
//!   ([`TerminalHandle::typed_line`]): only on the line the capture follows,
//!   with the cursor at its end, never in the alternate screen or at a
//!   password prompt.
//! - A command that the history filter takes for a secret (docs/security.md,
//!   section 10.2) is never suggested, even if it is in the history (an
//!   older entry, an import).
//!
//! Nothing here logs commands or typed text.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gpui::{AppContext as _, Context};
use shellpp_core::history::{MAX_COMMAND_CHARS, looks_like_secret};
use zeroize::Zeroize as _;

use super::model::TerminalHandle;
use super::view::TerminalView;

/// Most commands kept for suggestions.
pub const MAX_COMMANDS: usize = 5000;

/// Where a terminal's suggestions come from: the local history of its
/// host, newest first.
pub trait HistorySource: Send + Sync + 'static {
    /// Reads the commands (blocking: called off the UI thread).
    fn commands(&self) -> Vec<String>;
}

/// The commands suggestions are taken from, newest first, each once.
/// Wiped when dropped.
#[derive(Default)]
pub struct Suggestions {
    commands: Vec<String>,
}

impl std::fmt::Debug for Suggestions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Suggestions")
            .field("commands", &self.commands.len())
            .finish()
    }
}

impl Drop for Suggestions {
    fn drop(&mut self) {
        for command in &mut self.commands {
            command.zeroize();
        }
    }
}

/// Whether a command may be suggested: one line, not empty, not starting
/// with a space (such lines are never recorded), not too long, not a
/// secret.
fn suggestible(command: &str) -> bool {
    !command.trim().is_empty()
        && !command.starts_with(' ')
        && !command.contains(['\n', '\r'])
        && command.chars().count() <= MAX_COMMAND_CHARS
        && !looks_like_secret(command)
}

impl Suggestions {
    /// The list of `commands` (newest first): unsuitable ones and repeats
    /// left out, at most [`MAX_COMMANDS`].
    pub fn new(commands: impl IntoIterator<Item = String>) -> Self {
        let mut list = Self::default();
        for mut command in commands {
            if list.commands.len() >= MAX_COMMANDS {
                command.zeroize();
                break;
            }
            if suggestible(&command) && !list.commands.contains(&command) {
                list.commands.push(command);
            } else {
                command.zeroize();
            }
        }
        list
    }

    /// A command recorded now becomes the newest.
    pub fn remember(&mut self, command: &str) {
        if !suggestible(command) {
            return;
        }
        if let Some(at) = self.commands.iter().position(|known| known == command) {
            let mut known = self.commands.remove(at);
            known.zeroize();
        }
        self.commands.insert(0, command.to_owned());
        if self.commands.len() > MAX_COMMANDS
            && let Some(mut oldest) = self.commands.pop()
        {
            oldest.zeroize();
        }
    }

    /// The rest of the newest command that starts with `typed` and is
    /// longer; `None` when nothing is typed.
    pub fn complete(&self, typed: &str) -> Option<String> {
        if typed.trim().is_empty() {
            return None;
        }
        self.commands
            .iter()
            .find(|command| command.len() > typed.len() && command.starts_with(typed))
            .map(|command| command[typed.len()..].to_owned())
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[derive(Default)]
struct SlotState {
    source: Option<Arc<dyn HistorySource>>,
    list: Option<Suggestions>,
    loading: bool,
}

/// A terminal's history source and its list (shared by the view and the
/// session layer).
#[derive(Default)]
pub struct Slot {
    state: Mutex<SlotState>,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot").finish_non_exhaustive()
    }
}

impl Slot {
    fn lock(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A command the session recorded: added to a loaded list.
    pub fn remember(&self, command: &str) {
        if let Some(list) = self.lock().list.as_mut() {
            list.remember(command);
        }
    }
}

impl TerminalHandle {
    /// The suggestions of this terminal (shared with the session layer).
    pub fn autocomplete(&self) -> Arc<Slot> {
        self.outbox().autocomplete.clone()
    }

    /// Where suggestions come from; `None` for local terminals and quick
    /// connects (no history). The list is read again when next needed.
    pub fn set_history_source(&self, source: Option<Arc<dyn HistorySource>>) {
        let mut state = self.outbox().autocomplete.lock();
        state.source = source;
        state.list = None;
        state.loading = false;
    }
}

impl TerminalView {
    /// The suggestion to draw after the cursor now: the rest of a command
    /// of the history, when autocomplete is on and the typed line allows
    /// it. Starts reading the history the first time.
    pub fn suggestion(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if !self.settings().autocomplete {
            return None;
        }
        let slot = self.handle().autocomplete();
        {
            let mut state = slot.lock();
            if state.list.is_none() {
                if !state.loading
                    && let Some(source) = state.source.clone()
                {
                    state.loading = true;
                    let load = cx.background_spawn(async move { source.commands() });
                    let slot = slot.clone();
                    cx.spawn(async move |view, cx| {
                        let list = Suggestions::new(load.await);
                        {
                            let mut state = slot.lock();
                            if state.loading {
                                state.list = Some(list);
                                state.loading = false;
                            }
                        }
                        let _ = view.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                }
                return None;
            }
        }
        let typed = self.handle().typed_line()?;
        let state = slot.lock();
        state.list.as_ref()?.complete(&typed)
    }

    /// Tab or →: types the suggestion when one is shown; `false` (the key
    /// goes to the shell) otherwise.
    pub fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(rest) = self.suggestion(cx) else {
            return false;
        };
        // Typed input: the user chose it, and the line is recorded as any
        // typed line is.
        self.type_bytes(rest.into_bytes(), cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(commands: &[&str]) -> Suggestions {
        Suggestions::new(commands.iter().map(|command| (*command).to_owned()))
    }

    #[test]
    fn the_newest_matching_command_is_completed() {
        let list = list(&[
            "git status",
            "git stash pop",
            "git status",
            "ls -la",
            " secret-not-recorded",
            "",
        ]);
        assert_eq!(list.len(), 3);
        assert_eq!(list.complete("git st").as_deref(), Some("atus"));
        assert_eq!(list.complete("git sta").as_deref(), Some("tus"));
        assert_eq!(list.complete("ls").as_deref(), Some(" -la"));
        // Nothing typed, an exact match, or no match: no suggestion.
        assert_eq!(list.complete(""), None);
        assert_eq!(list.complete("  "), None);
        assert_eq!(list.complete("ls -la"), None);
        assert_eq!(list.complete("cd"), None);
    }

    #[test]
    fn secrets_are_never_suggested() {
        let token = "ghp_0123456789abcdefABCDEF0123456789abcd";
        let secret = format!("export GITHUB_TOKEN={token}");
        let mut list = list(&[&secret, "export PATH=$PATH:/opt/bin"]);
        assert_eq!(list.len(), 1);
        assert_eq!(
            list.complete("export ").as_deref(),
            Some("PATH=$PATH:/opt/bin")
        );
        list.remember(&secret);
        assert_eq!(list.complete("export G"), None);
    }

    #[test]
    fn a_recorded_command_becomes_the_newest() {
        let mut list = list(&["make test", "make build"]);
        assert_eq!(list.complete("make ").as_deref(), Some("test"));
        list.remember("make build");
        assert_eq!(list.complete("make ").as_deref(), Some("build"));
        assert_eq!(list.len(), 2);
        list.remember("make lint");
        assert_eq!(list.complete("make ").as_deref(), Some("lint"));
    }
}
