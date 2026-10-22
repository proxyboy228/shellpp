//! The keyword highlighting rules of Settings → Terminal (UI 12.4; D77):
//! each category's on/off and color, and the user's own rules (a regular
//! expression, case-insensitive or not, a color, on/off). Colors come in
//! pairs from a palette: one for a dark and one for a light terminal
//! background. Every change is saved at once as a `highlight_rule` record
//! and the terminals take it (`terminal::highlight::load`).

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Subscription,
    Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use serde_json::{Map, Value, json};
use shellpp_core::service::snippets::{HighlightRuleView, RecordInput};
use uuid::Uuid;

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::terminal::highlight::{Category, parse_color};
use crate::theme::highlight::{CategoryColors, PALETTE, fill, hex};
use crate::theme::metrics;

/// The palette entry a rule's dark color is, if any.
fn palette_index(color_dark: &str) -> Option<usize> {
    PALETTE
        .iter()
        .position(|pair| hex(pair[0]).eq_ignore_ascii_case(color_dark))
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Error => strings::HL_ERROR,
        Category::Warning => strings::HL_WARNING,
        Category::Ok => strings::HL_OK,
        Category::Info => strings::HL_INFO,
        Category::Debug => strings::HL_DEBUG,
        Category::Address => strings::HL_ADDRESS,
    }
}

const CATEGORIES: [Category; 6] = [
    Category::Error,
    Category::Warning,
    Category::Ok,
    Category::Info,
    Category::Debug,
    Category::Address,
];

pub struct HighlightRulesBlock {
    state: Entity<AppState>,
    rules: Vec<HighlightRuleView>,
    pattern: Entity<InputState>,
    case_insensitive: bool,
    color: usize,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for HighlightRulesBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightRulesBlock")
            .field("rules", &self.rules.len())
            .finish_non_exhaustive()
    }
}

