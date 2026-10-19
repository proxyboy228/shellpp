//! What the Hosts screen shows (UI 2.1-2.3): the layout (tiles or list) and
//! sort mode, kept in the core settings `view.hosts.layout` and
//! `view.hosts.sort`; the search and tag filter; the groups and hosts of the
//! current location in display order; and the record operations the screen
//! needs beyond the base API (move to a position, Copy to a group). Undo of
//! Remove is in [`super::undo`].
//!
//! The filter and ordering functions are pure, so they are unit-tested
//! without a window.

use std::collections::{BTreeMap, HashSet};

use gpui::Context;
use serde_json::{Value, json};
use shellpp_core::CoreError;
use shellpp_core::service::dto::{
    MovePosition, RecordMoveInput, RecordRef, RecordSaveInput, RecordType, RecordView, RecordsRef,
    SettingsSetInput,
};
use uuid::Uuid;

use super::{AppState, GroupItem, HostItem};

/// Setting key of the layout (`grid` or `list`).
pub const LAYOUT_KEY: &str = "view.hosts.layout";
/// Setting key of the sort mode (`manual`, `name`, `created`, `modified`).
pub const SORT_KEY: &str = "view.hosts.sort";

/// Tiles or list (UI 2.2 view toggle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostsLayout {
    #[default]
    Tiles,
    List,
}

impl HostsLayout {
    fn setting(self) -> &'static str {
        match self {
            Self::Tiles => "grid",
            Self::List => "list",
        }
    }

    fn from_setting(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some("list") => Self::List,
            _ => Self::Tiles,
        }
    }
}

/// Sort menu of UI 2.2: Manual (the default on Hosts), Name, Date created,
/// Date modified. Dates sort newest first, like the core's sort modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostsSort {
    #[default]
    Manual,
    Name,
    Created,
    Modified,
}

impl HostsSort {
    /// The menu order.
    pub const ALL: [Self; 4] = [Self::Manual, Self::Name, Self::Created, Self::Modified];

    fn setting(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Name => "name",
            Self::Created => "created",
            Self::Modified => "modified",
        }
    }

    fn from_setting(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some("name") => Self::Name,
            Some("created") => Self::Created,
            Some("modified") => Self::Modified,
            _ => Self::Manual,
        }
    }
}

/// The search text and the active tag filter chips.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostsFilter {
    text: String,
    /// Active tags, lower case, in the order they were turned on.
    tags: Vec<String>,
}

impl HostsFilter {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// The active tags (lower case).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        let tag = tag.to_lowercase();
        self.tags.contains(&tag)
    }

    /// Turns a tag on or off (a checkbox of the tag filter, a chip).
    pub fn toggle_tag(&mut self, tag: &str) {
        let tag = tag.to_lowercase();
        if let Some(position) = self.tags.iter().position(|active| *active == tag) {
            self.tags.remove(position);
        } else {
            self.tags.push(tag);
        }
    }

    /// Turns every tag off ("Clear" in the tag filter).
    pub fn clear_tags(&mut self) {
        self.tags.clear();
    }

    /// Drops active tags that no host carries any more.
    pub fn retain_tags(&mut self, known: &[String]) {
        let known: HashSet<String> = known.iter().map(|tag| tag.to_lowercase()).collect();
        self.tags.retain(|tag| known.contains(tag));
    }

    /// Whether the screen shows filtered results instead of a folder.
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || !self.tags.is_empty()
    }

    /// Search (case-insensitive substring of label, address, username or a
    /// tag) and every active tag.
    pub fn matches_host(&self, host: &HostItem) -> bool {
        text_matches_host(host, &self.text)
            && self
                .tags
                .iter()
                .all(|wanted| host.tags.iter().any(|tag| tag.to_lowercase() == *wanted))
    }

    /// Groups match by name, and only while no tag is active (groups carry
    /// no tags).
    pub fn matches_group(&self, group: &GroupItem) -> bool {
        let query = self.text.trim().to_lowercase();
        self.tags.is_empty() && !query.is_empty() && group.name.to_lowercase().contains(&query)
    }
}

