//! The shortcuts the user can change (UI 12.5; D67, D76): the app's own
//! actions and the terminal's (copy, paste, select all, splits, focus
//! between panes, search in the terminal), each with its keys in the PC
//! and the Mac set, the user's changes in `shortcuts.overrides` (action id
//! → keystroke, or `null` for unassigned), the set in `shortcuts.preset`,
//! conflicts, and how a change takes effect.
//!
//! [`apply`] binds every action's keys over the bindings each module made
//! at start: for each action, the keys either set could have given it and
//! the keys bound last are unbound for that action only (`Unbind` in its
//! key context), then the current keys of every action are bound after
//! them (later bindings win). So a change, a reset and a new set take
//! effect at once and in any order.

use std::collections::HashMap;

use gpui::{App, Global, KeyBinding, Keystroke, Unbind};
use serde_json::{Map, Value};

use super::actions::{
    ActivateTab, CloseTab, KEY_CONTEXT, LockApp, NewLocalTerminal, NewSerialConnection, NewTab,
    NextTab, OpenSearch, OpenSettings, PreviousTab, ShowSftp, ShowVaults, ToggleBroadcast,
};

/// The setting that holds the user's changes.
pub const OVERRIDES_KEY: &str = "shortcuts.overrides";
/// The setting of the set of keys (`pc` or `mac`).
pub const PRESET_KEY: &str = "shortcuts.preset";

/// The set of default keys (UI 12.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Ctrl for the app's keys, Ctrl+Shift in the terminal.
    Pc,
    /// Cmd, as macOS apps use it.
    Mac,
}

impl Preset {
    /// The set of the platform: Mac on macOS, PC elsewhere.
    pub fn platform() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Pc
        }
    }

    /// The stored set; the platform's when there is none.
    pub fn of(values: &Map<String, Value>) -> Self {
        match values.get(PRESET_KEY).and_then(Value::as_str) {
            Some("pc") => Self::Pc,
            Some("mac") => Self::Mac,
            _ => Self::platform(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pc => "pc",
            Self::Mac => "mac",
        }
    }
}

/// Where an action's keys work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The window (the shell's key context).
    App,
    /// Everywhere, over dialogs too (Lock).
    Global,
    /// A terminal that has the focus.
    Terminal,
    /// A tab of panes.
    Panes,
}

impl Scope {
    fn context(self) -> Option<&'static str> {
        match self {
            Scope::App => Some(KEY_CONTEXT),
            Scope::Global => None,
            Scope::Terminal => Some(crate::terminal::view::KEY_CONTEXT),
            Scope::Panes => Some(crate::panes::KEY_CONTEXT),
        }
    }
}

/// The groups of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    App,
    Terminal,
}

/// An action of the shortcuts table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The id in `shortcuts.overrides`.
    pub id: &'static str,
    pub label: &'static str,
    /// The default keys of the PC and of the Mac set.
    pub pc: &'static [&'static str],
    pub mac: &'static [&'static str],
    pub scope: Scope,
    pub group: Group,
}

impl Shortcut {
    pub fn defaults(&self, preset: Preset) -> &'static [&'static str] {
        match preset {
            Preset::Pc => self.pc,
            Preset::Mac => self.mac,
        }
    }
}

const fn app(
    id: &'static str,
    label: &'static str,
    pc: &'static [&'static str],
    mac: &'static [&'static str],
) -> Shortcut {
    Shortcut {
        id,
        label,
        pc,
        mac,
        scope: Scope::App,
        group: Group::App,
    }
}

const fn terminal(
    id: &'static str,
    label: &'static str,
    pc: &'static [&'static str],
    mac: &'static [&'static str],
    scope: Scope,
) -> Shortcut {
    Shortcut {
        id,
        label,
        pc,
        mac,
        scope,
        group: Group::Terminal,
    }
}