impl HighlightRulesBlock {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pattern =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::HL_PATTERN_PLACEHOLDER));
        let subscription = cx.subscribe_in(
            &pattern,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.add_rule(window, cx),
                InputEvent::Change => {
                    this.error = None;
                    cx.notify();
                }
                _ => {}
            },
        );
        let mut block = Self {
            state,
            rules: Vec::new(),
            pattern,
            case_insensitive: true,
            color: PALETTE.len() - 2,
            error: None,
            _subscriptions: vec![subscription],
        };
        block.reload(cx);
        block
    }

    pub fn rules(&self) -> &[HighlightRuleView] {
        &self.rules
    }

    pub fn error(&self) -> Option<&'static str> {
        self.error
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.rules = state
            .core()
            .zip(state.vault_id())
            .and_then(|(core, vault)| core.highlight_rules_list(vault).ok())
            .unwrap_or_default();
        cx.notify();
    }

    /// Saves `data` (as the record `record_id`), then the terminals take
    /// the rules. The reason of a refusal, if any.
    fn save(
        &mut self,
        record_id: Option<Uuid>,
        data: Value,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let (core, vault_id) = {
            let state = self.state.read(cx);
            (state.core().cloned(), state.vault_id())
        };
        let (Some(core), Some(vault_id)) = (core, vault_id) else {
            return Err(strings::HL_SAVE_FAILED);
        };
        let data: Map<String, Value> = data.as_object().cloned().unwrap_or_default();
        let saved = core.highlight_rule_save(RecordInput {
            vault_id,
            record_id,
            data,
        });
        let result = match saved {
            Ok(_) => Ok(()),
            Err(shellpp_core::error::CoreError::Validation(fields)) => Err(
                if fields
                    .iter()
                    .any(|field| field.reason == shellpp_core::ValidationReason::TooMany)
                {
                    strings::HL_TOO_MANY
                } else if fields
                    .iter()
                    .any(|field| field.reason == shellpp_core::ValidationReason::TooLong)
                {
                    strings::HL_TOO_LONG
                } else {
                    strings::HL_INVALID
                },
            ),
            Err(error) => {
                tracing::warn!(code = error.code(), "highlight rule not saved");
                Err(strings::HL_SAVE_FAILED)
            }
        };
        self.reload(cx);
        crate::terminal::highlight::load(&self.state, cx);
        result
    }

    /// The record of `category`, if the user changed it (the newest one).
    fn category_record(&self, category: Category) -> Option<&HighlightRuleView> {
        self.rules.iter().rev().find(|rule| {
            rule.kind == "category" && rule.category.as_deref() == Some(category.record_name())
        })
    }

    /// A category's on/off and colors as they are now.
    fn category_state(&self, category: Category) -> (bool, String) {
        match self.category_record(category) {
            Some(rule) => (rule.enabled, rule.color_dark.clone()),
            None => (true, hex(builtin_colors(category)[0])),
        }
    }

    pub fn set_category(
        &mut self,
        category: Category,
        enabled: bool,
        colors: CategoryColors,
        cx: &mut Context<Self>,
    ) {
        let record_id = self
            .category_record(category)
            .map(|rule| rule.record.record_id);
        let data = json!({
            "kind": "category",
            "category": category.record_name(),
            "color_dark": hex(colors[0]),
            "color_light": hex(colors[1]),
            "enabled": enabled,
        });
        if let Err(reason) = self.save(record_id, data, cx) {
            self.error = Some(reason);
        }
    }

    /// The colors of `category` now (a changed one's, else its built-in).
    fn category_colors(&self, category: Category) -> CategoryColors {
        let Some(rule) = self.category_record(category) else {
            return builtin_colors(category);
        };
        let pair = |text: &str| parse_color(text).map(|rgb| (rgb.r, rgb.g, rgb.b));
        let dark = pair(&rule.color_dark).unwrap_or(builtin_colors(category)[0]);
        let light = rule.color_light.as_deref().and_then(pair).unwrap_or(dark);
        [dark, light]
    }

    /// "Add rule": the typed pattern in the chosen color.
    pub fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pattern = self.pattern.read(cx).value().to_string();
        if pattern.trim().is_empty() {
            self.error = Some(strings::HL_INVALID);
            cx.notify();
            return;
        }
        let colors = PALETTE[self.color.min(PALETTE.len() - 1)];
        let data = json!({
            "kind": "custom",
            "pattern": pattern,
            "case_insensitive": self.case_insensitive,
            "color_dark": hex(colors[0]),
            "color_light": hex(colors[1]),
            "enabled": true,
        });
        match self.save(None, data, cx) {
            Ok(()) => {
                self.error = None;
                self.pattern
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(reason) => self.error = Some(reason),
        }
        cx.notify();
    }

    /// A custom rule on or off, or in another color.
    pub fn update_rule(
        &mut self,
        id: Uuid,
        enabled: Option<bool>,
        colors: Option<CategoryColors>,
        cx: &mut Context<Self>,
    ) {
        let Some(rule) = self
            .rules
            .iter()
            .find(|rule| rule.record.record_id == id)
            .cloned()
        else {
            return;
        };
        let (dark, light) = match colors {
            Some(colors) => (hex(colors[0]), hex(colors[1])),
            None => (
                rule.color_dark.clone(),
                rule.color_light
                    .clone()
                    .unwrap_or_else(|| rule.color_dark.clone()),
            ),
        };
        let data = json!({
            "kind": "custom",
            "pattern": rule.pattern.clone().unwrap_or_default(),
            "case_insensitive": rule.case_insensitive,
            "color_dark": dark,
            "color_light": light,
            "enabled": enabled.unwrap_or(rule.enabled),
        });
        if let Err(reason) = self.save(Some(id), data, cx) {
            self.error = Some(reason);
        }
    }

    pub fn remove_rule(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let removed = self
            .state
            .update(cx, |state, cx| state.delete_records(vec![id], cx));
        if let Err(error) = removed {
            tracing::warn!(code = error.code(), "highlight rule not removed");
        }
        self.reload(cx);
        crate::terminal::highlight::load(&self.state, cx);
    }

    fn swatches(
        &self,
        prefix: String,
        chosen: Option<usize>,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, usize, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let ring = cx.theme().ring;
        let border = cx.theme().border;
        h_flex()
            .gap(metrics::SPACE_1)
            .children(PALETTE.iter().enumerate().map(|(index, pair)| {
                let fill = fill(pair[0]);
                let on_pick = on_pick.clone();
                div()
                    .id(SharedString::from(format!("{prefix}-color-{index}")))
                    .test_support()
                    .size(metrics::rpx(18.))
                    .rounded_full()
                    .bg(fill)
                    .border_2()
                    .border_color(if chosen == Some(index) { ring } else { border })
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| on_pick(this, index, cx)),
                    )
            }))
            .into_any_element()
    }

    fn render_categories(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = CATEGORIES
            .iter()
            .map(|category| {
                let category = *category;
                let name = category.record_name();
                let (enabled, dark) = self.category_state(category);
                let switch = Switch::new(SharedString::from(format!("hl-category-{name}-on")))
                    .checked(enabled)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let colors = this.category_colors(category);
                        this.set_category(category, *checked, colors, cx)
                    }));
                let swatches = self.swatches(
                    format!("hl-category-{name}"),
                    palette_index(&dark),
                    cx,
                    move |this, index, cx| {
                        let (enabled, _) = this.category_state(category);
                        this.set_category(category, enabled, PALETTE[index], cx)
                    },
                );
                h_flex()
                    .id(SharedString::from(format!("hl-category-{name}")))
                    .test_support()
                    .w_full()
                    .gap(metrics::SPACE_3)
                    .items_center()
                    .child(
                        div()
                            .w(metrics::rpx(140.))
                            .text_size(metrics::FONT_SIZE_BODY)
                            .child(category_label(category)),
                    )
                    .child(swatches)
                    .child(div().flex_1())
                    .child(switch)
                    .into_any_element()
            })
            .collect();
        v_flex()
            .gap(metrics::SPACE_2)
            .children(rows)
            .into_any_element()
    }

    fn render_custom(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let custom: Vec<HighlightRuleView> = self
            .rules
            .iter()
            .filter(|rule| rule.kind == "custom")
            .cloned()
            .collect();
        let rows: Vec<AnyElement> = custom
            .iter()
            .map(|rule| {
                let id = rule.record.record_id;
                let switch = Switch::new(SharedString::from(format!("hl-rule-{id}-on")))
                    .checked(rule.enabled)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.update_rule(id, Some(*checked), None, cx)
                    }));
                let swatches = self.swatches(
                    format!("hl-rule-{id}"),
                    palette_index(&rule.color_dark),
                    cx,
                    move |this, index, cx| this.update_rule(id, None, Some(PALETTE[index]), cx),
                );
                let remove =
                    widgets::remove_button(SharedString::from(format!("hl-rule-{id}-remove")), cx)
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.remove_rule(id, cx)
                            }),
                        );
                h_flex()
                    .id(SharedString::from(format!("hl-rule-{id}")))
                    .test_support()
                    .w_full()
                    .gap(metrics::SPACE_3)
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                            .text_size(metrics::FONT_SIZE_DENSE)
                            .child(rule.pattern.clone().unwrap_or_default()),
                    )
                    .when(rule.case_insensitive, |this| {
                        this.child(widgets::caption(strings::HL_ANY_CASE, cx))
                    })
                    .child(swatches)
                    .child(switch)
                    .child(remove)
                    .into_any_element()
            })
            .collect();
        let new_swatches = self.swatches(
            "hl-new".to_owned(),
            Some(self.color),
            cx,
            |this, index, cx| {
                this.color = index;
                cx.notify();
            },
        );
        let case = Checkbox::new("hl-new-case")
            .checked(self.case_insensitive)
            .label(strings::HL_ANY_CASE)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.case_insensitive = *checked;
                cx.notify();
            }));
        v_flex()
            .gap(metrics::SPACE_2)
            .children(rows)
            .child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(div().flex_1().min_w_0().child(widgets::text_input(
                        &self.pattern,
                        "hl-new-pattern",
                        self.error.is_some(),
                        window,
                        cx,
                    )))
                    .child(
                        ControlButton::new("hl-new-add", ControlKind::Neutral)
                            .label(strings::HL_ADD)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_rule(window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap(metrics::SPACE_3)
                    .items_center()
                    .child(case)
                    .child(new_swatches),
            )
            .when_some(self.error, |this, error| {
                this.child(widgets::error_text(error, cx).id("hl-error").test_support())
            })
            .into_any_element()
    }
}

