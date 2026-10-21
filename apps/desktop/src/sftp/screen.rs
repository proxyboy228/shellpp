//! The SFTP tab (UI 5.1): two panels of equal width, the local file system
//! on the left and an empty panel on the right by default, and the transfer
//! queue at the bottom once the first transfer is added (UI 5.6).
//!
//! Copying goes from one panel into the folder the other one shows: "Copy
//! to target directory" (UI 5.4) or a drag from one panel onto the other
//! (UI 5.3). A transfer that ends lists its target folder again.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, ExternalPaths, FocusHandle, Focusable, IntoElement, SharedString,
    Subscription, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use shellpp_core::service::dto::RecordRef;

use super::listing::Entry;
use super::open::Opened;
use super::panel::{FilePanel, PanelEvent, Side, Source};
use super::queue::{Place, Transfers, TransfersEvent};
use super::{DragLabel, DraggedFiles};
use crate::state::AppState;

/// The SFTP tab.
pub struct SftpScreen {
    state: Entity<AppState>,
    left: Entity<FilePanel>,
    right: Entity<FilePanel>,
    transfers: Entity<Transfers>,
    /// Remote files open in local programs (UI 5.4).
    opened: Vec<Opened>,
    watch: Option<gpui::Task<()>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for SftpScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpScreen").finish_non_exhaustive()
    }
}

