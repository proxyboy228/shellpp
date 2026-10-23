//! Script generation from a description (beta; UI 9.5; run 8 decision
//! D144): a script for the snippet form's Script field from a sentence, by
//! an OpenAI-compatible chat completions API of the user's choice (a
//! hosted service with the user's own API key, or a local model such as
//! Ollama or llama.cpp on this machine).
//!
//! - Off by default (`snippets.generation`); the address
//!   (`snippets.generation_url`, `https://`, or `http://` to this machine
//!   only) and the model (`snippets.generation_model`) are settings of this
//!   device, never imported. The API key is kept in the OS credential
//!   store per host ([`api_key`]): the key saved for one service is never
//!   sent to another. It is read and written off the UI thread; a test run
//!   and `--demo` keep keys in memory, an unsigned macOS build has none.
//! - The description goes to the provider only when the user presses
//!   Generate, and the form shows where it goes first. The answer is only
//!   put into the Script field: it is never run.
//! - Tests and test runs never send anything: [`set_provider`] puts a fake
//!   in place, and a test run has no provider otherwise.
//!
//! Nothing here logs descriptions, scripts or keys.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{App, Global};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

/// The settings of this feature.
pub const KEY_ENABLED: &str = "snippets.generation";
pub const KEY_URL: &str = "snippets.generation_url";
pub const KEY_MODEL: &str = "snippets.generation_model";
/// The address offered when none is set.
pub const DEFAULT_URL: &str = "https://api.openai.com/v1";
/// Longest description sent, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 2000;
/// Largest answer read.
const MAX_ANSWER_BYTES: u64 = 256 * 1024;
/// How long a provider may take (a local model can be slow).
const TIMEOUT: Duration = Duration::from_secs(120);
/// The keychain entries of the API keys: one per host.
const KEYCHAIN_SERVICE: &str = "Shell++ script generation";

/// What the model is asked to do.
const INSTRUCTIONS: &str = "You write shell scripts for Linux servers. Answer with the \
     script only: no explanation, no Markdown. Use {{name}} for any value the user must \
     fill in when the snippet runs. Never include passwords or keys.";

/// Why no script came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GenerateError {
    /// The address is not `https://` (or `http://` to this machine).
    #[error("address not allowed")]
    Address,
    /// The provider could not be reached or refused the request.
    #[error("unreachable")]
    Unreachable,
    /// The answer holds no script.
    #[error("no script")]
    Empty,
}

/// A script generator.
pub trait Provider: Send + Sync + 'static {
    /// Where descriptions go, as the form shows it (the provider's host).
    fn destination(&self) -> String;
    /// Asks for a script (blocking: call it off the UI thread).
    fn generate(&self, description: &str) -> Result<String, GenerateError>;
}

/// An OpenAI-compatible chat completions API; the key saved for its host
/// is read when a script is asked for.
#[derive(Debug)]
pub struct OpenAiCompatible {
    pub base_url: String,
    pub model: String,
}

/// The host of an address as the form shows it, and whether it may be
/// asked: `https://`, or plain HTTP to this machine's loopback (a local
/// model). A user part (`user@`) is refused.
pub fn destination_of(url: &str) -> Result<(String, bool), GenerateError> {
    let uri: ureq::http::Uri = url.trim().parse().map_err(|_| GenerateError::Address)?;
    let authority = uri.authority().ok_or(GenerateError::Address)?;
    if authority.as_str().contains('@') {
        return Err(GenerateError::Address);
    }
    match uri.scheme_str().map(str::to_ascii_lowercase).as_deref() {
        Some("https") => Ok((authority.host().to_owned(), true)),
        Some("http") if matches!(authority.host(), "127.0.0.1" | "localhost" | "[::1]") => {
            Ok((authority.as_str().to_owned(), false))
        }
        _ => Err(GenerateError::Address),
    }
}

/// The JSON body of the chat completions request.
pub fn request_body(model: &str, description: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": INSTRUCTIONS},
            {"role": "user", "content": description},
        ],
        "temperature": 0.2,
    })
}

/// The script of a chat completions answer: the first choice's content,
/// or, when it holds a Markdown code fence, the first fenced block alone
/// (prose around it is left out: it would run as commands).
pub fn script_of(answer: &[u8]) -> Result<String, GenerateError> {
    let value: Value = serde_json::from_slice(answer).map_err(|_| GenerateError::Empty)?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or(GenerateError::Empty)?
        .replace("\r\n", "\n");
    let text = match content.split_once("```") {
        // ```bash\n…\n```: from the line after the opening fence to the
        // closing one (or the end).
        Some((_, fenced)) => {
            let body = fenced.split_once('\n').map_or("", |(_, body)| body);
            body.split("```").next().unwrap_or(body).trim()
        }
        None => content.trim(),
    };
    if text.is_empty() {
        return Err(GenerateError::Empty);
    }
    Ok(text.to_owned())
}

