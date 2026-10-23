//! Robustness of the PuTTY sessions and CSV imports (run 8, plan C6; D140,
//! D141), as crates/core/tests/fuzz_parsers.rs does for the other parsers:
//! many inputs made by mutating valid ones, with a fixed seed (each run sees
//! the same inputs; a failure names its iteration). None may panic, and an
//! import that took a mutated input adds nothing when it takes it again
//! (both imports are idempotent and never overwrite a host).
//!
//! No fuzzing tool is installed (the plan: nothing global): a small
//! xorshift generator stands in for one.

mod common;

use std::collections::BTreeMap;

use common::first_run;
use shellpp_core::import::csv;
use shellpp_core::import::putty::{self, FixedSessions, Session, Sessions, Value};
use shellpp_core::service::dto::{CsvInput, PuttyImportInput};

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

/// Pieces of the two formats worth inserting.
const TOKENS: &[&[u8]] = &[
    b",",
    b";",
    b"\t",
    b"|",
    b"\"",
    b"\"\"",
    b"\n",
    b"\r\n",
    b" / ",
    b"=",
    b":",
    b"@",
    b"[",
    b"]",
    b"%",
    b"%2",
    b"%E9",
    b"*",
    b"L",
    b"R",
    b"D",
    b"4",
    b"6",
    b"\xEF\xBB\xBF",
    b"\xFF\xFE",
    b"\xC0\xE1",
    b"\xE9",
    b"\x00",
    "\u{202e}".as_bytes(),
    "привет".as_bytes(),
    b"65536",
    b"-1",
    b"4294967295",
];

fn mutate(rng: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
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
                let token = TOKENS[rng.below(TOKENS.len())];
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
    bytes
}

fn mutate_text(rng: &mut Rng, seed: &str) -> String {
    String::from_utf8_lossy(&mutate(rng, seed.as_bytes())).into_owned()
}

const CSV_SEED: &str = "label,address,port,username,group,tags,notes,password\n\
\"web, one\",web1.example.com,22,deploy,prod / web,\"a,b\",\"two\nlines\",pw-one\n\
db,10.0.0.5,2222,root,prod,db;x,,\n\
\"quoted \"\"q\"\"\",[2001:db8::1],22,,,,,\n\
cache,cache.example.com,,ops,prod / web / cache,,note,\n";

#[test]
fn csv_parsers_never_panic() {
    let mut rng = Rng(0x0c57_c6c6_2026_1023);
    for _ in 0..3000 {
        let bytes = mutate(&mut rng, CSV_SEED.as_bytes());
        let encoding = csv::detect_encoding(&bytes);
        let Some(text) = csv::decode(&bytes, encoding) else {
            continue;
        };
        let delimiter = csv::detect_delimiter(&text);
        let records = csv::records(&text, delimiter);
        let _ = &records;
        for other in [',', ';', '\t', '|'] {
            let _ = csv::records(&text, other);
        }
        let first: Vec<String> = text
            .lines()
            .next()
            .unwrap_or_default()
            .split(delimiter)
            .map(str::to_owned)
            .collect();
        let _ = csv::guess_roles(&first);
    }
}

