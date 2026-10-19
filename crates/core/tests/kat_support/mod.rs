//! Shared helpers of the known-answer tests (`kat_*.rs`).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use uuid::Uuid;

/// The construction vectors computed by `vectors/gen_vectors.py`.
pub fn constructions() -> Value {
    serde_json::from_str(include_str!("../vectors/kat_constructions.json")).unwrap()
}

/// The AEAD and Argon2id vectors recorded from the implementation.
pub fn aead() -> Value {
    serde_json::from_str(include_str!("../vectors/kat_aead.json")).unwrap()
}

/// Lowercase hex to bytes.
pub fn unhex(text: &str) -> Vec<u8> {
    data_encoding::HEXLOWER.decode(text.as_bytes()).unwrap()
}

/// Bytes to lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// A JSON string member as hex-decoded bytes.
pub fn bytes(v: &Value, key: &str) -> Vec<u8> {
    unhex(v[key].as_str().unwrap_or_else(|| panic!("missing {key}")))
}

/// A JSON string member as a fixed-size array.
pub fn array<const N: usize>(v: &Value, key: &str) -> [u8; N] {
    bytes(v, key).try_into().unwrap()
}

/// A JSON string member as a UUID.
pub fn uuid(v: &Value, key: &str) -> Uuid {
    Uuid::parse_str(v[key].as_str().unwrap()).unwrap()
}

/// A JSON string member.
pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_else(|| panic!("missing {key}"))
}

/// Removes the `\\?\` prefix `canonicalize` adds on Windows, so the path can
/// be handed to other programs.
fn plain(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// `crates/core/tests`, with links resolved.
pub fn tests_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    plain(std::fs::canonicalize(dir).unwrap())
}

/// `crates/core/tests/vectors`.
pub fn vectors_dir() -> PathBuf {
    tests_dir().join("vectors")
}

/// `docs/storage-format.md` of the repository, if present.
pub fn storage_format_md() -> Option<PathBuf> {
    let repo = tests_dir().parent()?.parent()?.parent()?.to_path_buf();
    let path = repo.join("docs").join("storage-format.md");
    path.is_file().then_some(path)
}

/// A Python 3 interpreter on PATH (`python3`, `python` or `py -3`), if any.
pub fn python3() -> Option<Vec<String>> {
    let candidates: [&[&str]; 3] = [&["python3"], &["python"], &["py", "-3"]];
    for candidate in candidates {
        let Ok(out) = Command::new(candidate[0])
            .args(&candidate[1..])
            .args(["-I", "-c", "import sys; print(sys.version_info[0])"])
            .output()
        else {
            continue;
        };
        if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "3" {
            return Some(candidate.iter().map(|s| (*s).to_owned()).collect());
        }
    }
    None
}

/// Runs a Python 3 script in isolated mode (`-I`) with `args` and extra
/// environment variables.
pub fn run_python(python: &[String], script: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(&python[0]);
    cmd.args(&python[1..]).arg("-I").arg(script).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// Notes a skipped optional check on stderr (visible with `--nocapture`).
#[allow(clippy::print_stderr)]
pub fn skip(reason: &str) {
    eprintln!("skipped: {reason}");
}