impl Focusable for SftpScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SftpScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let left = cx.new(|cx| FilePanel::new(Side::Left, state.clone(), true, window, cx));
        let right = cx.new(|cx| FilePanel::new(Side::Right, state.clone(), false, window, cx));
        let transfers = cx.new(|cx| Transfers::new(state.clone(), cx));
        let subscriptions = vec![
            cx.subscribe_in(&left, window, |this, _, event, window, cx| {
                this.on_panel(Side::Left, event, window, cx)
            }),
            cx.subscribe_in(&right, window, |this, _, event, window, cx| {
                this.on_panel(Side::Right, event, window, cx)
            }),
            cx.subscribe(
                &transfers,
                |this, _, event: &TransfersEvent, cx| match event {
                    TransfersEvent::Finished(side) => {
                        this.panel(*side).update(cx, |panel, cx| panel.refresh(cx))
                    }
                },
            ),
        ];
        Self {
            state,
            left,
            right,
            transfers,
            opened: Vec::new(),
            watch: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn panel(&self, side: Side) -> &Entity<FilePanel> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    pub fn transfers(&self) -> &Entity<Transfers> {
        &self.transfers
    }

    fn other(side: Side) -> Side {
        match side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Connect → with SFTP (UI 2.4): `host` in the right panel.
    pub fn open_host(
        &mut self,
        host: RecordRef,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right
            .update(cx, |panel, cx| panel.connect(host, title, window, cx));
    }

    fn on_panel(
        &mut self,
        side: Side,
        event: &PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PanelEvent::Changed => cx.notify(),
            PanelEvent::Copy(entries) => self.copy(side, entries.clone(), window, cx),
            PanelEvent::Open(entry) => self.open(side, entry.clone(), None, window, cx),
            PanelEvent::OpenWith(entry) => {
                let entry = entry.clone();
                let chosen = super::open::choose_program(cx);
                cx.spawn_in(window, async move |this, cx| {
                    if let Some(program) = chosen.await {
                        let _ = this.update_in(cx, |this, window, cx| {
                            this.open(side, entry, Some(program), window, cx)
                        });
                    }
                })
                .detach();
            }
        }
    }

    /// The folder a panel shows, as a transfer place.
    fn place(&self, side: Side, cx: &App) -> Option<Place> {
        let panel = self.panel(side).read(cx);
        if panel.path().is_empty() {
            return None;
        }
        match panel.source() {
            Source::Local => Some(Place::Local(PathBuf::from(panel.path()))),
            Source::Remote(_) => Some(Place::Remote {
                handle: panel.handle()?.clone(),
                sftp: panel.sftp()?,
                dir: panel.path().to_owned(),
            }),
            Source::Empty => None,
        }
    }

    /// Copies `entries` of the panel at `from` into the other panel's folder.
    pub fn copy(
        &mut self,
        from: Side,
        entries: Vec<Entry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to = Self::other(from);
        let (Some(source), Some(target)) = (self.place(from, cx), self.place(to, cx)) else {
            return;
        };
        if entries.is_empty() {
            return;
        }
        self.transfers.update(cx, |transfers, cx| {
            transfers.add(entries, source, target, to, window, cx)
        });
        cx.notify();
    }

    /// Open (UI 5.4): in `program` (Open with…), else in "Program for Open",
    /// else in the program the system assigns. A remote file is opened as a
    /// downloaded copy that is uploaded back when it is saved.
    pub fn open(
        &mut self,
        side: Side,
        entry: Entry,
        program: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let program = program.or_else(|| super::open::open_program(self.state.read(cx)));
        let panel = self.panel(side).read(cx);
        if panel.is_local() {
            let path = entry
                .local_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(panel.child_path(&entry.name)));
            super::open::launch(&path, program.as_deref(), window, cx);
            return;
        }
        let (Some(handle), Some(sftp)) = (panel.handle().cloned(), panel.sftp()) else {
            return;
        };
        let Some(root) = self.state.read(cx).tmp_dir() else {
            return;
        };
        let remote = panel.child_path(&entry.name);
        let name = entry.name.clone();
        cx.spawn_in(window, async move |this, cx| {
            let copy = super::open::download(handle.clone(), sftp.clone(), remote, root).await;
            let _ = this.update_in(cx, |this, window, cx| match copy {
                Ok(copy) => {
                    let opened = Opened::new(copy, handle, sftp, name);
                    super::open::launch(opened.local_path(), program.as_deref(), window, cx);
                    crate::shell::show_toast(super::strings::OPEN_NOT_ENCRYPTED, window, cx);
                    this.opened.push(opened);
                    if this.watch.is_none() {
                        let screen = cx.entity().downgrade();
                        this.watch = Some(cx.spawn_in(window, async move |_, cx| {
                            super::open::watch(screen, |this: &mut Self| &mut this.opened, cx).await
                        }));
                    }
                }
                Err(reason) => crate::shell::show_toast(
                    super::strings::failed(super::strings::OPEN_FAILED, &reason),
                    window,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Files dropped on the panel at `side` from the system's file manager
    /// (UI 5.3, stage 4 item 4): copied into its folder, each from its own
    /// folder.
    fn dropped_paths(
        &mut self,
        side: Side,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.place(side, cx) else {
            return;
        };
        for path in paths.paths() {
            let (Some(entry), Some(parent)) = (super::local::entry_of(path), path.parent()) else {
                continue;
            };
            let source = Place::Local(parent.to_path_buf());
            self.transfers.update(cx, |transfers, cx| {
                transfers.add(vec![entry], source, target.clone(), side, window, cx)
            });
        }
        cx.notify();
    }

    /// Files dropped on the panel at `side` from the other one.
    fn dropped(
        &mut self,
        side: Side,
        dragged: &DraggedFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if dragged.side == side {
            return;
        }
        self.copy(dragged.side, dragged.entries.clone(), window, cx);
    }
}

impl Render for SftpScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (drop_bg, border, background) = {
            let theme = cx.theme();
            (theme.drop_target, theme.border, theme.background)
        };
        let side_of = |side: Side, panel: &Entity<FilePanel>, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(match side {
                    Side::Left => "sftp-left",
                    Side::Right => "sftp-right",
                }))
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .drag_over::<DraggedFiles>(move |style, dragged, _, _| {
                    if dragged.side == side {
                        style
                    } else {
                        style.bg(drop_bg)
                    }
                })
                .on_drop(
                    cx.listener(move |this, dragged: &DraggedFiles, window, cx| {
                        this.dropped(side, dragged, window, cx)
                    }),
                )
                .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_bg))
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                    this.dropped_paths(side, paths, window, cx)
                }))
                .child(panel.clone())
        };
        let panels = h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .items_stretch()
            .child(side_of(Side::Left, &self.left.clone(), cx))
            .child(div().w(gpui::px(1.)).h_full().bg(border))
            .child(side_of(Side::Right, &self.right.clone(), cx));
        let _ = DragLabel::default();
        v_flex()
            .id("sftp-screen")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .bg(background)
            .child(panels)
            .when(!self.transfers.read(cx).is_empty(), |this| {
                this.child(self.transfers.clone())
            })
    }
}
