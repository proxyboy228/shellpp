//! Snapshot tests of the Tauri configuration, the capabilities and the
//! command lists (plan 4.7 "Tests"; docs/security.md, sections 13.1, 13.2).

use std::path::{Path, PathBuf};

use serde_json::Value;
use shellpp_core::ids;
use shellpp_desktop::commands::COMMAND_ACCESS;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {}", path.display()))
}

fn json(relative: &str) -> Value {
    serde_json::from_str(&read(&manifest_dir().join(relative))).expect("valid JSON")
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The quoted strings between `start` and the next `end` in `text`.
fn quoted_between(text: &str, start: &str, end: &str) -> Vec<String> {
    let begin = text.find(start).expect("start marker") + start.len();
    let rest = &text[begin..];
    let stop = rest.find(end).expect("end marker");
    rest[..stop]
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter_map(|line| {
            let open = line.find('"')?;
            let close = line[open + 1..].find('"')? + open + 1;
            Some(line[open + 1..close].to_owned())
        })
        .collect()
}

/// COMMANDS of build.rs, in order.
fn build_rs_commands() -> Vec<String> {
    quoted_between(
        &read(&manifest_dir().join("build.rs")),
        "const COMMANDS: &[&str] = &[",
        "];",
    )
}

/// The command names of `generate_handler!` in src/lib.rs, in order.
fn handler_commands() -> Vec<String> {
    let text = read(&manifest_dir().join("src").join("lib.rs"));
    let start = "tauri::generate_handler![";
    let begin = text.find(start).expect("generate_handler!") + start.len();
    let rest = &text[begin..];
    let stop = rest.find("])").expect("end of generate_handler!");
    rest[..stop]
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("commands::"))
        .map(|line| {
            line.trim_end_matches(',')
                .rsplit("::")
                .next()
                .expect("command name")
                .to_owned()
        })
        .collect()
}

/// Splits a parameter list on commas outside `<...>`.
fn split_params(params: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in params.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

fn kebab(name: &str) -> String {
    name.replace('_', "-")
}

/// The normative release CSP: the `text` block of docs/security.md, section 13.1.
fn security_md_csp() -> String {
    let doc = read(&manifest_dir().join("../../../docs/security.md"));
    let section = &doc[doc.find("### 13.1").expect("section 13.1")..];
    let block = &section[section.find("```text").expect("csp block") + "```text".len()..];
    let block = &block[..block.find("```").expect("end of csp block")];
    normalize(block)
}

#[test]
fn tauri_conf_identifiers() {
    let conf = json("tauri.conf.json");
    assert_eq!(conf["productName"], ids::DISPLAY_NAME);
    assert_eq!(conf["identifier"], ids::BUNDLE_ID);
    assert_eq!(conf["mainBinaryName"], ids::BINARY_NAME);
    assert_eq!(conf["build"]["devUrl"], "http://localhost:5173");
    assert_eq!(conf["build"]["frontendDist"], "../dist");
}

#[test]
fn tauri_conf_security() {
    let conf = json("tauri.conf.json");
    let app = &conf["app"];
    let security = &app["security"];
    assert_eq!(app["withGlobalTauri"], false);
    assert_eq!(security["freezePrototype"], true);
    assert_eq!(security["assetProtocol"]["enable"], false);
    assert_eq!(security["capabilities"], serde_json::json!(["default"]));
    assert_eq!(
        normalize(security["csp"].as_str().expect("csp")),
        security_md_csp()
    );
    // Tauri's nonce and hash injection stays on.
    assert!(
        security
            .get("dangerousDisableAssetCspModification")
            .is_none()
    );
    assert!(app.get("security").and_then(|s| s.get("pattern")).is_none());

    let release = security["csp"].as_str().expect("csp");
    assert!(!release.contains("unsafe-inline"));
    assert!(!release.contains("unsafe-eval"));

    let dev = security["devCsp"].as_str().expect("devCsp");
    assert!(!dev.contains("unsafe-eval"));
    let script_src = dev
        .split(';')
        .map(str::trim)
        .find(|directive| directive.starts_with("script-src"))
        .expect("script-src");
    assert!(!script_src.contains("unsafe-inline"));
    assert!(!dev.contains('*'));

    // The dev CSP is the release CSP plus the Vite origin and the recorded
    // `style-src 'unsafe-inline'` exception (docs/security.md, section 13.1).
    let release = directives(release);
    let dev = directives(dev);
    assert_eq!(
        dev.keys().collect::<Vec<_>>(),
        release.keys().collect::<Vec<_>>()
    );
    for (name, sources) in &dev {
        let expected = &release[name];
        for source in sources {
            let (name, source) = (name.as_str(), source.as_str());
            let allowed = expected.contains(source)
                || (!expected.contains("'none'") && source == VITE_ORIGIN)
                || (name == "connect-src" && source == VITE_SOCKET)
                || (name == "style-src" && source == "'unsafe-inline'");
            assert!(allowed, "devCsp {name} adds {source}");
        }
        assert!(expected.is_subset(sources), "devCsp {name} drops a source");
    }
}

const VITE_ORIGIN: &str = "http://localhost:5173";
const VITE_SOCKET: &str = "ws://localhost:5173";

/// A CSP as directive name -> sources.
fn directives(csp: &str) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    csp.split(';')
        .map(str::trim)
        .filter(|directive| !directive.is_empty())
        .map(|directive| {
            let mut parts = directive.split_whitespace().map(str::to_owned);
            let name = parts.next().expect("directive name");
            (name, parts.collect())
        })
        .collect()
}

