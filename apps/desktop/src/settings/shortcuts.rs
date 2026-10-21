//! The Shortcuts section of the Settings window (UI 12.5; D67, D76): the
//! set of keys (PC or Mac), then the app's actions and the terminal's with
//! their keys. A click on the keys records new ones (Esc cancels); × leaves
//! the action unassigned; ↺ brings its default back; actions that share
//! keys are marked. Changes are written to `shortcuts.overrides` and
//! `shortcuts.preset` and bound at once ([`crate::shell::shortcuts::apply`]).

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, KeyDownEvent, SharedString, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use serde_json::{Map, Value, json};

use super::{SettingsView, strings};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::shell::shortcuts::{self, Group, OVERRIDES_KEY, PRESET_KEY, Preset, SHORTCUTS};
use crate::theme::{self, metrics};

/// The element id of an action's keys and of its buttons.
pub fn keys_id(id: &str) -> SharedString {
    format!("shortcut-{id}").into()
}

pub fn unassign_id(id: &str) -> SharedString {
    format!("shortcut-unassign-{id}").into()
}

pub fn reset_id(id: &str) -> SharedString {
    format!("shortcut-reset-{id}").into()
}

/// Keys that only modify: recording waits for the key they go with.
fn is_modifier(key: &str) -> bool {
    matches!(
        key,
        "shift" | "control" | "ctrl" | "alt" | "platform" | "cmd" | "function" | "fn" | "super"
    )
}

impl SettingsView {
    pub(super) fn overrides(&self, cx: &gpui::App) -> Map<String, Value> {
        self.values(cx)
            .get(OVERRIDES_KEY)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    /// The set of keys in use.
    pub fn preset(&self, cx: &gpui::App) -> Preset {
        Preset::of(&self.values(cx))
    }

    /// Writes the user's shortcuts and binds them now.
    fn write_overrides(&mut self, overrides: Map<String, Value>, cx: &mut Context<Self>) {
        self.set_value(OVERRIDES_KEY, Value::Object(overrides.clone()), cx);
        if self.error.is_none() {
            let preset = self.preset(cx);
            shortcuts::apply(&overrides, preset, cx);
        }
    }

    /// The PC or the Mac set (the user's own keys stay).
    pub fn set_preset(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.set_value(PRESET_KEY, json!(preset.name()), cx);
        if self.error.is_none() {
            let overrides = self.overrides(cx);
            shortcuts::apply(&overrides, preset, cx);
        }
    }

    /// Starts recording the keys of `id` (a second click cancels).
    pub fn record_shortcut(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.recording = (self.recording != Some(id)).then_some(id);
        window.focus(&self.shortcuts_focus, cx);
        cx.notify();
    }

    /// Sets the keys of `id` (`None`: unassigned).
    pub fn set_shortcut(&mut self, id: &'static str, keys: Option<String>, cx: &mut Context<Self>) {
        let mut overrides = self.overrides(cx);
        overrides.insert(id.to_owned(), keys.map_or(Value::Null, Value::String));
        self.recording = None;
        self.write_overrides(overrides, cx);
    }

    /// The default keys of `id` again.
    pub fn reset_shortcut(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let mut overrides = self.overrides(cx);
        overrides.remove(id);
        self.write_overrides(overrides, cx);
    }

    pub fn reset_all_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        self.write_overrides(Map::new(), cx);
    }

