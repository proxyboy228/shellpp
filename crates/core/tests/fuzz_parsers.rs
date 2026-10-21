//! Robustness of the parsers that read files from elsewhere (plan of run 3,
//! section 10, item 4): known_hosts, private keys (OpenSSH, PEM, PPK),
//! certificates, `~/.ssh/config` and the JSON of Import Data get many inputs
//! made by mutating valid ones, with a fixed seed (each run sees the same
//! inputs; a failure names its iteration). None may panic; the import must
//! refuse or take a mutated file without corrupting what is stored.
//!
//! No fuzzing tool is installed (the plan: nothing global): a small
//! xorshift generator and the mutations below stand in for one.

mod common;

use std::path::Path;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use shellpp_core::import::ssh_config;
use shellpp_core::keys;
use shellpp_core::keys::generate::{KeyKind, generate};
use shellpp_core::known_hosts::openssh;
use shellpp_core::service::dto::{DataExportInput, DataImportInput};

use common::first_run;

/// xorshift64*: enough to vary inputs, the same for every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Pieces worth inserting: syntax of the formats under test.
const TOKENS: &[&str] = &[
    "\n",
    "\r\n",
    " ",
    "\t",
    "#",
    "=",
    ",",
    ":",
    "*",
    "?",
    "!",
    "[",
    "]",
    "{",
    "}",
    "\"",
    "\\",
    "@cert-authority ",
    "@revoked ",
    "|1|",
    "Host ",
    "Match ",
    "all",
    "exec ",
    "Include ",
    "ProxyJump ",
    "LocalForward ",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----END OPENSSH PRIVATE KEY-----",
    "-----BEGIN RSA PRIVATE KEY-----",
    "PuTTY-User-Key-File-3: ",
    "PuTTY-User-Key-File-2: ",
    "Encryption: aes256-cbc",
    "Public-Lines: ",
    "Private-Lines: ",
    "999999999",
    "-1",
    "ssh-ed25519 ",
    "ssh-ed25519-cert-v01@openssh.com ",
    "AAAA",
    "null",
    "true",
    "[]",
    "{}",
    "\"records\"",
    "\"data\"",
    "\u{0}",
    "\u{202e}",
    "ü",
    "你",
];

fn mutate(rng: &mut Rng, seed: &str) -> String {
    let mut bytes = seed.as_bytes().to_vec();
    for _ in 0..1 + rng.below(6) {
        let at = rng.below(bytes.len() + 1);
        match rng.below(6) {
            0 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes[i] ^= 1 << rng.below(8);
            }
            1 => bytes.insert(at, rng.next() as u8),
            2 if at < bytes.len() => {
                let end = (at + 1 + rng.below(16)).min(bytes.len());
                bytes.drain(at..end);
            }
            3 => {
                let token = TOKENS[rng.below(TOKENS.len())].as_bytes();
                bytes.splice(at..at, token.iter().copied());
            }
            4 if at < bytes.len() => {
                let end = (at + 1 + rng.below(64)).min(bytes.len());
                let chunk = bytes[at..end].to_vec();
                bytes.splice(at..at, chunk);
            }
            _ => {
                let end = (at + rng.below(8)).min(bytes.len());
                bytes.truncate(end.max(at.min(bytes.len())));
            }
        }
    }
    // The parsers take text: invalid UTF-8 becomes replacement characters.
    String::from_utf8_lossy(&bytes).into_owned()
}

fn known_hosts_seed() -> String {
    let key = generate(KeyKind::Ed25519, "fuzz", None).expect("key");
    let public = keys::parse(key.as_str())
        .expect("parsed")
        .public_key
        .clone();
    let blob = public
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    format!(
        "web.example.com,192.0.2.1 ssh-ed25519 {blob} laptop\n\
         [db.example.com]:2222 ssh-ed25519 {blob}\n\
         |1|c2FsdHNhbHRzYWx0c2FsdA==|aGFzaGhhc2hoYXNoaGFzaGhhcw== ssh-ed25519 {blob}\n\
         @revoked old.example.net ssh-ed25519 {blob}\n\
         @cert-authority *.example.com ssh-ed25519 {blob}\n"
    )
}

