//! Record commands of the facade (stage 1 plan 5.4-5.6; docs/data-model.md,
//! sections 7, 8 and 18.1; UI 2-3).

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::session::{PendingWrite, Session};
use super::{Core, Inner};
use crate::crypto;
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::index::Query;
use crate::lock::ClipboardGuard;
use crate::model::group::{self, GroupRules};
use crate::model::host::{self, HostRules};
use crate::model::order_key;
use crate::model::time::IdGenerator;
use crate::model::tree::{MAX_DEPTH, Tree};
use crate::model::types::RecordKind;
use crate::model::wipe::{WipeOnDrop, wipe_map};
use crate::service::dto::{
    CoreEventKind, LockState, MovePosition, RecordDeleteResult, RecordMoveInput, RecordRef,
    RecordSaveInput, RecordView, RecordsQuery, RecordsRef, SecretCopyTarget, SecretFieldRef,
    TagColor, VaultKind, VaultStatus, VaultView,
};
use crate::store::{RecordRow, Store};

/// The personal vault as the UI sees it (`name` and `color` from `vault_meta`).
pub(super) fn vault_view(session: &Session) -> VaultView {
    let meta = session.index.singleton(RecordKind::VaultMeta);
    let name = meta
        .and_then(|record| record.data().get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("Personal")
        .to_owned();
    let color = meta
        .and_then(|record| record.data().get("color"))
        .and_then(|color| serde_json::from_value::<TagColor>(color.clone()).ok())
        .unwrap_or_default();
    VaultView {
        vault_id: session.ring.vault_id(),
        kind: VaultKind::Personal,
        status: VaultStatus::Active,
        role: None,
        name,
        color,
    }
}

/// Disjoint borrows of the unlocked state (also used by the stage 2 facade
/// bodies in `connect.rs`).
pub(super) struct Parts<'a> {
    pub store: &'a mut Store,
    pub session: &'a mut Session,
    pub ids: &'a IdGenerator,
    pub v: &'a mut u64,
    pub connlog: &'a mut crate::connlog::Tracker,
}

pub(super) fn parts(inner: &mut Inner) -> Result<Parts<'_>, CoreError> {
    let Inner {
        store,
        session,
        state,
        ids,
        v,
        connlog,
        ..
    } = inner;
    match (state, session.as_mut()) {
        (LockState::Unlocked, Some(session)) => Ok(Parts {
            store,
            session,
            ids,
            v,
            connlog,
        }),
        _ => Err(CoreError::Locked),
    }
}

pub(super) fn check_vault(session: &Session, vault_id: Option<Uuid>) -> Result<(), CoreError> {
    match vault_id {
        Some(id) if id != session.ring.vault_id() => Err(CoreError::NotFound),
        _ => Ok(()),
    }
}

