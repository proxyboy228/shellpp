//! Connections without a tab (stage 5, the core of the forwarding rules and
//! of snippets on many hosts; their screens come with the interface):
//! a saved host's route and credentials as a session would use them, but
//! nobody to ask.
//!
//! - A host key is accepted only when Known Hosts already holds it for that
//!   address and port; an unknown, changed or revoked key fails the attempt
//!   (the TOFU and warning dialogs are the interface's).
//! - Prompts are cancelled: only saved credentials, the agent and the key
//!   files (with passphrases remembered for this app session) log in.
//! - Nothing is saved; agent forwarding is never requested.
//!
//! [`ForwardingService`] keeps each host's rules on one connection
//! (`shellpp_ssh::HostForwarder`) and starts the rules marked for
//! autostart when a session to their host connects and after unlock.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shellpp_core::service::dto::{RecordRef, SessionTarget};
use shellpp_core::service::host_api::{HostKeyCheck, HostKeyStatus, SessionConfig};
use shellpp_core::service::snippets::{ForwardType, ForwardingRuleView};
use shellpp_ssh::{
    AuthPlan, BoxFuture, ConnectSpec, ConnectStep, Connection, FailureKind, ForwardKind,
    ForwardSpec, HostForwarder, HostKeyDecision, HostKeyInfo, Interaction, JumpHost, KeyCredential,
    Prompt, PromptAnswer, ProxyKind, ProxySpec, RuleStatus, SshError,
};
use uuid::Uuid;

use super::{CONNECT_TIMEOUT, Shared};

/// Host key decisions from Known Hosts alone; prompts cancelled.
pub(crate) struct KnownOnly {
    shared: Arc<Shared>,
    address: String,
    port: u16,
}

impl KnownOnly {
    fn new(shared: &Arc<Shared>, address: &str, port: u16) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::clone(shared),
            address: address.to_owned(),
            port,
        })
    }
}

impl Interaction for KnownOnly {
    fn step(&self, _step: ConnectStep) {}

    fn verify_host_key<'a>(&'a self, key: &'a HostKeyInfo) -> BoxFuture<'a, HostKeyDecision> {
        Box::pin(async move {
            let check = HostKeyCheck {
                address: self.address.clone(),
                port: self.port,
                key_type: key.key_type.clone(),
                public_key: key.public_key_base64.clone(),
            };
            match self
                .shared
                .blocking(move |core| core.known_host_check(&check))
                .await
            {
                Some(Ok(HostKeyStatus::Match { .. })) => HostKeyDecision::Accept,
                _ => {
                    tracing::info!("background connection: host key not in Known Hosts");
                    HostKeyDecision::Reject
                }
            }
        })
    }

    fn prompt<'a>(&'a self, _prompt: Prompt) -> BoxFuture<'a, Option<PromptAnswer>> {
        Box::pin(async { None })
    }
}

/// The connect spec of one host of a route from its configuration (saved
/// credentials only, nothing saved).
fn spec_of(config: &SessionConfig) -> ConnectSpec {
    ConnectSpec {
        address: config.address.clone(),
        port: config.port,
        username: config.username.clone(),
        auth: AuthPlan {
            key: config.key.as_ref().map(|key| KeyCredential {
                label: key.label.clone(),
                private_key: key.private_key.clone(),
                passphrase: key.passphrase.clone(),
            }),
            use_agent: false,
            password: config.password.clone(),
            keyboard_interactive: false,
            can_save_password: false,
            can_save_passphrase: false,
            file_keys: Vec::new(),
        },
        agent: None,
        keepalive: (config.keepalive_s > 0)
            .then(|| Duration::from_secs(u64::from(config.keepalive_s))),
        connect_timeout: CONNECT_TIMEOUT,
        preferred_host_key_types: config.known_key_types.clone(),
        allow_legacy_algorithms: config.allow_legacy_algorithms,
        jump_hosts: Vec::new(),
        proxy: None,
        forward_agent: None,
    }
}

/// Opens a connection to the saved host `host` without a tab (module
/// documentation): its route, its proxy, its saved credentials, the agent
/// and the key files.
pub(crate) async fn connect(shared: &Arc<Shared>, host: RecordRef) -> Result<Connection, SshError> {
    // A reconnect while the app is locked waits for the unlock
    // (docs/security.md, section 8): the core cannot resolve the host until
    // then, and the attempt must not end the rule.
    let mut unlocked = shared.unlocked();
    if unlocked.wait_for(|unlocked| *unlocked).await.is_err() {
        return Err(SshError::new(FailureKind::Cancelled));
    }
    let target = SessionTarget::Host {
        vault_id: host.vault_id,
        record_id: host.record_id,
    };
    let config = match shared
        .blocking(move |core| core.session_config(&target))
        .await
    {
        Some(Ok(config)) => config,
        // A host that is gone or a locked core: nothing to connect.
        _ => return Err(SshError::new(FailureKind::Cancelled)),
    };
    if config.route_problem.is_some() {
        return Err(SshError::new(FailureKind::Cancelled));
    }
    let mut spec = spec_of(&config);
    spec.proxy = config.proxy.as_ref().map(|proxy| ProxySpec {
        kind: match proxy.kind {
            shellpp_core::service::host_api::ProxyType::Http => ProxyKind::Http,
            shellpp_core::service::host_api::ProxyType::Socks5 => ProxyKind::Socks5,
        },
        address: proxy.address.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
    });
    spec.jump_hosts = config
        .jump_hosts
        .iter()
        .map(|hop| JumpHost {
            spec: spec_of(hop),
            interaction: KnownOnly::new(shared, &hop.address, hop.port),
        })
        .collect();
    super::ssh::add_key_sources(shared, &mut spec, None).await;
    for hop in &mut spec.jump_hosts {
        super::ssh::add_key_sources(shared, &mut hop.spec, None).await;
    }
    let interaction = KnownOnly::new(shared, &config.address, config.port);
    shellpp_ssh::connect(spec, interaction).await
}

