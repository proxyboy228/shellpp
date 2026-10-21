//! The shortcuts of the app's own actions that the user can change (UI
//! 12.5; stage 6, D67): the defaults of [`super::actions::bind_keys`], the
//! user's changes in `shortcuts.overrides` (action id → keystroke, or
//! `null` for unassigned), conflicts, and how a change takes effect.
//!
//! A change is bound on top of the defaults: the keys the action had are
//! unbound for that action (`Unbind`) and every action's current keys are
//! bound again after them, later bindings taking precedence. Keys of the terminal (copy, paste, splits, focus
//! between panes) keep their defaults for now (D67).

use std::collections::HashMap;

use gpui::{App, Global, KeyBinding, Keystroke, Unbind};
use serde_json::{Map, Value};

use super::actions::{
    ActivateTab, CloseTab, KEY_CONTEXT, LockApp, NewLocalTerminal, NewSerialConnection, NewTab,
    NextTab, OpenSearch, OpenSettings, PreviousTab, ShowSftp, ShowVaults,
};

/// The setting that holds the user's changes.
pub const OVERRIDES_KEY: &str = "shortcuts.overrides";

/// An action of the shortcuts table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The id in `shortcuts.overrides`.
    pub id: &'static str,
    pub label: &'static str,
    /// The default keys (GPUI keystrokes; `secondary` is Ctrl, Cmd on macOS).
    pub defaults: &'static [&'static str],
    /// Bound in every key context (Lock works over dialogs too).
    pub global: bool,
}

/// Every action of the table, in its order.
pub const SHORTCUTS: [Shortcut; 19] = [
    Shortcut {
        id: "show_vaults",
        label: "Go to Vaults",
        defaults: &["secondary-1"],
        global: false,
    },
    Shortcut {
        id: "show_sftp",
        label: "Go to SFTP",
        defaults: &["secondary-2"],
        global: false,
    },
    Shortcut {
        id: "tab_3",
        label: "Go to tab 3",
        defaults: &["secondary-3"],
        global: false,
    },
    Shortcut {
        id: "tab_4",
        label: "Go to tab 4",
        defaults: &["secondary-4"],
        global: false,
    },
    Shortcut {
        id: "tab_5",
        label: "Go to tab 5",
        defaults: &["secondary-5"],
        global: false,
    },
    Shortcut {
        id: "tab_6",
        label: "Go to tab 6",
        defaults: &["secondary-6"],
        global: false,
    },
    Shortcut {
        id: "tab_7",
        label: "Go to tab 7",
        defaults: &["secondary-7"],
        global: false,
    },
    Shortcut {
        id: "tab_8",
        label: "Go to tab 8",
        defaults: &["secondary-8"],
        global: false,
    },
    Shortcut {
        id: "tab_9",
        label: "Go to tab 9",
        defaults: &["secondary-9"],
        global: false,
    },
    Shortcut {
        id: "new_local_terminal",
        label: "New local terminal",
        defaults: &["secondary-l"],
        global: false,
    },
    Shortcut {
        id: "lock",
        label: "Lock the app",
        defaults: &["secondary-shift-l"],
        global: true,
    },
    Shortcut {
        id: "new_serial",
        label: "New serial connection",
        defaults: &["secondary-alt-s"],
        global: false,
    },
    Shortcut {
        id: "new_tab",
        label: "New tab",
        defaults: &["secondary-t", "secondary-shift-t"],
        global: false,
    },
    Shortcut {
        id: "close_tab",
        label: "Close tab or pane",
        defaults: &["secondary-shift-w"],
        global: false,
    },
    Shortcut {
        id: "next_tab",
        label: "Next tab",
        defaults: &["alt-right"],
        global: false,
    },
    Shortcut {
        id: "previous_tab",
        label: "Previous tab",
        defaults: &["alt-left"],
        global: false,
    },
    Shortcut {
        id: "open_search",
        label: "Search hosts and tabs",
        defaults: &["secondary-k", "secondary-shift-k"],
        global: false,
    },
    Shortcut {
        id: "open_settings",
        label: "Settings",
        defaults: &["secondary-,"],
        global: false,
    },
    Shortcut {
        id: "open_hosts",
        label: "Open the Hosts screen",
        defaults: &[],
        global: false,
    },
];

/// Whether a keystroke text is one GPUI reads (one or more keystrokes).
pub fn is_valid(keys: &str) -> bool {
    !keys.trim().is_empty()
        && keys
            .split_whitespace()
            .all(|part| Keystroke::parse(part).is_ok())
}

/// The keys of `shortcut` with the user's `overrides`.
pub fn effective(shortcut: &Shortcut, overrides: &Map<String, Value>) -> Vec<String> {
    match overrides.get(shortcut.id) {
        Some(Value::Null) => Vec::new(),
        Some(Value::String(keys)) if is_valid(keys) => vec![keys.clone()],
        _ => shortcut
            .defaults
            .iter()
            .map(|keys| (*keys).to_owned())
            .collect(),
    }
}

/// Keys of the terminal and the panes that keep their defaults (UI 12.5):
/// an action given one of them is marked as a conflict.
pub const FIXED: [&str; 10] = [
    "secondary-shift-c",
    "secondary-shift-v",
    "secondary-shift-a",
    "secondary-shift-d",
    "secondary-shift-e",
    "secondary-shift-f",
    "secondary-alt-left",
    "secondary-alt-right",
    "secondary-alt-up",
    "secondary-alt-down",
];

