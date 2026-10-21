//! Export to host (UI 7.3, the key panel's "Key export" block; stage 4,
//! item 8; D33): the saved hosts with a search and checkboxes, then the
//! result on each host. The key is appended to `~/.ssh/authorized_keys`
//! like `ssh-copy-id` (`shellpp_ssh::export_public_key`) over a connection
//! without a tab: saved credentials, the agent and key files, and only a
//! host key Known Hosts holds (D61), at most four hosts at once.

use std::collections::BTreeSet;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, IntoElement, SharedString, Subscription, Window,
    div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use shellpp_core::service::dto::RecordRef;
use shellpp_ssh::{ExportOutcome, FailureKind};

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// Where the export stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Choosing,
    Running,
    /// The result on each chosen host, in the order of the list.
    Done(Vec<(RecordRef, Result<ExportOutcome, FailureKind>)>),
}

/// The Export to host dialog.
pub struct ExportToHost {
    state: Entity<AppState>,
    public_key: String,
    search: Entity<InputState>,
    chosen: BTreeSet<uuid::Uuid>,
    step: Step,
    _subscription: Subscription,
}

impl std::fmt::Debug for ExportToHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportToHost")
            .field("chosen", &self.chosen.len())
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

/// The text of one host's result.
pub fn result_text(result: &Result<ExportOutcome, FailureKind>) -> &'static str {
    match result {
        Ok(ExportOutcome::Added) => strings::EXPORT_ADDED,
        Ok(ExportOutcome::AlreadyPresent) => strings::EXPORT_ALREADY,
        Err(FailureKind::HostKeyRejected) => strings::EXPORT_UNKNOWN_KEY,
        Err(FailureKind::AuthFailed | FailureKind::Cancelled) => strings::EXPORT_NO_CREDENTIALS,
        Err(kind) => {
            crate::session_ui::bridge::failure_of(crate::sessions::SessionError::Ssh(*kind)).text()
        }
    }
}

/// The height of a host row (and of the "Select all" row).
const ROW_HEIGHT: gpui::Pixels = px(32.);

