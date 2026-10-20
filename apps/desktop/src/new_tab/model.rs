//! The lists of the new tab page, the Ctrl+K overlay and the pane picker
//! (UI 6, 4.7) as plain data: which rows a list shows for a search text and
//! what choosing a row does. No GPUI state here, so the rules are tested
//! without a window.
//!
//! - **Empty search**: Recent connections (the core's connection log rows:
//!   newest first, one per target, at most 10). The overlay and the pane
//!   picker put the heading "Recent connections" over them (the page shows
//!   it in its own header with the Restore button). The pane picker starts
//!   with the split pane's target ([`PickerOrigin`]) and leaves that target
//!   out of the recent rows below it.
//! - **Typing**: open tabs first (title, or the tab's host by name, address,
//!   user or tag), then hosts (the same fields as the Hosts search, UI 2.1),
//!   each under its heading. A host whose label or address is exactly the
//!   text comes first among the hosts. When the text is a connection address
//!   (UI 2.1) and no host has exactly that label or address, the last row
//!   connects to it without saving ("Connect to ..."). The pane picker shows
//!   no open tabs: its pane needs a new session.

use gpui::{AnyWindowHandle, SharedString};
use gpui_kit::assets::IconName;
use shellpp_core::service::dto::{OsFamily, RecentConnection, TagColor};
use uuid::Uuid;

use super::strings;
use super::{NewTabEvent, OpenTab, PickerOrigin};
use crate::hosts::{QuickTarget, layout::os_icon, parse_address};
use crate::shell::SessionId;
use crate::state::hosts_view::text_matches_host;
use crate::state::{GroupItem, HostItem};

/// The SSH port a quick-connect address leaves out.
const DEFAULT_SSH_PORT: u16 = 22;

/// Which list is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    /// The new tab page (its header names Recent connections).
    Page,
    /// The Ctrl+K overlay.
    Overlay,
    /// The host picker of a new pane.
    Compact,
}

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Bring an open tab (and its window) to the front.
    Tab {
        window: AnyWindowHandle,
        tab: SessionId,
    },
    /// Connect to a saved host.
    Host(Uuid),
    /// Connect to an address without saving it.
    Quick(QuickTarget),
    /// Open a local terminal (`None`: the default shell).
    Local { shell_id: Option<String> },
}

impl Choice {
    /// The event of this choice; `new_window` (Alt held) opens a session in
    /// a new window (UI 2.4). An open tab is only brought to the front.
    pub fn event(&self, new_window: bool) -> NewTabEvent {
        match self {
            Choice::Tab { window, tab } => NewTabEvent::ActivateTab {
                window: *window,
                tab: *tab,
            },
            Choice::Host(host_id) => NewTabEvent::OpenHost {
                host_id: *host_id,
                new_window,
            },
            Choice::Quick(target) => NewTabEvent::QuickConnect {
                target: target.clone(),
                new_window,
            },
            Choice::Local { shell_id } => NewTabEvent::OpenLocal {
                shell_id: shell_id.clone(),
                new_window,
            },
        }
    }
}

/// One choosable row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Unique within the list; the row's element id is `{prefix}-row-{key}`.
    pub key: SharedString,
    pub choice: Choice,
    /// The OS icon (the server icon while unknown), or the tab's icon.
    pub icon: IconName,
    /// The tile color (the host's color, else `--tag-graphite`).
    pub color: TagColor,
    pub title: SharedString,
    /// Right-aligned secondary text: "vault / group", or the address of a
    /// quick connect whose title is a former label.
    pub detail: Option<SharedString>,
}

/// One entry of a list: a section heading or a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Heading(&'static str),
    Row(Row),
}

impl Item {
    pub fn row(&self) -> Option<&Row> {
        match self {
            Item::Row(row) => Some(row),
            Item::Heading(_) => None,
        }
    }
}

/// What a list is built from.
#[derive(Clone, Copy, Debug)]
pub struct Sources<'a> {
    pub hosts: &'a [HostItem],
    pub groups: &'a [GroupItem],
    pub tabs: &'a [OpenTab],
    pub recents: &'a [RecentConnection],
    pub origin: Option<&'a PickerOrigin>,
    /// The personal vault's name: the first part of a host's path.
    pub vault_name: &'a str,
}

/// The entries of a list for the search text `query`.
pub fn items(kind: PickerKind, query: &str, sources: &Sources<'_>) -> Vec<Item> {
    let query = query.trim();
    if query.is_empty() {
        idle_items(kind, sources)
    } else {
        search_items(kind, query, sources)
    }
}

/// The index of the first row (the default selection).
pub fn first_row(items: &[Item]) -> Option<usize> {
    items.iter().position(|item| item.row().is_some())
}