/// A forwarding rule record as the SSH layer's rule.
pub(crate) fn rule_spec(rule: &ForwardingRuleView) -> Option<ForwardSpec> {
    let kind = match rule.kind {
        ForwardType::Local => ForwardKind::Local {
            host: rule.dest_address.clone()?,
            port: rule.dest_port?,
        },
        ForwardType::Remote => ForwardKind::Remote {
            host: rule.dest_address.clone()?,
            port: rule.dest_port?,
        },
        ForwardType::Dynamic => ForwardKind::Dynamic,
    };
    Some(ForwardSpec {
        kind,
        bind_address: rule.bind_address.clone(),
        bind_port: rule.bind_port,
    })
}

/// The forwarding rules that run, each host's on one shared connection.
#[derive(Default)]
pub(crate) struct ForwardingService {
    /// One forwarder per host.
    hosts: Mutex<HashMap<Uuid, Arc<HostForwarder>>>,
    /// The forwarder's id of each rule record, and the rule's host.
    rules: Mutex<BTreeMap<Uuid, (u64, Uuid)>>,
    next: Mutex<u64>,
}

impl ForwardingService {
    fn forwarder(&self, shared: &Arc<Shared>, host: RecordRef) -> Arc<HostForwarder> {
        let mut hosts = self
            .hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(hosts.entry(host.record_id).or_insert_with(|| {
            let shared = Arc::clone(shared);
            Arc::new(HostForwarder::new(Arc::new(move || {
                let shared = Arc::clone(&shared);
                Box::pin(async move { connect(&shared, host).await.map(Arc::new) })
            })))
        }))
    }

    fn rule_id(&self, rule: Uuid, host: Uuid) -> u64 {
        let mut rules = self
            .rules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((id, _)) = rules.get(&rule) {
            return *id;
        }
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *next += 1;
        rules.insert(rule, (*next, host));
        *next
    }

    /// Starts `rule` on its host's connection (connecting first when there
    /// is none).
    pub(crate) async fn start(
        &self,
        shared: &Arc<Shared>,
        rule: &ForwardingRuleView,
    ) -> RuleStatus {
        let Some(spec) = rule_spec(rule) else {
            return RuleStatus::Failed(shellpp_ssh::ForwardError::AddressUnavailable);
        };
        let host = RecordRef {
            vault_id: rule.record.vault_id,
            record_id: rule.host_id,
        };
        // An edited rule may name another host now: off the old one first.
        let moved = self
            .rules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&rule.record.record_id)
            .is_some_and(|(_, old_host)| *old_host != rule.host_id);
        if moved {
            self.stop(rule.record.record_id).await;
            self.rules
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&rule.record.record_id);
        }
        let id = self.rule_id(rule.record.record_id, rule.host_id);
        self.forwarder(shared, host).start(id, spec).await
    }

    /// Stops `rule` (its host's connection closes with its last rule).
    pub(crate) async fn stop(&self, rule: Uuid) {
        let found = self
            .rules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&rule)
            .copied();
        let Some((id, host)) = found else {
            return;
        };
        let forwarder = self
            .hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&host)
            .cloned();
        if let Some(forwarder) = forwarder {
            forwarder.stop(id).await;
        }
    }

    /// The status of `rule`, while it was started.
    pub(crate) fn status(&self, rule: Uuid) -> Option<RuleStatus> {
        let (id, host) = self
            .rules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&rule)
            .copied()?;
        self.hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&host)?
            .status(id)
    }

    /// Stops every rule (the app locks or exits).
    pub(crate) async fn stop_all(&self) {
        let forwarders: Vec<Arc<HostForwarder>> = self
            .hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .map(|(_, forwarder)| forwarder)
            .collect();
        for forwarder in forwarders {
            forwarder.shutdown().await;
        }
        self.rules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Starts the rules of the personal vault that `pick` selects and that
    /// are not running yet (autostart, data-model 11: team vaults ignore the
    /// flags).
    pub(crate) async fn autostart(
        &self,
        shared: &Arc<Shared>,
        pick: impl Fn(&ForwardingRuleView) -> bool,
    ) {
        let rules = match shared.blocking(|core| core.forwarding_rules()).await {
            Some(Ok(rules)) => rules,
            _ => return,
        };
        for rule in rules.iter().filter(|rule| pick(rule)) {
            if matches!(
                self.status(rule.record.record_id),
                Some(RuleStatus::Running(_))
            ) {
                continue;
            }
            let status = self.start(shared, rule).await;
            tracing::info!(
                running = matches!(status, RuleStatus::Running(_)),
                "forwarding rule autostarted"
            );
        }
    }
}