impl Provider for OpenAiCompatible {
    fn destination(&self) -> String {
        destination_of(&self.base_url)
            .map(|(host, _)| host)
            .unwrap_or_default()
    }

    fn generate(&self, description: &str) -> Result<String, GenerateError> {
        let (host, https) = destination_of(&self.base_url)?;
        // The key saved for this host only (none for a local model).
        let api_key = api_key(&host);
        let description: String = description.chars().take(MAX_DESCRIPTION_CHARS).collect();
        let url = format!(
            "{}/chat/completions",
            self.base_url.trim().trim_end_matches('/')
        );
        let mut config = ureq::Agent::config_builder();
        if !https {
            // This machine: never through a proxy of the environment.
            config = config.proxy(None);
        }
        let agent: ureq::Agent = config
            .timeout_global(Some(TIMEOUT))
            .https_only(https)
            .max_redirects(0)
            .http_status_as_error(true)
            .build()
            .into();
        let body = request_body(&self.model, &description).to_string();
        let mut request = agent.post(&url).header("Content-Type", "application/json");
        if let Some(key) = &api_key {
            request = request.header("Authorization", format!("Bearer {}", key.expose_secret()));
        }
        let mut response = request
            .send(body.as_bytes())
            .map_err(|_| GenerateError::Unreachable)?;
        let mut answer = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_ANSWER_BYTES + 1)
            .read_to_end(&mut answer)
            .map_err(|_| GenerateError::Unreachable)?;
        if answer.len() as u64 > MAX_ANSWER_BYTES {
            return Err(GenerateError::Empty);
        }
        script_of(&answer)
    }
}

/// A provider put in place by tests and the demo.
#[derive(Clone)]
struct Override(Option<Arc<dyn Provider>>);

impl Global for Override {}

/// Replaces the provider (tests; `None`: none at all).
pub fn set_provider(provider: Option<Arc<dyn Provider>>, cx: &mut App) {
    cx.set_global(Override(provider));
}

/// The provider in effect: the override; none in a test run; else the
/// settings' API when generation is on with a model (its key is read when a
/// script is asked for).
pub fn provider(values: &BTreeMap<String, Value>, cx: &App) -> Option<Arc<dyn Provider>> {
    if let Some(found) = cx.try_global::<Override>() {
        return found.0.clone();
    }
    if crate::dirs::is_test_run() {
        return None;
    }
    if values.get(KEY_ENABLED).and_then(Value::as_str) != Some("on") {
        return None;
    }
    let base_url = values
        .get(KEY_URL)
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(DEFAULT_URL)
        .to_owned();
    let model = values
        .get(KEY_MODEL)
        .and_then(Value::as_str)
        .filter(|model| !model.trim().is_empty())?
        .to_owned();
    Some(Arc::new(OpenAiCompatible { base_url, model }))
}

/// Keys stay in memory (`--demo`; a test run always does).
static MEMORY_ONLY: AtomicBool = AtomicBool::new(false);
/// The keys kept in memory, by account.
static MEMORY: Mutex<BTreeMap<String, SecretString>> = Mutex::new(BTreeMap::new());

/// `--demo`: API keys never reach the OS credential store.
pub fn keep_keys_in_memory() {
    MEMORY_ONLY.store(true, Ordering::Release);
}

/// Where keys are kept: memory, the OS credential store, or nowhere (an
/// unsigned macOS build, whose keychain items the system would not keep
/// for it, D7).
enum Store {
    Memory,
    Os,
    None,
}

fn store() -> Store {
    if crate::dirs::is_test_run() || MEMORY_ONLY.load(Ordering::Acquire) {
        Store::Memory
    } else if cfg!(target_os = "macos") && option_env!("SHELLPP_MACOS_SIGNED").is_none() {
        Store::None
    } else {
        Store::Os
    }
}

fn account(host: &str) -> String {
    format!("api-key:{}", host.to_ascii_lowercase())
}

/// The API key saved for `host` ([`destination_of`]'s host), if any.
/// Blocking: call it off the UI thread.
pub fn api_key(host: &str) -> Option<SecretString> {
    match store() {
        Store::Memory => MEMORY
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&account(host))
            .cloned(),
        Store::Os => keyring::Entry::new(KEYCHAIN_SERVICE, &account(host))
            .and_then(|entry| entry.get_password())
            .ok()
            .filter(|key| !key.is_empty())
            .map(SecretString::from),
        Store::None => None,
    }
}

