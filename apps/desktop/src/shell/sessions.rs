//! Session tabs of the top bar (UI 1.1, 4.4, 4.6): the model only. A tab
//! shows the host OS icon (the server icon until the OS is known; a
//! monitor for a local terminal; never the terminal glyph), its name, the
//! state dot and the type icon on the right; the close button replaces the
//! OS icon on the active and the hovered tab; closing the active tab
//! applies the neighbor rule of 4.6. The name is the host of the active
//! pane, unless the tab was renamed (4.4: the custom title lives as long as
//! the tab). The content of a tab is owned by the shell.

use gpui::SharedString;
use gpui_kit::assets::IconName as Lucide;
use uuid::Uuid;

use crate::hosts::QuickTarget;
use crate::i18n;

/// Longest title Rename keeps, in characters: what Restore saves
/// (`shellpp_core::service::last_session::MAX_TITLE`).
pub const MAX_TITLE_CHARS: usize = shellpp_core::service::last_session::MAX_TITLE;

/// A title the Rename field accepts while typing: at most
/// [`MAX_TITLE_CHARS`] characters, no control characters (a pasted tab).
pub fn title_ok(text: &str) -> bool {
    text.chars().count() <= MAX_TITLE_CHARS && !text.chars().any(char::is_control)
}

/// A title as Rename keeps it: control characters removed, cut to
/// [`MAX_TITLE_CHARS`] characters, trimmed; `None` when nothing is left.
pub fn clean_title(text: &str) -> Option<String> {
    let title: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

/// Identity of a session tab (unique per window, never reused).
pub type SessionId = u64;

/// What a session tab holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    /// SSH terminal session to a host (Connect, double click, quick connect).
    Ssh,
    /// SFTP session tab ("Connect via SFTP", UI 5.1).
    Sftp,
    /// Local terminal (Terminal button, New Local Terminal).
    Local,
    /// Serial port connection (Serial button, New Serial Connection).
    Serial,
    /// Telnet terminal session to a host's Telnet block (D87).
    Telnet,
    /// The new tab page of "+" (UI 6) until a host is chosen.
    NewTab,
}

impl SessionKind {
    /// The icon on the right side of the tab: terminal or SFTP (UI 1.1).
    /// The new tab page has none.
    pub fn type_icon(self) -> Option<Lucide> {
        match self {
            SessionKind::Ssh | SessionKind::Local | SessionKind::Serial | SessionKind::Telnet => {
                Some(Lucide::SquareTerminal)
            }
            SessionKind::Sftp => Some(Lucide::FolderOpen),
            SessionKind::NewTab => None,
        }
    }

    /// The icon on the left when the tab has no host OS icon.
    pub fn default_icon(self) -> Lucide {
        match self {
            SessionKind::Ssh | SessionKind::Sftp | SessionKind::Telnet => Lucide::Server,
            SessionKind::Local => Lucide::Monitor,
            SessionKind::Serial => Lucide::Usb,
            SessionKind::NewTab => Lucide::LayoutGrid,
        }
    }

    /// The default title of a tab of this kind.
    pub fn default_title(self) -> &'static str {
        match self {
            SessionKind::Ssh => i18n::sessions::SSH,
            SessionKind::Sftp => i18n::sessions::SFTP,
            SessionKind::Local => i18n::sessions::LOCAL_TERMINAL,
            SessionKind::Serial => i18n::sessions::SERIAL,
            SessionKind::Telnet => i18n::sessions::TELNET,
            SessionKind::NewTab => i18n::sessions::NEW_TAB,
        }
    }
}

/// What a caller asks for when opening a session tab
/// ([`crate::shell::open_session_tab`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRequest {
    kind: SessionKind,
    title: Option<SharedString>,
    host: Option<Uuid>,
    /// A quick-connect address of the search bar (no saved host).
    quick: Option<QuickTarget>,
    icon: Option<Lucide>,
}