/// The row after (`forward`) or before `current`; stays put at either end.
pub fn step(items: &[Item], current: Option<usize>, forward: bool) -> Option<usize> {
    let Some(current) = current.filter(|ix| *ix < items.len()) else {
        return first_row(items);
    };
    let found = if forward {
        (current + 1..items.len()).find(|ix| items[*ix].row().is_some())
    } else {
        (0..current).rev().find(|ix| items[*ix].row().is_some())
    };
    Some(found.unwrap_or(current))
}

fn idle_items(kind: PickerKind, sources: &Sources<'_>) -> Vec<Item> {
    let mut items = Vec::new();
    let origin = match kind {
        PickerKind::Compact => sources
            .origin
            .and_then(|origin| origin_row(origin, sources)),
        PickerKind::Page | PickerKind::Overlay => None,
    };
    let recents: Vec<Row> = sources
        .recents
        .iter()
        .map(|recent| recent_row(recent, sources))
        .filter(|row| {
            origin
                .as_ref()
                .is_none_or(|origin| origin.choice != row.choice)
        })
        .collect();
    if let Some(origin) = origin {
        items.push(Item::Row(origin));
    }
    if !recents.is_empty() && kind != PickerKind::Page {
        items.push(Item::Heading(strings::RECENT_CONNECTIONS));
    }
    items.extend(recents.into_iter().map(Item::Row));
    items
}

fn search_items(kind: PickerKind, query: &str, sources: &Sources<'_>) -> Vec<Item> {
    let mut items = Vec::new();
    if kind != PickerKind::Compact {
        let tabs: Vec<Row> = sources
            .tabs
            .iter()
            .filter(|tab| tab_matches(tab, query, sources.hosts))
            .map(|tab| tab_row(tab, sources.hosts))
            .collect();
        if !tabs.is_empty() {
            items.push(Item::Heading(strings::OPEN_TABS));
            items.extend(tabs.into_iter().map(Item::Row));
        }
    }
    let exact = crate::hosts::exact_match(sources.hosts, query);
    let mut hosts: Vec<&HostItem> = sources
        .hosts
        .iter()
        .filter(|host| Some(host.record_id) == exact || text_matches_host(host, query))
        .collect();
    // The exact match first, the rest in the vault's order (stable sort).
    hosts.sort_by_key(|host| Some(host.record_id) != exact);
    if !hosts.is_empty() {
        items.push(Item::Heading(strings::HOSTS));
        items.extend(
            hosts
                .into_iter()
                .map(|host| Item::Row(host_row(host, "host", sources))),
        );
    }
    if exact.is_none()
        && let Some(target) = parse_address(query)
    {
        items.push(Item::Row(Row {
            key: "quick".into(),
            title: strings::connect_to(&target.display()).into(),
            choice: Choice::Quick(target),
            icon: IconName::Server,
            color: TagColor::Graphite,
            detail: None,
        }));
    }
    items
}

/// An open tab matches by its title, or by its host (name, address, user,
/// tags) when it has one.
fn tab_matches(tab: &OpenTab, query: &str, hosts: &[HostItem]) -> bool {
    crate::panels::picker::matches(query, tab.title())
        || tab
            .host_id()
            .and_then(|id| hosts.iter().find(|host| host.record_id == id))
            .is_some_and(|host| text_matches_host(host, query))
}

fn tab_row(tab: &OpenTab, hosts: &[HostItem]) -> Row {
    let color = tab
        .host_id()
        .and_then(|id| hosts.iter().find(|host| host.record_id == id))
        .map_or(TagColor::Graphite, |host| host.color);
    Row {
        key: format!("tab-{}-{}", window_key(tab.window()), tab.tab()).into(),
        choice: Choice::Tab {
            window: tab.window(),
            tab: tab.tab(),
        },
        icon: tab.icon_name(),
        color,
        title: tab.title().clone(),
        detail: None,
    }
}

/// A short stable key of a window for element ids.
fn window_key(window: AnyWindowHandle) -> u64 {
    window.window_id().as_u64()
}

/// A saved host's row; `key` names the list part (`host`, `recent-host`,
/// `origin`).
fn host_row(host: &HostItem, key: &str, sources: &Sources<'_>) -> Row {
    Row {
        key: format!("{key}-{}", host.record_id).into(),
        choice: Choice::Host(host.record_id),
        icon: os_icon(host.os_family.as_deref()),
        color: host.color,
        title: host.title().to_owned().into(),
        detail: Some(host_path(host, sources).into()),
    }
}

