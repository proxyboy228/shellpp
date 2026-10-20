//! Import of `~/.ssh/config` (SPEC "Import"; UI 2.5; night plan, stage 3
//! item 10; night decision D35): the preview and the import, over the
//! parser of `crate::import::ssh_config`.
//!
//! Mapping of each concrete alias (its effective options):
//! - `HostName` (`%h` = the alias) → address, else the alias; the alias is
//!   the label; `Port`, `User` → `ssh.port`, `ssh.username`;
//! - `IdentityFile` (`~`, `%d`) → the key files are imported into the
//!   Keychain (an encrypted one with the passphrase given in the preview)
//!   and the first one is linked as `ssh.key_id`;
//! - `ProxyJump` → `ssh.jump_chain` when every hop is a host of this
//!   config or of an earlier import (Host Chaining works from stage 5);
//!   otherwise no chain is set, the host's notes say so and the summary
//!   counts it, so a host never silently connects directly;
//! - `ForwardAgent` → `ssh.agent_forwarding`, `ServerAliveInterval` →
//!   `ssh.keepalive_s` (at most 3600), `SetEnv` → `ssh.env`;
//! - `ProxyCommand` → the host is not imported ("not supported");
//! - `SendEnv`, the forwardings (stage 5), `CertificateFile` and every
//!   other keyword are listed in the host's notes as not imported;
//! - commands (`ProxyCommand`, `LocalCommand`, `PermitLocalCommand`,
//!   `KnownHostsCommand`, `Match exec`) are never run or stored, and the
//!   summary counts them (security.md 13.5).
//!
//! Idempotent: a host carries `import_key` = `ssh_config:<alias>`
//! (docs/data-model.md, section 18.4); a host with that key is "already
//! exists" and never overwritten. Keys match by fingerprint.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::Core;
use super::dto::{
    ImportStatus, KeyFileStatus, KeyImportInput, SshConfigHostPreview, SshConfigImportInput,
    SshConfigImportSummary, SshConfigKeyPreview, SshConfigPreview, SshConfigSource,
};
use super::records::{Plan, parts};
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::import::ssh_config::{self, Config, Options, ProblemKind};
use crate::keys;
use crate::model::host::{self, HostRules};
use crate::model::types::RecordKind;

/// The largest key file read.
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;
/// Most `ProxyJump` hops kept (`ssh.jump_chain` holds at most 8).
const MAX_JUMP_HOPS: usize = 8;

/// Keywords the import maps, or knows to leave out without a note.
const MAPPED: &[&str] = &[
    "hostname",
    "port",
    "user",
    "identityfile",
    "proxyjump",
    "proxycommand",
    "forwardagent",
    "serveraliveinterval",
    "setenv",
    "sendenv",
    "localforward",
    "remoteforward",
    "dynamicforward",
    "certificatefile",
    // The app uses only the linked key anyway.
    "identitiesonly",
];

/// Keywords that hold or allow a local command: never run or stored, and
/// counted in the summary (security.md 13.5).
const COMMANDS: [&str; 4] = [
    "proxycommand",
    "localcommand",
    "permitlocalcommand",
    "knownhostscommand",
];

/// The lines of the file (Include'd files too) that hold a command, each
/// counted once: `PermitLocalCommand no` and `ProxyCommand none` hold none.
fn commands_in(config: &Config) -> usize {
    config
        .blocks
        .iter()
        .flat_map(|block| &block.options)
        .filter(|directive| COMMANDS.contains(&directive.keyword.as_str()))
        .filter(|directive| {
            directive.args.first().is_some_and(|value| {
                !value.eq_ignore_ascii_case("none") && !value.eq_ignore_ascii_case("no")
            })
        })
        .count()
}

/// The `notes` of an imported host: what the import left out.
fn host_notes(notes: &[String]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut text = String::from("Imported from an OpenSSH config.");
    for note in notes {
        let line = format!("\n- {note}");
        if text.len() + line.len() > host::MAX_NOTES_BYTES {
            break;
        }
        text.push_str(&line);
    }
    Some(text)
}

/// The import key of a host from `~/.ssh/config`.
pub fn import_key(alias: &str) -> String {
    format!("ssh_config:{alias}")
}

