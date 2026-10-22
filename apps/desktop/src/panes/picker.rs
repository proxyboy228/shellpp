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
        // Enter on the first row opens another session of the same kind: a
        // Telnet pane's host by Telnet again (D87).
        let telnet_origin = match &origin {
            Some((PaneTarget::Telnet { record_id, .. }, _)) => Some(*record_id),
            _ => None,
        };
        let origin = origin.and_then(|(target, title)| match target {
            PaneTarget::Host { record_id, .. } | PaneTarget::Telnet { record_id, .. } => {
                Some(PickerOrigin::Host(record_id))
            }
            PaneTarget::Quick(quick) => Some(PickerOrigin::Quick(quick)),
            PaneTarget::Local { shell_id } => Some(PickerOrigin::Local { shell_id, title }),
            // A port opens once (D88): nothing to offer first.
            PaneTarget::Serial(_) => None,
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
                    let target = if telnet_origin == Some(*host_id) {
                        PaneTarget::telnet(state, *host_id)
                    } else {
                        PaneTarget::primary(state, *host_id)
                    };
                    let Some(target) = target else {
                        return;
                    };
                    open(target, *new_window)
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
