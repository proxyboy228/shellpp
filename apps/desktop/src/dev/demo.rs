//! `--demo`: a temporary profile (`%TEMP%\shellpp-demo-<random>` on Windows)
//! with a generated master password (never shown; the app starts unlocked)
//! and fictional data written through `shellpp-core`: 5 groups (one nested)
//! and 300 hosts on documentation addresses (`example.com`, `192.0.2.0/24`,
//! `198.51.100.0/24`). About a third of the hosts have tags, about a quarter
//! a tile color other than graphite, and about half live in groups. The
//! Keychain gets three keys generated at seeding time and two identities.
//! The profile is deleted when the app exits.
//!
//! [`open_fresh`] opens an empty temporary profile at a chosen lock stage
//! (first run, unlocked, locked) for the screenshots of the first-run, unlock
//! and empty screens. Every profile here keeps the device key ("remember on
//! this device") in memory, never in the OS keychain.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use shellpp_core::lock::FirstRunOptions;
use shellpp_core::service::dto::KeyGenerateInput;
use shellpp_core::service::dto::{
    LockState, RecordSaveInput, RecordType, RecordView, TagColor, VaultKind,
};
use shellpp_core::service::keychain::IdentitySaveInput;
use shellpp_core::{Core, CoreConfig, CoreError};
use uuid::Uuid;

use crate::state::records_edit::{KeyKind, PasswordChange};

use crate::platform::{self, events::event_channel};
use crate::profile::TempProfile;
use crate::state::OpenedCore;
use crate::testing::generated_password;
use crate::theme::palette::TAG_COLORS;

/// Directory name prefix of demo profiles.
pub const DEMO_PREFIX: &str = "shellpp-demo";
pub const DEMO_GROUPS: usize = 5;
pub const DEMO_HOSTS: usize = 300;
/// The demo keys of the Keychain: label and type.
pub const DEMO_KEYS: [(&str, KeyKind); 3] = [
    ("deploy@web-01", KeyKind::Ed25519),
    ("CI runner", KeyKind::EcdsaP256),
    ("Backup server", KeyKind::Ed25519),
];
pub const DEMO_IDENTITIES: usize = 2;

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
    pub keys: usize,
    pub identities: usize,
}

/// Creates the temporary demo profile directory.
pub fn create_profile() -> Result<TempProfile, DemoError> {
    create_profile_in(&std::env::temp_dir())
}

/// Creates the demo profile directory `<parent>/shellpp-demo-<random>`
/// (`--screenshots` puts it inside the `SHELLPP_DATA_DIR` it is given).
/// It is deleted like any temporary profile.
pub fn create_profile_in(parent: &std::path::Path) -> Result<TempProfile, DemoError> {
    let profile = TempProfile::create_in(parent, DEMO_PREFIX).map_err(DemoError::Profile)?;
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
    let mut opened = open_core(profile)?;
    first_run(&opened.core)?;
    let summary = seed(&opened.core).map_err(DemoError::Core)?;
    // The seeding events are already reflected in the first load.
    while opened.events.try_recv().is_ok() {}
    Ok((opened, summary))
}

/// The lock stage of a profile opened by [`open_fresh`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreshStage {
    /// Nothing set up: the app shows the first-run screen (UI 13.1).
    FirstRun,
    /// First run done with a generated password: the empty Hosts screen.
    Unlocked,
    /// First run done, then locked: the unlock screen (UI 13.2).
    Locked,
}

/// Opens an empty temporary profile at `stage` (no records; device key in
/// memory). The returned value owns the profile, so the app state deletes
/// it at shutdown.
pub fn open_fresh(profile: TempProfile, stage: FreshStage) -> Result<OpenedCore, DemoError> {
    let mut opened = open_core(profile)?;
    if stage != FreshStage::FirstRun {
        first_run(&opened.core)?;
    }
    if stage == FreshStage::Locked {
        opened.core.lock().map_err(DemoError::Core)?;
    }
    while opened.events.try_recv().is_ok() {}
    Ok(opened)
}