fn first<'a>(options: &'a Options, keyword: &str) -> Option<&'a str> {
    options
        .get(keyword)
        .and_then(|values| values.first())
        .and_then(|args| args.first())
        .map(String::as_str)
}

/// `~` and `%d` (the home folder), `%%`; `None` when another token is left.
fn expand_path(raw: &str, home: &Path) -> Option<PathBuf> {
    let home_text = home.to_string_lossy();
    let replaced = raw.replace("%d", &home_text).replace("%%", "\u{0}");
    if replaced.contains('%') {
        return None;
    }
    Some(ssh_config::expand_home(
        &replaced.replace('\u{0}', "%"),
        home,
    ))
}

/// What the import does with one alias.
struct HostPlan {
    alias: String,
    address: String,
    port: Option<u16>,
    username: Option<String>,
    /// `ssh` members other than `key_id` and `jump_chain`.
    ssh: Map<String, Value>,
    key_files: Vec<PathBuf>,
    /// The hop host names of `ProxyJump` (`[user@]host[:port]` → host).
    jump: Vec<String>,
    jump_raw: Vec<String>,
    notes: Vec<String>,
    error: Option<String>,
}

fn plan_host(alias: &str, options: &Options, home: &Path) -> HostPlan {
    let mut notes = Vec::new();
    let mut error = None;
    let mut ssh = Map::new();
    if let Some(command) = first(options, "proxycommand")
        && !command.eq_ignore_ascii_case("none")
    {
        error = Some("ProxyCommand is not supported".to_owned());
    }
    let address = match first(options, "hostname") {
        Some(name) => name.replace("%h", alias).replace("%%", "%"),
        None => alias.to_owned(),
    };
    if address.contains('%') && error.is_none() {
        error = Some("HostName uses a token other than %h".to_owned());
    }
    let port = match first(options, "port").map(str::parse::<u16>) {
        Some(Ok(port)) if port > 0 => {
            ssh.insert("port".into(), json!(port));
            Some(port)
        }
        Some(_) => {
            error.get_or_insert_with(|| "Port is not a valid port".to_owned());
            None
        }
        None => None,
    };
    let username = first(options, "user").map(str::to_owned);
    match &username {
        Some(user) if user.contains('%') => {
            notes.push("User uses a token and is not imported".into())
        }
        Some(user) => {
            ssh.insert("username".into(), json!(user));
        }
        None => {}
    }
    if first(options, "forwardagent").is_some_and(|value| {
        value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("true")
    }) {
        ssh.insert("agent_forwarding".into(), json!(true));
    }
    if let Some(raw) = first(options, "serveraliveinterval") {
        match raw.parse::<u32>() {
            Ok(seconds) => {
                if seconds > 3600 {
                    notes.push("ServerAliveInterval over 3600 s is imported as 3600".into());
                }
                ssh.insert("keepalive_s".into(), json!(seconds.min(3600)));
            }
            Err(_) => notes.push("ServerAliveInterval is not a number".into()),
        }
    }
    let mut env: Vec<Value> = Vec::new();
    let mut names = HashSet::new();
    for args in options.get("setenv").into_iter().flatten() {
        for pair in args {
            match pair.split_once('=') {
                Some((name, value)) if !name.is_empty() => {
                    if names.insert(name.to_owned()) {
                        env.push(json!({"name": name, "value": value}));
                    }
                }
                _ => notes.push("A SetEnv value without NAME=value is not imported".into()),
            }
        }
    }
    if !env.is_empty() {
        ssh.insert("env".into(), Value::Array(env));
    }
    if options.contains_key("sendenv") {
        notes.push("SendEnv is not imported: its values come from the local environment".into());
    }
    for keyword in ["localforward", "remoteforward", "dynamicforward"] {
        if options.contains_key(keyword) {
            notes.push("Port forwarding rules are imported with stage 5".into());
            break;
        }
    }
    if options.contains_key("certificatefile") {
        notes.push("CertificateFile is not imported yet".into());
    }
    let mut key_files = Vec::new();
    for args in options.get("identityfile").into_iter().flatten() {
        let Some(raw) = args.first() else { continue };
        if raw.eq_ignore_ascii_case("none") {
            continue;
        }
        match expand_path(raw, home) {
            Some(path) => {
                if !key_files.contains(&path) {
                    key_files.push(path);
                }
            }
            None => notes.push("An IdentityFile with tokens is not imported".into()),
        }
    }
    let mut jump = Vec::new();
    let mut jump_raw = Vec::new();
    if let Some(chain) = first(options, "proxyjump")
        && !chain.eq_ignore_ascii_case("none")
    {
        for hop in chain
            .split(',')
            .map(str::trim)
            .filter(|hop| !hop.is_empty())
        {
            jump_raw.push(hop.to_owned());
            let host = hop.rsplit_once('@').map_or(hop, |(_, host)| host);
            let host = match host.strip_prefix('[') {
                Some(rest) => rest.split(']').next().unwrap_or(rest),
                None => host.split(':').next().unwrap_or(host),
            };
            if host != hop {
                notes.push(format!(
                    "ProxyJump {hop}: the user and port of the hop are not kept; \
                     the jump host's own are used"
                ));
            }
            jump.push(host.to_owned());
        }
        if jump.len() > MAX_JUMP_HOPS {
            notes.push("ProxyJump has more than 8 hops; the chain is not imported".into());
            jump.clear();
        }
    }
    let others: BTreeSet<&str> = options
        .keys()
        .map(String::as_str)
        .filter(|keyword| !MAPPED.contains(keyword))
        .collect();
    if !others.is_empty() {
        notes.push(format!(
            "Not imported: {}",
            others.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    HostPlan {
        alias: alias.to_owned(),
        address,
        port,
        username,
        ssh,
        key_files,
        jump,
        jump_raw,
        notes,
        error,
    }
}

/// A readable line for a problem of the parser.
fn problem_text(problem: &ssh_config::Problem) -> String {
    let what = match &problem.kind {
        ProblemKind::Malformed => "a line that cannot be read".to_owned(),
        ProblemKind::IncludeMissing => "Include matches no readable file".to_owned(),
        ProblemKind::IncludeLoop => "Include loops or nests too deep".to_owned(),
        ProblemKind::IncludeNetwork => "Include of a network path is not followed".to_owned(),
        ProblemKind::TooLarge => "the file is too large".to_owned(),
        ProblemKind::MatchUnsupported(name) => {
            format!("Match {name} is not supported; the block is ignored")
        }
    };
    let file = problem
        .line
        .file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{file}:{}: {what}", problem.line.line)
}

/// Reads a key file within the size limit; a network path is never opened
/// ([`ssh_config::is_network_path`]).
fn read_key_file(path: &Path) -> Result<String, KeyFileStatus> {
    if ssh_config::is_network_path(path) {
        return Err(KeyFileStatus::Invalid(
            "a network path is not read".to_owned(),
        ));
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_KEY_FILE_BYTES => {
            Err(KeyFileStatus::Invalid("the file is too large".to_owned()))
        }
        Ok(_) => std::fs::read_to_string(path)
            .map_err(|_| KeyFileStatus::Invalid("the file cannot be read".to_owned())),
        Err(_) => Err(KeyFileStatus::Missing),
    }
}

impl Core {
    /// The config and the plans of its hosts; the main file must exist.
    fn ssh_config_plans(source: &SshConfigSource) -> Result<(Config, Vec<HostPlan>), CoreError> {
        if !source.path.is_file() {
            return Err(CoreError::Validation(vec![FieldError::new(
                "path",
                ValidationReason::Invalid,
            )]));
        }
        let config = ssh_config::read(&source.path, &source.home);
        let plans = config
            .hosts()
            .into_iter()
            .map(|(alias, _)| {
                let options = config.options_for(&alias);
                plan_host(&alias, &options, &source.home)
            })
            .collect();
        Ok((config, plans))
    }

    /// The live hosts of earlier `~/.ssh/config` imports: alias (lowercase) → id.
    fn imported_aliases(&self) -> Result<HashMap<String, Uuid>, CoreError> {
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        Ok(parts
            .session
            .index
            .of_kind(RecordKind::Host)
            .filter_map(|record| {
                record
                    .data()
                    .get("import_key")
                    .and_then(Value::as_str)
                    .and_then(|key| key.strip_prefix("ssh_config:"))
                    .map(|alias| (alias.to_lowercase(), record.record_id))
            })
            .collect())
    }

    pub(super) fn ssh_config_preview_impl(
        &self,
        source: SshConfigSource,
    ) -> Result<SshConfigPreview, CoreError> {
        let (config, plans) = Self::ssh_config_plans(&source)?;
        let imported = self.imported_aliases()?;
        let known: HashSet<String> = self
            .keys_list()?
            .into_iter()
            .filter_map(|key| key.fingerprint)
            .collect();
        let aliases: HashSet<String> = plans.iter().map(|plan| plan.alias.to_lowercase()).collect();
        let refused: HashMap<String, String> = plans
            .iter()
            .filter_map(|plan| {
                plan.error
                    .clone()
                    .map(|reason| (plan.alias.to_lowercase(), reason))
            })
            .collect();
        let mut keys: Vec<SshConfigKeyPreview> = Vec::new();
        let mut seen_keys = HashSet::new();
        let mut hosts = Vec::new();
        for plan in plans {
            let mut notes = plan.notes.clone();
            for hop in &plan.jump {
                let hop = hop.to_lowercase();
                if imported.contains_key(&hop) {
                    continue;
                }
                if !aliases.contains(&hop) {
                    notes.push(format!(
                        "ProxyJump {hop} is not a host of this config; the chain is not imported"
                    ));
                } else if let Some(reason) = refused.get(&hop) {
                    notes.push(format!(
                        "ProxyJump {hop} is not imported ({reason}); the chain is not imported"
                    ));
                }
            }
            let status = if plan.error.is_some() {
                ImportStatus::Error
            } else if imported.contains_key(&plan.alias.to_lowercase()) {
                ImportStatus::Exists
            } else {
                ImportStatus::New
            };
            for path in &plan.key_files {
                if !seen_keys.insert(path.clone()) {
                    continue;
                }
                let status = match read_key_file(path) {
                    Err(status) => status,
                    Ok(text) => match keys::inspect(&text) {
                        Err(problem) => KeyFileStatus::Invalid(problem.to_string()),
                        Ok((_, encrypted)) => {
                            let print = keys::prepare(&text, None)
                                .ok()
                                .and_then(|openssh| keys::parse(&openssh).ok())
                                .map(|parsed| parsed.fingerprint);
                            match print {
                                Some(print) if known.contains(&print) => KeyFileStatus::Exists,
                                _ if encrypted => KeyFileStatus::NeedsPassphrase,
                                _ => KeyFileStatus::New,
                            }
                        }
                    },
                };
                keys.push(SshConfigKeyPreview {
                    path: path.display().to_string(),
                    status,
                });
            }
            hosts.push(SshConfigHostPreview {
                alias: plan.alias,
                address: plan.address,
                port: plan.port,
                username: plan.username,
                status,
                reason: plan.error,
                notes,
                key_files: plan
                    .key_files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                jump: plan.jump_raw,
            });
        }
        let templates = config
            .blocks
            .iter()
            .filter(|block| {
                matches!(&block.condition, ssh_config::Condition::Host(patterns)
                    if !patterns.iter().any(|pattern| ssh_config::is_concrete(pattern)))
            })
            .count();
        Ok(SshConfigPreview {
            hosts,
            keys,
            problems: config.problems.iter().map(problem_text).collect(),
            templates,
        })
    }

    pub(super) fn ssh_config_import_impl(
        &self,
        input: SshConfigImportInput,
    ) -> Result<SshConfigImportSummary, CoreError> {
        let vault_id = {
            let inner = self.inner();
            Self::require_session(&inner)?.ring.vault_id()
        };
        let (config, plans) = Self::ssh_config_plans(&input.source)?;
        let mut imported = self.imported_aliases()?;
        let mut summary = SshConfigImportSummary {
            commands_skipped: commands_in(&config)
                + config
                    .problems
                    .iter()
                    .filter(|problem| {
                        matches!(&problem.kind, ProblemKind::MatchUnsupported(name)
                            if name.eq_ignore_ascii_case("exec"))
                    })
                    .count(),
            ..SshConfigImportSummary::default()
        };
        let chosen: Option<HashSet<String>> = input
            .aliases
            .as_ref()
            .map(|aliases| aliases.iter().map(|alias| alias.to_lowercase()).collect());
        let mut selected: Vec<HostPlan> = Vec::new();
        for plan in plans {
            let alias = plan.alias.to_lowercase();
            if chosen
                .as_ref()
                .is_some_and(|chosen| !chosen.contains(&alias))
            {
                continue;
            }
            if imported.contains_key(&alias) {
                summary.skipped += 1;
                continue;
            }
            if let Some(reason) = &plan.error {
                summary.errors.push((plan.alias.clone(), reason.clone()));
                continue;
            }
            selected.push(plan);
        }

        // Keys first (each import takes the core mutex itself; encrypted
        // keys are slow), by path.
        let mut key_ids: HashMap<PathBuf, Uuid> = HashMap::new();
        let mut tried: HashSet<PathBuf> = HashSet::new();
        for path in selected.iter().flat_map(|plan| plan.key_files.iter()) {
            if !tried.insert(path.clone()) {
                continue;
            }
            let Ok(text) = read_key_file(path) else {
                summary.keys_skipped += 1;
                continue;
            };
            let shown = path.display().to_string();
            let passphrase: Option<SecretString> = input.passphrases.get(&shown).cloned();
            let label = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            match self.key_import(KeyImportInput {
                vault_id,
                label,
                private_key: SecretString::from(text),
                passphrase,
                save_passphrase: input.save_passphrases,
            }) {
                Ok(result) => {
                    if result.created {
                        summary.keys_added += 1;
                    } else {
                        summary.keys_existing += 1;
                    }
                    key_ids.insert(path.clone(), result.key.record_id);
                }
                Err(_) => summary.keys_skipped += 1,
            }
        }

        // Hosts: ids first, so that jump chains can point at each other.
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let ids: Vec<Uuid> = selected.iter().map(|_| parts.ids.next(now)).collect();
        for (plan, id) in selected.iter().zip(&ids) {
            imported.insert(plan.alias.to_lowercase(), *id);
        }
        let mut plan_writes = Plan::default();
        let mut writes = 0usize;
        for (plan, id) in selected.into_iter().zip(ids) {
            let mut ssh = plan.ssh.clone();
            if let Some(key_id) = plan.key_files.iter().find_map(|path| key_ids.get(path)) {
                ssh.insert("key_id".into(), json!(key_id.to_string()));
            }
            let mut notes = plan.notes.clone();
            let chain: Option<Vec<String>> = plan
                .jump
                .iter()
                .map(|hop| imported.get(&hop.to_lowercase()).map(Uuid::to_string))
                .collect();
            match chain {
                Some(chain) if !chain.is_empty() => {
                    ssh.insert("jump_chain".into(), json!(chain));
                }
                Some(_) => {}
                None => {
                    // A hop that is neither imported now nor before: no
                    // chain, and the host says so instead of connecting
                    // directly without a word.
                    let missing: Vec<&str> = plan
                        .jump
                        .iter()
                        .filter(|hop| !imported.contains_key(&hop.to_lowercase()))
                        .map(String::as_str)
                        .collect();
                    notes.push(format!(
                        "ProxyJump {} was not imported, so no jump hosts are set",
                        missing.join(", ")
                    ));
                    summary.jump_chains_left_out += 1;
                }
            }
            let mut data = Map::new();
            data.insert("label".into(), json!(plan.alias));
            data.insert("address".into(), json!(plan.address));
            data.insert("ssh".into(), Value::Object(ssh));
            if let Some(notes) = host_notes(&notes) {
                data.insert("notes".into(), json!(notes));
            }
            match host::build(
                &data,
                None,
                &BTreeMap::new(),
                &HostRules {
                    record_id: Some(id),
                    personal_vault: true,
                },
            ) {
                Ok(mut built) => {
                    built.insert("import_key".into(), json!(import_key(&plan.alias)));
                    plan_writes.set(RecordKind::Host, id, built);
                    writes += 1;
                }
                Err(errors) => {
                    let fields: Vec<String> = errors.into_iter().map(|error| error.field).collect();
                    summary
                        .errors
                        .push((plan.alias, format!("invalid {}", fields.join(", "))));
                }
            }
        }
        if writes > 0 {
            let prepared = plan_writes.prepare(&mut parts, now)?;
            self.commit_records(&mut parts, prepared, Vec::new(), Vec::new())?;
        }
        summary.added = writes;
        tracing::info!(
            added = summary.added,
            skipped = summary.skipped,
            errors = summary.errors.len(),
            keys_added = summary.keys_added,
            "ssh config imported"
        );
        Ok(summary)
    }
}
