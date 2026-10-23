//! Script generation from a description (beta; UI 9.5; run 8 decision
//! D144): a script for the snippet form's Script field from a sentence, by
//! an OpenAI-compatible chat completions API of the user's choice (a
//! hosted service with the user's own API key, or a local model such as
//! Ollama or llama.cpp on this machine).
//!
//! - Off by default (`snippets.generation`); the address
//!   (`snippets.generation_url`, `https://`, or `http://` to this machine
//!   only) and the model (`snippets.generation_model`) are settings, the API
//!   key is kept in the OS keychain ([`api_key`]), never in a setting.
//! - The description goes to the provider only when the user presses
//!   Generate, and the form shows where it goes first. The answer is only
//!   put into the Script field: it is never run.
//! - Tests and test runs never send anything: [`set_provider`] puts a fake
//!   in place, and a test run has no provider otherwise.
//!
//! Nothing here logs descriptions, scripts or keys.

use std::io::Read as _;
use std::sync::Arc;
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
/// The keychain entry of the API key.
const KEYCHAIN_SERVICE: &str = "Shell++ script generation";
const KEYCHAIN_ACCOUNT: &str = "api-key";

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

/// An OpenAI-compatible chat completions API.
pub struct OpenAiCompatible {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<SecretString>,
}

impl std::fmt::Debug for OpenAiCompatible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatible")
            .field("api_key", &self.api_key.is_some())
            .finish_non_exhaustive()
    }
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
/// without a Markdown code fence around it.
pub fn script_of(answer: &[u8]) -> Result<String, GenerateError> {
    let value: Value = serde_json::from_slice(answer).map_err(|_| GenerateError::Empty)?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or(GenerateError::Empty)?;
    let mut text = content.trim();
    if let Some(fenced) = text.strip_prefix("```") {
        // ```bash\n…\n```: drop the language word and the closing fence.
        let body = fenced.split_once('\n').map_or("", |(_, body)| body);
        text = body.trim_end().strip_suffix("```").unwrap_or(body).trim();
    }
    if text.is_empty() {
        return Err(GenerateError::Empty);
    }
    Ok(text.replace("\r\n", "\n"))
}

impl Provider for OpenAiCompatible {
    fn destination(&self) -> String {
        destination_of(&self.base_url)
            .map(|(host, _)| host)
            .unwrap_or_default()
    }

    fn generate(&self, description: &str) -> Result<String, GenerateError> {
        let (_, https) = destination_of(&self.base_url)?;
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
        if let Some(key) = &self.api_key {
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
/// settings' API with the key from the OS keychain when generation is on.
pub fn provider(
    values: &std::collections::BTreeMap<String, Value>,
    cx: &App,
) -> Option<Arc<dyn Provider>> {
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
    Some(Arc::new(OpenAiCompatible {
        base_url,
        model,
        api_key: api_key(),
    }))
}

/// The API key in the OS keychain, if one is saved (never in a test run).
pub fn api_key() -> Option<SecretString> {
    if crate::dirs::is_test_run() {
        return None;
    }
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .and_then(|entry| entry.get_password())
        .ok()
        .filter(|key| !key.is_empty())
        .map(SecretString::from)
}

/// Saves the API key in the OS keychain (`None` removes it). `false` when
/// the keychain refused (none on this system, or a test run).
pub fn set_api_key(key: Option<&SecretString>) -> bool {
    if crate::dirs::is_test_run() {
        return false;
    }
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) else {
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
        assert_eq!(script_of(&answer("   ")), Err(GenerateError::Empty));
        assert_eq!(script_of(b"{\"error\":1}"), Err(GenerateError::Empty));
        assert_eq!(script_of(b"not json"), Err(GenerateError::Empty));
    }

    #[test]
    fn a_test_run_sends_nothing() {
        assert!(api_key().is_none());
        assert!(!set_api_key(None));
    }
}