/// Every action of the table, in its order.
pub const SHORTCUTS: [Shortcut; 33] = [
    app("show_vaults", "Go to Vaults", &["ctrl-1"], &["cmd-1"]),
    app("show_sftp", "Go to SFTP", &["ctrl-2"], &["cmd-2"]),
    app("tab_3", "Go to tab 3", &["ctrl-3"], &["cmd-3"]),
    app("tab_4", "Go to tab 4", &["ctrl-4"], &["cmd-4"]),
    app("tab_5", "Go to tab 5", &["ctrl-5"], &["cmd-5"]),
    app("tab_6", "Go to tab 6", &["ctrl-6"], &["cmd-6"]),
    app("tab_7", "Go to tab 7", &["ctrl-7"], &["cmd-7"]),
    app("tab_8", "Go to tab 8", &["ctrl-8"], &["cmd-8"]),
    app("tab_9", "Go to tab 9", &["ctrl-9"], &["cmd-9"]),
    app(
        "new_local_terminal",
        "New local terminal",
        &["ctrl-l"],
        &["cmd-l"],
    ),
    Shortcut {
        id: "lock",
        label: "Lock the app",
        pc: &["ctrl-shift-l"],
        mac: &["cmd-shift-l"],
        scope: Scope::Global,
        group: Group::App,
    },
    app(
        "new_serial",
        "New serial connection",
        &["ctrl-alt-s"],
        &["cmd-alt-s"],
    ),
    app(
        "new_tab",
        "New tab",
        &["ctrl-t", "ctrl-shift-t"],
        &["cmd-t", "cmd-shift-t"],
    ),
    app(
        "close_tab",
        "Close tab or pane",
        &["ctrl-shift-w"],
        &["cmd-w"],
    ),
    app("next_tab", "Next tab", &["alt-right"], &["cmd-shift-]"]),
    app(
        "previous_tab",
        "Previous tab",
        &["alt-left"],
        &["cmd-shift-["],
    ),
    app(
        "open_search",
        "Search hosts and tabs",
        &["ctrl-k", "ctrl-shift-k"],
        &["cmd-k", "cmd-shift-k"],
    ),
    app("open_settings", "Settings", &["ctrl-,"], &["cmd-,"]),
    // Outside a terminal: there they go to the program (D96).
    app("zoom_in", "Zoom in", &["ctrl-="], &["cmd-="]),
    app("zoom_out", "Zoom out", &["ctrl--"], &["cmd--"]),
    app("actual_size", "Actual size", &["ctrl-0"], &["cmd-0"]),
    app("open_hosts", "Open the Hosts screen", &[], &[]),
    terminal(
        "copy",
        "Copy",
        &["ctrl-shift-c"],
        &["cmd-c"],
        Scope::Terminal,
    ),
    terminal(
        "paste",
        "Paste",
        &["ctrl-shift-v"],
        &["cmd-v"],
        Scope::Terminal,
    ),
    terminal(
        "select_all",
        "Select all",
        &["ctrl-shift-a"],
        &["cmd-a"],
        Scope::Terminal,
    ),
    terminal(
        "split_right",
        "Split to the right",
        &["ctrl-shift-d"],
        &["cmd-d"],
        Scope::Panes,
    ),
    terminal(
        "split_down",
        "Split down",
        &["ctrl-shift-e"],
        &["cmd-shift-d"],
        Scope::Panes,
    ),
    terminal(
        "focus_left",
        "Pane on the left",
        &["ctrl-alt-left"],
        &["cmd-alt-left"],
        Scope::Panes,
    ),
    terminal(
        "focus_right",
        "Pane on the right",
        &["ctrl-alt-right"],
        &["cmd-alt-right"],
        Scope::Panes,
    ),
    terminal(
        "focus_up",
        "Pane above",
        &["ctrl-alt-up"],
        &["cmd-alt-up"],
        Scope::Panes,
    ),
    terminal(
        "focus_down",
        "Pane below",
        &["ctrl-alt-down"],
        &["cmd-alt-down"],
        Scope::Panes,
    ),
    terminal(
        "find",
        "Search in the terminal",
        &["ctrl-shift-f"],
        &["cmd-f"],
        Scope::App,
    ),
    terminal(
        "broadcast_input",
        "Broadcast input on or off",
        &["ctrl-shift-b"],
        &["cmd-shift-b"],
        Scope::App,
    ),
];

/// Whether a keystroke text is one GPUI reads (one or more keystrokes).
pub fn is_valid(keys: &str) -> bool {
    !keys.trim().is_empty()
        && keys
            .split_whitespace()
            .all(|part| Keystroke::parse(part).is_ok())
}

/// The keys of `shortcut` with the user's `overrides` over the `preset`.
pub fn effective(
    shortcut: &Shortcut,
    overrides: &Map<String, Value>,
    preset: Preset,
) -> Vec<String> {
    match overrides.get(shortcut.id) {
        Some(Value::Null) => Vec::new(),
        Some(Value::String(keys)) if is_valid(keys) => vec![keys.clone()],
        _ => shortcut
            .defaults(preset)
            .iter()
            .map(|keys| (*keys).to_owned())
            .collect(),
    }
}

