//! `shellpp://` links (docs/security.md, section 13.3; UI 2.4 Copy Link).
//!
//! Stage 1 understands only `shellpp://host/<record_id>`. Links come from the
//! command line of the first launch, from a second launch (single instance)
//! and, in e2e builds, from `e2e_mock {kind: "inject_link"}`. A link never
//! connects or runs anything: it selects the host after unlock. Any other or
//! malformed `shellpp://` link is ignored.
//!
//! Links are held while the app is locked and delivered as the window event
//! `link:open {kind: "host", record_id}` to the focused (or last focused)
//! window once it is unlocked.

use std::sync::Mutex;

use serde::Serialize;
use shellpp_core::ids;
use uuid::Uuid;

/// A parsed link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Link {
    /// `shellpp://host/<record_id>`.
    Host { record_id: Uuid },
}

/// Window event name of a delivered link.
pub const LINK_OPEN_EVENT: &str = "link:open";

/// Parses one link. Accepts exactly `shellpp://host/<uuid>` (scheme
/// case-insensitive, lowercase or uppercase hyphenated UUID, an optional
/// trailing slash); no user info, port, query or fragment.
pub fn parse_link(text: &str) -> Option<Link> {
    let text = text.trim();
    if text.len() > 256 {
        return None;
    }
    let url = url::Url::parse(text).ok()?;
    if url.scheme() != ids::URL_SCHEME
        || url.host_str() != Some("host")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let id = url.path().strip_prefix('/')?;
    let id = id.strip_suffix('/').unwrap_or(id);
    if id.len() != 36 {
        return None;
    }
    let record_id = Uuid::try_parse(id).ok()?;
    Some(Link::Host { record_id })
}

/// The first link among command line arguments (the first element, the
/// executable path, is skipped by the caller when needed).
pub fn find_link<I, S>(args: I) -> Option<Link>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().find_map(|arg| parse_link(arg.as_ref()))
}

/// Links waiting for unlock, and the host's view of the lock state used to
/// decide whether a link can be delivered right away.
#[derive(Debug, Default)]
pub struct LinkQueue {
    inner: Mutex<LinkQueueInner>,
}

#[derive(Debug, Default)]
struct LinkQueueInner {
    unlocked: bool,
    pending: Vec<Link>,
}

/// Pending links are capped; a flood of second launches cannot grow memory.
const MAX_PENDING: usize = 16;

impl LinkQueue {
    pub fn new(unlocked: bool) -> Self {
        Self {
            inner: Mutex::new(LinkQueueInner {
                unlocked,
                pending: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LinkQueueInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Queues the link, or returns it for immediate delivery when unlocked.
    pub fn push(&self, link: Link) -> Option<Link> {
        let mut inner = self.lock();
        if inner.unlocked {
            return Some(link);
        }
        if inner.pending.len() >= MAX_PENDING {
            inner.pending.remove(0);
        }
        inner.pending.push(link);
        None
    }

    /// Puts back links that found no ready window (oldest first, capped).
    pub fn requeue(&self, links: Vec<Link>) {
        let mut inner = self.lock();
        let mut merged = links;
        merged.append(&mut inner.pending);
        let excess = merged.len().saturating_sub(MAX_PENDING);
        merged.drain(..excess);
        inner.pending = merged;
    }

    /// The waiting links when unlocked (a window became ready).
    pub fn take_ready(&self) -> Vec<Link> {
        let mut inner = self.lock();
        if inner.unlocked {
            std::mem::take(&mut inner.pending)
        } else {
            Vec::new()
        }
    }

    /// Records a lock state change; returns the links to deliver now.
    pub fn set_unlocked(&self, unlocked: bool) -> Vec<Link> {
        let mut inner = self.lock();
        inner.unlocked = unlocked;
        if unlocked {
            std::mem::take(&mut inner.pending)
        } else {
            Vec::new()
        }
    }

    /// Number of links waiting (tests).
    pub fn pending(&self) -> usize {
        self.lock().pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0199f6a1-b2c3-7d4e-8f00-112233445566";

    fn host(id: &str) -> Option<Link> {
        Some(Link::Host {
            record_id: Uuid::parse_str(id).unwrap(),
        })
    }

    #[test]
    fn parses_host_links() {
        assert_eq!(parse_link(&format!("shellpp://host/{ID}")), host(ID));
        assert_eq!(parse_link(&format!("shellpp://host/{ID}/")), host(ID));
        assert_eq!(parse_link(&format!("  SHELLPP://host/{ID} ")), host(ID));
        assert_eq!(
            parse_link(&format!("shellpp://host/{}", ID.to_uppercase())),
            host(ID)
        );
    }

    #[test]
    fn ignores_everything_else() {
        for text in [
            "",
            "shellpp://",
            "shellpp://host",
            "shellpp://host/",
            "shellpp://host/not-a-uuid",
            "shellpp://host/0199f6a1b2c37d4e8f00112233445566",
            &format!("shellpp://host/{ID}/extra"),
            &format!("shellpp://host/{ID}?connect=1"),
            &format!("shellpp://host/{ID}#x"),
            &format!("shellpp://user@host/{ID}"),
            &format!("shellpp://host:22/{ID}"),
            &format!("shellpp://invite/{ID}"),
            &format!("https://host/{ID}"),
            &format!("shellpp:host/{ID}"),
            "C:\\Program Files\\Shell++\\shellpp.exe",
            "--flag",
        ] {
            assert_eq!(parse_link(text), None, "{text}");
        }
        let long = format!("shellpp://host/{ID}{}", "/".repeat(300));
        assert_eq!(parse_link(&long), None);
    }

    #[test]
    fn finds_the_first_link_in_arguments() {
        let args = [
            "shellpp.exe".to_owned(),
            "--x".to_owned(),
            format!("shellpp://host/{ID}"),
            "shellpp://host/0199f6a1-b2c3-7d4e-8f00-000000000000".to_owned(),
        ];
        assert_eq!(find_link(args.iter().skip(1)), host(ID));
        assert_eq!(find_link(["a", "b"]), None);
    }

    #[test]
    fn queue_holds_links_until_unlocked() {
        let queue = LinkQueue::new(false);
        let link = host(ID).unwrap();
        assert_eq!(queue.push(link), None);
        assert_eq!(queue.pending(), 1);
        assert!(queue.set_unlocked(false).is_empty());
        assert_eq!(queue.set_unlocked(true), vec![link]);
        assert_eq!(queue.pending(), 0);
        assert_eq!(queue.push(link), Some(link));
        queue.set_unlocked(false);
        for _ in 0..40 {
            queue.push(link);
        }
        assert_eq!(queue.pending(), MAX_PENDING);
        assert!(queue.take_ready().is_empty());
    }

    #[test]
    fn requeued_links_wait_for_a_window() {
        let queue = LinkQueue::new(true);
        let first = host(ID).unwrap();
        let second = host("0199f6a1-b2c3-7d4e-8f00-000000000009").unwrap();
        queue.requeue(vec![first]);
        queue.requeue(vec![second]);
        assert_eq!(queue.pending(), 2);
        assert_eq!(queue.take_ready(), vec![second, first]);
        queue.requeue(vec![first; 40]);
        assert_eq!(queue.pending(), MAX_PENDING);
    }

    #[test]
    fn link_event_payload_shape() {
        let link = host(ID).unwrap();
        assert_eq!(
            serde_json::to_value(link).unwrap(),
            serde_json::json!({"kind": "host", "record_id": ID})
        );
    }
}