#[test]
fn csv_imports_never_panic_and_take_a_file_once() {
    let u = first_run();
    let mut rng = Rng(0x0c57_1a7e_2026_1023);
    let mut taken = 0;
    for iteration in 0..250 {
        let bytes = mutate(&mut rng, CSV_SEED.as_bytes());
        let input = || CsvInput {
            bytes: zeroize::Zeroizing::new(bytes.clone()),
            encoding: None,
            delimiter: None,
            has_header: None,
            roles: None,
        };
        let _ = u.core.csv_inspect(&input());
        let _ = u.core.csv_preview(&input());
        if let Ok(first) = u.core.csv_import(&input()) {
            taken += usize::from(first.added > 0);
            let again = u.core.csv_import(&input()).expect("the same file again");
            assert_eq!(again.added, 0, "iteration {iteration}: {again:?}");
            assert_eq!(again.groups_added, 0, "iteration {iteration}: {again:?}");
        }
    }
    // Not a test of refusals only: many mutated files still add hosts.
    assert!(taken > 25, "{taken} imports added hosts");
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Sessions as PuTTY saves them, every mapped value used.
fn putty_seed() -> Vec<Session> {
    let session = |key: &str, values: Vec<(&str, Value)>| Session {
        key: key.to_owned(),
        values: values
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    };
    vec![
        session(
            "Default%20Settings",
            vec![("PortNumber", Value::Number(22))],
        ),
        session(
            "web%20one",
            vec![
                ("HostName", text("deploy@web1.example.com:2222")),
                ("PortNumber", Value::Number(22)),
                ("UserName", text("ops")),
                ("Protocol", text("ssh")),
                (
                    "PortForwardings",
                    text("L8080=localhost:80,R2222=127.0.0.1:22,D1080,4L127.0.0.1:9000=db:5432"),
                ),
                ("AgentFwd", Value::Number(1)),
                ("PingInterval", Value::Number(1)),
                ("PingIntervalSecs", Value::Number(30)),
                ("Environment", text("LANG=C,TZ=UTC")),
                ("BackspaceIsDelete", Value::Number(0)),
                (
                    "LineCodePage",
                    text("ISO-8859-1:1998 (Latin-1, West Europe)"),
                ),
            ],
        ),
        session(
            "proxied%E9",
            vec![
                ("HostName", text("db.internal")),
                ("ProxyMethod", Value::Number(2)),
                ("ProxyHost", text("proxy.example.com")),
                ("ProxyPort", Value::Number(1080)),
                ("ProxyUsername", text("p")),
                ("ProxyPassword", text("pw-proxy")),
                ("ProxyExcludeList", text("*.local, 10.*, ::1")),
                ("ProxyLocalhost", Value::Number(0)),
            ],
        ),
        session(
            "router",
            vec![
                ("HostName", text("192.0.2.1")),
                ("Protocol", text("telnet")),
                ("PortNumber", Value::Number(23)),
            ],
        ),
    ]
}

fn mutate_session(rng: &mut Rng, session: &Session) -> Session {
    let mut session = session.clone();
    if rng.below(4) == 0 {
        session.key = mutate_text(rng, &session.key);
    }
    for value in session.values.values_mut() {
        if rng.below(3) != 0 {
            continue;
        }
        *value = match value {
            Value::Text(old) => Value::Text(mutate_text(rng, old)),
            Value::Number(_) => Value::Number(rng.next() as u32),
        };
    }
    session
}

#[test]
fn putty_parsers_never_panic() {
    let mut rng = Rng(0x9077_7c6c_2026_1023);
    for _ in 0..3000 {
        let forwards = mutate_text(
            &mut rng,
            "L8080=localhost:80,R2222=127.0.0.1:22,D1080,6L[::1]:9000=db:5432",
        );
        let _ = putty::forwards(&forwards);
        let _ = putty::map_entries(&mutate_text(&mut rng, "LANG=C,TZ=UTC,X=a=b"));
        let _ = putty::proxy_bypassed(
            &mutate_text(&mut rng, "db.internal"),
            &mutate_text(&mut rng, "*.local, 10.*, ::1, 192.168.0.0/16"),
            rng.below(2) == 0,
        );
        let _ = putty::session_name(
            &mutate_text(&mut rng, "My%20Server%E9%2"),
            [
                None,
                Some(1251),
                Some(1252),
                Some(65001),
                Some(rng.next() as u32),
            ][rng.below(5)],
        );
        let _ = putty::charset_label(&mutate_text(&mut rng, "Win1251 (Cyrillic)"));
    }
}

#[test]
fn putty_imports_never_panic_and_take_a_session_once() {
    let u = first_run();
    let seed = putty_seed();
    let mut rng = Rng(0x9077_1a7e_2026_1023);
    let mut taken = 0;
    for iteration in 0..200 {
        let sessions: Vec<Session> = seed
            .iter()
            .map(|session| mutate_session(&mut rng, session))
            .collect();
        let source = FixedSessions(Sessions {
            sessions,
            unreadable: Vec::new(),
            code_page: [None, Some(1251), Some(1252)][rng.below(3)],
        });
        let _ = u.core.putty_preview(&source);
        let import = || PuttyImportInput {
            sessions: None,
            passphrases: BTreeMap::new(),
            save_passphrases: false,
        };
        if let Ok(first) = u.core.putty_import(&source, import()) {
            taken += usize::from(first.added > 0);
            let again = u
                .core
                .putty_import(&source, import())
                .expect("the same sessions again");
            assert_eq!(again.added, 0, "iteration {iteration}: {again:?}");
        }
    }
    assert!(taken > 25, "{taken} imports added hosts");
}
