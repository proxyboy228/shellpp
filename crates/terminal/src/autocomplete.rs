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

use std::collections::HashSet;
use std::hash::{Hash as _, Hasher as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use shellpp_core::history::{MAX_COMMAND_CHARS, looks_like_secret};
use zeroize::Zeroize as _;

use super::model::TerminalHandle;

/// Most commands kept for suggestions.
pub const MAX_COMMANDS: usize = shellpp_core::history::AUTOCOMPLETE_COMMANDS;

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
    /// left out, at most [`MAX_COMMANDS`]. Checks every command with the
    /// secret detector: build it off the UI thread.
    pub fn new(commands: impl IntoIterator<Item = String>) -> Self {
        let mut list = Self::default();
        let mut seen: HashSet<u64> = HashSet::new();
        for mut command in commands {
            if list.commands.len() >= MAX_COMMANDS {
                command.zeroize();
                continue;
            }
            // Repeats by hash first, then by text (a hash collision only
            // costs a comparison).
            let hash = {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                command.hash(&mut hasher);
                hasher.finish()
            };
            let repeat = !seen.insert(hash) && list.commands.contains(&command);
            if suggestible(&command) && !repeat {
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
    /// Bumped whenever the list is dropped; a load that started under an
    /// older generation is thrown away.
    generation: u64,
    /// The generation a load runs for, if one runs.
    loading: Option<u64>,
    /// Commands recorded while a load runs (added when it lands).
    recorded: Vec<String>,
}

impl SlotState {
    fn forget(&mut self) {
        self.list = None;
        self.loading = None;
        for command in &mut self.recorded {
            command.zeroize();
        }
        self.recorded.clear();
        self.generation += 1;
    }
}

/// A terminal's history source and its list (shared by the view, the
/// parser thread and the session layer).
#[derive(Default)]
pub struct Slot {
    state: Mutex<SlotState>,
    /// Keys were typed since the last output: the screen does not show
    /// them yet (no echo), so no suggestion is drawn or accepted.
    typed_ahead: AtomicBool,
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

    /// A command the session recorded: added to the list (or kept for it
    /// while it loads).
    pub fn remember(&self, command: &str) {
        let mut state = self.lock();
        if let Some(list) = state.list.as_mut() {
            list.remember(command);
        } else if state.loading.is_some() {
            state.recorded.push(command.to_owned());
        }
    }

    /// Drops the list (the app locked, history changed): it is read again
    /// when next needed (docs/security.md, section 8).
    pub fn invalidate(&self) {
        self.lock().forget();
    }

    /// Input went to the session.
    pub fn typed(&self) {
        self.typed_ahead.store(true, Ordering::Release);
    }

    /// The parser took in output.
    pub fn output(&self) {
        self.typed_ahead.store(false, Ordering::Release);
    }

    /// Keys were typed since the last output: no suggestion is drawn or
    /// accepted until the screen shows them.
    pub fn typed_ahead(&self) -> bool {
        self.typed_ahead.load(Ordering::Acquire)
    }

    /// Where the list stands; when it is missing and none loads, the load
    /// starts here ([`ListState::Load`]): the caller builds the list from
    /// the source off its own thread ([`Suggestions::new`]; the secret
    /// detector runs over every command) and hands it to
    /// [`Self::finish_load`]. A caller that gets `Load` must finish it: until
    /// then the slot counts as loading, and only [`Self::invalidate`] starts
    /// over.
    pub fn list_state(&self) -> ListState {
        let mut state = self.lock();
        if state.list.is_some() {
            return ListState::Ready;
        }
        if state.loading.is_some() {
            return ListState::Missing;
        }
        let Some(source) = state.source.clone() else {
            return ListState::Missing;
        };
        let generation = state.generation;
        state.loading = Some(generation);
        ListState::Load { generation, source }
    }

    /// The list a load of `generation` built: kept with the commands
    /// recorded meanwhile, or dropped when the list was dropped since.
    pub fn finish_load(&self, generation: u64, mut list: Suggestions) {
        let mut state = self.lock();
        if state.loading == Some(generation) {
            for command in std::mem::take(&mut state.recorded) {
                list.remember(&command);
                let mut command = command;
                command.zeroize();
            }
            state.list = Some(list);
            state.loading = None;
        }
    }

    /// The rest of the newest command that starts with `typed`, from the
    /// list when it is there.
    pub fn complete(&self, typed: &str) -> Option<String> {
        self.lock().list.as_ref()?.complete(typed)
    }
}

/// Where a slot's list stands ([`Slot::list_state`]).
pub enum ListState {
    /// The list is there.
    Ready,
    /// No list yet: it loads, or there is no source.
    Missing,
    /// No list: build it from `source` and hand it to [`Slot::finish_load`]
    /// with `generation`.
    Load {
        generation: u64,
        source: Arc<dyn HistorySource>,
    },
}

impl std::fmt::Debug for ListState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => f.write_str("Ready"),
            Self::Missing => f.write_str("Missing"),
            Self::Load { generation, .. } => f
                .debug_struct("Load")
                .field("generation", generation)
                .finish_non_exhaustive(),
        }
    }
}

impl TerminalHandle {
    /// The suggestions of this terminal (shared with the session layer).
    pub fn autocomplete(&self) -> Arc<Slot> {
        self.outbox().autocomplete.clone()
    }

    /// Where suggestions come from; `None` for local terminals and quick
    /// connects (no history) and when the session lets go of the terminal.
    /// The list is read again when next needed.
    pub fn set_history_source(&self, source: Option<Arc<dyn HistorySource>>) {
        let mut state = self.outbox().autocomplete.lock();
        state.forget();
        state.source = source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(commands: &[&str]) -> Suggestions {
        Suggestions::new(commands.iter().map(|command| (*command).to_owned()))
    }

    struct Source(Vec<String>);

    impl HistorySource for Source {
        fn commands(&self) -> Vec<String> {
            self.0.clone()
        }
    }

    /// The loading protocol a view drives (`list_state`, `finish_load`;
    /// review of run 13): a command recorded while the list loads joins it
    /// when it lands; a load that started before `invalidate` is dropped and
    /// the next state starts a new one.
    #[test]
    fn a_load_keeps_what_was_recorded_and_a_stale_load_is_dropped() {
        let slot = Slot::default();
        assert!(matches!(slot.list_state(), ListState::Missing), "no source");
        slot.lock().source = Some(Arc::new(Source(vec!["git status".to_owned()])));
        let ListState::Load {
            generation: stale, ..
        } = slot.list_state()
        else {
            panic!("a load starts");
        };
        assert!(matches!(slot.list_state(), ListState::Missing), "it loads");
        slot.remember("echo before the history changed");
        slot.invalidate();
        slot.finish_load(stale, list(&["git status"]));
        assert_eq!(slot.complete("git s"), None, "the stale list is dropped");
        let ListState::Load { generation, source } = slot.list_state() else {
            panic!("a new load starts");
        };
        assert_ne!(generation, stale);
        slot.remember("ls -la /etc");
        slot.finish_load(generation, Suggestions::new(source.commands()));
        assert!(matches!(slot.list_state(), ListState::Ready));
        assert_eq!(slot.complete("git s").as_deref(), Some("tatus"));
        assert_eq!(slot.complete("ls -").as_deref(), Some("la /etc"));
        assert_eq!(slot.complete("echo b"), None, "forgotten with the old list");
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