/// Keys in one canonical form (GPUI's own order of modifiers), so the same
/// keys written differently compare equal.
fn normalized(keys: &str) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            Keystroke::parse(stroke)
                .map(|stroke| stroke.unparse())
                .unwrap_or_else(|_| stroke.to_owned())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The ids of the actions whose keys are also another action's (the app's
/// and the terminal's alike).
pub fn conflicts(overrides: &Map<String, Value>, preset: Preset) -> Vec<&'static str> {
    let mut owners: HashMap<String, Vec<&'static str>> = HashMap::new();
    for shortcut in &SHORTCUTS {
        for keys in effective(shortcut, overrides, preset) {
            owners
                .entry(normalized(&keys))
                .or_default()
                .push(shortcut.id);
        }
    }
    let mut ids: Vec<&'static str> = owners
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flatten()
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// How keys read in the table: `Ctrl+Shift+L`, `Alt+→`, `Cmd+]`.
pub fn display(keys: &str) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            let Ok(stroke) = Keystroke::parse(stroke) else {
                return stroke.to_owned();
            };
            let modifiers = stroke.modifiers;
            let mut parts: Vec<String> = Vec::new();
            if modifiers.function {
                parts.push("Fn".into());
            }
            if modifiers.control {
                parts.push("Ctrl".into());
            }
            if modifiers.alt {
                parts.push("Alt".into());
            }
            if modifiers.platform {
                parts.push(
                    if cfg!(target_os = "macos") {
                        "Cmd"
                    } else {
                        "Win"
                    }
                    .into(),
                );
            }
            if modifiers.shift {
                parts.push("Shift".into());
            }
            parts.push(match stroke.key.as_str() {
                "right" => "→".to_owned(),
                "left" => "←".to_owned(),
                "up" => "↑".to_owned(),
                "down" => "↓".to_owned(),
                key if key.chars().count() == 1 => key.to_uppercase(),
                key => {
                    let mut chars = key.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
            });
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The binding of `keys` to the action of `id`.
fn binding(id: &str, keys: &str, context: Option<&str>) -> Option<KeyBinding> {
    if !is_valid(keys) {
        return None;
    }
    let tab = |n| Some(KeyBinding::new(keys, ActivateTab(n), context));
    match id {
        "show_vaults" => Some(KeyBinding::new(keys, ShowVaults, context)),
        "show_sftp" => Some(KeyBinding::new(keys, ShowSftp, context)),
        "tab_3" => tab(3),
        "tab_4" => tab(4),
        "tab_5" => tab(5),
        "tab_6" => tab(6),
        "tab_7" => tab(7),
        "tab_8" => tab(8),
        "tab_9" => tab(9),
        "new_local_terminal" => Some(KeyBinding::new(keys, NewLocalTerminal, context)),
        "lock" => Some(KeyBinding::new(keys, LockApp, context)),
        "new_serial" => Some(KeyBinding::new(keys, NewSerialConnection, context)),
        "new_tab" => Some(KeyBinding::new(keys, NewTab, context)),
        "close_tab" => Some(KeyBinding::new(keys, CloseTab, context)),
        "next_tab" => Some(KeyBinding::new(keys, NextTab, context)),
        "previous_tab" => Some(KeyBinding::new(keys, PreviousTab, context)),
        "open_search" => Some(KeyBinding::new(keys, OpenSearch, context)),
        "open_settings" => Some(KeyBinding::new(keys, OpenSettings, context)),
        "zoom_in" => Some(KeyBinding::new(keys, super::actions::ZoomIn, context)),
        "zoom_out" => Some(KeyBinding::new(keys, super::actions::ZoomOut, context)),
        "actual_size" => Some(KeyBinding::new(keys, super::actions::ActualSize, context)),
        "open_hosts" => Some(KeyBinding::new(keys, super::actions::ShowHosts, context)),
        "copy" => Some(KeyBinding::new(keys, crate::terminal::view::Copy, context)),
        "paste" => Some(KeyBinding::new(keys, crate::terminal::view::Paste, context)),
        "select_all" => Some(KeyBinding::new(
            keys,
            crate::terminal::view::SelectAll,
            context,
        )),
        "split_right" => Some(KeyBinding::new(keys, crate::panes::SplitRight, context)),
        "split_down" => Some(KeyBinding::new(keys, crate::panes::SplitDown, context)),
        "focus_left" => Some(KeyBinding::new(keys, crate::panes::FocusLeft, context)),
        "focus_right" => Some(KeyBinding::new(keys, crate::panes::FocusRight, context)),
        "focus_up" => Some(KeyBinding::new(keys, crate::panes::FocusUp, context)),
        "focus_down" => Some(KeyBinding::new(keys, crate::panes::FocusDown, context)),
        "find" => Some(KeyBinding::new(
            keys,
            crate::terminal_sidebar::FindInTerminal,
            context,
        )),
        "broadcast_input" => Some(KeyBinding::new(keys, ToggleBroadcast, context)),
        _ => None,
    }
}

/// The keys each action was bound to last, to unbind them before the next
/// binding.
#[derive(Default)]
struct Applied(HashMap<&'static str, Vec<String>>);

impl Global for Applied {}

/// Binds every action's current keys (`overrides` over `preset`). Call it
/// after the modules bound their defaults and after each change. GPUI's
/// keymap only grows, and every keystroke looks through it: an action whose
/// keys did not change since the last call is left as it is (a new window
/// binds nothing; a change binds the changed actions only).
pub fn apply(overrides: &Map<String, Value>, preset: Preset, cx: &mut App) {
    let mut applied = if cx.has_global::<Applied>() {
        cx.remove_global::<Applied>()
    } else {
        Applied::default()
    };
    let mut off = Vec::new();
    let mut on = Vec::new();
    for shortcut in &SHORTCUTS {
        let context = shortcut.scope.context();
        let keys = effective(shortcut, overrides, preset);
        if applied.0.get(shortcut.id) == Some(&keys) {
            continue;
        }
        let previous = applied.0.remove(shortcut.id).unwrap_or_default();
        // Whatever either set or a module's own start could have bound,
        // and what this function bound last.
        let mut stale: Vec<String> = shortcut
            .pc
            .iter()
            .chain(shortcut.mac.iter())
            .map(|keys| (*keys).to_owned())
            .chain(previous)
            .collect();
        stale.sort_unstable();
        stale.dedup();
        for keys in stale {
            if let Some(bound) = binding(shortcut.id, &keys, context) {
                let name = bound.action().name();
                off.push(KeyBinding::new(&keys, Unbind(name.into()), context));
            }
        }
        on.extend(
            keys.iter()
                .filter_map(|keys| binding(shortcut.id, keys, context)),
        );
        applied.0.insert(shortcut.id, keys);
    }
    if !off.is_empty() || !on.is_empty() {
        cx.bind_keys(off);
        cx.bind_keys(on);
    }
    cx.set_global(applied);
}

/// [`apply`] with the stored changes and set.
pub fn apply_stored(core: &shellpp_core::Core, cx: &mut App) {
    let values: Map<String, Value> = core
        .settings_get()
        .ok()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let overrides = values
        .get(OVERRIDES_KEY)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    apply(&overrides, Preset::of(&values), cx);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A view in the terminal's key context that counts its copies.
    struct Probe {
        copies: usize,
        focus: gpui::FocusHandle,
    }

    impl gpui::Render for Probe {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};
            gpui::div()
                .key_context(crate::terminal::view::KEY_CONTEXT)
                .track_focus(&self.focus)
                .size_full()
                .on_action(
                    cx.listener(|this, _: &crate::terminal::view::Copy, _, _| this.copies += 1),
                )
                .child("probe")
        }
    }

    /// D76: a terminal key moved by the user, and the other set, change
    /// what the keys do at once (the old keys stop).
    #[gpui_kit::test]
    fn changes_and_sets_rebind_the_terminal_keys(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            crate::terminal::view::bind_keys(cx);
        });
        let (handle, probe) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui::WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let focus = cx.focus_handle();
                        window.focus(&focus, cx);
                        Probe { copies: 0, focus }
                    })
                })
            })
            .unwrap();
        let press = |cx: &mut gpui::TestAppContext, keys: &str| {
            let keys = keys.to_owned();
            cx.update_window(handle, move |_, window, cx| {
                window.render_frame(cx);
                window.press(&keys, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let copies = |cx: &mut gpui::TestAppContext| probe.read_with(cx, |probe, _| probe.copies);
        cx.update(|cx| apply(&Map::new(), Preset::Pc, cx));
        press(cx, "ctrl-shift-c");
        assert_eq!(copies(cx), 1);
        cx.update(|cx| apply(&map(json!({"copy": "ctrl-insert"})), Preset::Pc, cx));
        press(cx, "ctrl-shift-c");
        assert_eq!(copies(cx), 1, "the old keys stop");
        press(cx, "ctrl-insert");
        assert_eq!(copies(cx), 2);
        // The Mac set, the user's change reset: Cmd+C.
        cx.update(|cx| apply(&Map::new(), Preset::Mac, cx));
        press(cx, "ctrl-insert");
        press(cx, "ctrl-shift-c");
        assert_eq!(copies(cx), 2);
        press(cx, "cmd-c");
        assert_eq!(copies(cx), 3);
        // Back to PC.
        cx.update(|cx| apply(&Map::new(), Preset::Pc, cx));
        press(cx, "cmd-c");
        press(cx, "ctrl-shift-c");
        assert_eq!(copies(cx), 4);
        // The same keys again (a new window): the keymap does not grow; a
        // change binds the changed action only (section 12 review).
        let bindings = |cx: &mut gpui::TestAppContext| {
            cx.update(|cx| cx.key_bindings().borrow().bindings().len())
        };
        let before = bindings(cx);
        for _ in 0..5 {
            cx.update(|cx| apply(&Map::new(), Preset::Pc, cx));
        }
        assert_eq!(bindings(cx), before);
        cx.update(|cx| apply(&map(json!({"copy": "ctrl-insert"})), Preset::Pc, cx));
        assert!(bindings(cx) - before <= 4, "{}", bindings(cx) - before);
        press(cx, "ctrl-insert");
        assert_eq!(copies(cx), 5);
    }

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn keys_read_and_conflict() {
        assert_eq!(display("ctrl-shift-l"), "Ctrl+Shift+L");
        assert_eq!(display("alt-right"), "Alt+→");
        assert_eq!(display("ctrl-,"), "Ctrl+,");
        assert!(is_valid("ctrl-shift-j") && !is_valid("") && !is_valid("  "));
        let none = Map::new();
        for preset in [Preset::Pc, Preset::Mac] {
            assert!(
                conflicts(&none, preset).is_empty(),
                "{preset:?}: {:?}",
                conflicts(&none, preset)
            );
        }
        let new_tab = SHORTCUTS.iter().find(|s| s.id == "new_tab").unwrap();
        assert_eq!(effective(new_tab, &none, Preset::Pc).len(), 2);
        // The sets: Ctrl on PC, Cmd on Mac; the terminal copies with
        // Ctrl+Shift+C or Cmd+C.
        let copy = SHORTCUTS.iter().find(|s| s.id == "copy").unwrap();
        assert_eq!(effective(copy, &none, Preset::Pc), ["ctrl-shift-c"]);
        assert_eq!(effective(copy, &none, Preset::Mac), ["cmd-c"]);
        // Ctrl+L for Settings as well: both are marked.
        let changed = map(json!({"open_settings": "ctrl-l"}));
        assert_eq!(
            conflicts(&changed, Preset::Pc),
            ["new_local_terminal", "open_settings"]
        );
        // A terminal key given to an app action conflicts; the order of
        // modifiers does not count.
        assert_eq!(
            conflicts(&map(json!({"new_tab": "shift-ctrl-c"})), Preset::Pc),
            ["copy", "new_tab"]
        );
        // A terminal key moved: no conflict, and the old keys are free.
        assert!(conflicts(&map(json!({"copy": "ctrl-insert"})), Preset::Pc).is_empty());
        // Unassigned has no keys; an unreadable value keeps the defaults.
        let lock = SHORTCUTS.iter().find(|s| s.id == "lock").unwrap();
        assert!(effective(lock, &map(json!({"lock": null})), Preset::Pc).is_empty());
        assert_eq!(
            effective(lock, &map(json!({"lock": " "})), Preset::Pc),
            ["ctrl-shift-l"]
        );
        assert_eq!(normalized("alt-ctrl-l"), normalized("ctrl-alt-l"));
        assert_eq!(display("ctrl--"), "Ctrl+-");
        assert_eq!(
            Preset::of(&map(json!({"shortcuts.preset": "mac"}))),
            Preset::Mac
        );
        assert_eq!(Preset::of(&Map::new()), Preset::platform());
        // Every action of the table binds, in both sets.
        for shortcut in &SHORTCUTS {
            assert!(
                binding(shortcut.id, "ctrl-j", None).is_some(),
                "{}",
                shortcut.id
            );
            for keys in shortcut.pc.iter().chain(shortcut.mac.iter()) {
                assert!(is_valid(keys), "{}: {keys}", shortcut.id);
            }
        }
    }
}