/// Case-insensitive match of the search text against the label, address,
/// username and tags of a host (UI 2.1). An empty query matches everything.
pub fn text_matches_host(host: &HostItem, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let contains = |value: &str| value.to_lowercase().contains(&query);
    host.label.as_deref().is_some_and(contains)
        || contains(&host.address)
        || host.username.as_deref().is_some_and(contains)
        || host.tags.iter().any(|tag| contains(tag))
}

/// The groups and hosts on screen, as indices into the state's lists, in
/// display order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostsListing {
    pub groups: Vec<usize>,
    pub hosts: Vec<usize>,
}

/// What the screen shows for `location`: its subgroups and hosts, or, while
/// the filter is active, the matching groups and hosts of the whole subtree
/// (the whole vault at the top level).
pub fn listing(
    hosts: &[HostItem],
    groups: &[GroupItem],
    location: Option<Uuid>,
    filter: &HostsFilter,
    sort: HostsSort,
) -> HostsListing {
    let (mut group_ix, mut host_ix): (Vec<usize>, Vec<usize>) = if filter.is_active() {
        let in_subtree = |path: &[Uuid]| location.is_none_or(|location| path.contains(&location));
        (
            (0..groups.len())
                .filter(|&ix| {
                    let group = &groups[ix];
                    Some(group.record_id) != location
                        && in_subtree(&group.path)
                        && filter.matches_group(group)
                })
                .collect(),
            (0..hosts.len())
                .filter(|&ix| in_subtree(&hosts[ix].path) && filter.matches_host(&hosts[ix]))
                .collect(),
        )
    } else {
        (
            (0..groups.len())
                .filter(|&ix| groups[ix].parent_id == location)
                .collect(),
            (0..hosts.len())
                .filter(|&ix| hosts[ix].group_id == location)
                .collect(),
        )
    };
    sort_indices(&mut group_ix, sort, |ix| {
        let group = &groups[ix];
        (
            group.name.as_str(),
            group.record_id,
            group.modified_at.as_str(),
        )
    });
    sort_indices(&mut host_ix, sort, |ix| {
        let host = &hosts[ix];
        (host.title(), host.record_id, host.modified_at.as_str())
    });
    HostsListing {
        groups: group_ix,
        hosts: host_ix,
    }
}

/// Sorts indices whose items are already in manual order. `key` gives the
/// name, the record id (UUID v7: creation time) and the modification time.
fn sort_indices<'a>(
    indices: &mut [usize],
    sort: HostsSort,
    key: impl Fn(usize) -> (&'a str, Uuid, &'a str),
) {
    match sort {
        HostsSort::Manual => {}
        HostsSort::Name => indices.sort_by(|a, b| {
            let (name_a, id_a, _) = key(*a);
            let (name_b, id_b, _) = key(*b);
            name_a
                .to_lowercase()
                .cmp(&name_b.to_lowercase())
                .then_with(|| id_a.cmp(&id_b))
        }),
        HostsSort::Created => indices.sort_by(|a, b| {
            let (_, id_a, _) = key(*a);
            let (_, id_b, _) = key(*b);
            id_b.cmp(&id_a)
        }),
        HostsSort::Modified => indices.sort_by(|a, b| {
            let (_, id_a, modified_a) = key(*a);
            let (_, id_b, modified_b) = key(*b);
            modified_b.cmp(modified_a).then_with(|| id_b.cmp(&id_a))
        }),
    }
}

/// Every tag in use, once (case-insensitive, first spelling kept), sorted.
pub fn all_tags(hosts: &[HostItem]) -> Vec<String> {
    tag_counts(hosts).into_iter().map(|tag| tag.name).collect()
}

/// A tag of the tag filter with the number of hosts that carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagCount {
    /// The first spelling met in manual order.
    pub name: String,
    pub hosts: usize,
}

impl TagCount {
    /// The case-insensitive key (the filter keeps tags in lower case).
    pub fn key(&self) -> String {
        self.name.to_lowercase()
    }
}

/// Every tag in use with its host count (a host counts once per tag, in any
/// spelling), sorted case-insensitively: the options of the tag filter.
pub fn tag_counts(hosts: &[HostItem]) -> Vec<TagCount> {
    let mut tags: BTreeMap<String, TagCount> = BTreeMap::new();
    for host in hosts {
        let mut seen: HashSet<String> = HashSet::new();
        for tag in &host.tags {
            let key = tag.to_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            tags.entry(key)
                .or_insert_with(|| TagCount {
                    name: tag.clone(),
                    hosts: 0,
                })
                .hosts += 1;
        }
    }
    tags.into_values().collect()
}

