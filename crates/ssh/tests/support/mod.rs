//! Shared helpers of the shellpp-ssh integration tests: a scripted
//! [`Interaction`], shell output readers, a test SSH agent on a temporary
//! socket or pipe, a TCP relay that can freeze a link, a fake server that
//! captures the client's KEXINIT, and the OpenSSH provider glue.
//!
//! Keys and passwords are generated at run time (CONTRIBUTING); servers
//! listen on 127.0.0.1 only.

#![allow(dead_code)] // each test file uses a subset

pub mod agent;
pub mod json;
pub mod lab;
pub mod net;
pub mod openssh;
pub mod proxy;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::SecretString;
use shellpp_ssh::{
    AuthPlan, BoxFuture, ConnectSpec, ConnectStep, HostKeyDecision, HostKeyInfo, Interaction,
    Prompt, PromptAnswer, SessionEnd, Shell, ShellEvent, ShellRequest,
};
use shellpp_test_sshd::{KiQuestion, ServerConfig, TestServer, UserConfig};

/// A random secret for test passwords and answers.
pub fn random_secret(prefix: &str) -> String {
    format!(
        "{prefix}-{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
}

pub fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

/// A user of the russh test server with no credentials yet.
pub fn user(name: &str) -> UserConfig {
    UserConfig {
        name: name.to_owned(),
        password: None,
        authorized_keys: Vec::new(),
        keyboard_interactive: Vec::new(),
        ki_name: String::new(),
        ki_instruction: String::new(),
    }
}

pub fn password_user(name: &str, password: &str) -> UserConfig {
    UserConfig {
        password: Some(secret(password)),
        ..user(name)
    }
}

pub fn key_user(name: &str, public_openssh: &str) -> UserConfig {
    UserConfig {
        authorized_keys: vec![public_openssh.to_owned()],
        ..user(name)
    }
}

pub fn question(prompt: &str, echo: bool, answer: &str) -> KiQuestion {
    KiQuestion {
        prompt: prompt.to_owned(),
        echo,
        answer: secret(answer),
    }
}

/// Starts the russh test server with these users.
pub async fn start_server(users: Vec<UserConfig>) -> TestServer {
    start_with(ServerConfig::new(users)).await
}

pub async fn start_with(config: ServerConfig) -> TestServer {
    TestServer::start(config).await.expect("test server starts")
}

/// A connect spec for the test server at `addr`.
pub fn spec(addr: SocketAddr, username: Option<&str>, auth: AuthPlan) -> ConnectSpec {
    ConnectSpec {
        address: addr.ip().to_string(),
        port: addr.port(),
        username: username.map(str::to_owned),
        auth,
        agent: None,
        keepalive: None,
        connect_timeout: Duration::from_secs(10),
        preferred_host_key_types: Vec::new(),
        jump_hosts: Vec::new(),
        proxy: None,
    }
}

pub fn password_plan(password: &str) -> AuthPlan {
    AuthPlan {
        password: Some(secret(password)),
        ..AuthPlan::default()
    }
}

pub fn shell_request() -> ShellRequest {
    ShellRequest {
        term: "xterm-256color".to_owned(),
        cols: 80,
        rows: 24,
        env: Vec::new(),
        agent_forwarding: false,
    }
}

/// A scripted prompt answer.
#[derive(Clone)]
pub enum Answer {
    Text(String, bool),
    Cancel,
}

pub fn text(value: &str) -> Answer {
    Answer::Text(value.to_owned(), false)
}

pub fn saved(value: &str) -> Answer {
    Answer::Text(value.to_owned(), true)
}

type VerifyHook = Box<dyn FnOnce() + Send>;

/// An [`Interaction`] that records everything and answers from a script.
/// Prompts beyond the script are cancelled.
pub struct TestInteraction {
    decisions: Mutex<VecDeque<HostKeyDecision>>,
    default_decision: HostKeyDecision,
    answers: Mutex<VecDeque<Answer>>,
    steps: Mutex<Vec<ConnectStep>>,
    prompts: Mutex<Vec<Prompt>>,
    host_keys: Mutex<Vec<HostKeyInfo>>,
    banners: Mutex<Vec<String>>,
    on_verify: Mutex<Option<VerifyHook>>,
}

impl TestInteraction {
    /// Accepts every host key.
    pub fn accepting() -> Arc<Self> {
        Arc::new(Self::with_decision(HostKeyDecision::Accept))
    }

    /// Rejects every host key.
    pub fn rejecting() -> Arc<Self> {
        Arc::new(Self::with_decision(HostKeyDecision::Reject))
    }

    fn with_decision(decision: HostKeyDecision) -> Self {
        Self {
            decisions: Mutex::new(VecDeque::new()),
            default_decision: decision,
            answers: Mutex::new(VecDeque::new()),
            steps: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
            host_keys: Mutex::new(Vec::new()),
            banners: Mutex::new(Vec::new()),
            on_verify: Mutex::new(None),
        }
    }

    /// Accepts host keys and answers prompts with `answers`, in order.
    pub fn answering(answers: Vec<Answer>) -> Arc<Self> {
        let interaction = Self::with_decision(HostKeyDecision::Accept);
        *interaction.answers.lock().unwrap() = answers.into();
        Arc::new(interaction)
    }

    /// Decisions for the next host key checks (then the default).
    pub fn queue_decisions(&self, decisions: &[HostKeyDecision]) {
        self.decisions
            .lock()
            .unwrap()
            .extend(decisions.iter().copied());
    }

    /// Runs `hook` during the first host key check, before deciding.
    pub fn on_first_verify(&self, hook: impl FnOnce() + Send + 'static) {
        *self.on_verify.lock().unwrap() = Some(Box::new(hook));
    }

    pub fn steps(&self) -> Vec<ConnectStep> {
        self.steps.lock().unwrap().clone()
    }

    pub fn prompts(&self) -> Vec<Prompt> {
        self.prompts.lock().unwrap().clone()
    }

    pub fn host_keys(&self) -> Vec<HostKeyInfo> {
        self.host_keys.lock().unwrap().clone()
    }

    pub fn banners(&self) -> Vec<String> {
        self.banners.lock().unwrap().clone()
    }

    pub fn as_dyn(self: &Arc<Self>) -> Arc<dyn Interaction> {
        Arc::clone(self) as Arc<dyn Interaction>
    }
}

impl Interaction for TestInteraction {
    fn step(&self, step: ConnectStep) {
        self.steps.lock().unwrap().push(step);
    }

    fn verify_host_key<'a>(&'a self, key: &'a HostKeyInfo) -> BoxFuture<'a, HostKeyDecision> {
        Box::pin(async move {
            self.host_keys.lock().unwrap().push(key.clone());
            let hook = self.on_verify.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
                // Let the closed connection reach the client first.
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let queued = self.decisions.lock().unwrap().pop_front();
            queued.unwrap_or(self.default_decision)
        })
    }

    fn prompt<'a>(&'a self, prompt: Prompt) -> BoxFuture<'a, Option<PromptAnswer>> {
        Box::pin(async move {
            self.prompts.lock().unwrap().push(prompt);
            match self.answers.lock().unwrap().pop_front() {
                Some(Answer::Text(value, save)) => Some(PromptAnswer {
                    value: secret(&value),
                    save,
                }),
                Some(Answer::Cancel) | None => None,
            }
        })
    }

    fn banner(&self, text: &str) {
        self.banners.lock().unwrap().push(text.to_owned());
    }
}

