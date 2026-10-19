//! `--demo`: a temporary profile (`%TEMP%\shellpp-demo-<random>` on Windows)
//! with a generated master password (never shown; the app starts unlocked)
//! and fictional data written through `shellpp-core`: 5 groups (one nested)
//! and 300 hosts on documentation addresses (`example.com`, `192.0.2.0/24`,
//! `198.51.100.0/24`). About a third of the hosts have tags, about a quarter
//! a tile color other than graphite, and about half live in groups.
//! The profile is deleted when the app exits.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use shellpp_core::service::dto::{
    LockState, RecordSaveInput, RecordType, RecordView, TagColor, VaultKind,
};
use shellpp_core::{Core, CoreConfig, CoreError};
use uuid::Uuid;

use crate::platform::{self, events::event_channel};
use crate::profile::TempProfile;
use crate::state::OpenedCore;
use crate::testing::generated_password;
use crate::theme::palette::TAG_COLORS;

/// Directory name prefix of demo profiles.
pub const DEMO_PREFIX: &str = "shellpp-demo";
pub const DEMO_GROUPS: usize = 5;
pub const DEMO_HOSTS: usize = 300;

/// Why the demo could not start.
#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("the temporary profile could not be created")]
    Profile(#[source] std::io::Error),
    #[error("the core failed: {}", .0.code())]
    Core(#[source] CoreError),
}

/// What [`seed`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedSummary {
    pub groups: Vec<Uuid>,
    pub hosts: usize,
    pub hosts_with_tags: usize,
    pub hosts_with_color: usize,
    pub hosts_in_groups: usize,
}

/// Creates the temporary demo profile directory.
pub fn create_profile() -> Result<TempProfile, DemoError> {
    let profile = TempProfile::create(DEMO_PREFIX).map_err(DemoError::Profile)?;
    profile.dirs().create().map_err(|error| match error {
        crate::dirs::DirsError::Create(io) => DemoError::Profile(io),
        other => DemoError::Profile(std::io::Error::other(other.to_string())),
    })?;
    Ok(profile)
}