#[test]
fn first_window_config() {
    let conf = json("tauri.conf.json");
    let windows = conf["app"]["windows"].as_array().expect("windows");
    assert_eq!(windows.len(), 1);
    let window = &windows[0];
    assert_eq!(window["label"], shellpp_desktop::FIRST_WINDOW_LABEL);
    assert_eq!(window["decorations"], false);
    assert_eq!(window["create"], false);
    assert_eq!(window["title"], ids::DISPLAY_NAME);

    let mac = json("tauri.macos.conf.json");
    let mac_window = &mac["app"]["windows"][0];
    assert_eq!(mac_window["label"], shellpp_desktop::FIRST_WINDOW_LABEL);
    assert_eq!(mac_window["create"], false);
    assert_eq!(mac_window["decorations"], true);
    assert_eq!(mac_window["titleBarStyle"], "Overlay");
}

#[test]
fn default_capability_grants_exactly_the_stage_1_commands() {
    let cap = json("capabilities/default.json");
    assert_eq!(cap["identifier"], "default");
    assert_eq!(cap["windows"], serde_json::json!(["w-*"]));
    assert_eq!(cap["local"], true);
    assert!(cap.get("remote").is_none());
    assert!(cap.get("webviews").is_none());

    let mut expected: Vec<String> = [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:window:allow-minimize",
        "core:window:allow-toggle-maximize",
        "core:window:allow-is-maximized",
        "core:window:allow-close",
        "core:window:allow-start-dragging",
        "core:window:allow-set-fullscreen",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    expected.extend(
        build_rs_commands()
            .iter()
            .filter(|name| *name != "e2e_mock")
            .map(|name| format!("allow-{}", kebab(name))),
    );
    let granted: Vec<String> = cap["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .map(|permission| permission.as_str().expect("plain permission id").to_owned())
        .collect();
    assert_eq!(granted, expected);
    assert!(
        !granted
            .iter()
            .any(|p| p == "core:default" || p.ends_with(":default"))
    );
}

#[test]
fn e2e_capability_grants_only_the_mock_command() {
    let cap = json("capabilities/e2e.json");
    assert_eq!(cap["identifier"], "e2e");
    assert_eq!(cap["windows"], serde_json::json!(["w-*"]));
    assert_eq!(cap["local"], true);
    assert!(cap.get("remote").is_none());
    assert_eq!(cap["permissions"], serde_json::json!(["allow-e2e-mock"]));
}

#[test]
fn handler_list_matches_build_rs() {
    let commands = build_rs_commands();
    assert_eq!(commands.len(), 31);
    assert_eq!(handler_commands(), commands);
}

#[test]
fn every_command_has_a_lock_rule() {
    let rules: Vec<String> = COMMAND_ACCESS
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    assert_eq!(rules, build_rs_commands());
}

#[test]
fn commands_follow_the_one_input_rule() {
    // Every command file declares `rename_all = "snake_case"` and at most one
    // non-injected argument named `input` (plan decision 5).
    let dir = manifest_dir().join("src").join("commands");
    for entry in std::fs::read_dir(&dir).expect("commands dir") {
        let path = entry.expect("entry").path();
        let text = read(&path);
        let mut rest = text.as_str();
        while let Some(index) = rest.find("#[tauri::command") {
            let attr_end = rest[index..].find(']').expect("attribute end") + index;
            assert!(
                rest[index..attr_end].contains("rename_all = \"snake_case\""),
                "{}: command without rename_all",
                path.display()
            );
            let sig_start = rest[attr_end..].find("pub async fn").expect("async fn") + attr_end;
            let sig_end = rest[sig_start..].find('{').expect("body") + sig_start;
            let signature = &rest[sig_start..sig_end];
            let open = signature.find('(').expect("(");
            let mut depth = 0usize;
            let mut close = open;
            for (offset, c) in signature[open..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = open + offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let params = &signature[open + 1..close];
            for param in split_params(params) {
                let name = param.split(':').next().expect("name").trim();
                let ty = param.split(':').nth(1).unwrap_or("").trim();
                let injected = ty.starts_with("State<")
                    || ty.starts_with("AppHandle")
                    || ty.starts_with("WebviewWindow");
                assert!(
                    injected || name == "input",
                    "{}: argument `{name}` must be named `input`",
                    path.display()
                );
            }
            rest = &rest[sig_end..];
        }
    }
}