/// Keys in one canonical form (GPUI's own order of modifiers, `secondary`
/// resolved), so the same keys written differently compare equal.
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

/// The ids of the actions whose keys are also another action's, or one of
/// the [`FIXED`] keys.
pub fn conflicts(overrides: &Map<String, Value>) -> Vec<&'static str> {
    let mut owners: HashMap<String, Vec<&'static str>> = HashMap::new();
    for keys in FIXED {
        owners.entry(normalized(keys)).or_default().push("");
    }
    for shortcut in &SHORTCUTS {
        for keys in effective(shortcut, overrides) {
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
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// How keys read in the table: `Ctrl+Shift+L`, `Alt+→`.
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
        "open_hosts" => Some(KeyBinding::new(keys, super::actions::ShowHosts, context)),
        _ => None,
    }
}

/// The actions the user's changes touched so far, with the keys they had
/// (to unbind them when the change changes again). An entry with no keys
/// is an action left unassigned.
#[derive(Default)]
struct Applied(HashMap<&'static str, Vec<String>>);

impl Global for Applied {}

/// Binds the user's changes (`shortcuts.overrides`) on top of the defaults.
/// Call it after the defaults are bound and after each change.
///
/// Each touched action's old keys are unbound for that action only
/// (`Unbind` with its name: other actions on the same keys stay), then the
/// current keys of every action are bound again, after the unbinds (later
/// bindings win): so swaps, resets and conflicts resolve in any order.
pub fn apply(overrides: &Map<String, Value>, cx: &mut App) {
    let mut applied = if cx.has_global::<Applied>() {
        cx.remove_global::<Applied>()
    } else {
        Applied::default()
    };
    let touched = SHORTCUTS
        .iter()
        .any(|shortcut| overrides.contains_key(shortcut.id) || applied.0.contains_key(shortcut.id));
    if touched {
        let mut off = Vec::new();
        let mut on = Vec::new();
        for shortcut in &SHORTCUTS {
            let context = (!shortcut.global).then_some(KEY_CONTEXT);
            let previous = applied.0.remove(shortcut.id);
            if overrides.contains_key(shortcut.id) || previous.is_some() {
                for keys in shortcut
                    .defaults
                    .iter()
                    .map(|keys| (*keys).to_owned())
                    .chain(previous.unwrap_or_default())
                {
                    if let Some(bound) = binding(shortcut.id, &keys, context) {
                        let name = bound.action().name();
                        off.push(KeyBinding::new(&keys, Unbind(name.into()), context));
                    }
                }
            }
            let keys = effective(shortcut, overrides);
            on.extend(
                keys.iter()
                    .filter_map(|keys| binding(shortcut.id, keys, context)),
            );
            if overrides.contains_key(shortcut.id) {
                applied.0.insert(shortcut.id, keys);
            }
        }
        cx.bind_keys(off);
        cx.bind_keys(on);
    }
    cx.set_global(applied);
}

/// [`apply`] with the stored changes.
pub fn apply_stored(core: &shellpp_core::Core, cx: &mut App) {
    let overrides = core
        .settings_get()
        .ok()
        .and_then(|values| values.get(OVERRIDES_KEY).cloned())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    apply(&overrides, cx);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn keys_read_and_conflict() {
        let ctrl = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        assert_eq!(display("secondary-shift-l"), format!("{ctrl}+Shift+L"));
        assert_eq!(display("alt-right"), "Alt+→");
        assert_eq!(display("secondary-,"), format!("{ctrl}+,"));
        assert!(is_valid("ctrl-shift-j") && !is_valid("") && !is_valid("  "));
        let none = Map::new();
        assert!(conflicts(&none).is_empty(), "{:?}", conflicts(&none));
        let new_tab = SHORTCUTS.iter().find(|s| s.id == "new_tab").unwrap();
        assert_eq!(effective(new_tab, &none).len(), 2);
        // Ctrl+L for Settings as well: both are marked.
        let changed = map(json!({"open_settings": "secondary-l"}));
        assert_eq!(conflicts(&changed), ["new_local_terminal", "open_settings"]);
        // The same keys written as ctrl- conflict too (on Windows and Linux).
        if !cfg!(target_os = "macos") {
            let changed = map(json!({"open_settings": "ctrl-l"}));
            assert_eq!(conflicts(&changed).len(), 2);
        }
        // Unassigned has no keys; an unreadable value keeps the defaults.
        let lock = SHORTCUTS.iter().find(|s| s.id == "lock").unwrap();
        assert!(effective(lock, &map(json!({"lock": null}))).is_empty());
        assert_eq!(
            effective(lock, &map(json!({"lock": " "}))),
            ["secondary-shift-l"]
        );
        // The terminal's fixed keys count; the order of modifiers does not.
        if !cfg!(target_os = "macos") {
            assert_eq!(
                conflicts(&map(json!({"new_tab": "ctrl-shift-c"}))),
                ["new_tab"]
            );
        }
        assert_eq!(normalized("alt-ctrl-l"), normalized("ctrl-alt-l"));
        assert_eq!(display("ctrl--"), "Ctrl+-");
        // Every action of the table binds.
        for shortcut in &SHORTCUTS {
            assert!(
                binding(shortcut.id, "ctrl-j", None).is_some(),
                "{}",
                shortcut.id
            );
        }
    }
}
