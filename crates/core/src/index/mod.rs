//! The decrypted in-memory index of records: load at unlock, wipe at lock,
//! queries with search, tag filter and sorting, the resolved group tree,
//! inheritance placeholders and counts (docs/data-model.md, sections 7-8;
//! docs/decisions.md, decision 10; stage 1 plan 5.4).
//!
//! The index holds every live record of the personal vault whose type stage 1
//! understands. Records of unknown types or newer payload versions are listed
//! as unsupported, damaged rows as damaged; neither is in the queries. Dropping
//! the index zeroizes every decrypted string (see [`crate::model::wipe`]).
//!
//! Owner: W2.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::model::group;
use crate::model::hlc::Hlc;
use crate::model::host;
use crate::model::inherit;
use crate::model::order_key;
use crate::model::payload::Payload;
use crate::model::text::fold;
use crate::model::time::{format_unix_ms, uuid_v7_ms, uuid_v7_timestamp};
use crate::model::tree::{GroupNode, Tree};
use crate::model::types::RecordKind;
use crate::service::dto::{
    GroupView, HostView, InheritedValues, QueryScope, RecordType, RecordView, SortMode,
};

/// One live record.
pub struct IndexedRecord {
    pub record_id: Uuid,
    pub kind: RecordKind,
    pub revision: u64,
    pub key_epoch: u32,
    /// A live payload: `data` is always present.
    pub payload: Payload,
}

impl IndexedRecord {
    pub fn new(
        record_id: Uuid,
        kind: RecordKind,
        revision: u64,
        key_epoch: u32,
        payload: Payload,
    ) -> Self {
        Self {
            record_id,
            kind,
            revision,
            key_epoch,
            payload,
        }
    }

    /// The record's `data` object.
    pub fn data(&self) -> &Map<String, Value> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        self.payload
            .data
            .as_ref()
            .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    pub fn hlc(&self) -> Hlc {
        self.payload.hlc
    }

    /// The manual order key, if any.
    pub fn order_key(&self) -> Option<&str> {
        self.data().get("order_key").and_then(Value::as_str)
    }
}

impl fmt::Debug for IndexedRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexedRecord")
            .field("record_id", &self.record_id)
            .field("kind", &self.kind)
            .field("revision", &self.revision)
            .field("key_epoch", &self.key_epoch)
            .finish_non_exhaustive()
    }
}

/// Query parameters (`RecordsQuery` without the vault).
#[derive(Clone, Debug, Default)]
pub struct Query {
    pub types: Vec<RecordType>,
    pub scope: QueryScope,
    pub text: Option<String>,
    pub tags: Vec<String>,
    pub sort: SortMode,
}

/// The index of one vault.
pub struct Index {
    vault_id: Uuid,
    records: HashMap<Uuid, IndexedRecord>,
    unsupported: Vec<Uuid>,
    damaged: Vec<Uuid>,
}

/// Values computed once per query or view batch.
struct Context {
    tree: Tree,
    /// Resolved group of each host (`None` = top level).
    host_group: HashMap<Uuid, Option<Uuid>>,
    host_count: HashMap<Uuid, u32>,
    child_count: HashMap<Uuid, u32>,
}

impl Index {
    pub fn new(vault_id: Uuid) -> Self {
        Self {
            vault_id,
            records: HashMap::new(),
            unsupported: Vec::new(),
            damaged: Vec::new(),
        }
    }

    pub fn vault_id(&self) -> Uuid {
        self.vault_id
    }

    pub fn insert(&mut self, record: IndexedRecord) {
        self.records.insert(record.record_id, record);
    }

    pub fn remove(&mut self, record_id: &Uuid) -> Option<IndexedRecord> {
        self.records.remove(record_id)
    }

    pub fn get(&self, record_id: &Uuid) -> Option<&IndexedRecord> {
        self.records.get(record_id)
    }