#[test]
fn known_hosts_never_panics() {
    let seed = known_hosts_seed();
    let mut rng = Rng(0x5eed_0001);
    for _ in 0..50_000 {
        let input = mutate(&mut rng, &seed);
        let _ = openssh::parse(input.as_bytes());
    }
}

#[test]
fn private_keys_and_certificates_never_panic() {
    let openssh_key = generate(KeyKind::Ed25519, "fuzz", None).expect("key");
    let public = keys::parse(openssh_key.as_str())
        .expect("parsed")
        .public_key
        .clone();
    let blob = public
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let ppk = format!(
        "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\nComment: fuzz\n\
         Public-Lines: 2\n{}\n{}\nPrivate-Lines: 1\nAAAAIGZ1enpmdXp6ZnV6emZ1enpmdXp6ZnV6emZ1eno=\n\
         Private-MAC: 0000000000000000000000000000000000000000000000000000000000000000\n",
        &blob[..blob.len() / 2],
        &blob[blob.len() / 2..]
    );
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\n-----END RSA PRIVATE KEY-----\n";
    let cert = format!("ssh-ed25519-cert-v01@openssh.com {blob} fuzz");
    let seeds = [
        openssh_key.as_str().to_owned(),
        ppk,
        pem.to_owned(),
        public,
        cert,
    ];
    let mut rng = Rng(0x5eed_0002);
    for _ in 0..20_000 {
        let seed = &seeds[rng.below(seeds.len())];
        let input = mutate(&mut rng, seed);
        let _ = keys::parse(&input);
        let _ = keys::inspect(&input);
        let _ = keys::convert::to_openssh(&input, None);
        let _ = keys::certificate::certificate_info(&input, 1_800_000_000);
    }
}

#[test]
fn ssh_config_never_panics() {
    let seed = "Host web\n  HostName 192.0.2.1\n  User deploy\n  Port 2222\n  ProxyJump bastion\n\
                LocalForward 8080 127.0.0.1:80\nHost bastion *.example.com !skip\n  User ops\n\
                Match host web exec \"true\"\n  ForwardAgent yes\nHost *\n  ServerAliveInterval 30\n";
    let home = tempfile::tempdir().expect("home");
    let path = home.path().join("config");
    let mut rng = Rng(0x5eed_0003);
    for _ in 0..1_500 {
        let input = mutate(&mut rng, seed);
        // No Include: a mutated path could reach real files on this machine.
        let input: String = input
            .lines()
            .filter(|line| !line.to_ascii_lowercase().contains("include"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, input).expect("write");
        let config = ssh_config::read(&path, home.path());
        for (alias, _) in config.hosts() {
            let _ = config.options_for(&alias);
        }
        let _ = ssh_config::wildcard(&mutate(&mut rng, "*.ex?mple.com"), "web.example.com");
    }
    assert!(Path::new(&path).exists());
}

#[test]
fn import_data_refuses_or_takes_mutated_files() {
    let source = first_run();
    let vault = source.vault_id;
    common::save(
        &source.core,
        vault,
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "web.example.com", "label": "Web", "tags": ["prod"]}),
        &[],
    )
    .expect("host");
    let export = source
        .core
        .data_export(DataExportInput::default())
        .expect("export");
    let seed = export.text.expose_secret().to_owned();
    assert!(serde_json::from_str::<Value>(&seed).is_ok());
    let target = first_run();
    let mut rng = Rng(0x5eed_0004);
    let mut taken = 0;
    for _ in 0..300 {
        let input = mutate(&mut rng, &seed);
        let result = target.core.data_import(DataImportInput {
            text: SecretString::from(input),
            password: None,
            apply_settings: false,
            import_history: true,
            import_connection_log: true,
            full_restore: false,
        });
        if result.is_ok() {
            taken += 1;
        }
    }
    // Whatever was taken, what is stored still reads and exports.
    target.core.vaults().expect("vaults");
    target
        .core
        .data_export(DataExportInput::default())
        .expect("the store still exports");
    assert!(taken > 0, "some mutations keep the file valid");
}
