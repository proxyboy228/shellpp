//! The snippet panel's form (UI 9.3): description, package, tags, script
//! and targets. Run and Save sit in the panel's footer (the screen's).
//!
//! Tags are chips (D74): Enter or a comma adds the typed tag, the
//! suggestions are the tags of the vault's other snippets, × and
//! Backspace in the empty field remove one. The script is a monospaced
//! editor with the shell highlighting of [`super::shell_highlight`]; the
//! `{{name}}` variables it holds are listed under it.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Subscription,
    Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{
    Backspace, Editor, EditorState, InputEvent, InputHighlighter, InputState,
};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use serde_json::{Map, Value, json};
use shellpp_core::error::{FieldError, ValidationReason};
use shellpp_core::service::snippets::SnippetView;
use uuid::Uuid;

use super::shell_highlight::{self, ShellHighlighter};
use super::strings;
use crate::panels::picker::{self, PickValue, PickerRow};
use crate::panels::widgets;
use crate::state::AppState;
use crate::state::records_edit::{has_tag, normalize_tag};
use crate::theme::{self, metrics};

/// The height of the script editor: six lines, as the field it replaced.
const SCRIPT_HEIGHT: gpui::Pixels = px(128.);

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
    /// The tags as chips, and the field a new one is typed in.
    tags: Vec<String>,
    tag_input: Entity<InputState>,
    script: Entity<EditorState>,
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
            (String::new(), Vec::new(), String::new()),
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
            (String::new(), Vec::new(), command.to_owned()),
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
                snippet.tags.clone(),
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
        (label, tags, script): (String, Vec<String>, String),
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
        let tag_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::TAGS_PLACEHOLDER));
        let script = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(shell_highlight::LANGUAGE)
                .line_number(false)
                .folding(false)
                .default_value(script)
        });
        script.update(cx, |editor, cx| {
            editor.set_highlighter_factory(
                Rc::new(|language: &str| {
                    (language == shell_highlight::LANGUAGE)
                        .then(|| Box::new(ShellHighlighter::default()) as Box<dyn InputHighlighter>)
                }),
                cx,
            )
        });
        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(&label, |form: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    form.errors.remove("label");
                    cx.notify();
                }
            }),
        );
        // Enter adds the typed tag; a comma adds what is before it.
        subscriptions.push(cx.subscribe_in(
            &tag_input,
            window,
            |form: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    let typed = form.tag_input.read(cx).value().to_string();
                    form.add_tags(&typed, window, cx);
                }
                InputEvent::Change => {
                    let typed = form.tag_input.read(cx).value().to_string();
                    if let Some(comma) = typed.rfind(',') {
                        form.add_tags(&typed[..comma], window, cx);
                        let rest = typed[comma + 1..].trim_start().to_owned();
                        form.tag_input
                            .update(cx, |input, cx| input.set_value(rest, window, cx));
                    }
                    form.errors.remove("tags");
                    cx.notify();
                }
                _ => {}
            },
        ));
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
            tag_input,
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
            tags: self.all_typed_tags(cx),
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

    /// The tags as chips (a tag still in the field is not one yet).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// The chips and what the tag field still holds: Save takes both.
    fn all_typed_tags(&self, cx: &App) -> Vec<String> {
        let mut tags = self.tags.clone();
        for tag in parse_tags(&self.tag_input.read(cx).value()) {
            if !has_tag(&tags, &tag) {
                tags.push(tag);
            }
        }
        tags
    }

    /// Every tag of the vault's snippets, once each, sorted (the
    /// suggestions of the tag field).
    fn vault_tags(&self, cx: &App) -> Vec<String> {
        let state = self.state.read(cx);
        let snippets = state
            .core()
            .zip(state.vault_id())
            .and_then(|(core, vault)| core.snippets_list(vault).ok())
            .unwrap_or_default();
        let mut tags: Vec<String> = Vec::new();
        for tag in snippets.into_iter().flat_map(|snippet| snippet.tags) {
            if !has_tag(&tags, &tag) {
                tags.push(tag);
            }
        }
        tags.sort_by_key(|tag| tag.to_lowercase());
        tags
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
        let tags = self.all_typed_tags(cx);
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

    /// Adds the tags of `typed` (comma-separated) as chips; one the core
    /// would refuse leaves its reason under the field.
    pub fn add_tags(&mut self, typed: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut added = false;
        for raw in typed.split(',').filter(|raw| !raw.trim().is_empty()) {
            match normalize_tag(raw) {
                Ok(tag) => {
                    if !has_tag(&self.tags, &tag) {
                        self.tags.push(tag);
                    }
                    added = true;
                }
                Err(reason) => {
                    self.errors.insert("tags", reason_text(reason));
                }
            }
        }
        if added && !typed.contains(',') {
            picker::clear_search(&self.tag_input, window, cx);
        }
        cx.notify();
    }

    pub fn remove_tag(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tags.len() {
            self.tags.remove(index);
            self.errors.remove("tags");
            cx.notify();
        }
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
        let script = Editor::new(&self.script)
            .h(SCRIPT_HEIGHT)
            .text_size(metrics::FONT_SIZE_DENSE)
            .font_family(crate::terminal::fonts::JETBRAINS_MONO)
            .bg(tokens.bg_input.hsla())
            .rounded(metrics::RADIUS_CONTROL)
            .when(self.errors.contains_key("script"), |this| {
                this.border_color(danger)
            });
        let tags = self.render_tags(window, cx);
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
                widgets::field(strings::TAGS, tags, self.errors.get("tags").copied(), cx)
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

    /// The chips, the field and, while something is typed, the matching
    /// tags of the vault's snippets (or "Create tag").
    fn render_tags(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let chips: Vec<AnyElement> = self
            .tags
            .iter()
            .enumerate()
            .map(|(index, tag)| {
                let remove = widgets::remove_button(
                    SharedString::from(format!("snippet-tag-remove-{index}")),
                    cx,
                )
                .on_click(
                    cx.listener(move |form, _: &ClickEvent, _, cx| form.remove_tag(index, cx)),
                );
                widgets::chip(
                    SharedString::from(format!("snippet-tag-{index}")),
                    tag.clone(),
                    remove,
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let query = self.tag_input.read(cx).value().trim().to_owned();
        let suggestions = (!query.is_empty()).then(|| {
            let all = self.vault_tags(cx);
            let mut rows: Vec<PickerRow> = all
                .iter()
                .filter(|tag| picker::matches(&query, tag) && !has_tag(&self.tags, tag))
                .take(8)
                .enumerate()
                .map(|(index, tag)| {
                    PickerRow::new(
                        format!("suggest-{index}"),
                        PickValue::Tag(tag.clone()),
                        tag.clone(),
                    )
                })
                .collect();
            if !has_tag(&all, &query) && !has_tag(&self.tags, &query) {
                rows.push(PickerRow::new(
                    "create",
                    PickValue::Create(query.clone()),
                    crate::panels::strings::create_tag(&query),
                ));
            }
            picker::render_list(
                "snippet-tag",
                None,
                rows,
                window,
                cx,
                Rc::new(|form: &mut Self, value, window, cx| {
                    if let PickValue::Tag(tag) | PickValue::Create(tag) = value {
                        form.add_tags(&tag, window, cx);
                        picker::clear_search(&form.tag_input, window, cx);
                    }
                }),
            )
        });
        v_flex()
            .id("snippet-tags-field")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_2)
            // Backspace in the empty field removes the last chip (the field
            // passes it on when there is nothing before the cursor).
            .on_action(cx.listener(|form, _: &Backspace, _, cx| {
                if form.tag_input.read(cx).value().is_empty() && !form.tags.is_empty() {
                    let last = form.tags.len() - 1;
                    form.remove_tag(last, cx);
                } else {
                    cx.propagate();
                }
            }))
            .when(!chips.is_empty(), |this| {
                this.child(h_flex().flex_wrap().gap(metrics::SPACE_1).children(chips))
            })
            .child(widgets::text_input(
                &self.tag_input,
                "snippet-tags",
                self.errors.contains_key("tags"),
                window,
                cx,
            ))
            .children(suggestions)
            .into_any_element()
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