    /// The keys pressed while recording: Esc cancels, a key with its
    /// modifiers becomes the action's keys.
    fn on_shortcut_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(id) = self.recording else {
            return;
        };
        let stroke = &event.keystroke;
        if is_modifier(&stroke.key) {
            return;
        }
        cx.stop_propagation();
        if stroke.key == "escape" && !stroke.modifiers.modified() {
            self.recording = None;
            cx.notify();
            return;
        }
        // A key without Ctrl, Alt, Cmd or Fn would fire while typing (the
        // F-keys excepted): recording waits for one with a modifier.
        let modifiers = stroke.modifiers;
        let function_key = stroke.key.len() > 1
            && stroke.key.starts_with('f')
            && stroke.key[1..].bytes().all(|byte| byte.is_ascii_digit());
        if !(modifiers.control || modifiers.alt || modifiers.platform || modifiers.function)
            && !function_key
        {
            return;
        }
        let keys = stroke.unparse();
        if shortcuts::is_valid(&keys) {
            self.set_shortcut(id, Some(keys), cx);
        }
    }

    pub(super) fn render_shortcuts(&self, cx: &mut Context<Self>) -> AnyElement {
        let foreground = cx.theme().popover_foreground;
        let tokens = theme::tokens(cx);
        let overrides = self.overrides(cx);
        let preset = self.preset(cx);
        let conflicts = shortcuts::conflicts(&overrides, preset);
        let row = |shortcut: &shortcuts::Shortcut, cx: &mut Context<Self>| {
            let theme = cx.theme();
            let id = shortcut.id;
            let keys = shortcuts::effective(shortcut, &overrides, preset);
            let recording = self.recording == Some(id);
            let conflict = conflicts.contains(&id);
            let text: SharedString = if recording {
                strings::PRESS_KEYS.into()
            } else if keys.is_empty() {
                strings::UNASSIGNED.into()
            } else {
                keys.iter()
                    .map(|keys| shortcuts::display(keys))
                    .collect::<Vec<_>>()
                    .join(" / ")
                    .into()
            };
            let border = if recording {
                theme.ring
            } else if conflict {
                theme.danger
            } else {
                theme.input
            };
            let hover = theme.secondary_hover;
            let face = widgets::pressable(keys_id(id), true, theme.secondary, cx)
                .flex_none()
                .min_w(px(180.))
                .h(px(32.))
                .px(metrics::SPACE_3)
                .flex()
                .items_center()
                .rounded(metrics::RADIUS_CONTROL)
                .border_1()
                .border_color(border)
                .bg(tokens.bg_input.hsla())
                .text_size(metrics::FONT_SIZE_DENSE)
                .text_color(if conflict {
                    theme.danger
                } else if keys.is_empty() && !recording {
                    theme.muted_foreground
                } else {
                    theme.popover_foreground
                })
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(text)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.record_shortcut(id, window, cx)
                }));
            let changed = overrides.contains_key(id);
            h_flex()
                .w_full()
                .min_h(px(40.))
                .gap(metrics::SPACE_2)
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .text_color(theme.popover_foreground)
                                .child(shortcut.label),
                        )
                        .when(conflict, |this| {
                            this.child(
                                div()
                                    .text_size(metrics::FONT_SIZE_CAPTION)
                                    .text_color(theme.danger)
                                    .child(strings::CONFLICT),
                            )
                        }),
                )
                .child(face)
                .child(
                    ControlButton::new(unassign_id(id), ControlKind::Ghost)
                        .icon(gpui_kit::component::IconName::Close)
                        .tooltip(strings::UNASSIGN)
                        .height(px(32.))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_shortcut(id, None, cx)
                        })),
                )
                .child(div().w(px(32.)).children(changed.then(|| {
                    ControlButton::new(reset_id(id), ControlKind::Ghost)
                        .icon(gpui_kit::assets::IconName::RotateCcw)
                        .tooltip(strings::RESET_SHORTCUT)
                        .height(px(32.))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.reset_shortcut(id, cx)
                        }))
                })))
                .into_any_element()
        };
        let group = |group: Group, title: &'static str, cx: &mut Context<Self>| {
            let rows: Vec<AnyElement> = SHORTCUTS
                .iter()
                .filter(|shortcut| shortcut.group == group)
                .map(|shortcut| row(shortcut, cx))
                .collect();
            v_flex()
                .w_full()
                .child(
                    div()
                        .pt(metrics::SPACE_2)
                        .pb(metrics::SPACE_1)
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(title),
                )
                .children(rows)
        };
        let segment = |id: &'static str,
                       label: &'static str,
                       set: Preset,
                       cx: &mut Context<Self>| {
            widgets::segment(id, label, preset == set, cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_preset(set, cx)))
                .into_any_element()
        };
        let presets = widgets::segments([
            segment("shortcuts-preset-pc", strings::PRESET_PC, Preset::Pc, cx),
            segment("shortcuts-preset-mac", strings::PRESET_MAC, Preset::Mac, cx),
        ]);
        let app_rows = group(Group::App, strings::GROUP_APP, cx);
        let terminal_rows = group(Group::Terminal, strings::GROUP_TERMINAL, cx);
        v_flex()
            .id("settings-shortcuts")
            .test_support()
            .track_focus(&self.shortcuts_focus)
            .capture_key_down(
                cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_shortcut_key(event, cx)),
            )
            // Esc while recording ends the recording, not the window (the
            // dialog's own Cancel comes here first).
            .on_action(
                cx.listener(|this, _: &gpui_kit::base::actions::Cancel, _, cx| {
                    if this.recording.take().is_some() {
                        cx.notify();
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .gap(metrics::SPACE_3)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_HEADING)
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(foreground)
                            .child(strings::SHORTCUTS),
                    )
                    .when(!overrides.is_empty(), |this| {
                        this.child(
                            ControlButton::new("shortcuts-reset-all", ControlKind::Neutral)
                                .label(strings::RESET_ALL)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.reset_all_shortcuts(cx)
                                })),
                        )
                    }),
            )
            .child(widgets::caption(strings::SHORTCUTS_HINT, cx))
            .child(
                v_flex()
                    .id("shortcuts-preset")
                    .test_support()
                    .gap(metrics::SPACE_1)
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(foreground)
                            .child(strings::PRESET),
                    )
                    .child(presets)
                    .child(widgets::caption(strings::PRESET_HINT, cx)),
            )
            .child(app_rows)
            .child(terminal_rows)
            .into_any_element()
    }
}