impl ExportToHost {
    pub fn new(
        state: Entity<AppState>,
        public_key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(strings::EXPORT_SEARCH));
        let subscription = cx.subscribe(&search, |_: &mut Self, _, _: &InputEvent, cx| cx.notify());
        search.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            public_key,
            search,
            chosen: BTreeSet::new(),
            step: Step::Choosing,
            _subscription: subscription,
        }
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    pub fn chosen(&self) -> usize {
        self.chosen.len()
    }

    /// The hosts the list shows for the search: (host, title).
    pub fn hosts(&self, cx: &App) -> Vec<(RecordRef, String)> {
        let text = self.search.read(cx).value().trim().to_lowercase();
        self.state
            .read(cx)
            .hosts()
            .iter()
            .filter(|host| {
                text.is_empty()
                    || host.address.to_lowercase().contains(&text)
                    || host
                        .label
                        .as_deref()
                        .is_some_and(|label| label.to_lowercase().contains(&text))
            })
            .map(|host| {
                (
                    RecordRef {
                        vault_id: host.vault_id,
                        record_id: host.record_id,
                    },
                    host.label.clone().unwrap_or_else(|| host.address.clone()),
                )
            })
            .collect()
    }

    pub fn toggle(&mut self, host: uuid::Uuid, cx: &mut Context<Self>) {
        if !self.chosen.remove(&host) {
            self.chosen.insert(host);
        }
        cx.notify();
    }

    /// Whether every host the search shows is chosen.
    pub fn all_shown_chosen(&self, cx: &App) -> bool {
        let shown = self.hosts(cx);
        !shown.is_empty()
            && shown
                .iter()
                .all(|(host, _)| self.chosen.contains(&host.record_id))
    }

    /// "Select all": every host the search shows; when they all are
    /// chosen already, none of them (hosts chosen under another search
    /// stay).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let shown: Vec<uuid::Uuid> = self
            .hosts(cx)
            .into_iter()
            .map(|(host, _)| host.record_id)
            .collect();
        if self.all_shown_chosen(cx) {
            for id in &shown {
                self.chosen.remove(id);
            }
        } else {
            self.chosen.extend(shown);
        }
        cx.notify();
    }

    /// Export: the key on every chosen host.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chosen.is_empty() || self.step == Step::Running {
            return;
        }
        let Some(manager) = crate::sessions::ui::manager(cx) else {
            return;
        };
        let hosts: Vec<RecordRef> = self
            .state
            .read(cx)
            .hosts()
            .iter()
            .filter(|host| self.chosen.contains(&host.record_id))
            .map(|host| RecordRef {
                vault_id: host.vault_id,
                record_id: host.record_id,
            })
            .collect();
        let work = manager.export_key(hosts, self.public_key.clone());
        self.step = Step::Running;
        cx.spawn_in(window, async move |this, cx| {
            let results = work.await;
            let _ = this.update(cx, |this, cx| {
                this.step = Step::Done(results);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ExportToHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let titles: std::collections::HashMap<uuid::Uuid, String> = self
            .state
            .read(cx)
            .hosts()
            .iter()
            .map(|host| {
                (
                    host.record_id,
                    host.label.clone().unwrap_or_else(|| host.address.clone()),
                )
            })
            .collect();
        let body = match &self.step {
            Step::Done(results) => v_flex()
                .id("kc-export-results")
                .test_support()
                .gap(metrics::SPACE_2)
                .children(results.iter().map(|(host, result)| {
                    h_flex()
                        .id(SharedString::from(format!(
                            "kc-export-result-{}",
                            host.record_id
                        )))
                        .test_support()
                        .gap(metrics::SPACE_3)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .child(titles.get(&host.record_id).cloned().unwrap_or_default()),
                        )
                        .child(
                            div()
                                .text_size(metrics::FONT_SIZE_DENSE)
                                .text_color(if result.is_ok() {
                                    muted
                                } else {
                                    cx.theme().danger
                                })
                                .child(result_text(result)),
                        )
                }))
                .into_any_element(),
            Step::Running => div()
                .id("kc-export-running")
                .test_support()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(muted)
                .child(strings::EXPORT_RUNNING)
                .into_any_element(),
            Step::Choosing => {
                let hosts = self.hosts(cx);
                let all = self.all_shown_chosen(cx);
                let hover = cx.theme().secondary_hover;
                let select_all = h_flex()
                    .id("kc-export-select-row")
                    .test_support()
                    .h(ROW_HEIGHT)
                    .px(metrics::SPACE_2)
                    .items_center()
                    .justify_between()
                    .child(
                        Checkbox::new("kc-export-select-all")
                            .checked(all)
                            .label(strings::EXPORT_SELECT_ALL)
                            .disabled(hosts.is_empty())
                            .on_click(cx.listener(|this, _: &bool, _, cx| this.select_all(cx))),
                    )
                    .child(
                        div()
                            .id("kc-export-count")
                            .test_support()
                            .text_size(metrics::FONT_SIZE_DENSE)
                            .text_color(muted)
                            .child(strings::export_selected(self.chosen.len())),
                    );
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(widgets::caption(strings::EXPORT_HOSTS_HINT, cx))
                    .child(widgets::text_input(
                        &self.search,
                        "kc-export-search",
                        false,
                        window,
                        cx,
                    ))
                    .child(select_all)
                    .child(
                        v_flex()
                            .id("kc-export-hosts")
                            .test_support()
                            .h(px(224.))
                            .overflow_y_scroll()
                            .when(hosts.is_empty(), |this| {
                                this.child(
                                    div()
                                        .p(metrics::SPACE_2)
                                        .text_size(metrics::FONT_SIZE_BODY)
                                        .text_color(muted)
                                        .child(strings::EXPORT_NO_HOSTS),
                                )
                            })
                            .children(hosts.into_iter().map(|(host, title)| {
                                let id = host.record_id;
                                // One row of 32 px per host, highlighted under
                                // the pointer.
                                h_flex()
                                    .id(SharedString::from(format!("kc-export-row-{id}")))
                                    .test_support()
                                    .flex_none()
                                    .h(ROW_HEIGHT)
                                    .px(metrics::SPACE_2)
                                    .items_center()
                                    .rounded(metrics::RADIUS_CONTROL)
                                    .hover(move |style| style.bg(hover))
                                    .child(
                                        Checkbox::new(SharedString::from(format!(
                                            "kc-export-host-{id}"
                                        )))
                                        .checked(self.chosen.contains(&id))
                                        .label(title)
                                        .on_click(
                                            cx.listener(move |this, _: &bool, _, cx| {
                                                this.toggle(id, cx)
                                            }),
                                        ),
                                    )
                            })),
                    )
                    .into_any_element()
            }
        };
        v_flex()
            .id("kc-export")
            .test_support()
            .gap(metrics::SPACE_3)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::EXPORT_DIALOG_TITLE),
            )
            .child(body)
            .child(
                h_flex()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("kc-export-close", ControlKind::Neutral)
                            .label(match self.step {
                                Step::Done(_) => strings::EXPORT_CLOSE,
                                _ => strings::EXPORT_CANCEL,
                            })
                            .on_click(|_: &ClickEvent, window, cx| {
                                use gpui_kit::component::WindowExt as _;
                                window.close_dialog(cx)
                            }),
                    )
                    .when(!matches!(self.step, Step::Done(_)), |this| {
                        this.child(
                            ControlButton::new("kc-export-run", ControlKind::Accent)
                                .label(strings::EXPORT_RUN)
                                .disabled(self.chosen.is_empty() || self.step == Step::Running)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.run(window, cx)
                                })),
                        )
                    }),
            )
    }
}