/// "vault / group / subgroup" of a host (UI 6).
pub fn host_path(host: &HostItem, sources: &Sources<'_>) -> String {
    std::iter::once(sources.vault_name)
        .chain(host.path.iter().filter_map(|id| {
            sources
                .groups
                .iter()
                .find(|group| group.record_id == *id)
                .map(|group| group.name.as_str())
        }))
        .collect::<Vec<_>>()
        .join(strings::PATH_SEPARATOR)
}

/// A row of Recent connections. A row of a saved host connects to the host;
/// any other row (a quick connect, a removed host) connects to the address,
/// port and user it recorded (UI 6).
fn recent_row(recent: &RecentConnection, sources: &Sources<'_>) -> Row {
    let host = recent.host.and_then(|link| {
        sources
            .hosts
            .iter()
            .find(|host| host.record_id == link.record_id)
    });
    let os = recent.os.as_ref().map(|os| os_family_name(os.family));
    if let Some(host) = host {
        return Row {
            key: format!("recent-host-{}", host.record_id).into(),
            choice: Choice::Host(host.record_id),
            icon: os_icon(host.os_family.as_deref().or(os)),
            color: host.color,
            title: recent.title.clone().into(),
            detail: (!recent.path.is_empty())
                .then(|| recent.path.join(strings::PATH_SEPARATOR).into()),
        };
    }
    let target = QuickTarget {
        username: recent.username.clone(),
        host: recent.address.clone(),
        port: (recent.port != DEFAULT_SSH_PORT).then_some(recent.port),
    };
    let display = target.display();
    Row {
        key: format!("recent-{}-{}", recent.address.to_lowercase(), recent.port).into(),
        icon: os_icon(os),
        color: TagColor::Graphite,
        title: recent.title.clone().into(),
        detail: (display != recent.title).then(|| display.into()),
        choice: Choice::Quick(target),
    }
}

/// The first row of the pane picker: another session to the split pane's
/// target. `None` when its host no longer exists.
fn origin_row(origin: &PickerOrigin, sources: &Sources<'_>) -> Option<Row> {
    match origin {
        PickerOrigin::Host(id) => sources
            .hosts
            .iter()
            .find(|host| host.record_id == *id)
            .map(|host| host_row(host, "origin", sources)),
        PickerOrigin::Quick(target) => Some(Row {
            key: "origin".into(),
            choice: Choice::Quick(target.clone()),
            icon: IconName::Server,
            color: TagColor::Graphite,
            title: target.display().into(),
            detail: None,
        }),
        PickerOrigin::Local { shell_id, title } => Some(Row {
            key: "origin".into(),
            choice: Choice::Local {
                shell_id: shell_id.clone(),
            },
            icon: IconName::Monitor,
            color: TagColor::Graphite,
            title: title.clone(),
            detail: Some(strings::LOCAL_TERMINAL.into()),
        }),
    }
}