impl SessionRequest {
    pub fn new(kind: SessionKind) -> Self {
        Self {
            kind,
            title: None,
            host: None,
            quick: None,
            icon: None,
        }
    }

    /// The tab name (the host's label or address); the kind's default title
    /// otherwise.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The saved host the session belongs to.
    pub fn host(mut self, host: Uuid) -> Self {
        self.host = Some(host);
        self
    }

    /// A quick connect without a saved host (UI 2.1).
    pub fn quick(mut self, target: QuickTarget) -> Self {
        self.quick = Some(target);
        self
    }

    /// The host OS icon (neutral server icon when not set).
    pub fn icon(mut self, icon: Lucide) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    /// The saved host, if any.
    pub fn host_id(&self) -> Option<Uuid> {
        self.host
    }

    /// The quick-connect address, if any.
    pub fn quick_target(&self) -> Option<&QuickTarget> {
        self.quick.as_ref()
    }

    /// The tab title asked for, if any.
    pub fn requested_title(&self) -> Option<&SharedString> {
        self.title.as_ref()
    }
}

/// One session tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTab {
    id: SessionId,
    kind: SessionKind,
    /// The host of the active pane (its label or address), the shell's
    /// name of a local terminal, or the kind's default title.
    title: SharedString,
    /// The title set with Rename (UI 4.4), for the lifetime of the tab.
    custom_title: Option<SharedString>,
    host: Option<Uuid>,
    icon: Lucide,
}

impl SessionTab {
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    /// The title on display: the one set with Rename, else the host's.
    pub fn title(&self) -> &SharedString {
        self.custom_title.as_ref().unwrap_or(&self.title)
    }

    /// The host's title (the active pane's host), whatever the tab is
    /// called.
    pub fn host_title(&self) -> &SharedString {
        &self.title
    }

    /// The title set with Rename, if any.
    pub fn custom_title(&self) -> Option<&SharedString> {
        self.custom_title.as_ref()
    }

    /// Stable element id of the tab's state dot.
    pub fn dot_element_id(&self) -> SharedString {
        format!("session-tab-{}-dot", self.id).into()
    }

    /// Stable element id of the tab's rename field.
    pub fn rename_element_id(&self) -> SharedString {
        format!("session-tab-{}-rename", self.id).into()
    }

    pub fn host(&self) -> Option<Uuid> {
        self.host
    }

    /// The OS (or kind) icon on the left.
    pub fn icon(&self) -> Lucide {
        self.icon
    }

    /// Stable element id of the tab (`session-tab-<id>`).
    pub fn element_id(&self) -> SharedString {
        format!("session-tab-{}", self.id).into()
    }

    /// Stable element id of the tab's close button.
    pub fn close_element_id(&self) -> SharedString {
        format!("session-tab-{}-close", self.id).into()
    }
}

/// The open session tabs of one window, left to right.
#[derive(Clone, Debug, Default)]
pub struct SessionTabs {
    tabs: Vec<SessionTab>,
    last_id: SessionId,
}

