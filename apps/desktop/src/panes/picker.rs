//! The host selection of a new pane (UI 4.7): the compact new tab page
//! ([`CompactPicker`]) with the split pane's target first, so Enter opens
//! another session to it. This module adapts the picker to the pane group:
//! it creates it for a pane and turns its events into a [`Choice`].

use gpui::prelude::*;
use gpui::{
    AnyView, Context, Entity, FocusHandle, Focusable as _, SharedString, Subscription, Window,
};

use super::PaneGroup;
use super::target::PaneTarget;
use crate::new_tab::{CompactPicker, NewTabEvent, PickerOrigin};
use crate::state::AppState;

/// What the picker of a pane asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Open a session to this target in the pane. Alt held (Alt+Enter,
    /// Alt + click) opens it here too: the pane needs a session
    /// [decision].
    Open(PaneTarget),
    /// Esc: the pane closes (the split is undone).
    Dismiss,
}

/// The picker shown in a pane.
pub struct Picker {
    view: Entity<CompactPicker>,
    _subscription: Subscription,
}

impl Picker {
    /// The picker for a new pane split from a pane with `origin` (its
    /// target and title); its choices go to `on_choice` on the group.
    pub fn new(
        state: &Entity<AppState>,
        origin: Option<(PaneTarget, SharedString)>,
        window: &mut Window,
        cx: &mut Context<PaneGroup>,
        on_choice: impl Fn(&mut PaneGroup, Choice, &mut Window, &mut Context<PaneGroup>) + 'static,
    ) -> Self {
        let origin = origin.map(|(target, title)| match target {
            // A Telnet pane's host comes first too (its protocol is chosen in
            // the list).
            PaneTarget::Host { record_id, .. } | PaneTarget::Telnet { record_id, .. } => {
                PickerOrigin::Host(record_id)
            }
            PaneTarget::Quick(quick) => PickerOrigin::Quick(quick),
            PaneTarget::Local { shell_id } => PickerOrigin::Local { shell_id, title },
        });
        let view = {
            let state = state.clone();
            cx.new(|cx| CompactPicker::new(state, origin, window, cx))
        };
        let state = state.clone();
        let subscription = cx.subscribe_in(&view, window, move |group, _, event, window, cx| {
            let choice = match event {
                NewTabEvent::OpenHost {
                    host_id,
                    new_window,
                } => {
                    let state = state.read(cx);
                    let Some(vault_id) = state
                        .host(*host_id)
                        .map(|host| host.vault_id)
                        .or_else(|| state.vault_id())
                    else {
                        return;
                    };
                    open(
                        PaneTarget::Host {
                            vault_id,
                            record_id: *host_id,
                        },
                        *new_window,
                    )
                }
                NewTabEvent::QuickConnect { target, new_window } => {
                    open(PaneTarget::Quick(target.clone()), *new_window)
                }
                NewTabEvent::OpenLocal {
                    shell_id,
                    new_window,
                } => open(
                    PaneTarget::Local {
                        shell_id: shell_id.clone(),
                    },
                    *new_window,
                ),
                NewTabEvent::Dismissed => Choice::Dismiss,
                // The pane picker lists no open tabs and has no Restore.
                NewTabEvent::ActivateTab { .. }
                | NewTabEvent::Restore
                | NewTabEvent::CreateWorkspace
                | NewTabEvent::OpenWorkspace(_)
                | NewTabEvent::RemoveWorkspace(_)
                | NewTabEvent::RenameWorkspace(_) => return,
            };
            on_choice(group, choice, window, cx)
        });
        Self {
            view,
            _subscription: subscription,
        }
    }

    pub fn view(&self) -> AnyView {
        self.view.clone().into()
    }

    /// The search field.
    pub fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.view.focus_handle(cx)
    }
}

fn open(target: PaneTarget, _new_window: bool) -> Choice {
    Choice::Open(target)
}