/// `group` and every group below it.
pub fn subtree_of(groups: &[GroupItem], group: Uuid) -> HashSet<Uuid> {
    let mut ids: HashSet<Uuid> = groups
        .iter()
        .filter(|candidate| candidate.path.contains(&group))
        .map(|candidate| candidate.record_id)
        .collect();
    ids.insert(group);
    ids
}

/// Groups in tree order (parents before children), with their depth: the
/// targets of Move to (UI 2.4).
pub fn group_tree(groups: &[GroupItem]) -> Vec<(usize, usize)> {
    fn walk(
        groups: &[GroupItem],
        parent: Option<Uuid>,
        depth: usize,
        out: &mut Vec<(usize, usize)>,
    ) {
        let mut children: Vec<usize> = (0..groups.len())
            .filter(|&ix| groups[ix].parent_id == parent)
            .collect();
        children.sort_by_key(|&ix| groups[ix].name.to_lowercase());
        for ix in children {
            out.push((ix, depth));
            walk(groups, Some(groups[ix].record_id), depth + 1, out);
        }
    }
    let mut out = Vec::with_capacity(groups.len());
    walk(groups, None, 0, &mut out);
    out
}

impl AppState {
    /// The saved layout of the Hosts screen (tiles by default).
    pub fn hosts_layout(&self) -> HostsLayout {
        let settings = self.core().and_then(|core| core.settings_get().ok());
        HostsLayout::from_setting(settings.as_ref().and_then(|values| values.get(LAYOUT_KEY)))
    }

    /// The saved sort mode of the Hosts screen (Manual by default).
    pub fn hosts_sort(&self) -> HostsSort {
        let settings = self.core().and_then(|core| core.settings_get().ok());
        HostsSort::from_setting(settings.as_ref().and_then(|values| values.get(SORT_KEY)))
    }

    /// Remembers the layout (the choice is kept across restarts).
    pub fn set_hosts_layout(&mut self, layout: HostsLayout) -> Result<(), CoreError> {
        self.set_view_setting(LAYOUT_KEY, layout.setting())
    }

    /// Remembers the sort mode.
    pub fn set_hosts_sort(&mut self, sort: HostsSort) -> Result<(), CoreError> {
        self.set_view_setting(SORT_KEY, sort.setting())
    }

    fn set_view_setting(&mut self, key: &str, value: &str) -> Result<(), CoreError> {
        let mut values = BTreeMap::new();
        values.insert(key.to_owned(), json!(value));
        self.require_core()?.settings_set(SettingsSetInput {
            values,
            reset: Vec::new(),
        })?;
        Ok(())
    }

