//! The snippet panel's form (UI 9.3): description, package, tags, script
//! and targets. Run and Save sit in the panel's footer (the screen's).
//!
//! Tags are typed as one comma-separated field (the chips of the host
//! panel are not shared yet, D66). The script field is monospaced; the
//! `{{name}}` variables it holds are listed under it.

use std::collections::BTreeMap;

use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Entity, IntoElement, Render, Subscription, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use serde_json::{Map, Value, json};
use shellpp_core::error::{FieldError, ValidationReason};
use shellpp_core::service::snippets::SnippetView;
use uuid::Uuid;

use super::strings;
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::{self, metrics};

/// Whether the form makes a new snippet or edits a saved one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormMode {
    New,
    Edit(Uuid),
}

/// Tags typed as `web, deploy`: trimmed, empty ones left out.
pub fn parse_tags(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The element id of a target row.
pub fn target_id(id: Uuid) -> String {
    format!("snippet-target-{id}")
}

/// The element id of a host in the targets picker.
pub fn pick_id(id: Uuid) -> String {
    format!("snippet-pick-{id}")
}

/// What the form holds, as compared with what was saved (D68: leaving
/// with changes not saved asks first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Values {
    label: String,
    tags: Vec<String>,
    script: String,
    package: Option<Uuid>,
    targets: Vec<Uuid>,
}

fn reason_text(reason: ValidationReason) -> &'static str {
    match reason {
        ValidationReason::Required => strings::ERROR_SCRIPT,
        ValidationReason::TooLong => strings::ERROR_TOO_LONG,
        _ => strings::ERROR_INVALID,
    }
}

pub struct SnippetForm {
    mode: FormMode,
    label: Entity<InputState>,
    tags: Entity<InputState>,
    script: Entity<TextareaState>,
    package: Option<Uuid>,
    package_open: bool,
    targets: Vec<Uuid>,
    picker_open: bool,
    errors: BTreeMap<&'static str, &'static str>,
    /// The values as saved (empty for a new snippet).
    saved: Values,
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for SnippetForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnippetForm")
            .field("mode", &self.mode)
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

impl SnippetForm {
    /// A new snippet (in `package`, if any).
    pub fn new(
        package: Option<Uuid>,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let form = Self::build(
            FormMode::New,
            (String::new(), String::new(), String::new()),
            package,
            Vec::new(),
            state,
            window,
            cx,
        );
        form.label.update(cx, |input, cx| input.focus(window, cx));
        form
    }

    /// Save as snippet from Shell History: the command as the script, its
    /// host as the target. Nothing of it is saved yet.
    pub fn from_command(
        command: &str,
        host: Option<Uuid>,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self::build(
            FormMode::New,
            (String::new(), String::new(), command.to_owned()),
            None,
            host.into_iter().collect(),
            state,
            window,
            cx,
        );
        form.saved = Values::default();
        form.label.update(cx, |input, cx| input.focus(window, cx));
        form
    }

    /// A saved snippet. Targets and a package removed since are left out:
    /// the core refuses references to removed records.
    pub fn edit(
        snippet: &SnippetView,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let targets: Vec<Uuid> = {
            let state = state.read(cx);
            snippet
                .target_host_ids
                .iter()
                .copied()
                .filter(|id| state.host(*id).is_some())
                .collect()
        };
        let package = snippet.package_id.filter(|id| {
            Self::packages(&state, cx)
                .iter()
                .any(|p| p.record.record_id == *id)
        });
        Self::build(
            FormMode::Edit(snippet.record.record_id),
            (
                snippet.label.clone().unwrap_or_default(),
                snippet.tags.join(", "),
                snippet.script.clone(),
            ),
            package,
            targets,
            state,
            window,
            cx,
        )
    }

    fn packages(
        state: &Entity<AppState>,
        cx: &App,
    ) -> Vec<shellpp_core::service::snippets::SnippetPackageView> {
        let state = state.read(cx);
        state
            .core()
            .zip(state.vault_id())
            .and_then(|(core, vault)| core.snippet_packages_list(vault).ok())
            .unwrap_or_default()
    }