/// Distinct ids in their first order.
fn distinct(ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

/// The kind of every id (hosts and groups only), or `not_found`.
fn kinds_of(session: &Session, ids: &[Uuid]) -> Result<HashMap<Uuid, RecordKind>, CoreError> {
    let mut kinds = HashMap::new();
    for id in ids {
        match session.index.get(id).map(|record| record.kind) {
            Some(kind @ (RecordKind::Host | RecordKind::Group)) => {
                kinds.insert(*id, kind);
            }
            _ => return Err(CoreError::NotFound),
        }
    }
    Ok(kinds)
}

/// The raw container member of a record's data (`group_id` / `parent_id`).
fn container_member(kind: RecordKind, data: &Map<String, Value>) -> Option<Uuid> {
    match kind {
        RecordKind::Host => host::group_id(data),
        RecordKind::Group => group::parent_id(data),
        _ => None,
    }
}

fn set_container(kind: RecordKind, data: &mut Map<String, Value>, container: Option<Uuid>) {
    let member = match kind {
        RecordKind::Host => "group_id",
        RecordKind::Group => "parent_id",
        _ => return,
    };
    match container {
        Some(id) => {
            data.insert(member.into(), json!(id));
        }
        None => {
            data.remove(member);
        }
    }
}

/// `cycle` / `too_deep` for a group whose parent becomes `parent`.
fn group_tree_errors(
    tree: &Tree,
    record_id: Option<Uuid>,
    parent: Option<Uuid>,
    field: &str,
) -> Vec<FieldError> {
    let Some(parent) = parent.filter(|parent| tree.contains(parent)) else {
        return Vec::new();
    };
    let height = match record_id {
        Some(id) => {
            if tree.in_subtree(&parent, &id) {
                return vec![FieldError::new(field, ValidationReason::Cycle)];
            }
            tree.height(&id)
        }
        None => 1,
    };
    if tree.depth(&parent) + height > MAX_DEPTH {
        return vec![FieldError::new(field, ValidationReason::TooDeep)];
    }
    Vec::new()
}

/// A container for manual ordering: the parent group (`None` = top level) and the record kind.
type Container = (Option<Uuid>, RecordKind);

/// Final data of every record touched by one command; each record is
/// written once with a new HLC. Data still held when the plan is dropped (a
/// command that failed) is wiped: it may hold decrypted secrets.
#[derive(Default)]
pub(super) struct Plan {
    order: Vec<Uuid>,
    data: HashMap<Uuid, (RecordKind, Map<String, Value>)>,
}

impl Drop for Plan {
    fn drop(&mut self) {
        for (_, data) in self.data.values_mut() {
            wipe_map(data);
        }
    }
}

impl Plan {
    /// The planned data of a record, starting from its indexed data.
    fn entry(&mut self, session: &Session, kind: RecordKind, id: Uuid) -> &mut Map<String, Value> {
        let order = &mut self.order;
        &mut self
            .data
            .entry(id)
            .or_insert_with(|| {
                order.push(id);
                let data = session
                    .index
                    .get(&id)
                    .map(|record| record.data().clone())
                    .unwrap_or_default();
                (kind, data)
            })
            .1
    }

    pub(super) fn set(&mut self, kind: RecordKind, id: Uuid, data: Map<String, Value>) {
        if self.data.insert(id, (kind, data)).is_none() {
            self.order.push(id);
        }
    }

    /// Order keys from [`order_key::place`].
    fn apply_keys(&mut self, session: &Session, kind: RecordKind, keys: Vec<(Uuid, String)>) {
        for (id, key) in keys {
            self.entry(session, kind, id)
                .insert("order_key".into(), Value::String(key));
        }
    }

    pub(super) fn prepare(
        mut self,
        parts: &mut Parts<'_>,
        now: SystemTime,
    ) -> Result<Vec<PendingWrite>, CoreError> {
        let mut writes = Vec::with_capacity(self.order.len());
        for id in std::mem::take(&mut self.order) {
            if let Some((kind, data)) = self.data.remove(&id) {
                writes.push(
                    parts
                        .session
                        .prepare_write(parts.store, kind, id, data, now)?,
                );
            }
        }
        Ok(writes)
    }
}

impl Core {
    /// Writes `writes`, deletes `deletions` and puts `tombstones` in one
    /// transaction, updates the index and emits `records:changed`.
    pub(super) fn commit_records(
        &self,
        parts: &mut Parts<'_>,
        writes: Vec<PendingWrite>,
        deletions: Vec<(Uuid, u8)>,
        tombstones: Vec<RecordRow>,
    ) -> Result<(), CoreError> {
        let vault_id = parts.session.ring.vault_id();
        parts.store.write(|tx| {
            for write in &writes {
                tx.record_put(&write.row)?;
            }
            for (id, _) in &deletions {
                tx.record_delete(&vault_id, id)?;
            }
            for row in &tombstones {
                tx.record_put(row)?;
            }
            Ok(())
        })?;
        let mut changed: Vec<Uuid> = writes.iter().map(|write| write.record_id).collect();
        let mut classes: Vec<u8> = writes.iter().map(|write| write.kind.class()).collect();
        for (id, class) in &deletions {
            changed.push(*id);
            classes.push(*class);
            parts.session.index.remove(id);
        }
        for row in &tombstones {
            changed.push(row.record_id);
            classes.push(row.class);
            parts.session.index.remove(&row.record_id);
        }
        classes.sort_unstable();
        classes.dedup();
        parts.session.commit(writes);
        if !changed.is_empty() {
            self.emit_with(
                parts.v,
                CoreEventKind::RecordsChanged {
                    vault_id,
                    record_ids: changed,
                    classes,
                },
            );
        }
        Ok(())
    }

    pub(super) fn vaults_impl(&self) -> Result<Vec<VaultView>, CoreError> {
        let inner = self.inner();
        Ok(vec![vault_view(Self::require_session(&inner)?)])
    }

    pub(super) fn records_query_impl(
        &self,
        query: RecordsQuery,
    ) -> Result<Vec<RecordView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, query.vault_id)?;
        Ok(session.index.query(&Query {
            types: query.types,
            scope: query.scope,
            text: query.text,
            tags: query.tags,
            sort: query.sort,
        }))
    }

    pub(super) fn record_get_impl(&self, record: RecordRef) -> Result<RecordView, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(record.vault_id))?;
        session
            .index
            .view(&record.record_id)
            .ok_or(CoreError::NotFound)
    }

    pub(super) fn record_save_impl(&self, input: RecordSaveInput) -> Result<RecordView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        let kind = RecordKind::from_record_type(input.record_type);
        // A copy of the stored data, secrets included: wiped on every path.
        let stored: Option<WipeOnDrop> = match input.record_id {
            Some(id) => Some(WipeOnDrop::new(
                parts
                    .session
                    .index
                    .get_kind(&id, kind)
                    .ok_or(CoreError::NotFound)?
                    .data()
                    .clone(),
            )),
            None => None,
        };
        let tree = parts.session.index.tree();
        let built = match kind {
            RecordKind::Host => host::build(
                &input.data,
                stored.as_deref(),
                &input.secrets,
                &HostRules {
                    record_id: input.record_id,
                    personal_vault: true,
                },
            ),
            _ => {
                let built = group::build(
                    &input.data,
                    stored.as_deref(),
                    &input.secrets,
                    &GroupRules {
                        record_id: input.record_id,
                    },
                );
                let parent = input
                    .data
                    .get("parent_id")
                    .and_then(Value::as_str)
                    .and_then(|raw| Uuid::parse_str(raw.trim()).ok())
                    .filter(|parent| Some(*parent) != input.record_id);
                let tree_errors = group_tree_errors(&tree, input.record_id, parent, "parent_id");
                match built {
                    Ok(data) if tree_errors.is_empty() => Ok(data),
                    Ok(_) => Err(tree_errors),
                    Err(mut errors) => {
                        errors.extend(tree_errors);
                        Err(errors)
                    }
                }
            }
        };
        let mut data = built.map_err(CoreError::Validation)?;

        let record_id = input.record_id.unwrap_or_else(|| parts.ids.next(now));
        let new_container = container_member(kind, &data);
        let old_container = stored
            .as_deref()
            .and_then(|stored| container_member(kind, stored));
        let mut plan = Plan::default();
        if input.record_id.is_none() || new_container != old_container {
            let resolved = new_container.filter(|id| tree.contains(id));
            let order = parts
                .session
                .index
                .container_order(&tree, kind, resolved, &[record_id]);
            let keys = order_key::place(&order, order.len(), &[record_id]);
            for (id, key) in keys {
                if id == record_id {
                    data.insert("order_key".into(), Value::String(key));
                } else {
                    plan.entry(parts.session, kind, id)
                        .insert("order_key".into(), Value::String(key));
                }
            }
        }
        plan.set(kind, record_id, data);
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        parts
            .session
            .index
            .view(&record_id)
            .ok_or(CoreError::Internal("saved record missing"))
    }

    pub(super) fn record_delete_impl(
        &self,
        input: RecordsRef,
    ) -> Result<RecordDeleteResult, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        let ids = distinct(&input.record_ids);
        let kinds = kinds_of(parts.session, &ids)?;
        let tree = parts.session.index.tree();
        let removed: HashSet<Uuid> = ids.iter().copied().collect();

        // Children of removed groups move to the nearest remaining ancestor
        // (docs/data-model.md, section 7.1), appended at the end in order.
        let mut moves: Vec<(Option<Uuid>, RecordKind, Uuid)> = Vec::new();
        for id in ids.iter().filter(|id| kinds[*id] == RecordKind::Group) {
            let target = tree.nearest_ancestor_outside(id, &removed);
            for kind in [RecordKind::Group, RecordKind::Host] {
                for (child, _) in parts
                    .session
                    .index
                    .container_order(&tree, kind, Some(*id), &[])
                {
                    if !removed.contains(&child) {
                        moves.push((target, kind, child));
                    }
                }
            }
        }
        let moved: Vec<Uuid> = moves.iter().map(|(_, _, id)| *id).collect();
        let mut plan = Plan::default();
        let mut buckets: Vec<(Container, Vec<Uuid>)> = Vec::new();
        for (target, kind, id) in &moves {
            set_container(*kind, plan.entry(parts.session, *kind, *id), *target);
            match buckets.iter_mut().find(|(key, _)| *key == (*target, *kind)) {
                Some((_, list)) => list.push(*id),
                None => buckets.push(((*target, *kind), vec![*id])),
            }
        }
        let mut exclude: Vec<Uuid> = ids.clone();
        exclude.extend(moved.iter().copied());
        for ((target, kind), children) in buckets {
            let order = parts
                .session
                .index
                .container_order(&tree, kind, target, &exclude);
            let keys = order_key::place(&order, order.len(), &children);
            plan.apply_keys(parts.session, kind, keys);
        }

        // Rows: removed at once in local mode (vault not bound, record never
        // synced), otherwise tombstones (docs/storage-format.md, section 4.6).
        let vault_id = parts.session.ring.vault_id();
        let bound = parts.store.vault(&vault_id)?.is_some_and(|row| row.bound);
        let mut deletions = Vec::new();
        let mut tombstones = Vec::new();
        for id in &ids {
            let Some(previous) = parts.store.record(&vault_id, id)? else {
                continue;
            };
            if !bound && previous.synced_revision == 0 {
                deletions.push((*id, previous.class));
            } else {
                tombstones.push(parts.session.prepare_tombstone(kinds[id], &previous, now)?);
            }
        }
        let writes = plan.prepare(&mut parts, now)?;
        let undo: Vec<(Uuid, RecordKind, crate::model::payload::Payload)> = ids
            .iter()
            .filter_map(|id| {
                parts
                    .session
                    .index
                    .get(id)
                    .map(|record| (*id, kinds[id], record.payload.clone()))
            })
            .collect();
        self.commit_records(&mut parts, writes, deletions, tombstones)?;
        for (id, kind, payload) in undo {
            parts.session.undo.put(id, kind, payload, now);
        }
        tracing::info!(removed = ids.len(), moved = moved.len(), "records removed");
        Ok(RecordDeleteResult {
            deleted: ids,
            moved,
        })
    }

    pub(super) fn record_restore_impl(
        &self,
        input: RecordsRef,
    ) -> Result<Vec<RecordView>, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        parts.session.undo.purge(now);
        let ids = distinct(&input.record_ids);
        let mut plan = Plan::default();
        for id in &ids {
            match parts.session.undo.get(id) {
                Some(entry) if parts.session.index.get(id).is_none() => {
                    plan.set(
                        entry.kind,
                        *id,
                        entry.payload.data.clone().unwrap_or_default(),
                    );
                }
                _ => return Err(CoreError::NotFound),
            }
        }
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        for id in &ids {
            parts.session.undo.remove(id);
        }
        tracing::info!(restored = ids.len(), "records restored");
        Ok(parts.session.index.views(&ids))
    }

    pub(super) fn record_move_impl(
        &self,
        input: RecordMoveInput,
    ) -> Result<Vec<RecordView>, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        let ids = distinct(&input.record_ids);
        let kinds = kinds_of(parts.session, &ids)?;
        let tree = parts.session.index.tree();
        let target = input.target_group_id;

        let mut errors: Vec<FieldError> = Vec::new();
        if let Some(target) = target
            && parts
                .session
                .index
                .get_kind(&target, RecordKind::Group)
                .is_none()
        {
            errors.push(FieldError::new(
                "target_group_id",
                ValidationReason::MissingRef,
            ));
        }
        if errors.is_empty()
            && let Some(problem) = ids
                .iter()
                .filter(|id| kinds[*id] == RecordKind::Group)
                .map(|id| group_tree_errors(&tree, Some(*id), target, "target_group_id"))
                .find(|problems| !problems.is_empty())
        {
            errors.extend(problem);
        }
        let anchor = match input.position {
            MovePosition::End => None,
            MovePosition::Before { record_id } => Some((record_id, 0usize)),
            MovePosition::After { record_id } => Some((record_id, 1usize)),
        };
        let mut anchor_kind = None;
        if let Some((anchor_id, _)) = anchor {
            let index = &parts.session.index;
            match index.get(&anchor_id).filter(|record| {
                matches!(record.kind, RecordKind::Host | RecordKind::Group)
                    && !kinds.contains_key(&anchor_id)
                    && index.container_of(&tree, record) == target
            }) {
                Some(record) => anchor_kind = Some(record.kind),
                None => errors.push(FieldError::new("position", ValidationReason::Invalid)),
            }
        }
        if !errors.is_empty() {
            return Err(CoreError::Validation(errors));
        }

        let mut plan = Plan::default();
        for kind in [RecordKind::Group, RecordKind::Host] {
            let moved: Vec<Uuid> = ids.iter().copied().filter(|id| kinds[id] == kind).collect();
            if moved.is_empty() {
                continue;
            }
            let order = parts
                .session
                .index
                .container_order(&tree, kind, target, &ids);
            let position = match anchor {
                Some((anchor_id, offset)) if anchor_kind == Some(kind) => order
                    .iter()
                    .position(|(id, _)| *id == anchor_id)
                    .map_or(order.len(), |position| position + offset),
                _ => order.len(),
            };
            for id in &moved {
                set_container(kind, plan.entry(parts.session, kind, *id), target);
            }
            let keys = order_key::place(&order, position, &moved);
            plan.apply_keys(parts.session, kind, keys);
        }
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        Ok(parts.session.index.views(&ids))
    }

    pub(super) fn record_duplicate_impl(&self, record: RecordRef) -> Result<RecordView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(record.vault_id))?;
        let index = &parts.session.index;
        let original = match index.get(&record.record_id) {
            Some(original) if original.kind == RecordKind::Host => original,
            Some(original) if original.kind == RecordKind::Group => {
                return Err(CoreError::field("type", ValidationReason::Invalid));
            }
            _ => return Err(CoreError::NotFound),
        };
        let mut data = original.data().clone();
        let base: String = host::title(&data).chars().take(251).collect();
        data.insert("label".into(), Value::String(format!("{base} Copy")));
        data.remove("import_key");
        data.remove("order_key");
        let tree = index.tree();
        let container = index.container_of(&tree, original);
        let order = index.container_order(&tree, RecordKind::Host, container, &[]);
        let position = order
            .iter()
            .position(|(id, _)| *id == record.record_id)
            .map_or(order.len(), |position| position + 1);
        let new_id = parts.ids.next(now);
        let mut plan = Plan::default();
        plan.set(RecordKind::Host, new_id, data);
        let keys = order_key::place(&order, position, &[new_id]);
        plan.apply_keys(parts.session, RecordKind::Host, keys);
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        parts
            .session
            .index
            .view(&new_id)
            .ok_or(CoreError::Internal("duplicate missing"))
    }

    /// The value of a secret field (reveal and copy): a host's passwords, a
    /// key's private key and saved passphrase (UI 3.1, 7.3).
    fn secret_text(
        session: &Session,
        vault_id: Uuid,
        record_id: Uuid,
        path: &str,
    ) -> Result<SecretString, CoreError> {
        check_vault(session, Some(vault_id))?;
        let record = session
            .index
            .get(&record_id)
            .filter(|record| {
                matches!(
                    record.kind,
                    RecordKind::Host | RecordKind::Group | RecordKind::Key
                )
            })
            .ok_or(CoreError::NotFound)?;
        if !record.kind.secret_paths().contains(&path) {
            return Err(CoreError::field("path", ValidationReason::NotAllowed));
        }
        let value = match record.kind {
            RecordKind::Key => crate::model::key::secret_value(record.data(), path),
            _ => host::secret_value(record.data(), path),
        };
        value
            .map(|value| SecretString::from(value.to_owned()))
            .ok_or(CoreError::NotFound)
    }

    pub(super) fn secret_reveal_impl(
        &self,
        field: SecretFieldRef,
    ) -> Result<SecretString, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        Self::secret_text(session, field.vault_id, field.record_id, &field.path)
    }

    pub(super) fn secret_copy_impl(&self, target: SecretCopyTarget) -> Result<(), CoreError> {
        let now = self.now();
        let mut inner = self.inner();
        let session = Self::require_session(&inner)?;
        let text = match target {
            SecretCopyTarget::RecordField {
                vault_id,
                record_id,
                path,
            } => Self::secret_text(session, vault_id, record_id, &path)?,
            SecretCopyTarget::RecoveryKey => {
                if !inner.recovery_grant.allowed(now) {
                    return Err(CoreError::WrongState);
                }
                crypto::encode_recovery_key(&super::lock_ops::recovery_key_of(session)?)
            }
        };
        self.platform
            .clipboard
            .write_sensitive(&text)
            .map_err(|_| CoreError::Unavailable)?;
        inner.clipboard_guard = Some(ClipboardGuard::new(text.expose_secret(), now));
        Ok(())
    }
}
