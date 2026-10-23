//! Import of hosts from a CSV file with a column mapping (SPEC "Import";
//! UI 2.5; run 8 decision D141), over the reader of `crate::import::csv`.
//!
//! - [`Core::csv_inspect`]: the character set, the delimiter and the header
//!   (detected unless given), the columns with their guessed roles and the
//!   first rows, for the mapping step.
//! - [`Core::csv_preview`]: every row with its status (new, already exists,
//!   not imported with the reason), the groups the import creates and the
//!   number of rows with a password.
//! - [`Core::csv_import`]: imports the new rows.
//!
//! Each row is a host: `address` is required; `label`, `port`, `username`,
//! `notes`; `group` is a path with ` / ` between levels (missing groups are
//! created, existing ones matched by parent and name); `tags` are separated
//! by `,` or `;`; `password` is stored as the host's `ssh.password` (the
//! dialog warns that the file holds it in clear text).
//!
//! Idempotent (docs/data-model.md, section 18.4): CSV has no source key, so
//! a row matches a live host with the same address (case-insensitively),
//! effective port and effective username after inheritance; a match is
//! "already exists" and never overwritten. Duplicates inside the file are
//! reduced to the first row.

use std::collections::{BTreeMap, HashMap, HashSet};

use secrecy::SecretString;
use serde_json::{Map, Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::Core;
use super::dto::{
    CsvImportSummary, CsvInput, CsvInspection, CsvPreview, CsvRowPreview, ImportStatus,
};
use super::records::{Plan, parts};
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::import::csv::{self, Role};
use crate::index::Index;
use crate::model::group::{self, GroupRules};
use crate::model::host::{self, HostRules};
use crate::model::tree::Tree;
use crate::model::types::RecordKind;
use crate::model::wipe::WipeOnDrop;

/// Rows of the mapping step's sample.
const SAMPLE_ROWS: usize = 5;
/// The separator of group path levels (docs/data-model.md, section 18.4).
const GROUP_SEPARATOR: &str = " / ";
/// Most levels of a group path (the tree allows 32).
const MAX_GROUP_LEVELS: usize = 16;

fn invalid(field: &str) -> CoreError {
    CoreError::Validation(vec![FieldError::new(field, ValidationReason::Invalid)])
}

/// The file read with its settings: detected where `input` leaves them out.
struct Read {
    encoding: String,
    delimiter: char,
    has_header: bool,
    roles: Vec<Role>,
    /// Every record (the header first when there is one).
    table: csv::Records,
}

fn read(input: &CsvInput) -> Result<Read, CoreError> {
    if input.bytes.len() > csv::MAX_FILE_BYTES {
        return Err(CoreError::TooLarge);
    }
    let encoding = match &input.encoding {
        Some(label) => label.trim().to_ascii_lowercase(),
        None => csv::detect_encoding(&input.bytes).to_owned(),
    };
    let text = Zeroizing::new(
        csv::decode(&input.bytes, &encoding)
            .ok_or_else(|| invalid("encoding"))?
            .into_owned(),
    );
    let delimiter = match input.delimiter {
        Some(delimiter) if !matches!(delimiter, '"' | '\r' | '\n') => delimiter,
        Some(_) => return Err(invalid("delimiter")),
        None => csv::detect_delimiter(&text),
    };
    let table = csv::records(&text, delimiter);
    // A quote left open would put the rest of the file (other rows'
    // passwords too) into one field; a cut file would lose rows silently.
    if table.unterminated {
        return Err(invalid("quote"));
    }
    if table.truncated {
        return Err(CoreError::TooLarge);
    }
    let width = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    let (guessed, header) = table
        .rows
        .first()
        .map(|first| csv::guess_roles(first))
        .unwrap_or_default();
    let has_header = input.has_header.unwrap_or(header);
    let mut roles = match &input.roles {
        Some(roles) => roles.clone(),
        None if header == has_header => guessed,
        // The user changed the header choice: columns in the SPEC's order.
        None => (0..width)
            .map(|index| Role::ORDER.get(index).copied().unwrap_or(Role::Ignore))
            .collect(),
    };
    roles.resize(width, Role::Ignore);
    Ok(Read {
        encoding,
        delimiter,
        has_header,
        roles,
        table,
    })
}

/// What one row becomes.
struct RowPlan {
    /// 1-based record number in the file (the header is 1).
    line: usize,
    label: Option<String>,
    address: String,
    port: Option<u16>,
    username: Option<String>,
    /// The group path's levels.
    group: Vec<String>,
    tags: Vec<String>,
    notes: Option<String>,
    password: Option<SecretString>,
    error: Option<String>,
}

fn field<'a>(record: &'a [String], roles: &[Role], role: Role) -> Option<&'a str> {
    let index = roles.iter().position(|candidate| *candidate == role)?;
    record
        .get(index)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn plan_row(line: usize, record: &[String], roles: &[Role]) -> RowPlan {
    let get = |role| field(record, roles, role);
    let mut error = None;
    let address = get(Role::Address).unwrap_or_default().to_owned();
    if address.is_empty() {
        error = Some("the address is empty".to_owned());
    }
    let port = match get(Role::Port).map(str::parse::<u16>) {
        Some(Ok(port)) if port > 0 => Some(port),
        Some(_) => {
            error.get_or_insert_with(|| "the port is not valid".to_owned());
            None
        }
        None => None,
    };
    let group: Vec<String> = get(Role::Group)
        .map(|path| {
            path.split(GROUP_SEPARATOR)
                .map(str::trim)
                .filter(|level| !level.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if group.len() > MAX_GROUP_LEVELS {
        error.get_or_insert_with(|| "the group path is too deep".to_owned());
    }
    let tags = get(Role::Tags)
        .map(|tags| {
            tags.split([',', ';'])
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    RowPlan {
        line,
        label: get(Role::Label).map(str::to_owned),
        address,
        port,
        username: get(Role::Username).map(str::to_owned),
        group,
        tags,
        notes: get(Role::Notes).map(str::to_owned),
        password: get(Role::Password).map(|password| SecretString::from(password.to_owned())),
        error,
    }
}

/// The host data of a row in `group_id`, and its secrets.
fn row_data(
    row: &RowPlan,
    group_id: Option<Uuid>,
) -> (Map<String, Value>, BTreeMap<String, Option<SecretString>>) {
    let mut data = Map::new();
    data.insert("address".into(), json!(row.address));
    if let Some(label) = &row.label {
        data.insert("label".into(), json!(label));
    }
    if let Some(group_id) = group_id {
        data.insert("group_id".into(), json!(group_id.to_string()));
    }
    if !row.tags.is_empty() {
        data.insert("tags".into(), json!(row.tags));
    }
    if let Some(notes) = &row.notes {
        data.insert("notes".into(), json!(notes));
    }
    let mut ssh = Map::new();
    if let Some(port) = row.port {
        ssh.insert("port".into(), json!(port));
    }
    if let Some(user) = &row.username {
        ssh.insert("username".into(), json!(user));
    }
    data.insert("ssh".into(), Value::Object(ssh));
    let mut secrets = BTreeMap::new();
    if let Some(password) = &row.password {
        secrets.insert("ssh.password".to_owned(), Some(password.clone()));
    }
    (data, secrets)
}

/// The groups of the vault by (parent, name), and the tree.
struct Groups {
    tree: Tree,
    by_name: HashMap<(Option<Uuid>, String), Uuid>,
}

impl Groups {
    fn of(index: &Index) -> Self {
        let tree = index.tree();
        let by_name = index
            .of_kind(RecordKind::Group)
            .filter_map(|record| {
                let name = record.data().get("name").and_then(Value::as_str)?;
                Some((
                    (tree.parent(&record.record_id), name.to_owned()),
                    record.record_id,
                ))
            })
            .collect();
        Self { tree, by_name }
    }

    /// The deepest existing group along `path` (`None`: the top level) and
    /// the number of levels that exist.
    fn deepest(&self, path: &[String]) -> (Option<Uuid>, usize) {
        let mut parent = None;
        for (depth, name) in path.iter().enumerate() {
            match self.by_name.get(&(parent, name.clone())) {
                Some(id) => parent = Some(*id),
                None => return (parent, depth),
            }
        }
        (parent, path.len())
    }

    /// The existing group at `path` (`None` when a level is missing or the
    /// path is empty).
    fn find(&self, path: &[String]) -> Option<Uuid> {
        match self.deepest(path) {
            (found, levels) if levels == path.len() => found,
            _ => None,
        }
    }
}

/// A host's comparison key: address (lowercase), effective port and
/// effective username after inheritance.
type HostKey = (String, u16, String);

/// The effective port and username of host `data` (its own, else its
/// identity's username, else inherited from its groups).
fn host_key(index: &Index, tree: &Tree, data: &Map<String, Value>) -> HostKey {
    let ssh = data.get("ssh").and_then(Value::as_object);
    let own = |name: &str| ssh.and_then(|ssh| ssh.get(name));
    let inherited = index.inherited_for_host(tree, data);
    let port = own("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .or(inherited.port.map(|port| port.value))
        .unwrap_or(22);
    let identity_user = own("identity_id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<Uuid>().ok())
        .and_then(|id| index.get_kind(&id, RecordKind::Identity))
        .and_then(|identity| identity.data().get("username").and_then(Value::as_str))
        .map(str::to_owned);
    let username = own("username")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(identity_user)
        .or(inherited.username.map(|user| user.value))
        .unwrap_or_default();
    let address = data
        .get("address")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    (address, port, username)
}

/// Rows with their statuses, the keys of the vault's hosts, and the new
/// group paths.
struct Planned {
    rows: Vec<(RowPlan, ImportStatus)>,
    new_groups: Vec<Vec<String>>,
    passwords: usize,
}

fn plan_rows(read: &Read, index: &Index) -> Result<Planned, CoreError> {
    if !read.roles.contains(&Role::Address) {
        return Err(CoreError::Validation(vec![FieldError::new(
            "roles",
            ValidationReason::Required,
        )]));
    }
    let groups = Groups::of(index);
    let existing: HashSet<HostKey> = index
        .of_kind(RecordKind::Host)
        .map(|record| host_key(index, &groups.tree, record.data()))
        .collect();
    let mut seen: HashSet<HostKey> = HashSet::new();
    let mut new_groups: Vec<Vec<String>> = Vec::new();
    let mut rows = Vec::new();
    let mut passwords = 0;
    let skip = usize::from(read.has_header);
    for (number, record) in read.table.rows.iter().enumerate().skip(skip) {
        let mut row = plan_row(number + 1, record, &read.roles);
        if row.password.is_some() {
            passwords += 1;
        }
        let status = if row.error.is_some() {
            ImportStatus::Error
        } else {
            // The host as the rules build it (the address normalized, as
            // it is stored), under its group when the path exists, else
            // under the deepest group that exists: new groups have no
            // settings, so that is where its inherited values come from.
            let (ancestor, levels) = groups.deepest(&row.group);
            let (data, secrets) = row_data(&row, ancestor);
            let rules = HostRules {
                record_id: None,
                personal_vault: true,
            };
            match host::build(&data, None, &secrets, &rules) {
                Err(errors) => {
                    let fields: Vec<String> = errors.into_iter().map(|error| error.field).collect();
                    row.error = Some(format!("invalid {}", fields.join(", ")));
                    ImportStatus::Error
                }
                Ok(built) => {
                    // It holds the password in clear: wiped when dropped.
                    let built = WipeOnDrop::new(built);
                    let key = host_key(index, &groups.tree, &built);
                    if existing.contains(&key) || !seen.insert(key) {
                        ImportStatus::Exists
                    } else {
                        for depth in levels..row.group.len() {
                            let path = row.group[..=depth].to_vec();
                            if !new_groups.contains(&path) {
                                new_groups.push(path);
                            }
                        }
                        ImportStatus::New
                    }
                }
            }
        };
        rows.push((row, status));
    }
    Ok(Planned {
        rows,
        new_groups,
        passwords,
    })
}

impl Core {
    pub(super) fn csv_inspect_impl(&self, input: &CsvInput) -> Result<CsvInspection, CoreError> {
        let read = read(input)?;
        let width = read.roles.len();
        let columns: Vec<String> = (0..width)
            .map(|index| {
                read.table
                    .rows
                    .first()
                    .filter(|_| read.has_header)
                    .and_then(|header| header.get(index))
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("Column {}", index + 1))
            })
            .collect();
        let skip = usize::from(read.has_header);
        let sample = read
            .table
            .rows
            .iter()
            .skip(skip)
            .take(SAMPLE_ROWS)
            .cloned()
            .collect();
        Ok(CsvInspection {
            encoding: read.encoding.clone(),
            delimiter: read.delimiter,
            has_header: read.has_header,
            columns,
            roles: read.roles.clone(),
            sample,
            rows: read.table.rows.len().saturating_sub(skip),
        })
    }

    pub(super) fn csv_preview_impl(&self, input: &CsvInput) -> Result<CsvPreview, CoreError> {
        let read = read(input)?;
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        let planned = plan_rows(&read, &parts.session.index)?;
        Ok(CsvPreview {
            rows: planned
                .rows
                .iter()
                .map(|(row, status)| CsvRowPreview {
                    line: row.line,
                    label: row.label.clone(),
                    address: row.address.clone(),
                    port: row.port,
                    username: row.username.clone(),
                    group: (!row.group.is_empty()).then(|| row.group.join(GROUP_SEPARATOR)),
                    status: *status,
                    reason: row.error.clone(),
                })
                .collect(),
            groups_new: planned.new_groups.len(),
            passwords: planned.passwords,
        })
    }

    pub(super) fn csv_import_impl(&self, input: &CsvInput) -> Result<CsvImportSummary, CoreError> {
        let read = read(input)?;
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let planned = plan_rows(&read, &parts.session.index)?;
        let groups = Groups::of(&parts.session.index);
        let mut summary = CsvImportSummary {
            passwords: planned.passwords,
            ..CsvImportSummary::default()
        };
        let mut plan = Plan::default();
        // New groups first, parents before children.
        let mut created: HashMap<Vec<String>, Uuid> = HashMap::new();
        for path in &planned.new_groups {
            let parent = match path.len() {
                1 => None,
                n => {
                    let parent_path = &path[..n - 1];
                    created
                        .get(parent_path)
                        .copied()
                        .or_else(|| groups.find(parent_path))
                }
            };
            let id = parts.ids.next(now);
            let mut data = Map::new();
            data.insert("name".into(), json!(path[path.len() - 1]));
            if let Some(parent) = parent {
                data.insert("parent_id".into(), json!(parent.to_string()));
            }
            match group::build(
                &data,
                None,
                &BTreeMap::new(),
                &GroupRules {
                    record_id: Some(id),
                },
            ) {
                Ok(built) => {
                    plan.set(RecordKind::Group, id, built);
                    created.insert(path.clone(), id);
                    summary.groups_added += 1;
                }
                Err(_) => {
                    // Its hosts fail below ("invalid group").
                }
            }
        }
        for (row, status) in &planned.rows {
            match status {
                ImportStatus::Exists => {
                    summary.skipped += 1;
                    continue;
                }
                ImportStatus::Error => {
                    summary.errors.push((
                        row.line,
                        row.error.clone().unwrap_or_else(|| "not imported".into()),
                    ));
                    continue;
                }
                ImportStatus::New => {}
            }
            let group_id = if row.group.is_empty() {
                None
            } else {
                match created
                    .get(&row.group)
                    .copied()
                    .or_else(|| groups.find(&row.group))
                {
                    Some(id) => Some(id),
                    None => {
                        summary.errors.push((row.line, "invalid group".into()));
                        continue;
                    }
                }
            };
            let id = parts.ids.next(now);
            let (data, secrets) = row_data(row, group_id);
            let rules = HostRules {
                record_id: Some(id),
                personal_vault: true,
            };
            match host::build(&data, None, &secrets, &rules) {
                Ok(built) => {
                    plan.set(RecordKind::Host, id, built);
                    summary.added += 1;
                }
                Err(errors) => {
                    let fields: Vec<String> = errors.into_iter().map(|error| error.field).collect();
                    summary
                        .errors
                        .push((row.line, format!("invalid {}", fields.join(", "))));
                }
            }
        }
        if summary.added > 0 || summary.groups_added > 0 {
            let prepared = plan.prepare(&mut parts, now)?;
            self.commit_records(&mut parts, prepared, Vec::new(), Vec::new())?;
        }
        tracing::info!(
            added = summary.added,
            skipped = summary.skipped,
            errors = summary.errors.len(),
            groups = summary.groups_added,
            "csv imported"
        );
        Ok(summary)
    }
}