/// The built-in colors of a category.
fn builtin_colors(category: Category) -> CategoryColors {
    let dark = category.color(true);
    let light = category.color(false);
    [(dark.r, dark.g, dark.b), (light.r, light.g, light.b)]
}

impl Render for HighlightRulesBlock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let categories = self.render_categories(cx);
        let custom = self.render_custom(window, cx);
        v_flex()
            .id("settings-highlight-rules")
            .test_support()
            .gap(metrics::SPACE_3)
            .child(super::label(strings::HL_CATEGORIES, cx))
            .child(widgets::caption(strings::HL_CATEGORIES_HINT, cx))
            .child(categories)
            .child(super::label(strings::HL_CUSTOM, cx))
            .child(widgets::caption(strings::HL_CUSTOM_HINT, cx))
            .child(custom)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{App, TestAppContext};
    use gpui_kit::test::TestWindowExt as _;

    use super::*;
    use crate::testing::TestProfile;

    /// D77: a rule is added, a refused one says why, a category takes
    /// another color or goes off, a rule goes; the terminals take each set.
    #[gpui_kit::test]
    fn rules_are_added_changed_and_removed(cx: &mut TestAppContext) {
        cx.update(crate::testing::init_ui);
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        let block_state = state.clone();
        let (handle, block) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui::WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| HighlightRulesBlock::new(block_state, window, cx))
                })
            })
            .unwrap();
        let frame = |cx: &mut TestAppContext, f: &dyn Fn(&mut Window, &mut App)| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        // A pattern that does not compile.
        frame(cx, &|window, cx| {
            window.click("hl-new-pattern", cx);
            window.input("(", cx);
        });
        frame(cx, &|window, cx| window.click("hl-new-add", cx));
        assert_eq!(
            block.read_with(cx, |block, _| block.error()),
            Some(strings::HL_INVALID)
        );
        // A good one, in violet, with Enter.
        frame(cx, &|window, cx| {
            window.click("hl-new-pattern", cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.input("time ?out", cx);
        });
        frame(cx, &|window, cx| window.click("hl-new-color-6", cx));
        frame(cx, &|window, cx| window.press("enter", cx));
        let rules = block.read_with(cx, |block, _| block.rules().to_vec());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern.as_deref(), Some("time ?out"));
        assert_eq!(rules[0].color_dark, hex(PALETTE[6][0]));
        assert!(rules[0].case_insensitive);
        assert_eq!(crate::terminal::highlight::current().custom_count(), 1);
        // The Warning category in orange, then off.
        frame(cx, &|window, cx| {
            window.click("hl-category-warning-color-7", cx)
        });
        let warning = |cx: &mut TestAppContext| {
            block.read_with(cx, |block, _| {
                block
                    .rules()
                    .iter()
                    .find(|rule| rule.category.as_deref() == Some("warning"))
                    .cloned()
            })
        };
        assert_eq!(warning(cx).unwrap().color_dark, hex(PALETTE[7][0]));
        frame(cx, &|window, cx| window.click("hl-category-warning-on", cx));
        let off = warning(cx).unwrap();
        assert!(!off.enabled);
        assert_eq!(off.color_dark, hex(PALETTE[7][0]), "the color stays");
        // The rule off, then removed.
        let id = rules[0].record.record_id;
        block.update(cx, |block, cx| block.update_rule(id, Some(false), None, cx));
        assert_eq!(crate::terminal::highlight::current().custom_count(), 0);
        frame(cx, &|window, cx| {
            window.click(SharedString::from(format!("hl-rule-{id}-remove")), cx)
        });
        assert!(block.read_with(cx, |block, _| {
            block.rules().iter().all(|rule| rule.kind != "custom")
        }));
        let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        state.update(cx, |state, _| state.shutdown());
        crate::terminal::highlight::install(crate::terminal::highlight::Rules::builtin());
    }
}