    /// The record if it has the given kind.
    pub fn get_kind(&self, record_id: &Uuid, kind: RecordKind) -> Option<&IndexedRecord> {
        self.records
            .get(record_id)
            .filter(|record| record.kind == kind)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn mark_unsupported(&mut self, record_id: Uuid) {
        self.unsupported.push(record_id);
    }

    pub fn mark_damaged(&mut self, record_id: Uuid) {
        self.damaged.push(record_id);
    }

    /// Records kept unchanged because a newer app wrote them.
    pub fn unsupported(&self) -> &[Uuid] {
        &self.unsupported
    }

    /// Rows that failed the read checks.
    pub fn damaged(&self) -> &[Uuid] {
        &self.damaged
    }

    pub fn iter(&self) -> impl Iterator<Item = &IndexedRecord> {
        self.records.values()
    }

    pub fn of_kind(&self, kind: RecordKind) -> impl Iterator<Item = &IndexedRecord> {
        self.records
            .values()
            .filter(move |record| record.kind == kind)
    }

    /// The greatest HLC of the indexed records.
    pub fn max_hlc(&self) -> Option<Hlc> {
        self.records.values().map(IndexedRecord::hlc).max()
    }

    /// The effective record of a singleton type: the greatest `(hlc, device_id)`.
    pub fn singleton(&self, kind: RecordKind) -> Option<&IndexedRecord> {
        self.of_kind(kind)
            .max_by(|a, b| a.payload.stamp().cmp(&b.payload.stamp()))
    }

    /// The resolved group tree.
    pub fn tree(&self) -> Tree {
        let nodes: Vec<GroupNode> = self
            .of_kind(RecordKind::Group)
            .map(|record| GroupNode {
                id: record.record_id,
                parent_id: group::parent_id(record.data()),
                hlc: record.payload.hlc,
                device_id: record.payload.device_id,
            })
            .collect();
        Tree::build(&nodes)
    }

    /// The resolved group of a host: its `group_id` when that is a live group.
    pub fn host_group(tree: &Tree, data: &Map<String, Value>) -> Option<Uuid> {
        host::group_id(data).filter(|id| tree.contains(id))
    }

    /// The container of a record for manual ordering (`None` = top level).
    pub fn container_of(&self, tree: &Tree, record: &IndexedRecord) -> Option<Uuid> {
        match record.kind {
            RecordKind::Host => Self::host_group(tree, record.data()),
            RecordKind::Group => tree.parent(&record.record_id),
            _ => None,
        }
    }

    /// The manual order of a container: records of `kind` whose resolved
    /// container is `container`, without `exclude`.
    pub fn container_order(
        &self,
        tree: &Tree,
        kind: RecordKind,
        container: Option<Uuid>,
        exclude: &[Uuid],
    ) -> Vec<(Uuid, Option<String>)> {
        let mut items: Vec<&IndexedRecord> = self
            .of_kind(kind)
            .filter(|record| !exclude.contains(&record.record_id))
            .filter(|record| self.container_of(tree, record) == container)
            .collect();
        items.sort_by(|a, b| manual(a, b));
        items
            .into_iter()
            .map(|record| (record.record_id, record.order_key().map(str::to_owned)))
            .collect()
    }

    /// Values a host inherits from its group chain.
    pub fn inherited_for_host(&self, tree: &Tree, data: &Map<String, Value>) -> InheritedValues {
        match Self::host_group(tree, data) {
            Some(group) => self.inherited_from(tree, group),
            None => InheritedValues::default(),
        }
    }

    /// Values a group inherits from its parent chain.
    pub fn inherited_for_group(&self, tree: &Tree, record_id: &Uuid) -> InheritedValues {
        match tree.parent(record_id) {
            Some(parent) => self.inherited_from(tree, parent),
            None => InheritedValues::default(),
        }
    }

    fn inherited_from(&self, tree: &Tree, first: Uuid) -> InheritedValues {
        let chain = tree.chain(first, inherit::MAX_LEVELS);
        inherit::resolve(chain.into_iter().filter_map(|id| {
            self.get_kind(&id, RecordKind::Group)
                .map(|record| (id, record.data()))
        }))
    }

    fn context(&self) -> Context {
        let tree = self.tree();
        let mut host_group = HashMap::new();
        let mut host_count: HashMap<Uuid, u32> = HashMap::new();
        let mut child_count: HashMap<Uuid, u32> = HashMap::new();
        for record in self.of_kind(RecordKind::Host) {
            let group = Self::host_group(&tree, record.data());
            host_group.insert(record.record_id, group);
            if let Some(group) = group {
                *child_count.entry(group).or_default() += 1;
                for ancestor in tree.chain(group, usize::MAX) {
                    *host_count.entry(ancestor).or_default() += 1;
                }
            }
        }
        for record in self.of_kind(RecordKind::Group) {
            if let Some(parent) = tree.parent(&record.record_id) {
                *child_count.entry(parent).or_default() += 1;
            }
        }
        Context {
            tree,
            host_group,
            host_count,
            child_count,
        }
    }

    fn host_view(&self, context: &Context, record: &IndexedRecord) -> HostView {
        let data = record.data();
        let (public, secrets_set) = host::public_view(data);
        let group = context.host_group.get(&record.record_id).copied().flatten();
        HostView {
            vault_id: self.vault_id,
            record_id: record.record_id,
            revision: record.revision,
            created_at: uuid_v7_timestamp(&record.record_id),
            modified_at: format_unix_ms(record.payload.hlc.ms()),
            data: Value::Object(public),
            secrets_set,
            inherited: self.inherited_for_host(&context.tree, data),
            path: context.tree.host_path(group),
        }
    }

    fn group_view(&self, context: &Context, record: &IndexedRecord) -> GroupView {
        let id = record.record_id;
        GroupView {
            vault_id: self.vault_id,
            record_id: id,
            revision: record.revision,
            created_at: uuid_v7_timestamp(&id),
            modified_at: format_unix_ms(record.payload.hlc.ms()),
            data: Value::Object(record.data().clone()),
            inherited: self.inherited_for_group(&context.tree, &id),
            path: context.tree.path(&id),
            host_count: context.host_count.get(&id).copied().unwrap_or(0),
            child_count: context.child_count.get(&id).copied().unwrap_or(0),
        }
    }

    fn view_in(&self, context: &Context, record: &IndexedRecord) -> Option<RecordView> {
        match record.kind {
            RecordKind::Host => Some(RecordView::Host(self.host_view(context, record))),
            RecordKind::Group => Some(RecordView::Group(self.group_view(context, record))),
            _ => None,
        }
    }

    /// The view of a host or group.
    pub fn view(&self, record_id: &Uuid) -> Option<RecordView> {
        let record = self.records.get(record_id)?;
        let context = self.context();
        self.view_in(&context, record)
    }

    /// Views of several hosts or groups, in the given order (unknown ids skipped).
    pub fn views(&self, record_ids: &[Uuid]) -> Vec<RecordView> {
        let context = self.context();
        record_ids
            .iter()
            .filter_map(|id| self.records.get(id))
            .filter_map(|record| self.view_in(&context, record))
            .collect()
    }

    /// `records_query` (stage 1 plan 5.4).
    pub fn query(&self, query: &Query) -> Vec<RecordView> {
        let context = self.context();
        let text = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(fold);
        let tags: Vec<String> = query
            .tags
            .iter()
            .map(|tag| fold(tag.trim()))
            .filter(|tag| !tag.is_empty())
            .collect();

        let mut groups: Vec<&IndexedRecord> = Vec::new();
        if query.types.contains(&RecordType::Group) && tags.is_empty() {
            groups = self
                .of_kind(RecordKind::Group)
                .filter(|record| match query.scope {
                    QueryScope::All => true,
                    QueryScope::TopLevel => context.tree.parent(&record.record_id).is_none(),
                    QueryScope::Group { group_id } => {
                        context.tree.parent(&record.record_id) == Some(group_id)
                    }
                })
                .filter(|record| match &text {
                    None => true,
                    Some(text) => fold(group::name(record.data())).contains(text.as_str()),
                })
                .collect();
        }

        let mut hosts: Vec<&IndexedRecord> = Vec::new();
        if query.types.contains(&RecordType::Host) {
            hosts = self
                .of_kind(RecordKind::Host)
                .filter(|record| {
                    let group = context.host_group.get(&record.record_id).copied().flatten();
                    match query.scope {
                        QueryScope::All => true,
                        QueryScope::TopLevel => group.is_none(),
                        QueryScope::Group { group_id } => {
                            host::group_id(record.data()) == Some(group_id)
                        }
                    }
                })
                .filter(|record| host_has_tags(record.data(), &tags))
                .filter(|record| match &text {
                    None => true,
                    Some(text) => self.host_matches(&context.tree, record.data(), text),
                })
                .collect();
        }

        sort(&mut groups, query.sort, |record| group::name(record.data()));
        sort(&mut hosts, query.sort, |record| host::title(record.data()));

        groups
            .into_iter()
            .chain(hosts)
            .filter_map(|record| self.view_in(&context, record))
            .collect()
    }

    fn host_matches(&self, tree: &Tree, data: &Map<String, Value>, text: &str) -> bool {
        let contains = |value: Option<&str>| value.is_some_and(|value| fold(value).contains(text));
        if contains(data.get("label").and_then(Value::as_str))
            || contains(data.get("address").and_then(Value::as_str))
        {
            return true;
        }
        if data
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|tags| tags.iter().any(|tag| contains(tag.as_str())))
        {
            return true;
        }
        let own = data
            .get("ssh")
            .and_then(Value::as_object)
            .and_then(|ssh| ssh.get("username"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        match own {
            Some(username) => contains(Some(username)),
            None => self
                .inherited_for_host(tree, data)
                .username
                .is_some_and(|inherited| contains(Some(&inherited.value))),
        }
    }

    /// Every distinct tag of the hosts (first spelling), sorted case-insensitively.
    pub fn tags(&self) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        for record in self.of_kind(RecordKind::Host) {
            if let Some(tags) = record.data().get("tags").and_then(Value::as_array) {
                for tag in tags.iter().filter_map(Value::as_str) {
                    if seen.insert(fold(tag)) {
                        out.push(tag.to_owned());
                    }
                }
            }
        }
        out.sort_by_key(|tag| fold(tag));
        out
    }
}

impl fmt::Debug for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Index")
            .field("vault_id", &self.vault_id)
            .field("records", &self.records.len())
            .field("unsupported", &self.unsupported.len())
            .field("damaged", &self.damaged.len())
            .finish()
    }
}

