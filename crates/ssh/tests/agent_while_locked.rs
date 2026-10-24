//! D194: the system agent's keys reach a forwarded agent channel only while
//! the app allows it ([`AgentKeys::system_agent_allowed`]: not while locked,
//! unless the user chose so). A russh server written here asks the
//! forwarded agent for its identities and for a signature by a key it knows
//! from before (as a server that listed the keys while the app was unlocked
//! would), each time the client types into the shell.

mod support;

use std::sync::Arc;
use std::sync::Mutex;

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId, Pty};
use secrecy::ExposeSecret;
use shellpp_ssh::agent::PrivateKeyArc;
use shellpp_ssh::{AgentKeys, ForwardedAgent, ShellRequest, SystemAgent, connect};
use shellpp_test_sshd::{KeyType, generate_key};
use support::agent::TestAgent;
use support::*;
use tokio::sync::mpsc;

/// What one probe of the server found: identities listed, signature made.
type Probe = (usize, bool);

/// No Keychain keys; the system agent behind a switch (the app's lock): it
/// allows this many more requests ([`OPEN`]: all of them, 0: none).
struct Gate {
    budget: Mutex<usize>,
}

const OPEN: usize = usize::MAX;

impl AgentKeys for Gate {
    fn keys(&self) -> Vec<Arc<PrivateKeyArc>> {
        Vec::new()
    }

    fn system_agent_allowed(&self) -> bool {
        let mut left = self.budget.lock().unwrap();
        match *left {
            0 => false,
            OPEN => true,
            _ => {
                *left -= 1;
                true
            }
        }
    }
}

/// Probes the forwarded agent whenever the client sends data.
struct Prober {
    known: PublicKey,
    probes: mpsc::UnboundedSender<Probe>,
    /// The shell channel: data on agent channels (the answers) is no cue.
    shell: Option<ChannelId>,
}

impl russh::server::Handler for Prober {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        mut channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        tokio::spawn(async move { while channel.wait().await.is_some() {} });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn agent_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shell = Some(channel);
        session.channel_success(channel)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.shell != Some(channel) {
            return Ok(());
        }
        let handle = session.handle();
        let known = AgentIdentity::PublicKey {
            key: self.known.clone(),
            comment: String::new(),
        };
        let probes = self.probes.clone();
        tokio::spawn(async move {
            let Ok(channel) = handle.channel_open_agent().await else {
                return;
            };
            let mut client = AgentClient::connect(channel.into_stream());
            let listed = client.request_identities().await.map_or(0, |ids| ids.len());
            let signed = client
                .sign_request(&known, None, b"shellpp test data".to_vec())
                .await
                .is_ok();
            let _ = probes.send((listed, signed));
        });
        Ok(())
    }
}

async fn start_prober(known: PublicKey) -> (std::net::SocketAddr, mpsc::UnboundedReceiver<Probe>) {
    let generated = generate_key(KeyType::Ed25519, "prober-host", None).expect("host key");
    let host_key = russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
        .expect("decodes");
    let config = Arc::new(russh::server::Config {
        keys: vec![host_key],
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = Prober {
                known: known.clone(),
                probes: tx.clone(),
                shell: None,
            };
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, stream, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    (addr, rx)
}

/// Sets the gate, makes the server probe once and returns what it found.
async fn probe(
    gate: &Gate,
    shell: &mut shellpp_ssh::Shell,
    probes: &mut mpsc::UnboundedReceiver<Probe>,
    budget: usize,
) -> Probe {
    *gate.budget.lock().unwrap() = budget;
    shell.writer().write(b"x").await.expect("write");
    tokio::time::timeout(SHORT, probes.recv())
        .await
        .expect("the server probed in time")
        .expect("probe")
}

/// A test agent with one key, the prober in front of a connection that
/// forwards it behind `gate`, and the shell that cues the prober.
struct Setup {
    agent: TestAgent,
    connection: shellpp_ssh::Connection,
    shell: shellpp_ssh::Shell,
    probes: mpsc::UnboundedReceiver<Probe>,
    gate: Arc<Gate>,
}

async fn setup() -> Setup {
    let generated = generate_key(KeyType::Ed25519, "agent", None).expect("key");
    let key: PrivateKey =
        russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
            .expect("decodes");
    let known = key.public_key().clone();
    let agent = TestAgent::start(vec![key]).await;
    let (addr, probes) = start_prober(known).await;
    let gate = Arc::new(Gate {
        budget: Mutex::new(0),
    });
    let mut spec = spec(addr, Some("mallory"), password_plan("anything"));
    spec.forward_agent = Some(ForwardedAgent {
        keys: gate.clone(),
        system: Some(SystemAgent::at(agent.location())),
        confirm: None,
    });
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let shell = connection
        .open_shell(ShellRequest {
            agent_forwarding: true,
            ..shell_request()
        })
        .await
        .expect("shell");
    Setup {
        agent,
        connection,
        shell,
        probes,
        gate,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_system_agent_is_not_forwarded_while_the_app_refuses_it() {
    let Setup {
        agent,
        connection,
        mut shell,
        mut probes,
        gate,
    } = setup().await;
    // Locked: nothing listed, the known key signs nothing, and the system
    // agent is never asked.
    assert_eq!(probe(&gate, &mut shell, &mut probes, 0).await, (0, false));
    assert_eq!(agent.requests(), 0);
    // Unlocked (or allowed while locked): listed and signed.
    assert_eq!(probe(&gate, &mut shell, &mut probes, OPEN).await, (1, true));
    assert_eq!(agent.signatures(), 1);
    // Locked again: refused at once.
    let asked = agent.requests();
    assert_eq!(probe(&gate, &mut shell, &mut probes, 0).await, (0, false));
    assert_eq!(agent.requests(), asked);
    assert_eq!(agent.signatures(), 1);
    connection.disconnect().await;
}

/// A lock that lands while a signature is under way: the gate is asked for
/// the listing (1), before the signature (2), before it goes to the system
/// agent (3) and once its answer is back (4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_during_a_signature_keeps_it_from_the_host() {
    let Setup {
        agent,
        connection,
        mut shell,
        mut probes,
        gate,
    } = setup().await;
    // Locked after the check of the request (a confirmation, say): the
    // system agent is not asked to sign.
    assert_eq!(probe(&gate, &mut shell, &mut probes, 2).await, (1, false));
    assert_eq!(agent.signatures(), 0);
    // Locked while the system agent signed: the host does not get it.
    assert_eq!(probe(&gate, &mut shell, &mut probes, 3).await, (1, false));
    assert_eq!(agent.signatures(), 1);
    connection.disconnect().await;
}
