//! The group panel (UI 3.4): view, create and edit a group in the details
//! panel, saved automatically by the rule of UI 1.3.
//!
//! Blocks from top to bottom: Vault (`group-block-vault`), Name
//! (`group-block-name`, required), Parent Group (`group-block-parent`, with
//! search; empty is the top level; never the group itself or its subtree),
//! Color (`group-block-color`, 8 `--tag-*`, graphite by default) and SSH
//! (`group-block-ssh`: username, identity or key, port, Host Chaining). SSH
//! values inherited from the parent groups show as "from group X".
//!
//! The "…" menu: Open, Edit, Move to ▸, Remove ([`super::menus::GROUP_MENU`]).
//! Removing asks first; a non-empty group's subgroups and hosts move one level
//! up.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, MouseDownEvent, Render,
    SharedString, Subscription, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, IconName, Sizable as _, h_flex, v_flex,
};
use shellpp_core::CoreError;
use shellpp_core::service::dto::RecordBase;
use uuid::Uuid;

use super::autosave::Autosave;
use super::menus::{self, GROUP_MENU, GroupMenuAction};
use super::picker::{self, OnPick, PickValue, PickerRow};
use super::{PanelEvent, PanelRequest, strings, widgets};
use crate::shell::details::DetailsHeader;
use crate::state::records_edit::{FieldErrors, GroupDraft, MAX_JUMP_HOSTS};
use crate::state::{AppState, AppStateEvent};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{controls, metrics, palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dropdown {
    Vault,
    ParentGroup,
    JumpHost,
}

pub struct GroupPanel {
    state: Entity<AppState>,
    record_id: Option<Uuid>,
    draft: GroupDraft,
    /// The version the form's fields were read from, updated by each save:
    /// a save writes only what the form changed against it (D220).
    origin: Option<RecordBase>,
    errors: FieldErrors,
    dropdown: Option<Dropdown>,
    name: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    group_search: Entity<InputState>,
    jump_search: Entity<InputState>,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for GroupPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupPanel")
            .field("record_id", &self.record_id)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<PanelEvent> for GroupPanel {}
impl EventEmitter<PanelRequest> for GroupPanel {}

fn new_input(
    placeholder: impl Into<SharedString>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<GroupPanel>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let value = value.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

impl GroupPanel {
    /// A panel for an existing group, or an empty one (`None`) for a new
    /// group inside the current location, with the focus in Name.
    pub fn new(
        state: Entity<AppState>,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let group = record_id.and_then(|id| state.read(cx).group(id).cloned());
        let record_id = group.as_ref().map(|group| group.record_id);
        let origin = group.as_ref().map(|group| RecordBase {
            revision: group.revision,
            data: group.data_map(),
        });
        let draft = match &group {
            Some(group) => GroupDraft::from_group(group),
            None => GroupDraft {
                parent_id: state.read(cx).location(),
                ..GroupDraft::default()
            },
        };
        let name = new_input(strings::NAME_PLACEHOLDER, &draft.name, window, cx);
        let port = new_input(strings::DEFAULT_PORT, &draft.port, window, cx);
        let username = new_input(strings::USERNAME, &draft.username, window, cx);
        let group_search = new_input(strings::SEARCH_GROUPS, "", window, cx);
        let jump_search = new_input(strings::SEARCH_HOSTS, "", window, cx);
        let mut subscriptions = Vec::new();
        for input in [&name, &port, &username] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        this.edited(cx);
                    }
                },
            ));
        }
        for search in [&group_search, &jump_search] {
            subscriptions.push(cx.subscribe_in(
                search,
                window,
                |_: &mut Self, _, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &state,
            window,
            |this: &mut Self, _, event: &AppStateEvent, window, cx| {
                if let AppStateEvent::RecordsChanged = event {
                    this.on_records_changed(window, cx);
                }
            },
        ));
        let mut this = Self {
            state,
            record_id,
            draft,
            origin,
            errors: FieldErrors::new(),
            dropdown: None,
            name,
            port,
            username,
            group_search,
            jump_search,
            autosave: Autosave::default(),
            _subscriptions: subscriptions,
        };
        this.refresh_placeholders(window, cx);
        if this.record_id.is_none() {
            this.focus_name(window, cx);
        }
        this
    }

    pub fn record_id(&self) -> Option<Uuid> {
        self.record_id
    }

    pub fn draft(&self) -> &GroupDraft {
        &self.draft
    }

    pub fn errors(&self) -> &FieldErrors {
        &self.errors
    }

    pub fn title(&self) -> &'static str {
        if self.record_id.is_some() {
            strings::GROUP_DETAILS
        } else {
            strings::NEW_GROUP
        }
    }

    pub fn has_pending_save(&self) -> bool {
        self.autosave.is_pending()
    }

    /// The placeholders of Port and Username (inherited: "from group X").
    pub fn ssh_placeholders(&self, cx: &gpui::App) -> (SharedString, SharedString) {
        (
            self.port.read(cx).presentation().placeholder().clone(),
            self.username.read(cx).presentation().placeholder().clone(),
        )
    }

    /// Puts the focus in Name (New Group, Edit).
    pub fn focus_name(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Saves a pending edit now; a draft without a name is discarded.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if self.autosave.is_pending() {
            self.save_now(cx);
        }
        self.autosave.cancel();
    }

    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.autosave.take_pending();
        self.read_inputs(cx);
        let record_id = self.record_id;
        let draft = self.draft.clone();
        let origin = self.origin.clone();
        let report = self.state.update(cx, |state, cx| {
            state.save_group_draft(record_id, &draft, origin.as_ref(), cx)
        });
        if report.origin.is_some() {
            self.origin = report.origin;
        }
        if let Some(group) = report.saved {
            self.record_id = Some(group.record_id);
        }
        self.errors = report.errors;
        cx.notify();
    }

    fn read_inputs(&mut self, cx: &gpui::App) {
        self.draft.name = self.name.read(cx).value().to_string();
        self.draft.port = self.port.read(cx).value().to_string();
        self.draft.username = self.username.read(cx).value().to_string();
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.read_inputs(cx);
        let mut errors = self.draft.validate();
        if self.record_id.is_none() && self.draft.name.trim().is_empty() {
            errors.remove("name");
        }
        self.errors
            .retain(|path, _| path != "name" && path != "ssh.port");
        self.errors.extend(errors);
        self.changed(cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.autosave.schedule(cx, |this, cx| this.save_now(cx));
        cx.notify();
    }

    fn on_records_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.record_id {
            let Some(group) = self.state.read(cx).group(id).cloned() else {
                self.autosave.cancel();
                cx.emit(PanelEvent::Close);
                return;
            };
            // The parent of a stored group changes only through
            // `move_records` (Parent Group here, at once; Move to and drags
            // elsewhere), never through the autosave: always take it from the
            // stored record, or a pending autosave would move the group back.
            self.draft.parent_id = group.parent_id;
            if !self.autosave.is_pending() {
                // A color picked elsewhere.
                self.draft.color = GroupDraft::from_group(&group).color;
            }
        }
        self.refresh_placeholders(window, cx);
        cx.notify();
    }

    fn refresh_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let inherited = state.inherited_ssh(self.draft.parent_id);
        let group_name = |id: Uuid| {
            state
                .group(id)
                .map(|group| group.name.clone())
                .unwrap_or_default()
        };
        let port = inherited
            .port
            .as_ref()
            .map(|port| strings::inherited(&port.value.to_string(), &group_name(port.group_id)))
            .unwrap_or_else(|| strings::DEFAULT_PORT.to_owned());
        let username = inherited
            .username
            .as_ref()
            .map(|user| strings::inherited(&user.value, &group_name(user.group_id)))
            .unwrap_or_else(|| strings::USERNAME.to_owned());
        self.port
            .update(cx, |input, cx| input.set_placeholder(port, window, cx));
        self.username
            .update(cx, |input, cx| input.set_placeholder(username, window, cx));
    }

    fn toggle_dropdown(&mut self, dropdown: Dropdown, window: &mut Window, cx: &mut Context<Self>) {
        if self.dropdown == Some(dropdown) {
            self.dropdown = None;
        } else {
            self.dropdown = Some(dropdown);
            let search = match dropdown {
                Dropdown::ParentGroup => Some(self.group_search.clone()),
                Dropdown::JumpHost => Some(self.jump_search.clone()),
                Dropdown::Vault => None,
            };
            if let Some(search) = search {
                picker::clear_search(&search, window, cx);
                search.update(cx, |input, cx| input.focus(window, cx));
            }
        }
        cx.notify();
    }

    /// Esc closes the open dropdown, then the panel (the shell saves a
    /// pending edit first).
    fn on_escape(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key != "escape" || keystroke.modifiers.modified() {
            return;
        }
        cx.stop_propagation();
        if self.dropdown.take().is_some() {
            cx.notify();
        } else {
            cx.emit(PanelEvent::Close);
        }
    }

    /// A click outside an open dropdown closes it.
    fn close_dropdown(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dropdown.take().is_some() {
            cx.notify();
        }
    }

    fn pick(&mut self, value: PickValue, window: &mut Window, cx: &mut Context<Self>) {
        match value {
            PickValue::TopLevel => self.set_parent(None, cx),
            PickValue::Group(id) => self.set_parent(Some(id), cx),
            PickValue::Host(id) => {
                let mut chain = self.draft.jump_chain.clone().unwrap_or_default();
                if !chain.contains(&id) && chain.len() < MAX_JUMP_HOSTS {
                    chain.push(id);
                }
                self.draft.jump_chain = Some(chain);
                self.changed(cx);
            }
            // Values of the host panel's pickers only.
            _ => {}
        }
        self.dropdown = None;
        self.refresh_placeholders(window, cx);
        cx.notify();
    }

    /// Parent Group: a stored group moves there at once (Move to); a draft
    /// just remembers it.
    fn set_parent(&mut self, parent: Option<Uuid>, cx: &mut Context<Self>) {
        self.errors.remove("parent_id");
        if self.record_id.is_some() {
            if self.draft.parent_id != parent {
                self.move_to(parent, cx);
            }
        } else {
            self.draft.parent_id = parent;
            self.changed(cx);
        }
    }

    fn on_pick(&self) -> OnPick<Self> {
        Rc::new(
            |this: &mut Self, value: PickValue, window: &mut Window, cx: &mut Context<Self>| {
                this.pick(value, window, cx)
            },
        )
    }

    fn placeholder(&mut self, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Toast(strings::IN_DEVELOPMENT.into()));
    }

    /// Runs an item of the group menu (UI 3.4).
    pub fn run_menu_action(
        &mut self,
        action: GroupMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush(cx);
        let Some(id) = self.record_id else {
            return;
        };
        match action {
            GroupMenuAction::Open => {
                self.state
                    .update(cx, |state, cx| state.set_location(Some(id), cx));
            }
            GroupMenuAction::Edit => self.focus_name(window, cx),
            // The targets are in the submenu (`move_to`).
            GroupMenuAction::MoveTo => {}
            GroupMenuAction::Remove => {
                let panel = cx.entity().downgrade();
                menus::confirm_remove_group(
                    self.state.clone(),
                    id,
                    window,
                    cx,
                    move |removed, _, cx| {
                        let _ = panel.update(cx, |panel, cx| {
                            panel.autosave.cancel();
                            cx.emit(PanelRequest::Removed(removed));
                            cx.emit(PanelEvent::Close);
                        });
                    },
                );
            }
        }
    }

    /// Move to ▸ a target (`None`: the top level).
    pub fn move_to(&mut self, target: Option<Uuid>, cx: &mut Context<Self>) {
        self.flush(cx);
        let Some(id) = self.record_id else {
            return;
        };
        let result = self
            .state
            .update(cx, |state, cx| state.move_records(vec![id], target, cx));
        match result {
            Ok(()) => {
                self.draft.parent_id = target;
                cx.notify();
            }
            Err(CoreError::Validation(fields)) => {
                // A cycle or a too deep tree: shown under Parent Group.
                for field in fields {
                    self.errors.insert("parent_id".into(), field.reason);
                }
                cx.notify();
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "group not moved");
                cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
            }
        }
    }

    fn error(&self, prefix: &str) -> Option<&'static str> {
        self.errors
            .iter()
            .find(|(path, _)| {
                path.as_str() == prefix
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('.'))
            })
            .map(|(path, reason)| strings::reason(path, *reason))
    }

    // -- Rendering ----------------------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let exists = self.record_id.is_some();
        let choices = self
            .record_id
            .map(|id| menus::group_move_choices(self.state.read(cx), id))
            .unwrap_or_default();
        let collapse_panel = panel.clone();
        DetailsHeader::new(self.title())
            .menu(move |mut menu, window, cx| {
                for action in GROUP_MENU {
                    let panel = panel.clone();
                    if action == GroupMenuAction::MoveTo {
                        let choices = choices.clone();
                        let disabled = !exists || choices.is_empty();
                        if disabled {
                            menu = menu.item(PopupMenuItem::new(action.label()).disabled(true));
                            continue;
                        }
                        menu = menu.submenu(action.label(), window, cx, move |mut sub, _, _| {
                            for (target, title) in &choices {
                                let panel = panel.clone();
                                let target = *target;
                                sub = sub.item(PopupMenuItem::new(title.clone()).on_click(
                                    move |_, _, cx| {
                                        let _ = panel.update(cx, |panel, cx| {
                                            panel.move_to(target, cx);
                                        });
                                    },
                                ));
                            }
                            sub
                        });
                        continue;
                    }
                    menu = menu.item(menus::menu_item(
                        action.label(),
                        action.is_danger(),
                        !exists,
                        move |window, cx| {
                            let _ = panel.update(cx, |panel, cx| {
                                panel.run_menu_action(action, window, cx);
                            });
                        },
                    ));
                }
                menu
            })
            .on_collapse(move |_, _, cx| {
                let _ = collapse_panel.update(cx, |panel, cx| {
                    panel.flush(cx);
                    cx.emit(PanelEvent::Close);
                });
            })
    }

    fn render_vault(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.dropdown == Some(Dropdown::Vault);
        let trigger = widgets::select_trigger(
            "group-vault",
            Some(crate::app::assets::vault_icon().small().into_any_element()),
            strings::PERSONAL_VAULT,
            open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_dropdown(Dropdown::Vault, window, cx)
        }));
        let list = open.then(|| {
            picker::render_list(
                "group-vault",
                None,
                vec![
                    PickerRow::new(
                        "personal",
                        PickValue::PersonalVault,
                        strings::PERSONAL_VAULT,
                    )
                    .checked(true),
                ],
                window,
                cx,
                Rc::new(
                    |this: &mut Self, _: PickValue, _: &mut Window, cx: &mut Context<Self>| {
                        this.dropdown = None;
                        cx.notify();
                    },
                ),
            )
        });
        widgets::block("group-block-vault", None, cx).child(widgets::field(
            strings::VAULT,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(trigger)
                .children(list)
                .when(open, |this| {
                    this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                }),
            None,
            cx,
        ))
    }

    fn render_name(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.error("name");
        widgets::block("group-block-name", None, cx).child(widgets::field(
            strings::NAME,
            h_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .items_center()
                .child(widgets::tile(
                    Lucide::Folder,
                    self.draft.color,
                    metrics::HEIGHT_CONTROL,
                ))
                .child(div().flex_1().min_w_0().child(widgets::text_input(
                    &self.name,
                    "group-name",
                    error.is_some(),
                    window,
                    cx,
                ))),
            error,
            cx,
        ))
    }

    fn render_parent(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.dropdown == Some(Dropdown::ParentGroup);
        let state = self.state.read(cx);
        let current = self
            .draft
            .parent_id
            .map(|id| state.group_path_label(id))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| strings::TOP_LEVEL.to_owned());
        let current_color = self
            .draft
            .parent_id
            .and_then(|id| state.group(id))
            .map(|group| group.color)
            .unwrap_or_default();
        let query = self.group_search.read(cx).value().to_string();
        let rows: Option<Vec<PickerRow>> = open.then(|| {
            let targets: Vec<Uuid> = match self.record_id {
                Some(id) => state
                    .group_move_targets(id)
                    .iter()
                    .map(|group| group.record_id)
                    .collect(),
                None => state.groups().iter().map(|group| group.record_id).collect(),
            };
            let mut rows = Vec::new();
            if picker::matches(&query, strings::TOP_LEVEL) {
                rows.push(
                    PickerRow::new("top", PickValue::TopLevel, strings::TOP_LEVEL)
                        .checked(self.draft.parent_id.is_none()),
                );
            }
            for id in targets {
                let Some(group) = state.group(id) else {
                    continue;
                };
                let path = state.group_path_label(id);
                if !picker::matches(&query, &path) {
                    continue;
                }
                rows.push(
                    PickerRow::new(id.to_string(), PickValue::Group(id), path)
                        .checked(self.draft.parent_id == Some(id))
                        .leading(widgets::tile(
                            Lucide::Folder,
                            group.color,
                            metrics::rpx(20.),
                        )),
                );
            }
            rows
        });
        let trigger = widgets::select_trigger(
            "group-parent",
            Some(
                widgets::tile(Lucide::Folder, current_color, metrics::rpx(20.)).into_any_element(),
            ),
            current,
            open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_dropdown(Dropdown::ParentGroup, window, cx)
        }));
        let list = rows.map(|rows| {
            picker::render_list(
                "group-parent",
                Some((&self.group_search, "group-parent-search")),
                rows,
                window,
                cx,
                self.on_pick(),
            )
        });
        widgets::block("group-block-parent", None, cx).child(widgets::field(
            strings::PARENT_GROUP,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(trigger)
                .children(list)
                .when(open, |this| {
                    this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                }),
            self.error("parent_id"),
            cx,
        ))
    }

    fn render_color(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let swatches: Vec<AnyElement> = palette::TAG_COLORS
            .iter()
            .map(|color| {
                let color = *color;
                widgets::swatch("group", color, self.draft.color == color, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.draft.color != color {
                            this.draft.color = color;
                            this.changed(cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        widgets::block("group-block-color", None, cx).child(widgets::field(
            strings::COLOR,
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(metrics::SPACE_2)
                .children(swatches),
            None,
            cx,
        ))
    }

    fn render_ssh(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port_error = self.error("ssh.port");
        let username_error = self.error("ssh.username");
        let state = self.state.read(cx);
        let inherited = state.inherited_ssh(self.draft.parent_id);
        let title_of = |id: Uuid| {
            state
                .host(id)
                .map(|host| host.title().to_owned())
                .unwrap_or_else(|| id.to_string())
        };
        let group_name = |id: Uuid| {
            state
                .group(id)
                .map(|group| group.name.clone())
                .unwrap_or_default()
        };
        // Identity or key: linked on the group, else inherited.
        let data = self
            .record_id
            .and_then(|id| state.group(id))
            .map(|group| group.data.clone());
        let own_credential = data
            .as_ref()
            .and_then(|data| data.get("ssh"))
            .is_some_and(|ssh| ssh.get("identity_id").is_some() || ssh.get("key_id").is_some());
        let credential_text = if own_credential {
            Some(format!(
                "{} \u{b7} {}",
                strings::CREDENTIAL_KEY,
                strings::LINKED
            ))
        } else {
            inherited
                .identity_id
                .as_ref()
                .map(|value| (strings::CREDENTIAL_IDENTITY, value.group_id))
                .or_else(|| {
                    inherited
                        .key_id
                        .as_ref()
                        .map(|value| (strings::CREDENTIAL_KEY, value.group_id))
                })
                .map(|(label, group)| {
                    format!("{label} \u{b7} {}", strings::from_group(&group_name(group)))
                })
        };
        let chain = self.draft.jump_chain.clone().unwrap_or_default();
        let inherited_chain = inherited
            .jump_chain
            .as_ref()
            .filter(|_| self.draft.jump_chain.is_none())
            .map(|chain| {
                let hops = if chain.value.is_empty() {
                    strings::DIRECT.to_owned()
                } else {
                    chain
                        .value
                        .iter()
                        .map(|id| title_of(*id))
                        .collect::<Vec<_>>()
                        .join(" \u{2192} ")
                };
                strings::inherited(&hops, &group_name(chain.group_id))
            });
        let chip_titles: Vec<String> = chain.iter().map(|id| title_of(*id)).collect();
        let query = self.jump_search.read(cx).value().to_string();
        let open = self.dropdown == Some(Dropdown::JumpHost);
        let rows: Option<Vec<PickerRow>> = open.then(|| {
            state
                .hosts()
                .iter()
                .filter(|host| !chain.contains(&host.record_id))
                .filter(|host| {
                    picker::matches(&query, host.title()) || picker::matches(&query, &host.address)
                })
                .take(50)
                .map(|host| {
                    PickerRow::new(
                        host.record_id.to_string(),
                        PickValue::Host(host.record_id),
                        host.title().to_owned(),
                    )
                    .secondary(host.address.clone())
                    .leading(widgets::tile(
                        crate::hosts::layout::os_icon(host.os_family.as_deref()),
                        host.color,
                        metrics::rpx(20.),
                    ))
                })
                .collect()
        });

        let chips: Vec<AnyElement> = chip_titles
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                let remove = widgets::remove_button(
                    SharedString::from(format!("group-jump-remove-{index}")),
                    cx,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(chain) = this.draft.jump_chain.as_mut()
                        && index < chain.len()
                    {
                        chain.remove(index);
                        if chain.is_empty() {
                            this.draft.jump_chain = None;
                        }
                        this.changed(cx);
                    }
                }));
                widgets::chip(
                    SharedString::from(format!("group-jump-{index}")),
                    title,
                    remove,
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let surface = cx.theme().secondary;
        let add_credential = Button::new("group-add-credential")
            .custom(controls::ghost(cx))
            .small()
            .rounded(metrics::to_px(metrics::RADIUS_CONTROL, cx))
            .focus_ring(false)
            .focus_ring_visible(surface, cx)
            .icon(IconName::Plus)
            .tooltip(strings::ADD_CREDENTIAL)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.placeholder(cx)));
        let add_jump = widgets::text_button(
            "group-jump-add",
            strings::ADD_JUMP_HOST,
            chain.len() < MAX_JUMP_HOSTS,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_dropdown(Dropdown::JumpHost, window, cx)
        }));
        let list = rows.map(|rows| {
            picker::render_list(
                "group-jump",
                Some((&self.jump_search, "group-jump-search")),
                rows,
                window,
                cx,
                self.on_pick(),
            )
        });
        let protocol_line = h_flex()
            .w_full()
            .gap(metrics::SPACE_3)
            .items_center()
            .child(
                div()
                    .flex_none()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(strings::SSH),
            )
            .child(widgets::caption(strings::PORT, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(metrics::rpx(96.))
                    .child(widgets::text_input(
                        &self.port,
                        "group-port",
                        port_error.is_some(),
                        window,
                        cx,
                    )),
            );
        widgets::block("group-block-ssh", None, cx)
            .child(protocol_line)
            .when_some(port_error, |this, error| {
                this.child(widgets::error_text(error, cx))
            })
            .child(widgets::field(
                strings::USERNAME,
                widgets::text_input(
                    &self.username,
                    "group-username",
                    username_error.is_some(),
                    window,
                    cx,
                ),
                username_error,
                cx,
            ))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(cx.theme().muted_foreground)
                            .children(credential_text),
                    )
                    .child(add_credential),
            )
            .child(widgets::field(
                strings::HOST_CHAINING,
                v_flex()
                    .id("group-jump-hosts")
                    .test_support()
                    .w_full()
                    .gap(metrics::SPACE_1)
                    .when(!chips.is_empty(), |this| {
                        this.child(h_flex().flex_wrap().gap(metrics::SPACE_1).children(chips))
                    })
                    .when_some(inherited_chain, |this, text| {
                        this.child(widgets::caption(text, cx))
                    })
                    .child(add_jump)
                    .children(list)
                    .when(open, |this| {
                        this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                    }),
                self.error("ssh.jump_chain"),
                cx,
            ))
    }
}

impl Render for GroupPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window: &Window = window;
        v_flex()
            .id("group-panel")
            .test_support()
            .on_key_down(cx.listener(Self::on_escape))
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(self.render_header(cx))
            .child(
                v_flex()
                    .id("group-panel-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(metrics::SPACE_4)
                    .gap(metrics::SPACE_3)
                    // UI 3.4, blocks from top to bottom.
                    .child(self.render_vault(window, cx))
                    .child(self.render_name(window, cx))
                    .child(self.render_parent(window, cx))
                    .child(self.render_color(cx))
                    .child(self.render_ssh(window, cx)),
            )
    }
}