impl SessionTabs {
    pub fn tabs(&self) -> &[SessionTab] {
        &self.tabs
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn get(&self, id: SessionId) -> Option<&SessionTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn position(&self, id: SessionId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// Adds a tab at the right end and returns its id.
    pub fn open(&mut self, request: SessionRequest) -> SessionId {
        self.open_at(self.tabs.len(), request)
    }

    /// Adds a tab right after `after` (Duplicate, UI 4.4), at the right end
    /// when `after` is not open.
    pub fn open_after(&mut self, after: SessionId, request: SessionRequest) -> SessionId {
        let index = self
            .position(after)
            .map_or(self.tabs.len(), |index| index + 1);
        self.open_at(index, request)
    }

    fn open_at(&mut self, index: usize, request: SessionRequest) -> SessionId {
        self.last_id += 1;
        let id = self.last_id;
        let kind = request.kind;
        self.tabs.insert(
            index.min(self.tabs.len()),
            SessionTab {
                id,
                kind,
                title: request.title.unwrap_or_else(|| kind.default_title().into()),
                custom_title: None,
                host: request.host,
                icon: request.icon.unwrap_or_else(|| kind.default_icon()),
            },
        );
        id
    }

    /// The new tab page of `id` chose its target (UI 6: the session opens
    /// in the same tab): the tab takes the request's kind, host, title and
    /// icon; a title set with Rename stays. Returns whether the tab exists.
    pub fn replace(&mut self, id: SessionId, request: SessionRequest) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        let kind = request.kind;
        tab.kind = kind;
        tab.title = request.title.unwrap_or_else(|| kind.default_title().into());
        tab.host = request.host;
        tab.icon = request.icon.unwrap_or_else(|| kind.default_icon());
        true
    }

    /// Rename (UI 4.4): `Some` sets the tab's own title ([`clean_title`]:
    /// no control characters, at most [`MAX_TITLE_CHARS`]); `None`, or a
    /// title that is empty after that, brings back the host's. Returns
    /// whether anything changed.
    pub fn rename(&mut self, id: SessionId, title: Option<&str>) -> bool {
        let title = title.and_then(clean_title).map(SharedString::from);
        match self.tabs.iter_mut().find(|tab| tab.id == id) {
            Some(tab) if tab.custom_title != title => {
                tab.custom_title = title;
                true
            }
            _ => false,
        }
    }

    /// The saved host of the active pane changed (a split pane became
    /// active, UI 4.7).
    pub fn set_host(&mut self, id: SessionId, host: Option<Uuid>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.host = host;
        }
    }

    /// Removes a tab. Returns the tab that becomes active when `id` was the
    /// active one (UI 4.6): the neighbor on the left, else the one on the
    /// right; `None` when no session tab remains (Vaults opens).
    pub fn close(&mut self, id: SessionId) -> Option<SessionId> {
        let index = self.position(id)?;
        self.tabs.remove(index);
        if index > 0 {
            Some(self.tabs[index - 1].id)
        } else {
            self.tabs.first().map(|tab| tab.id)
        }
    }

    /// The tab at `index` (0-based among session tabs).
    pub fn at(&self, index: usize) -> Option<&SessionTab> {
        self.tabs.get(index)
    }