    /// Copy to ▸ a group of this vault (UI 2.4, lead decision 7): a copy of
    /// each host with the same label and every field (secrets included) at
    /// the end of `target_group_id` (`None`: the top level). Groups are not
    /// copied. Returns the ids of the copies, in the order of `record_ids`.
    pub fn copy_hosts_to(
        &mut self,
        record_ids: Vec<Uuid>,
        target_group_id: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<Uuid>, CoreError> {
        let vault_id = self.require_vault()?;
        let core = self.require_core()?.clone();
        if target_group_id.is_some_and(|group| self.group(group).is_none()) {
            return Err(CoreError::NotFound);
        }
        let originals: Vec<(Uuid, Option<String>)> = record_ids
            .iter()
            .filter_map(|id| self.host(*id).map(|host| (*id, host.label.clone())))
            .collect();
        let mut copies = Vec::with_capacity(originals.len());
        let mut failure = None;
        for (id, label) in originals {
            // The core's Duplicate copies the secrets; the copy then gets the
            // original label and its group.
            let copy = match core.record_duplicate(RecordRef {
                vault_id,
                record_id: id,
            }) {
                Ok(RecordView::Host(view)) => view,
                Ok(RecordView::Group(_)) => continue,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            };
            copies.push(copy.record_id);
            let mut data = copy.data.as_object().cloned().unwrap_or_default();
            match label {
                Some(label) => data.insert("label".to_owned(), Value::String(label)),
                None => data.remove("label"),
            };
            match target_group_id {
                Some(group) => data.insert("group_id".to_owned(), json!(group)),
                None => data.remove("group_id"),
            };
            if let Err(error) = core.record_save(RecordSaveInput {
                vault_id,
                record_id: Some(copy.record_id),
                record_type: RecordType::Host,
                data,
                secrets: Default::default(),
            }) {
                failure = Some(error);
                break;
            }
        }
        if let Some(error) = failure {
            // No half-made copies: the ones made so far are removed again.
            if !copies.is_empty()
                && let Err(cleanup) = core.record_delete(RecordsRef {
                    vault_id,
                    record_ids: copies,
                })
            {
                tracing::warn!(code = cleanup.code(), "partial copies not removed");
            }
            self.records_written(cx);
            return Err(error);
        }
        tracing::info!(copies = copies.len(), "hosts copied");
        self.records_written(cx);
        Ok(copies)
    }

    /// Moves records into `target_group_id` (`None`: the top level) at
    /// `position` (drag to reorder in Manual mode, UI 2.3).
    pub fn move_records_to(
        &mut self,
        record_ids: Vec<Uuid>,
        target_group_id: Option<Uuid>,
        position: MovePosition,
        cx: &mut Context<Self>,
    ) -> Result<(), CoreError> {
        let vault_id = self.require_vault()?;
        self.require_core()?.record_move(RecordMoveInput {
            vault_id,
            record_ids,
            target_group_id,
            position,
        })?;
        self.records_written(cx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellpp_core::service::dto::TagColor;

    fn host(label: &str, address: &str, tags: &[&str], path: Vec<Uuid>) -> HostItem {
        HostItem {
            record_id: Uuid::now_v7(),
            vault_id: Uuid::nil(),
            revision: 1,
            label: (!label.is_empty()).then(|| label.to_owned()),
            address: address.to_owned(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            color: TagColor::default(),
            username: Some("deploy".to_owned()),
            port: None,
            os_family: None,
            group_id: path.last().copied(),
            path,
            created_at: String::new(),
            modified_at: String::new(),
            data: Value::Null,
            secrets_set: Vec::new(),
        }
    }

    fn group(name: &str, path: Vec<Uuid>) -> GroupItem {
        GroupItem {
            record_id: Uuid::now_v7(),
            vault_id: Uuid::nil(),
            revision: 1,
            name: name.to_owned(),
            color: TagColor::default(),
            parent_id: path.last().copied(),
            path,
            host_count: 0,
            child_count: 0,
            created_at: String::new(),
            modified_at: String::new(),
            data: Value::Null,
        }
    }

    #[test]
    fn folders_search_and_tags() {
        let production = group("Production", vec![]);
        let databases = group("Databases", vec![production.record_id]);
        let groups = vec![production.clone(), databases.clone()];
        let hosts = vec![
            host("Web", "web.example.com", &["prod"], vec![]),
            host("", "192.0.2.10", &[], vec![production.record_id]),
            host(
                "Primary DB",
                "db1.example.com",
                &["Prod", "db"],
                vec![production.record_id, databases.record_id],
            ),
        ];
        let mut filter = HostsFilter::default();

        // Folders: the top level, then inside Production.
        let top = listing(&hosts, &groups, None, &filter, HostsSort::Manual);
        assert_eq!(top.groups, vec![0]);
        assert_eq!(top.hosts, vec![0]);
        let inside = listing(
            &hosts,
            &groups,
            Some(production.record_id),
            &filter,
            HostsSort::Manual,
        );
        assert_eq!(inside.groups, vec![1]);
        assert_eq!(inside.hosts, vec![1]);

        // Search covers the subtree: name, address, user, tags; groups by name.
        filter.set_text("DB");
        let found = listing(&hosts, &groups, None, &filter, HostsSort::Manual);
        assert!(found.groups.is_empty());
        assert_eq!(found.hosts, vec![2]);
        filter.set_text("data");
        assert_eq!(
            listing(&hosts, &groups, None, &filter, HostsSort::Manual).groups,
            vec![1]
        );
        filter.set_text("192.0.2");
        assert_eq!(
            listing(&hosts, &groups, None, &filter, HostsSort::Manual).hosts,
            vec![1]
        );
        filter.set_text("deploy");
        assert_eq!(
            listing(
                &hosts,
                &groups,
                Some(databases.record_id),
                &filter,
                HostsSort::Manual
            )
            .hosts,
            vec![2]
        );

        // Tags: case-insensitive, every active tag, groups hidden.
        filter.set_text("");
        filter.toggle_tag("PROD");
        let tagged = listing(&hosts, &groups, None, &filter, HostsSort::Manual);
        assert_eq!(tagged.hosts, vec![0, 2]);
        assert!(tagged.groups.is_empty());
        filter.toggle_tag("db");
        assert_eq!(
            listing(&hosts, &groups, None, &filter, HostsSort::Manual).hosts,
            vec![2]
        );
        filter.retain_tags(&["prod".to_owned()]);
        assert_eq!(filter.tags(), &["prod".to_owned()]);
        filter.toggle_tag("Prod");
        assert!(!filter.is_active());

        assert_eq!(all_tags(&hosts), vec!["db".to_owned(), "prod".to_owned()]);
        // Host counts per tag, any spelling; a host counts once.
        let mut twice = host("Twice", "t.lan", &["PROD", "prod"], vec![]);
        twice.tags.push("db".to_owned());
        let mut with_twice = hosts.clone();
        with_twice.push(twice);
        assert_eq!(
            tag_counts(&with_twice),
            vec![
                TagCount {
                    name: "db".to_owned(),
                    hosts: 2
                },
                TagCount {
                    name: "prod".to_owned(),
                    hosts: 3
                },
            ]
        );
        filter.toggle_tag("db");
        filter.toggle_tag("prod");
        filter.clear_tags();
        assert!(!filter.is_active());
    }

    #[test]
    fn sort_modes() {
        let mut hosts = vec![
            host("beta", "b.lan", &[], vec![]),
            host("Alpha", "a.lan", &[], vec![]),
            host("", "c.lan", &[], vec![]),
        ];
        for (index, host) in hosts.iter_mut().enumerate() {
            host.record_id = Uuid::from_u128(index as u128 + 1);
        }
        hosts[0].modified_at = "2026-01-03T00:00:00Z".to_owned();
        hosts[1].modified_at = "2026-01-01T00:00:00Z".to_owned();
        hosts[2].modified_at = "2026-01-02T00:00:00Z".to_owned();
        let filter = HostsFilter::default();
        let order = |sort| listing(&hosts, &[], None, &filter, sort).hosts;
        assert_eq!(order(HostsSort::Manual), vec![0, 1, 2]);
        assert_eq!(order(HostsSort::Name), vec![1, 0, 2]);
        // Record ids (UUID v7) grow with creation time: newest first.
        assert_eq!(order(HostsSort::Created), vec![2, 1, 0]);
        assert_eq!(order(HostsSort::Modified), vec![0, 2, 1]);
    }

    #[test]
    fn settings_values_round_trip() {
        for layout in [HostsLayout::Tiles, HostsLayout::List] {
            assert_eq!(
                HostsLayout::from_setting(Some(&json!(layout.setting()))),
                layout
            );
        }
        for sort in HostsSort::ALL {
            assert_eq!(HostsSort::from_setting(Some(&json!(sort.setting()))), sort);
        }
        assert_eq!(HostsLayout::from_setting(None), HostsLayout::Tiles);
        assert_eq!(HostsSort::from_setting(None), HostsSort::Manual);
    }

    #[test]
    fn group_tree_and_subtree() {
        let production = group("Production", vec![]);
        let databases = group("Databases", vec![production.record_id]);
        let lab = group("Lab", vec![]);
        let groups = vec![production.clone(), databases.clone(), lab.clone()];
        assert_eq!(group_tree(&groups), vec![(2, 0), (0, 0), (1, 1)]);
        let subtree = subtree_of(&groups, production.record_id);
        assert!(subtree.contains(&databases.record_id));
        assert!(subtree.contains(&production.record_id));
        assert!(!subtree.contains(&lab.record_id));
    }
}