/// The `os.family` spelling of the data model (`linux`, `windows`, ...).
fn os_family_name(family: OsFamily) -> &'static str {
    match family {
        OsFamily::Linux => "linux",
        OsFamily::Windows => "windows",
        OsFamily::Macos => "macos",
        OsFamily::Bsd => "bsd",
        OsFamily::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shellpp_core::service::dto::{HostOs, OsSource, RecordRef};

    use super::*;

    fn host(label: Option<&str>, address: &str, path: Vec<Uuid>) -> HostItem {
        HostItem {
            record_id: Uuid::new_v4(),
            vault_id: Uuid::nil(),
            revision: 1,
            label: label.map(str::to_owned),
            address: address.to_owned(),
            tags: vec!["prod".to_owned()],
            color: TagColor::Ocean,
            username: Some("deploy".to_owned()),
            port: None,
            os_family: None,
            group_id: path.last().copied(),
            path,
            created_at: String::new(),
            modified_at: String::new(),
            data: json!({}),
            secrets_set: Vec::new(),
        }
    }

    fn group(name: &str) -> GroupItem {
        GroupItem {
            record_id: Uuid::new_v4(),
            vault_id: Uuid::nil(),
            revision: 1,
            name: name.to_owned(),
            color: TagColor::Graphite,
            parent_id: None,
            path: Vec::new(),
            host_count: 0,
            child_count: 0,
            created_at: String::new(),
            modified_at: String::new(),
            data: json!({}),
        }
    }

    fn recent(host: Option<&HostItem>, title: &str, address: &str, port: u16) -> RecentConnection {
        RecentConnection {
            host: host.map(|host| RecordRef {
                vault_id: host.vault_id,
                record_id: host.record_id,
            }),
            title: title.to_owned(),
            address: address.to_owned(),
            port,
            username: Some("root".to_owned()),
            os: Some(HostOs {
                family: OsFamily::Linux,
                distro: None,
                version: None,
                source: OsSource::Banner,
                detected_at: String::new(),
            }),
            path: if host.is_some() {
                vec!["Personal".to_owned(), "Prod".to_owned()]
            } else {
                Vec::new()
            },
            last_at: String::new(),
        }
    }

    fn sources<'a>(
        hosts: &'a [HostItem],
        groups: &'a [GroupItem],
        tabs: &'a [OpenTab],
        recents: &'a [RecentConnection],
        origin: Option<&'a PickerOrigin>,
    ) -> Sources<'a> {
        Sources {
            hosts,
            groups,
            tabs,
            recents,
            origin,
            vault_name: "Personal",
        }
    }

    fn titles(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Heading(heading) => format!("# {heading}"),
                Item::Row(row) => row.title.to_string(),
            })
            .collect()
    }

    #[test]
    fn idle_lists_show_recent_connections() {
        let prod = group("Prod");
        let web = host(Some("web"), "web.lan", vec![prod.record_id]);
        let hosts = [web.clone()];
        let groups = [prod];
        let recents = [
            recent(None, "203.0.113.4", "203.0.113.4", 2222),
            recent(Some(&web), "web", "web.lan", 22),
            recent(None, "Old label", "198.51.100.7", 22),
        ];
        let sources = sources(&hosts, &groups, &[], &recents, None);
        // The page has its own header: rows only, in the core's order.
        let page = items(PickerKind::Page, "  ", &sources);
        assert_eq!(titles(&page), ["203.0.113.4", "web", "Old label"]);
        // The overlay names the collection.
        let overlay = items(PickerKind::Overlay, "", &sources);
        assert_eq!(overlay[0], Item::Heading(strings::RECENT_CONNECTIONS));
        assert_eq!(overlay.len(), 4);

        let Item::Row(quick) = &page[0] else { panic!() };
        assert_eq!(
            quick.choice,
            Choice::Quick(QuickTarget {
                username: Some("root".into()),
                host: "203.0.113.4".into(),
                port: Some(2222),
            })
        );
        assert_eq!(quick.detail.as_deref(), Some("root@203.0.113.4:2222"));
        assert_eq!(quick.icon, crate::app::assets::LINUX_ICON);
        let Item::Row(saved) = &page[1] else { panic!() };
        assert_eq!(saved.choice, Choice::Host(web.record_id));
        assert_eq!(saved.detail.as_deref(), Some("Personal / Prod"));
        assert_eq!(saved.color, TagColor::Ocean);
        // A removed host's row connects to what it recorded (port 22 is
        // left out of the address).
        let Item::Row(removed) = &page[2] else {
            panic!()
        };
        assert_eq!(
            removed.choice,
            Choice::Quick(QuickTarget {
                username: Some("root".into()),
                host: "198.51.100.7".into(),
                port: None,
            })
        );
        assert_eq!(removed.detail.as_deref(), Some("root@198.51.100.7"));
        assert_eq!(first_row(&overlay), Some(1));
    }

    #[gpui_kit::test]
    fn searching_lists_open_tabs_first_then_hosts(cx: &mut gpui::TestAppContext) {
        let prod = group("Prod");
        let web = host(Some("web"), "web.lan", vec![prod.record_id]);
        let mail = host(Some("Mail"), "mail.lan", Vec::new());
        let db = host(None, "db.web.lan", Vec::new());
        let hosts = [db.clone(), web.clone(), mail.clone()];
        let groups = [prod];
        cx.add_empty_window();
        let window = cx.windows()[0];
        let tabs = [
            OpenTab::new(window, 3, "web").host(Some(web.record_id)),
            OpenTab::new(window, 4, "Local terminal").icon(IconName::Monitor),
        ];
        let sources = sources(&hosts, &groups, &tabs, &[], None);

        // "WEB" is the label of `web` (case-insensitive): it leads the hosts.
        let found = items(PickerKind::Page, "WEB", &sources);
        assert_eq!(
            titles(&found),
            ["# Open tabs", "web", "# Hosts", "web", "db.web.lan"]
        );
        let Item::Row(tab) = &found[1] else { panic!() };
        assert_eq!(tab.choice, Choice::Tab { window, tab: 3 });
        assert_eq!(tab.color, TagColor::Ocean);
        assert_eq!(first_row(&found), Some(1));
        // The host's other fields find its tab too (a tag here).
        assert_eq!(
            titles(&items(PickerKind::Overlay, "prod", &sources))[..2],
            ["# Open tabs", "web"]
        );

        // An exact label or address comes first among the hosts (before
        // `db.web.lan`, which is earlier in the vault), and no quick-connect
        // row is offered for it. The tab matches through its host's address.
        let exact = items(PickerKind::Page, "web.lan", &sources);
        assert_eq!(
            titles(&exact),
            ["# Open tabs", "web", "# Hosts", "web", "db.web.lan"]
        );
        assert_eq!(exact[3].row().unwrap().choice, Choice::Host(web.record_id));

        // The pane picker never lists open tabs.
        assert_eq!(
            titles(&items(PickerKind::Compact, "terminal", &sources)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_address_without_a_saved_host_connects_without_saving() {
        let hosts = [host(Some("web"), "web.lan", Vec::new())];
        let sources = sources(&hosts, &[], &[], &[], None);
        let found = items(PickerKind::Page, "ssh admin@192.0.2.9 -p 2200", &sources);
        let [Item::Row(row)] = &found[..] else {
            panic!("{found:?}")
        };
        assert_eq!(row.title.as_ref(), "Connect to admin@192.0.2.9:2200");
        assert_eq!(
            row.choice.event(true),
            NewTabEvent::QuickConnect {
                target: QuickTarget {
                    username: Some("admin".into()),
                    host: "192.0.2.9".into(),
                    port: Some(2200),
                },
                new_window: true,
            }
        );
        // A word that is not an address only searches.
        assert!(items(PickerKind::Page, "nothing", &sources).is_empty());
        // Hosts that match come before the address row.
        let found = items(PickerKind::Compact, "deploy@web", &sources);
        assert_eq!(titles(&found), ["Connect to deploy@web"]);
        let found = items(PickerKind::Compact, "web.l", &sources);
        assert_eq!(titles(&found), ["# Hosts", "web", "Connect to web.l"]);
    }

    #[test]
    fn the_pane_picker_starts_with_the_split_target() {
        let web = host(Some("web"), "web.lan", Vec::new());
        let hosts = [web.clone()];
        let recents = [
            recent(Some(&web), "web", "web.lan", 22),
            recent(None, "203.0.113.4", "203.0.113.4", 22),
        ];
        let origin = PickerOrigin::Host(web.record_id);
        let found = items(
            PickerKind::Compact,
            "",
            &sources(&hosts, &[], &[], &recents, Some(&origin)),
        );
        // The split host first, then the other recent rows (not the host again).
        assert_eq!(
            titles(&found),
            ["web", "# Recent connections", "203.0.113.4"]
        );
        assert_eq!(first_row(&found), Some(0));
        assert_eq!(
            found[0].row().unwrap().choice.event(false),
            NewTabEvent::OpenHost {
                host_id: web.record_id,
                new_window: false,
            }
        );

        let local = PickerOrigin::Local {
            shell_id: Some("pwsh".into()),
            title: "PowerShell 7".into(),
        };
        let found = items(
            PickerKind::Compact,
            "",
            &sources(&hosts, &[], &[], &[], Some(&local)),
        );
        assert_eq!(titles(&found), ["PowerShell 7"]);
        assert_eq!(
            found[0].row().unwrap().choice.event(false),
            NewTabEvent::OpenLocal {
                shell_id: Some("pwsh".into()),
                new_window: false,
            }
        );
        // Typing searches like the page; the split target is no longer pinned.
        let found = items(
            PickerKind::Compact,
            "zzz",
            &sources(&hosts, &[], &[], &[], Some(&local)),
        );
        assert!(found.is_empty());
        // A removed host is not offered.
        let gone = PickerOrigin::Host(Uuid::new_v4());
        assert!(
            items(
                PickerKind::Compact,
                "",
                &sources(&[], &[], &[], &[], Some(&gone))
            )
            .is_empty()
        );
    }

    #[test]
    fn steps_skip_headings_and_stop_at_the_ends() {
        let items = vec![
            Item::Heading("a"),
            Item::Row(test_row("1")),
            Item::Heading("b"),
            Item::Row(test_row("2")),
        ];
        assert_eq!(step(&items, None, true), Some(1));
        assert_eq!(step(&items, Some(1), true), Some(3));
        assert_eq!(step(&items, Some(3), true), Some(3));
        assert_eq!(step(&items, Some(3), false), Some(1));
        assert_eq!(step(&items, Some(1), false), Some(1));
        assert_eq!(step(&[], None, true), None);
    }

    fn test_row(key: &str) -> Row {
        Row {
            key: key.to_owned().into(),
            choice: Choice::Host(Uuid::nil()),
            icon: IconName::Server,
            color: TagColor::Graphite,
            title: key.to_owned().into(),
            detail: None,
        }
    }
}