/// Saves the API key of `host` (`None` removes it). `false` when the store
/// refused or there is none. Blocking: call it off the UI thread.
pub fn set_api_key(host: &str, key: Option<&SecretString>) -> bool {
    match store() {
        Store::Memory => {
            let mut keys = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
            match key {
                Some(key) => {
                    keys.insert(account(host), key.clone());
                }
                None => {
                    keys.remove(&account(host));
                }
            }
            true
        }
        Store::Os => {
            let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &account(host)) else {
                return false;
            };
            match key {
                Some(key) => entry.set_password(key.expose_secret()).is_ok(),
                None => matches!(
                    entry.delete_credential(),
                    Ok(()) | Err(keyring::Error::NoEntry)
                ),
            }
        }
        Store::None => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_https_or_this_machine() {
        assert_eq!(
            destination_of("https://api.example.com/v1"),
            Ok(("api.example.com".to_owned(), true))
        );
        assert_eq!(
            destination_of("http://localhost:11434/v1"),
            Ok(("localhost:11434".to_owned(), false))
        );
        assert_eq!(
            destination_of("http://api.example.com/v1"),
            Err(GenerateError::Address)
        );
        assert_eq!(
            destination_of("http://127.0.0.1:@elsewhere/v1"),
            Err(GenerateError::Address)
        );
        assert_eq!(destination_of("ftp://x/"), Err(GenerateError::Address));
    }

    #[test]
    fn the_request_names_the_model_and_the_description() {
        let body = request_body("small-model", "restart nginx");
        assert_eq!(body["model"], "small-model");
        assert_eq!(body["messages"][1]["content"], "restart nginx");
        assert_eq!(body["messages"][0]["role"], "system");
    }

    #[test]
    fn answers_become_scripts() {
        let answer = |content: &str| {
            json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
                .to_string()
                .into_bytes()
        };
        assert_eq!(
            script_of(&answer("sudo systemctl restart nginx\n")),
            Ok("sudo systemctl restart nginx".to_owned())
        );
        assert_eq!(
            script_of(&answer("```bash\nset -e\r\nls -la {{dir}}\n```")),
            Ok("set -e\nls -la {{dir}}".to_owned())
        );
        // Prose around the fence is left out: it would run as commands.
        assert_eq!(
            script_of(&answer("Here it is:\n```sh\nls\n```\nThis lists files.")),
            Ok("ls".to_owned())
        );
        assert_eq!(script_of(&answer("   ")), Err(GenerateError::Empty));
        assert_eq!(script_of(b"{\"error\":1}"), Err(GenerateError::Empty));
        assert_eq!(script_of(b"not json"), Err(GenerateError::Empty));
    }

    /// A one-request HTTP server on 127.0.0.1 answering `response`; returns
    /// the address and the request it got.
    fn serve_once(response: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            // Headers, then the body of its Content-Length.
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            stream.write_all(&response).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (address, handle)
    }

    fn http(status: &str, extra: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// The client against a local server: the request goes to
    /// `<address>/chat/completions` with the model and the description,
    /// the key of that host only, and the answer becomes the script; a
    /// redirect is not followed; an oversized answer is refused.
    #[test]
    fn the_client_asks_the_configured_host_only() {
        let body = json!({"choices": [{"message": {"content": "uptime"}}]}).to_string();
        let (address, server) = serve_once(http("200 OK", "", &body));
        let (host, https) = destination_of(&address).unwrap();
        assert!(!https);
        let key = SecretString::from("local-test-key");
        assert!(set_api_key(&host, Some(&key)));
        let client = OpenAiCompatible {
            base_url: address,
            model: "tiny".to_owned(),
        };
        assert_eq!(
            client.generate("how long is it up"),
            Ok("uptime".to_owned())
        );
        let request = server.join().unwrap();
        assert!(
            request.starts_with("POST /v1/chat/completions"),
            "{request}"
        );
        assert!(request.contains("Bearer local-test-key"));
        assert!(request.contains("how long is it up") && request.contains("\"tiny\""));
        assert!(set_api_key(&host, None));

        // A redirect elsewhere is not followed.
        let (address, server) = serve_once(http(
            "302 Found",
            "Location: https://elsewhere.example/v1/chat/completions\r\n",
            "",
        ));
        let client = OpenAiCompatible {
            base_url: address,
            model: "tiny".to_owned(),
        };
        assert!(client.generate("x").is_err());
        let request = server.join().unwrap();
        assert!(!request.contains("Authorization"), "no key saved for it");

        // An answer over 256 KiB.
        let huge = "a".repeat(MAX_ANSWER_BYTES as usize + 10);
        let (address, server) = serve_once(http("200 OK", "", &huge));
        let client = OpenAiCompatible {
            base_url: address,
            model: "tiny".to_owned(),
        };
        assert_eq!(client.generate("x"), Err(GenerateError::Empty));
        let _ = server.join();
    }

    /// A test run keeps keys in memory, one per host: a key saved for one
    /// service is not read for another.
    #[test]
    fn keys_belong_to_their_host() {
        let key = SecretString::from("not-a-real-key");
        assert!(set_api_key("one.example", Some(&key)));
        assert!(api_key("ONE.example").is_some());
        assert!(api_key("other.example").is_none());
        assert!(set_api_key("one.example", None));
        assert!(api_key("one.example").is_none());
    }
}