/// Output read so far and how reading stopped.
#[derive(Debug)]
pub struct Output {
    pub text: String,
    pub end: Option<SessionEnd>,
}

/// Reads shell output until `needle` appears; panics with the output on
/// timeout or when the shell ends first.
pub async fn read_until(shell: &mut Shell, needle: &str, timeout: Duration) -> String {
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if text.contains(needle) {
            return text;
        }
        match tokio::time::timeout_at(deadline, shell.next()).await {
            Ok(ShellEvent::Data(data)) => text.push_str(&String::from_utf8_lossy(&data)),
            Ok(ShellEvent::End(end)) => {
                panic!("shell ended ({end:?}) before {needle:?}; output: {text:?}")
            }
            Err(_) => panic!("timeout waiting for {needle:?}; output: {text:?}"),
        }
    }
}

/// Reads until the shell ends; returns the end and the output.
pub async fn read_to_end(shell: &mut Shell, timeout: Duration) -> Output {
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, shell.next()).await {
            Ok(ShellEvent::Data(data)) => text.push_str(&String::from_utf8_lossy(&data)),
            Ok(ShellEvent::End(end)) => {
                return Output {
                    text,
                    end: Some(end),
                };
            }
            Err(_) => return Output { text, end: None },
        }
    }
}

/// Polls `condition` until it holds or `timeout` passes.
pub async fn eventually(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub const SHORT: Duration = Duration::from_secs(10);
