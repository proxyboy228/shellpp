//! The "save as snippets" hint of the History tab (UI 4.2; docs/data-model.md,
//! sections 13.1 and 13.2): a host's three most frequent commands, and the
//! per-host `history_hint_state` that remembers "Close".
//!
//! Every offered command is checked again by the secret detector
//! (docs/security.md, section 10.2): an entry recorded before a rule was
//! added, or by another device, is never offered when it looks like a secret.
//! Nothing here logs commands.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::{parts, same_link};
use crate::error::CoreError;
use crate::history;
use crate::model::history_entry;
use crate::model::types::RecordKind;
use crate::service::Core;
use crate::service::dto::{HistoryHint, RecordRef};

/// Commands the hint offers at most.
pub const HINT_COMMANDS: usize = 3;

/// `history_hint_state.dismissed` (absent or malformed = not dismissed).
fn dismissed(data: &Map<String, Value>) -> bool {
    data.get("dismissed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The data of a dismissed `history_hint_state`.
fn dismissed_state(host: RecordRef) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("host".into(), history_entry::host_link(host));
    data.insert("dismissed".into(), Value::Bool(true));
    data
}

/// The three most frequent commands of the entries, most frequent first,
/// ties by the newest use, then by text; secret-looking commands and lines
/// that hold a password prompt (input typed before the prompt, recorded
/// before that rule existed) left out.
pub(crate) fn top_commands<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    // command -> (count, newest executed_at)
    let mut counts: HashMap<&str, (u32, &str)> = HashMap::new();
    for (command, executed_at) in entries {
        let slot = counts.entry(command).or_insert((0, ""));
        slot.0 = slot.0.saturating_add(1);
        if executed_at > slot.1 {
            slot.1 = executed_at;
        }
    }
    let mut ranked: Vec<(&str, (u32, &str))> = counts
        .into_iter()
        .filter(|(command, _)| {
            !command.trim().is_empty()
                && !history::looks_like_secret(command)
                && !history::line_holds_password_prompt(command)
        })
        .collect();
    ranked.sort_by(|(a, (count_a, at_a)), (b, (count_b, at_b))| {
        count_b
            .cmp(count_a)
            .then_with(|| at_b.cmp(at_a))
            .then_with(|| a.cmp(b))
    });
    ranked
        .into_iter()
        .take(HINT_COMMANDS)
        .map(|(command, _)| command.to_owned())
        .collect()
}

impl Core {
    /// The "save as snippets" hint of `host` (UI 4.2): its three most
    /// frequent commands, each checked again by the secret detector, and
    /// whether "Close" was chosen for the host.
    pub fn history_hint(&self, host: RecordRef) -> Result<HistoryHint, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_history(&mut parts, now)?;
        let index = &parts.session.index;
        let dismissed = index
            .of_kind(RecordKind::HistoryHintState)
            .filter(|record| same_link(record.data(), Some(host)))
            .any(|record| dismissed(record.data()));
        let commands = top_commands(
            index
                .of_kind(RecordKind::HistoryEntry)
                .filter(|record| same_link(record.data(), Some(host)))
                .map(|record| {
                    (
                        history_entry::command(record.data()),
                        history_entry::executed_at(record.data()),
                    )
                }),
        );
        Ok(HistoryHint {
            commands,
            dismissed,
        })
    }

    /// "Close" on the hint: the hint is not shown again for `host`
    /// (`history_hint_state.dismissed = true`, one record per host link).
    pub fn history_hint_dismiss(&self, host: RecordRef) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_history(&mut parts, now)?;
        let states: Vec<(uuid::Uuid, bool)> = parts
            .session
            .index
            .of_kind(RecordKind::HistoryHintState)
            .filter(|record| same_link(record.data(), Some(host)))
            .map(|record| (record.record_id, dismissed(record.data())))
            .collect();
        if states.iter().any(|(_, dismissed)| *dismissed) {
            return Ok(());
        }
        let record_id = match states.first() {
            Some((record_id, _)) => *record_id,
            None => parts.ids.next(now),
        };
        self.write_record(
            &mut parts,
            RecordKind::HistoryHintState,
            record_id,
            dismissed_state(host),
            now,
        )?;
        tracing::info!("history hint dismissed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_commands_rank_by_count_then_newest_use() {
        let entries = [
            ("ls", "2026-10-20T10:00:00.000Z"),
            ("git status", "2026-10-20T10:01:00.000Z"),
            ("ls", "2026-10-20T10:02:00.000Z"),
            ("make", "2026-10-20T10:03:00.000Z"),
            ("git status", "2026-10-20T10:04:00.000Z"),
            ("uptime", "2026-10-20T09:00:00.000Z"),
        ];
        // ls and git status twice (git status used last), then make (newer
        // than uptime).
        assert_eq!(
            top_commands(entries.into_iter()),
            ["git status", "ls", "make"]
        );
        assert!(top_commands(std::iter::empty()).is_empty());
    }

    #[test]
    fn secret_looking_commands_are_never_offered() {
        let token = format!("ghp_{}", "aB3dE6gH9jK2mN5pQ8sT1vW4yZ7bC0eF3hJ6");
        let commands = [
            "mysql -uroot -phunter2".to_owned(),
            format!("export GITHUB_TOKEN={token}"),
            format!("gh auth login --with-token {token}"),
            "curl -H 'Authorization: Bearer abc' https://example.com".to_owned(),
            "git clone https://bob:pw@example.com/repo.git".to_owned(),
            "echo q7Xv2LmR9tKp4WzN8sBe".to_owned(),
            // A password typed ahead of `su`'s prompt.
            "Ahead-pw-5Password:".to_owned(),
            "ls".to_owned(),
        ];
        let mut entries = Vec::new();
        for (n, command) in commands.iter().enumerate() {
            // Secret-looking commands are the most frequent ones.
            for _ in 0..(10 - n) {
                entries.push((command.as_str(), "2026-10-20T10:00:00.000Z"));
            }
        }
        assert_eq!(top_commands(entries.into_iter()), ["ls"]);
    }
}