/// Opens the core in `profile` (keychain in memory, never the OS keychain),
/// runs the first run with a generated password and seeds the data. The
/// returned value owns the profile, so the app deletes it at shutdown.
pub fn open(profile: TempProfile) -> Result<(OpenedCore, SeedSummary), DemoError> {
    let dirs = profile.dirs();
    let (sink, mut events) = event_channel();
    let (platform, extras) = platform::temporary(sink);
    let core = Core::open(
        CoreConfig {
            data_dir: dirs.data.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        platform,
    )
    .map_err(DemoError::Core)?;
    if core.lock_status().state == LockState::FirstRun {
        // The recovery key is dropped (zeroized) right away: the profile is
        // deleted on exit anyway.
        core.first_run_setup(generated_password())
            .map_err(DemoError::Core)?;
    }
    let summary = seed(&core).map_err(DemoError::Core)?;
    // The seeding events are already reflected in the first load.
    while events.try_recv().is_ok() {}
    Ok((
        OpenedCore {
            core: Arc::new(core),
            events,
            extras,
            dirs: Some(dirs),
            temp_profile: Some(profile),
        },
        summary,
    ))
}

const ROLES: [&str; 16] = [
    "web", "api", "db", "cache", "queue", "worker", "proxy", "build", "monitor", "backup", "mail",
    "vpn", "git", "search", "auth", "files",
];
const ENVIRONMENTS: [&str; 4] = ["prod", "staging", "office", "lab"];
const USERS: [&str; 6] = ["root", "admin", "deploy", "ubuntu", "ops", "backup"];
const TAGS: [&str; 8] = [
    "prod", "staging", "linux", "windows", "critical", "eu", "us", "legacy",
];

fn save(
    core: &Core,
    vault_id: Uuid,
    record_type: RecordType,
    data: Value,
) -> Result<Uuid, CoreError> {
    let data: Map<String, Value> = match data {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let view = core.record_save(RecordSaveInput {
        vault_id,
        record_id: None,
        record_type,
        data,
        secrets: Default::default(),
    })?;
    Ok(match view {
        RecordView::Host(view) => view.record_id,
        RecordView::Group(view) => view.record_id,
    })
}

fn color_name(color: TagColor) -> Value {
    serde_json::to_value(color).unwrap_or(Value::Null)
}

/// Writes the demo groups and hosts into the personal vault of an unlocked core.
pub fn seed(core: &Core) -> Result<SeedSummary, CoreError> {
    let vault_id = core
        .vaults()?
        .into_iter()
        .find(|vault| vault.kind == VaultKind::Personal)
        .map(|vault| vault.vault_id)
        .ok_or(CoreError::NotFound)?;

    let production = save(
        core,
        vault_id,
        RecordType::Group,
        json!({"name": "Production", "color": "jade"}),
    )?;
    let databases = save(
        core,
        vault_id,
        RecordType::Group,
        json!({"name": "Databases", "parent_id": production, "ssh": {"username": "postgres"}}),
    )?;
    let staging = save(
        core,
        vault_id,
        RecordType::Group,
        json!({"name": "Staging"}),
    )?;
    let office = save(
        core,
        vault_id,
        RecordType::Group,
        json!({"name": "Office network", "color": "ocean"}),
    )?;
    let lab = save(core, vault_id, RecordType::Group, json!({"name": "Lab"}))?;
    let groups = vec![production, databases, staging, office, lab];

    let mut summary = SeedSummary {
        groups: groups.clone(),
        hosts: 0,
        hosts_with_tags: 0,
        hosts_with_color: 0,
        hosts_in_groups: 0,
    };
    for index in 0..DEMO_HOSTS {
        let role = ROLES[index % ROLES.len()];
        let environment = ENVIRONMENTS[(index / ROLES.len()) % ENVIRONMENTS.len()];
        let number = index / (ROLES.len() * ENVIRONMENTS.len()) + 1;
        let address = match index % 3 {
            0 => format!("192.0.2.{}", index % 254 + 1),
            1 => format!("198.51.100.{}", index % 254 + 1),
            _ => format!("{role}-{number:02}.{environment}.example.com"),
        };
        let mut data = json!({ "address": address });
        if index % 5 != 4 {
            data["label"] = json!(format!("{role} {number:02} ({environment})"));
        }
        let mut ssh = json!({ "username": USERS[index % USERS.len()] });
        if index % 7 == 3 {
            ssh["port"] = json!(2222);
        }
        data["ssh"] = ssh;
        if index % 3 == 0 {
            let tags = [TAGS[index % TAGS.len()], TAGS[(index / 3 + 2) % TAGS.len()]];
            let mut tags: Vec<&str> = tags.to_vec();
            tags.dedup();
            data["tags"] = json!(tags);
            summary.hosts_with_tags += 1;
        }
        if index % 4 == 1 {
            // A non-default color (any of the seven besides graphite).
            let color = TAG_COLORS[1 + (index / 4) % (TAG_COLORS.len() - 1)];
            data["color"] = color_name(color);
            summary.hosts_with_color += 1;
        }
        let group = match index % 10 {
            0 => Some(production),
            1 => Some(databases),
            2 => Some(staging),
            3 => Some(office),
            4 => Some(lab),
            _ => None,
        };
        if let Some(group) = group {
            data["group_id"] = json!(group);
            summary.hosts_in_groups += 1;
        }
        save(core, vault_id, RecordType::Host, data)?;
        summary.hosts += 1;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestProfile;
    use shellpp_core::service::dto::{QueryScope, RecordsQuery, SortMode};

    #[test]
    fn seeds_groups_and_hosts_with_the_requested_shape() {
        let mut profile = TestProfile::new();
        let opened = profile.open_unlocked();
        let summary = seed(&opened.core).unwrap();
        assert_eq!(summary.groups.len(), DEMO_GROUPS);
        assert_eq!(summary.hosts, DEMO_HOSTS);
        // About a third with tags, about a quarter with a color.
        assert_eq!(summary.hosts_with_tags, DEMO_HOSTS / 3);
        assert_eq!(summary.hosts_with_color, DEMO_HOSTS / 4);
        assert!(summary.hosts_in_groups > 0 && summary.hosts_in_groups < DEMO_HOSTS);

        let views = opened
            .core
            .records_query(RecordsQuery {
                vault_id: None,
                types: vec![RecordType::Host, RecordType::Group],
                scope: QueryScope::All,
                text: None,
                tags: Vec::new(),
                sort: SortMode::Manual,
            })
            .unwrap();
        let (hosts, groups) = crate::state::model::split_views(&views);
        assert_eq!(hosts.len(), DEMO_HOSTS);
        assert_eq!(groups.len(), DEMO_GROUPS);
        // One nested group.
        assert_eq!(
            groups
                .iter()
                .filter(|group| group.parent_id.is_some())
                .count(),
            1
        );
        // Documentation addresses only.
        for host in hosts.iter() {
            assert!(
                host.address.ends_with(".example.com")
                    || host.address.starts_with("192.0.2.")
                    || host.address.starts_with("198.51.100."),
                "{}",
                host.address
            );
        }
        assert!(hosts.iter().all(|host| host.username.is_some()));
    }

    /// The whole `--demo` setup without the window: temporary profile, real
    /// first run (generated password), seeded data, deletion on drop.
    #[test]
    fn open_creates_an_unlocked_seeded_profile_that_is_removed() {
        let profile = create_profile().unwrap();
        let path = profile.path().to_path_buf();
        let (opened, summary) = open(profile).unwrap();
        assert_eq!(opened.core.lock_status().state, LockState::Unlocked);
        assert_eq!(summary.hosts, DEMO_HOSTS);
        assert!(path.join(shellpp_core::ids::DB_FILE_NAME).is_file());
        // The seeding events were drained before the app sees the receiver.
        let mut events = opened.events;
        assert!(events.try_recv().is_err());
        drop(events);
        drop(opened.core);
        drop(opened.temp_profile);
        assert!(!path.exists());
    }

    #[test]
    fn demo_profiles_are_temporary_and_removed() {
        let profile = create_profile().unwrap();
        let path = profile.path().to_path_buf();
        assert!(path.starts_with(std::env::temp_dir()));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("shellpp-demo-")
        );
        drop(profile);
        assert!(!path.exists());
    }
}