    fn build(
        mode: FormMode,
        (label, tags, script): (String, String, String),
        package: Option<Uuid>,
        targets: Vec<Uuid>,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(label)
                .placeholder(strings::LABEL_PLACEHOLDER)
        });
        let tags = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(tags)
                .placeholder(strings::TAGS_PLACEHOLDER)
        });
        let script = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(6, 16)
                .default_value(script)
        });
        let mut subscriptions = Vec::new();
        for (input, member) in [(&label, "label"), (&tags, "tags")] {
            subscriptions.push(cx.subscribe(
                input,
                move |form: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        form.errors.remove(member);
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(
            cx.subscribe(&script, |form: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    form.errors.remove("script");
                    cx.notify();
                }
            }),
        );
        let mut form = Self {
            mode,
            label,
            tags,
            script,
            package,
            package_open: false,
            targets,
            picker_open: false,
            errors: BTreeMap::new(),
            saved: Values::default(),
            state,
            _subscriptions: subscriptions,
        };
        form.saved = form.values(cx);
        form
    }

    fn values(&self, cx: &App) -> Values {
        Values {
            label: self.label.read(cx).value().trim().to_owned(),
            tags: parse_tags(&self.tags.read(cx).value()),
            script: self.script_text(cx),
            package: self.package,
            targets: self.targets.clone(),
        }
    }

    /// Whether the form holds changes that are not saved (D68).
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.values(cx) != self.saved
    }

    // -- Reading -----------------------------------------------------------------

    pub fn mode(&self) -> FormMode {
        self.mode
    }

    pub fn package(&self) -> Option<Uuid> {
        self.package
    }

    pub fn targets(&self) -> &[Uuid] {
        &self.targets
    }

    pub fn errors(&self) -> &BTreeMap<&'static str, &'static str> {
        &self.errors
    }

    pub fn script_text(&self, cx: &App) -> String {
        self.script.read(cx).value().to_string()
    }

    /// Run is possible: a script and at least one target (9.3).
    pub fn can_run(&self, cx: &App) -> bool {
        !self.script_text(cx).trim().is_empty() && !self.targets.is_empty()
    }

    /// The snippet's data (data-model 10.1), or the reasons under the
    /// fields this form checks itself.
    pub fn data(
        &self,
        cx: &App,
    ) -> Result<Map<String, Value>, BTreeMap<&'static str, &'static str>> {
        let script = self.script_text(cx);
        if script.trim().is_empty() {
            return Err(BTreeMap::from([("script", strings::ERROR_SCRIPT)]));
        }
        let mut data = Map::new();
        data.insert("script".into(), json!(script));
        let label = self.label.read(cx).value().trim().to_owned();
        if !label.is_empty() {
            data.insert("label".into(), json!(label));
        }
        // A package or host removed while the form is open is left out.
        if let Some(package) = self.package.filter(|id| {
            Self::packages(&self.state, cx)
                .iter()
                .any(|package| package.record.record_id == *id)
        }) {
            data.insert("package_id".into(), json!(package));
        }
        let tags = parse_tags(&self.tags.read(cx).value());
        if !tags.is_empty() {
            data.insert("tags".into(), json!(tags));
        }
        let targets: Vec<Uuid> = {
            let state = self.state.read(cx);
            self.targets
                .iter()
                .copied()
                .filter(|id| state.host(*id).is_some())
                .collect()
        };
        if !targets.is_empty() {
            data.insert("target_host_ids".into(), json!(targets));
        }
        Ok(data)
    }

    // -- Changes -------------------------------------------------------------------

    pub fn set_errors(
        &mut self,
        errors: BTreeMap<&'static str, &'static str>,
        cx: &mut Context<Self>,
    ) {
        self.errors = errors;
        cx.notify();
    }

    /// The core's refusal, under the fields it names (tags and targets
    /// under their blocks).
    pub fn set_core_errors(&mut self, fields: &[FieldError], cx: &mut Context<Self>) {
        self.errors = fields
            .iter()
            .map(|error| {
                let member = match error.field.split('.').next().unwrap_or_default() {
                    "label" => "label",
                    "tags" => "tags",
                    "package_id" => "package_id",
                    "target_host_ids" => "target_host_ids",
                    _ => "script",
                };
                (member, reason_text(error.reason))
            })
            .collect();
        cx.notify();
    }

    pub fn set_package(&mut self, package: Option<Uuid>, cx: &mut Context<Self>) {
        self.package = package;
        self.package_open = false;
        cx.notify();
    }

    /// Adds or removes a target host.
    pub fn toggle_target(&mut self, host: Uuid, cx: &mut Context<Self>) {
        if let Some(index) = self.targets.iter().position(|id| *id == host) {
            self.targets.remove(index);
        } else {
            self.targets.push(host);
        }
        self.errors.remove("target_host_ids");
        cx.notify();
    }

    pub fn set_picker_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.picker_open = open;
        cx.notify();
    }

    // -- Rendering -------------------------------------------------------------------

    fn render_main(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let packages = self
            .state
            .read(cx)
            .core()
            .zip(self.state.read(cx).vault_id())
            .and_then(|(core, vault)| core.snippet_packages_list(vault).ok())
            .unwrap_or_default();
        let chosen = self
            .package
            .and_then(|id| {
                packages
                    .iter()
                    .find(|package| package.record.record_id == id)
            })
            .map_or(strings::NO_PACKAGE.to_owned(), |package| {
                package.name.clone()
            });
        let package_list = self.package_open.then(|| {
            let none = widgets::dropdown_item(
                "snippet-package-none",
                None,
                strings::NO_PACKAGE,
                None,
                self.package.is_none(),
                cx,
            )
            .on_click(cx.listener(|form, _: &ClickEvent, _, cx| form.set_package(None, cx)));
            widgets::dropdown_list("snippet-package-list", cx)
                .child(none)
                .children(packages.iter().map(|package| {
                    let id = package.record.record_id;
                    widgets::dropdown_item(
                        format!("snippet-package-pick-{id}"),
                        Some(Icon::new(IconName::Folder).small().into_any_element()),
                        package.name.clone(),
                        None,
                        self.package == Some(id),
                        cx,
                    )
                    .on_click(cx.listener(
                        move |form, _: &ClickEvent, _, cx| form.set_package(Some(id), cx),
                    ))
                }))
        });
        let package = v_flex()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(
                widgets::select_trigger("snippet-package", None, chosen, self.package_open, cx)
                    .on_click(cx.listener(|form, _: &ClickEvent, _, cx| {
                        form.package_open = !form.package_open;
                        cx.notify();
                    })),
            )
            .children(package_list);
        let variables = shellpp_core::snippet::variables(&self.script_text(cx));
        let danger = theme.danger;
        let script = Textarea::new(&self.script)
            .px(metrics::SPACE_3 - gpui_kit::component::Size::Medium.input_px())
            .text_size(metrics::FONT_SIZE_DENSE)
            .font_family(crate::terminal::fonts::JETBRAINS_MONO)
            .bg(tokens.bg_input.hsla())
            .rounded(metrics::RADIUS_CONTROL)
            .when(self.errors.contains_key("script"), |this| {
                this.border_color(danger)
            });
        widgets::block("snippet-form-main", None, cx)
            .child(widgets::field(
                strings::LABEL,
                widgets::text_input(
                    &self.label,
                    "snippet-label",
                    self.errors.contains_key("label"),
                    window,
                    cx,
                ),
                self.errors.get("label").copied(),
                cx,
            ))
            .child(widgets::field(
                strings::PACKAGE,
                package,
                self.errors.get("package_id").copied(),
                cx,
            ))
            .child(
                widgets::field(
                    strings::TAGS,
                    widgets::text_input(
                        &self.tags,
                        "snippet-tags",
                        self.errors.contains_key("tags"),
                        window,
                        cx,
                    ),
                    self.errors.get("tags").copied(),
                    cx,
                )
                .child(widgets::caption(strings::TAGS_NOTE, cx)),
            )
            .child(
                widgets::field(
                    strings::SCRIPT,
                    div().id("snippet-script").test_support().child(script),
                    self.errors.get("script").copied(),
                    cx,
                )
                .child(
                    widgets::caption(
                        if variables.is_empty() {
                            strings::SCRIPT_NOTE.to_owned()
                        } else {
                            strings::variables(&variables)
                        },
                        cx,
                    )
                    .id("snippet-variables")
                    .test_support(),
                ),
            )
    }

    fn render_targets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let hosts = self.state.read(cx).hosts();
        let rows = self.targets.iter().map(|id| {
            let id = *id;
            let host = hosts.iter().find(|host| host.record_id == id);
            let (title, address) = host
                .map_or((strings::MISSING_HOST.to_owned(), String::new()), |host| {
                    (host.title().to_owned(), host.address.clone())
                });
            h_flex()
                .id(gpui::SharedString::from(target_id(id)))
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .items_center()
                .child(
                    Icon::new(Lucide::Server)
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .child(title),
                        )
                        .when(!address.is_empty(), |this| {
                            this.child(
                                div()
                                    .truncate()
                                    .text_size(metrics::FONT_SIZE_CAPTION)
                                    .text_color(theme.muted_foreground)
                                    .child(format!("SSH · {address}")),
                            )
                        }),
                )
                .child(
                    widgets::remove_button(
                        gpui::SharedString::from(format!("snippet-target-remove-{id}")),
                        cx,
                    )
                    .on_click(
                        cx.listener(move |form, _: &ClickEvent, _, cx| form.toggle_target(id, cx)),
                    ),
                )
        });
        let picker = self.picker_open.then(|| {
            widgets::dropdown_list("snippet-picker", cx)
                .max_h(widgets::PICKER_MAX_HEIGHT)
                .overflow_y_scroll()
                .children(hosts.iter().map(|host| {
                    let id = host.record_id;
                    widgets::dropdown_item(
                        gpui::SharedString::from(pick_id(id)),
                        None,
                        host.title().to_owned(),
                        Some(host.address.clone().into()),
                        self.targets.contains(&id),
                        cx,
                    )
                    .on_click(
                        cx.listener(move |form, _: &ClickEvent, _, cx| form.toggle_target(id, cx)),
                    )
                }))
        });
        let toggle = widgets::text_button(
            "snippet-add-targets",
            if self.picker_open {
                strings::DONE
            } else {
                strings::ADD_TARGETS
            },
            true,
            cx,
        )
        .on_click(cx.listener(|form, _: &ClickEvent, _, cx| {
            let open = !form.picker_open;
            form.set_picker_open(open, cx)
        }));
        widgets::block("snippet-form-targets", Some(strings::TARGETS), cx)
            .child(widgets::caption(strings::TARGETS_NOTE, cx))
            .children(rows)
            .child(h_flex().child(toggle))
            .children(picker)
            .when_some(
                self.errors.get("target_host_ids").copied(),
                |this, error| this.child(widgets::error_text(error, cx)),
            )
    }
}

impl Render for SnippetForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("snippet-form")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(self.render_main(window, cx))
            .child(self.render_targets(cx))
            .child(div().h(px(1.)))
    }
}
