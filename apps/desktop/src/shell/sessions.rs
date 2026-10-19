//! Session tabs of the top bar (UI 1.1, 4.6): the model only. Terminals and
//! SFTP sessions are not built yet, so every session tab shows the "In
//! development" placeholder (audit decision 6); the tab itself already
//! behaves as specified: OS icon + name, close button on the active and the
//! hovered tab, type icon on the right, the neighbor rule of 4.6 on close.

use gpui::SharedString;
use gpui_kit::assets::IconName as Lucide;
use uuid::Uuid;

use crate::i18n;

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
    /// The new tab page of "+" (UI 6) until a host is chosen.
    NewTab,
}

impl SessionKind {
    /// The icon on the right side of the tab: terminal or SFTP (UI 1.1).
    /// The new tab page has none.
    pub fn type_icon(self) -> Option<Lucide> {
        match self {
            SessionKind::Ssh | SessionKind::Local | SessionKind::Serial => {
                Some(Lucide::SquareTerminal)
            }
            SessionKind::Sftp => Some(Lucide::FolderOpen),
            SessionKind::NewTab => None,
        }
    }

    /// The icon on the left when the tab has no host OS icon.
    pub fn default_icon(self) -> Lucide {
        match self {
            SessionKind::Ssh | SessionKind::Sftp => Lucide::Server,
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
    icon: Option<Lucide>,
}

impl SessionRequest {
    pub fn new(kind: SessionKind) -> Self {
        Self {
            kind,
            title: None,
            host: None,
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

    /// The host OS icon (neutral server icon when not set).
    pub fn icon(mut self, icon: Lucide) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }
}

/// One session tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTab {
    id: SessionId,
    kind: SessionKind,
    title: SharedString,
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

    pub fn title(&self) -> &SharedString {
        &self.title
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
        self.last_id += 1;
        let id = self.last_id;
        let kind = request.kind;
        self.tabs.push(SessionTab {
            id,
            kind,
            title: request.title.unwrap_or_else(|| kind.default_title().into()),
            host: request.host,
            icon: request.icon.unwrap_or_else(|| kind.default_icon()),
        });
        id
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
}