    /// Changes a tab's title and icon (a local terminal's shell name, the
    /// host OS detected on connect). Returns whether anything changed.
    pub fn retitle(&mut self, id: SessionId, title: SharedString, icon: Lucide) -> bool {
        match self.tabs.iter_mut().find(|tab| tab.id == id) {
            Some(tab) if tab.title != title || tab.icon != icon => {
                tab.title = title;
                tab.icon = icon;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(count: usize) -> (SessionTabs, Vec<SessionId>) {
        let mut tabs = SessionTabs::default();
        let ids = (0..count)
            .map(|_| tabs.open(SessionRequest::new(SessionKind::Local)))
            .collect();
        (tabs, ids)
    }

    #[test]
    fn closing_activates_the_left_neighbor_then_the_right_one() {
        let (mut tabs, ids) = tabs(3);
        // Middle tab: the left neighbor.
        assert_eq!(tabs.close(ids[1]), Some(ids[0]));
        // First tab: there is no left neighbor, the right one.
        assert_eq!(tabs.close(ids[0]), Some(ids[2]));
        // The last tab: none remain, Vaults opens.
        assert_eq!(tabs.close(ids[2]), None);
        assert!(tabs.is_empty());
        // Unknown ids change nothing.
        assert_eq!(tabs.close(42), None);
    }

    #[test]
    fn ids_are_never_reused_and_titles_default_by_kind() {
        let (mut tabs, ids) = tabs(2);
        tabs.close(ids[1]);
        let next = tabs.open(SessionRequest::new(SessionKind::NewTab));
        assert!(next > ids[1]);
        assert_eq!(
            tabs.get(next).unwrap().title().as_ref(),
            i18n::sessions::NEW_TAB
        );
        let host = Uuid::new_v4();
        let ssh = tabs.open(
            SessionRequest::new(SessionKind::Ssh)
                .title("web1")
                .host(host),
        );
        let tab = tabs.get(ssh).unwrap();
        assert_eq!(tab.title().as_ref(), "web1");
        assert_eq!(tab.host(), Some(host));
        assert_eq!(tab.icon(), Lucide::Server);
        assert_eq!(tab.kind().type_icon(), Some(Lucide::SquareTerminal));
        assert_eq!(SessionKind::Sftp.type_icon(), Some(Lucide::FolderOpen));
        assert_eq!(SessionKind::NewTab.type_icon(), None);
    }

    /// UI 1.1: the left icon of a tab is never the terminal glyph of its
    /// type icon.
    #[test]
    fn the_left_icon_is_never_the_type_icon() {
        for kind in [
            SessionKind::Ssh,
            SessionKind::Sftp,
            SessionKind::Local,
            SessionKind::Serial,
            SessionKind::NewTab,
        ] {
            assert_ne!(Some(kind.default_icon()), kind.type_icon(), "{kind:?}");
            assert_ne!(kind.default_icon(), Lucide::SquareTerminal, "{kind:?}");
        }
    }

    /// UI 4.4: Rename sets the tab's own title; empty brings the host's
    /// back; the host's title keeps following the active pane.
    #[test]
    fn rename_and_duplicate_positions() {
        let (mut tabs, ids) = tabs(2);
        assert!(tabs.rename(ids[0], Some("  prod db  ")));
        assert!(!tabs.rename(ids[0], Some("prod db")), "unchanged");
        let tab = tabs.get(ids[0]).unwrap();
        assert_eq!(tab.title().as_ref(), "prod db");
        assert_eq!(tab.host_title().as_ref(), i18n::sessions::LOCAL_TERMINAL);
        tabs.retitle(ids[0], "pwsh".into(), Lucide::Monitor);
        assert_eq!(tabs.get(ids[0]).unwrap().title().as_ref(), "prod db");
        assert!(tabs.rename(ids[0], Some("   ")));
        assert_eq!(tabs.get(ids[0]).unwrap().title().as_ref(), "pwsh");
        assert!(!tabs.rename(ids[0], None));
        // Control characters are dropped and the title is cut to what
        // Restore keeps.
        assert!(tabs.rename(ids[0], Some("prod\tdb\u{7}")));
        assert_eq!(tabs.get(ids[0]).unwrap().title().as_ref(), "proddb");
        let long = "t".repeat(MAX_TITLE_CHARS + 10);
        assert!(tabs.rename(ids[0], Some(&long)));
        assert_eq!(
            tabs.get(ids[0]).unwrap().title().chars().count(),
            MAX_TITLE_CHARS
        );
        assert!(tabs.rename(ids[0], Some("\u{7}\t")));
        assert_eq!(tabs.get(ids[0]).unwrap().custom_title(), None);
        assert!(title_ok("prod db"));
        assert!(!title_ok("a\tb"));
        assert!(!title_ok(&long));
        // A duplicate opens right after its tab.
        let copy = tabs.open_after(ids[0], SessionRequest::new(SessionKind::Local));
        assert_eq!(tabs.position(copy), Some(1));
        assert_eq!(tabs.position(ids[1]), Some(2));
        // The new tab page's choice turns the tab into a session tab.
        let page = tabs.open(SessionRequest::new(SessionKind::NewTab));
        assert!(
            tabs.replace(
                page,
                SessionRequest::new(SessionKind::Ssh)
                    .title("web")
                    .host(Uuid::nil())
            )
        );
        let tab = tabs.get(page).unwrap();
        assert_eq!(tab.kind(), SessionKind::Ssh);
        assert_eq!(tab.title().as_ref(), "web");
        assert_eq!(tab.host(), Some(Uuid::nil()));
    }
}