/// Opens the core in `profile` with the platform of temporary profiles
/// (secret store in memory).
fn open_core(profile: TempProfile) -> Result<OpenedCore, DemoError> {
    let dirs = profile.dirs();
    let (sink, events) = event_channel();
    let (platform, extras) = platform::temporary(sink);
    // Script generation keys stay in memory too (D144).
    crate::snippets::generate::keep_keys_in_memory();
    let core = Core::open(
        CoreConfig {
            data_dir: dirs.data.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        platform,
    )
    .map_err(DemoError::Core)?;
    Ok(OpenedCore {
        core: Arc::new(core),
        events,
        extras,
        dirs: Some(dirs),
        temp_profile: Some(profile),
    })
}

/// The first run as the app runs it (decision D29: "Remember on this
/// device" on, here into the in-memory secret store) with a generated master
/// password, never shown or kept: the profile is temporary. Does nothing
/// when it already ran.
fn first_run(core: &Core) -> Result<(), DemoError> {
    if core.lock_status().state == LockState::FirstRun {
        core.first_run_setup_with(generated_password(), FirstRunOptions::default())
            .map_err(DemoError::Core)?;
    }
    Ok(())
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
/// The tags of the first host (the others have one or two): with all four
/// on and the host panel open, more chips are on than fit in the action bar
/// (the screenshot state tag-chips shows some chips and "+N").
pub const FIRST_HOST_TAGS: [&str; 4] = ["prod", "linux", "critical", "eu"];

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
        base: None,
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
        keys: 0,
        identities: 0,
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
            let mut tags: Vec<&str> = if index == 0 {
                FIRST_HOST_TAGS.to_vec()
            } else {
                vec![TAGS[index % TAGS.len()], TAGS[(index / 3 + 2) % TAGS.len()]]
            };
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
        // Half of the hosts live in the group of their environment, so a
        // folder never shows "(office)" hosts inside Production; the split
        // also puts tagged and colored hosts in every group.
        let home = match environment {
            "prod" if (index / 4) % 2 == 1 => databases,
            "prod" => production,
            "staging" => staging,
            "office" => office,
            _ => lab,
        };
        let group = ((index / 3) % 2 == 0).then_some(home);
        if let Some(group) = group {
            data["group_id"] = json!(group);
            summary.hosts_in_groups += 1;
        }
        save(core, vault_id, RecordType::Host, data)?;
        summary.hosts += 1;
    }
    seed_keychain(core, vault_id, &mut summary)?;
    Ok(summary)
}

/// The demo keys (generated now, from the OS random source; none is ever
/// used for a connection) and two identities for the Keychain screen (UI
/// 7.2). Keys without a passphrase and no RSA, so the seeding stays fast.
fn seed_keychain(core: &Core, vault_id: Uuid, summary: &mut SeedSummary) -> Result<(), CoreError> {
    let mut deploy = None;
    for (label, kind) in DEMO_KEYS {
        let key = core
            .key_generate(KeyGenerateInput {
                vault_id,
                label: label.to_owned(),
                kind,
                passphrase: None,
                save_passphrase: false,
            })?
            .key;
        deploy.get_or_insert(key.record_id);
        summary.keys += 1;
    }
    for (label, username, key_id) in [("Operations", "ops", deploy), ("Backups", "backup", None)] {
        core.identity_save(IdentitySaveInput {
            vault_id,
            record_id: None,
            label: label.to_owned(),
            username: username.to_owned(),
            key_id,
            password: PasswordChange::Keep,
        })?;
        summary.identities += 1;
    }
    Ok(())
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
        assert_eq!(summary.hosts_in_groups, DEMO_HOSTS / 2);
        // The Keychain: the demo keys and identities.
        assert_eq!(summary.keys, DEMO_KEYS.len());
        assert_eq!(summary.identities, DEMO_IDENTITIES);
        let keys = opened.core.keys_list().unwrap();
        for (label, kind) in DEMO_KEYS {
            assert!(
                keys.iter()
                    .any(|key| key.label == label && key.algorithm == kind.algorithm_name()),
                "{label}"
            );
        }
        assert_eq!(
            opened.core.identities_list().unwrap().len(),
            DEMO_IDENTITIES
        );

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
        // A grouped host carries its group's environment in its label.
        let environment = |group: Uuid| match summary.groups.iter().position(|id| *id == group) {
            Some(0 | 1) => "(prod)",
            Some(2) => "(staging)",
            Some(3) => "(office)",
            _ => "(lab)",
        };
        for host in hosts.iter() {
            if let (Some(group), Some(label)) = (host.group_id, host.label.as_deref()) {
                assert!(label.ends_with(environment(group)), "{label}");
            }
        }
    }

    /// The whole `--demo` setup without the window: temporary profile, real
    /// first run (generated password), seeded data, deletion on drop.
    #[test]
    fn open_creates_an_unlocked_seeded_profile_that_is_removed() {
        let profile = create_profile().unwrap();
        let path = profile.path().to_path_buf();
        let (opened, summary) = open(profile).unwrap();
        let status = opened.core.lock_status();
        assert_eq!(status.state, LockState::Unlocked);
        // "Remember on this device" as the app's first run (D29), with the
        // device key in the in-memory store of `platform::temporary`.
        assert!(status.remember_device_enabled);
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

    /// The empty profiles of the first-run, unlock and empty-state
    /// screenshots: the requested lock stage, no records, removed on drop.
    #[test]
    fn fresh_profiles_open_at_the_requested_stage() {
        for (stage, expected) in [
            (FreshStage::FirstRun, LockState::FirstRun),
            (FreshStage::Unlocked, LockState::Unlocked),
            (FreshStage::Locked, LockState::Locked),
        ] {
            let profile = create_profile().unwrap();
            let path = profile.path().to_path_buf();
            let opened = open_fresh(profile, stage).unwrap();
            assert_eq!(opened.core.lock_status().state, expected, "{stage:?}");
            if stage == FreshStage::Unlocked {
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
                assert!(views.is_empty(), "a fresh profile has no records");
            }
            drop(opened.events);
            drop(opened.core);
            drop(opened.temp_profile);
            assert!(!path.exists());
        }
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