fn host_has_tags(data: &Map<String, Value>, wanted: &[String]) -> bool {
    if wanted.is_empty() {
        return true;
    }
    let tags: Vec<String> = data
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| tags.iter().filter_map(Value::as_str).map(fold).collect())
        .unwrap_or_default();
    wanted.iter().all(|tag| tags.contains(tag))
}

fn manual(a: &IndexedRecord, b: &IndexedRecord) -> Ordering {
    order_key::compare((a.order_key(), &a.record_id), (b.order_key(), &b.record_id))
}

fn sort<F>(records: &mut [&IndexedRecord], mode: SortMode, name: F)
where
    F: Fn(&IndexedRecord) -> &str,
{
    match mode {
        SortMode::Manual => records.sort_by(|a, b| manual(a, b)),
        SortMode::Name => records.sort_by(|a, b| {
            fold(name(a))
                .cmp(&fold(name(b)))
                .then_with(|| a.record_id.cmp(&b.record_id))
        }),
        SortMode::Created => records.sort_by(|a, b| {
            uuid_v7_ms(&b.record_id)
                .cmp(&uuid_v7_ms(&a.record_id))
                .then_with(|| b.record_id.cmp(&a.record_id))
        }),
        SortMode::Modified => records.sort_by(|a, b| {
            b.payload
                .hlc
                .cmp(&a.payload.hlc)
                .then_with(|| b.record_id.cmp(&a.record_id))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, UNIX_EPOCH};

    use crate::model::time::new_record_id;

    struct Fixture {
        index: Index,
        clock_ms: u64,
        device: Uuid,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                index: Index::new(Uuid::now_v7()),
                clock_ms: 1_792_000_000_000,
                device: Uuid::now_v7(),
            }
        }

        fn add(&mut self, kind: RecordKind, data: Value) -> Uuid {
            self.clock_ms += 1000;
            let id = new_record_id(UNIX_EPOCH + Duration::from_millis(self.clock_ms));
            let payload = Payload::live(
                kind.name(),
                1,
                Hlc::new(self.clock_ms, 0),
                self.device,
                data.as_object().unwrap().clone(),
            );
            self.index
                .insert(IndexedRecord::new(id, kind, 1, 1, payload));
            id
        }

        fn ids(&self, query: Query) -> Vec<Uuid> {
            self.index
                .query(&query)
                .iter()
                .map(RecordView::record_id)
                .collect()
        }
    }

    fn q(types: &[RecordType], scope: QueryScope) -> Query {
        Query {
            types: types.to_vec(),
            scope,
            ..Query::default()
        }
    }

    const BOTH: &[RecordType] = &[RecordType::Group, RecordType::Host];

    #[test]
    fn scopes_tree_counts_and_inheritance() {
        let mut f = Fixture::new();
        let prod = f.add(
            RecordKind::Group,
            json!({"name": "prod", "order_key": "V", "ssh": {"port": 2222, "username": "root"}}),
        );
        let web = f.add(RecordKind::Group, json!({"name": "web", "parent_id": prod.to_string(), "order_key": "V", "ssh": {"username": "deploy"}}));
        let lonely = f.add(RecordKind::Group, json!({"name": "lonely", "parent_id": Uuid::now_v7().to_string(), "order_key": "k", "ssh": {}}));
        let h1 = f.add(RecordKind::Host, json!({"address": "web1.example.com", "group_id": web.to_string(), "order_key": "V", "ssh": {}}));
        let h2 = f.add(RecordKind::Host, json!({"address": "db.example.com", "group_id": prod.to_string(), "order_key": "V", "ssh": {"username": "pg"}}));
        let h3 = f.add(
            RecordKind::Host,
            json!({"address": "198.51.100.7", "order_key": "V", "ssh": {}}),
        );
        let h4 = f.add(RecordKind::Host, json!({"address": "orphan.example.com", "group_id": Uuid::now_v7().to_string(), "order_key": "k", "ssh": {}}));

        assert_eq!(
            f.ids(q(BOTH, QueryScope::TopLevel)),
            vec![prod, lonely, h3, h4]
        );
        assert_eq!(
            f.ids(q(BOTH, QueryScope::Group { group_id: prod })),
            vec![web, h2]
        );
        assert_eq!(
            f.ids(q(BOTH, QueryScope::Group { group_id: web })),
            vec![h1]
        );
        assert_eq!(f.ids(q(&[RecordType::Host], QueryScope::All)).len(), 4);

        let views = f.index.views(&[prod, web, h1]);
        let RecordView::Group(prod_view) = &views[0] else {
            panic!()
        };
        assert_eq!(prod_view.host_count, 2);
        assert_eq!(prod_view.child_count, 2);
        assert!(prod_view.path.is_empty());
        let RecordView::Group(web_view) = &views[1] else {
            panic!()
        };
        assert_eq!(web_view.host_count, 1);
        assert_eq!(web_view.path, vec![prod]);
        assert_eq!(web_view.inherited.port.as_ref().unwrap().value, 2222);
        assert_eq!(
            web_view.inherited.username.as_ref().unwrap().from_group_id,
            prod
        );
        let RecordView::Host(h1_view) = &views[2] else {
            panic!()
        };
        assert_eq!(h1_view.path, vec![prod, web]);
        assert_eq!(h1_view.inherited.port.as_ref().unwrap().from_group_id, prod);
        assert_eq!(h1_view.inherited.username.as_ref().unwrap().value, "deploy");
        assert_eq!(
            h1_view.inherited.username.as_ref().unwrap().from_group_id,
            web
        );
        let RecordView::Host(h4_view) = &f.index.views(&[h4])[0] else {
            panic!()
        };
        assert!(h4_view.path.is_empty());
        assert_eq!(h4_view.inherited, InheritedValues::default());
    }

    #[test]
    fn search_by_name_address_tags_and_username() {
        let mut f = Fixture::new();
        let prod = f.add(
            RecordKind::Group,
            json!({"name": "Production", "ssh": {"username": "Deployer"}}),
        );
        let label = f.add(
            RecordKind::Host,
            json!({"address": "10.example.com", "label": "Billing API", "ssh": {}}),
        );
        let address = f.add(
            RecordKind::Host,
            json!({"address": "mail.example.org", "ssh": {}}),
        );
        let tagged = f.add(
            RecordKind::Host,
            json!({"address": "t.example.com", "tags": ["Critical"], "ssh": {}}),
        );
        let own_user = f.add(
            RecordKind::Host,
            json!({"address": "u.example.com", "ssh": {"username": "alice"}}),
        );
        let inherited_user = f.add(
            RecordKind::Host,
            json!({"address": "i.example.com", "group_id": prod.to_string(), "ssh": {}}),
        );

        let search = |text: &str| {
            f.ids(Query {
                types: BOTH.to_vec(),
                scope: QueryScope::All,
                text: Some(text.to_owned()),
                ..Query::default()
            })
        };
        assert_eq!(search("billing"), vec![label]);
        assert_eq!(search("EXAMPLE.ORG"), vec![address]);
        assert_eq!(search("critic"), vec![tagged]);
        assert_eq!(search("ALICE"), vec![own_user]);
        assert_eq!(search("deployer"), vec![inherited_user]);
        assert_eq!(search("product"), vec![prod]);
        assert_eq!(search("   ").len(), 6);
        assert!(search("nothing-matches").is_empty());
    }

    #[test]
    fn tag_filter_is_and_and_excludes_groups() {
        let mut f = Fixture::new();
        f.add(RecordKind::Group, json!({"name": "g", "ssh": {}}));
        let both = f.add(
            RecordKind::Host,
            json!({"address": "a.example.com", "tags": ["prod", "DB"], "ssh": {}}),
        );
        f.add(
            RecordKind::Host,
            json!({"address": "b.example.com", "tags": ["prod"], "ssh": {}}),
        );
        let ids = f.ids(Query {
            types: BOTH.to_vec(),
            scope: QueryScope::All,
            tags: vec!["Prod".into(), "db".into()],
            ..Query::default()
        });
        assert_eq!(ids, vec![both]);
        assert_eq!(f.index.tags(), vec!["DB".to_owned(), "prod".to_owned()]);
    }

    #[test]
    fn sort_modes() {
        let mut f = Fixture::new();
        let b = f.add(
            RecordKind::Host,
            json!({"address": "b.example.com", "order_key": "k", "ssh": {}}),
        );
        let a = f.add(
            RecordKind::Host,
            json!({"address": "z.example.com", "label": "a", "order_key": "V", "ssh": {}}),
        );
        let keyless = f.add(
            RecordKind::Host,
            json!({"address": "C.example.com", "ssh": {}}),
        );
        let g2 = f.add(
            RecordKind::Group,
            json!({"name": "beta", "order_key": "V", "ssh": {}}),
        );
        let g1 = f.add(
            RecordKind::Group,
            json!({"name": "Alpha", "order_key": "k", "ssh": {}}),
        );
        let with = |f: &Fixture, sort| {
            f.ids(Query {
                types: BOTH.to_vec(),
                scope: QueryScope::All,
                sort,
                ..Query::default()
            })
        };
        assert_eq!(with(&f, SortMode::Manual), vec![g2, g1, a, b, keyless]);
        assert_eq!(with(&f, SortMode::Name), vec![g1, g2, a, b, keyless]);
        assert_eq!(with(&f, SortMode::Created), vec![g1, g2, keyless, a, b]);
        // Touch `b`: newest HLC first.
        let mut record = f.index.remove(&b).unwrap();
        record.payload.hlc = Hlc::new(f.clock_ms + 10_000, 0);
        f.index.insert(record);
        assert_eq!(with(&f, SortMode::Modified), vec![g1, g2, b, keyless, a]);
    }

    #[test]
    fn secrets_are_not_in_views() {
        let mut f = Fixture::new();
        let id = f.add(
            RecordKind::Host,
            json!({"address": "s.example.com", "ssh": {"password": "canary-pass"}}),
        );
        let RecordView::Host(view) = f.index.view(&id).unwrap() else {
            panic!()
        };
        assert_eq!(view.secrets_set, vec!["ssh.password".to_owned()]);
        assert!(!view.data.to_string().contains("canary-pass"));
        assert!(!format!("{:?}", f.index).contains("canary"));
    }

    #[test]
    fn container_order_and_singletons() {
        let mut f = Fixture::new();
        let g = f.add(RecordKind::Group, json!({"name": "g", "ssh": {}}));
        let first = f.add(RecordKind::Host, json!({"address": "1.example.com", "group_id": g.to_string(), "order_key": "V", "ssh": {}}));
        let second = f.add(RecordKind::Host, json!({"address": "2.example.com", "group_id": g.to_string(), "order_key": "k", "ssh": {}}));
        f.add(
            RecordKind::Host,
            json!({"address": "top.example.com", "order_key": "V", "ssh": {}}),
        );
        let tree = f.index.tree();
        let order = f
            .index
            .container_order(&tree, RecordKind::Host, Some(g), &[]);
        assert_eq!(
            order,
            vec![(first, Some("V".into())), (second, Some("k".into()))]
        );
        assert_eq!(
            f.index
                .container_order(&tree, RecordKind::Host, Some(g), &[first])
                .len(),
            1
        );

        let old = f.add(RecordKind::VaultMeta, json!({"name": "Old"}));
        let new = f.add(RecordKind::VaultMeta, json!({"name": "New"}));
        assert_ne!(old, new);
        assert_eq!(
            f.index.singleton(RecordKind::VaultMeta).unwrap().record_id,
            new
        );
        assert_eq!(f.index.max_hlc(), Some(Hlc::new(f.clock_ms, 0)));
    }
}