/// Opens the dialog for `public_key`.
pub fn open(state: Entity<AppState>, public_key: String, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    let dialog = cx.new(|cx| ExportToHost::new(state, public_key, window, cx));
    window.open_dialog(cx, move |modal, _, cx| {
        modal
            .w(px(460.))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .child(dialog.clone())
    });
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, TestAppContext, WindowBounds, WindowOptions, size};
    use gpui_kit::test::TestWindowExt as _;
    use serde_json::{Map, Value, json};

    use super::*;
    use crate::testing::TestProfile;

    /// Plan of run 4, item 6: host rows are 32 px high; "Select all" takes
    /// the hosts the search shows (and clears them when all are chosen);
    /// the counter follows.
    #[gpui_kit::test]
    fn rows_select_all_and_the_counter(cx: &mut TestAppContext) {
        cx.update(crate::testing::init_ui);
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            for host in [
                json!({"address": "web1.example.com"}),
                json!({"address": "web2.example.com"}),
                json!({"address": "db.example.com"}),
            ] {
                let data: Map<String, Value> = host.as_object().cloned().unwrap();
                state.save_host(None, data, cx).unwrap();
            }
        });
        let dialog_state = state.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(800.), px(700.)),
            ))),
            ..Default::default()
        };
        let (handle, dialog) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| {
                        ExportToHost::new(dialog_state, "ssh-ed25519 AAAA test".into(), window, cx)
                    })
                })
            })
            .unwrap();
        let frame = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.run_until_parked();
        };
        frame(cx);
        let ids: Vec<uuid::Uuid> = cx.update(|cx| {
            dialog
                .read(cx)
                .hosts(cx)
                .into_iter()
                .map(|(host, _)| host.record_id)
                .collect()
        });
        assert_eq!(ids.len(), 3);
        let row = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .try_find(SharedString::from(format!("kc-export-row-{}", ids[0])))
                    .map(|found| found.bounds().size.height)
            })
            .unwrap();
        assert_eq!(row, Some(ROW_HEIGHT));
        // The search shows the two web hosts; Select all takes them.
        cx.update_window(handle, |_, window, cx| {
            window.click("kc-export-search", cx);
            window.input("web", cx);
        })
        .unwrap();
        frame(cx);
        dialog.update(cx, |dialog, cx| dialog.select_all(cx));
        assert_eq!(dialog.read_with(cx, |dialog, _| dialog.chosen()), 2);
        assert!(cx.update(|cx| dialog.read(cx).all_shown_chosen(cx)));
        assert_eq!(strings::export_selected(2), "2 selected");
        // Again: the shown hosts are cleared.
        dialog.update(cx, |dialog, cx| dialog.select_all(cx));
        assert_eq!(dialog.read_with(cx, |dialog, _| dialog.chosen()), 0);
        // The click on the checkbox itself.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("kc-export-select-all", cx);
        })
        .unwrap();
        frame(cx);
        assert_eq!(dialog.read_with(cx, |dialog, _| dialog.chosen()), 2);
        let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        state.update(cx, |state, _| state.shutdown());
    }
}
