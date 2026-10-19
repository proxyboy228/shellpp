//! Quick-connect address recognition of the search bar (UI 2.1).
//!
//! Recognized forms: `host`, `user@host`, `user@host:port`, `host:port` and
//! `ssh [user@]host [-p port] [-l user]` (options in any order). A single
//! word (the `host` form, without a user) is an address only if it contains
//! a dot or a colon, is an IP address, or is `localhost`; any other single
//! word is only a search query. IPv6 addresses are accepted bare (`::1`) or
//! in brackets with a port (`[2001:db8::1]:2222`).

use std::net::IpAddr;

/// A connection target parsed from the search text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickTarget {
    pub username: Option<String>,
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    pub port: Option<u16>,
}

impl QuickTarget {
    /// `user@host` or `host` (the session tab title).
    pub fn title(&self) -> String {
        super::strings::session_title(self.username.as_deref(), &self.host)
    }
}

/// Parses `input` as a connection address; `None` when it is a search query.
pub fn parse_address(input: &str) -> Option<QuickTarget> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let mut words = input.split_whitespace();
    let first = words.next()?;
    if first.eq_ignore_ascii_case("ssh") {
        return parse_ssh_command(words);
    }
    if words.next().is_some() {
        // Several words that are not an ssh command: a search query.
        return None;
    }
    let target = parse_destination(first)?;
    if target.username.is_none() && target.port.is_none() && !is_address_word(&target.host) {
        return None;
    }
    Some(target)
}

/// `ssh` arguments: one destination plus `-p <port>` / `-p<port>` and
/// `-l <user>` / `-l<user>`. Any other option makes it a search query.
fn parse_ssh_command<'a>(words: impl Iterator<Item = &'a str>) -> Option<QuickTarget> {
    let mut words = words.peekable();
    let mut destination = None;
    let mut port = None;
    let mut login = None;
    while let Some(word) = words.next() {
        if let Some(option) = word.strip_prefix('-') {
            let flag = option.chars().next()?;
            let inline = &option[flag.len_utf8()..];
            let value = if inline.is_empty() {
                words.next()?
            } else {
                inline
            };
            match flag {
                'p' => port = Some(parse_port(value)?),
                'l' => login = Some(parse_username(value)?),
                _ => return None,
            }
        } else if destination.is_none() {
            destination = Some(word);
        } else {
            // A remote command after the destination is not a quick connect.
            return None;
        }
    }
    let mut target = parse_destination(destination?)?;
    if target.username.is_none() {
        target.username = login;
    }
    if port.is_some() {
        target.port = port;
    }
    Some(target)
}

/// `[user@]host[:port]`, `[user@][v6]:port`, `[user@]v6`.
fn parse_destination(word: &str) -> Option<QuickTarget> {
    let (username, rest) = match word.rsplit_once('@') {
        Some((user, rest)) => (Some(parse_username(user)?), rest),
        None => (None, word),
    };
    let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
        let (v6, after) = bracketed.split_once(']')?;
        v6.parse::<std::net::Ipv6Addr>().ok()?;
        let port = match after {
            "" => None,
            _ => Some(parse_port(after.strip_prefix(':')?)?),
        };
        (v6.to_owned(), port)
    } else if rest.parse::<std::net::Ipv6Addr>().is_ok() {
        (rest.to_owned(), None)
    } else if let Some((host, port)) = rest.rsplit_once(':') {
        (host.to_owned(), Some(parse_port(port)?))
    } else {
        (rest.to_owned(), None)
    };
    if !is_host_name(&host) {
        return None;
    }
    Some(QuickTarget {
        username,
        host,
        port,
    })
}

fn parse_port(value: &str) -> Option<u16> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u16>().ok().filter(|port| *port > 0)
}

/// A login name: no whitespace, `@`, `:` or `/`, at most 256 characters.
fn parse_username(value: &str) -> Option<String> {
    let valid = !value.is_empty()
        && value.chars().count() <= 256
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '@' | ':' | '/'));
    valid.then(|| value.to_owned())
}

/// A host name (letters, digits, `-`, `_`, dots between non-empty labels)
/// or an IP address.
fn is_host_name(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let host = host.strip_suffix('.').unwrap_or(host);
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

/// The single-word rule of UI 2.1: a dot or a colon, an IP, or `localhost`.
fn is_address_word(host: &str) -> bool {
    host.contains('.')
        || host.contains(':')
        || host.parse::<IpAddr>().is_ok()
        || host.eq_ignore_ascii_case("localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(username: Option<&str>, host: &str, port: Option<u16>) -> Option<QuickTarget> {
        Some(QuickTarget {
            username: username.map(str::to_owned),
            host: host.to_owned(),
            port,
        })
    }

    #[test]
    fn the_four_forms_of_ui_2_1() {
        assert_eq!(
            parse_address("db.example.com"),
            target(None, "db.example.com", None)
        );
        assert_eq!(
            parse_address("root@db.example.com"),
            target(Some("root"), "db.example.com", None)
        );
        assert_eq!(
            parse_address("root@db.example.com:2222"),
            target(Some("root"), "db.example.com", Some(2222))
        );
        assert_eq!(
            parse_address("ssh root@db.example.com -p 2222"),
            target(Some("root"), "db.example.com", Some(2222))
        );
        assert_eq!(
            parse_address("  ssh -p2200 -l admin 192.0.2.7 "),
            target(Some("admin"), "192.0.2.7", Some(2200))
        );
        assert_eq!(parse_address("ssh prod"), target(None, "prod", None));
    }

    #[test]
    fn single_words_need_a_dot_colon_ip_or_localhost() {
        assert_eq!(parse_address("prod"), None);
        assert_eq!(parse_address("web-server"), None);
        assert_eq!(parse_address("prod.lan"), target(None, "prod.lan", None));
        assert_eq!(parse_address("10.0.0.1"), target(None, "10.0.0.1", None));
        assert_eq!(parse_address("localhost"), target(None, "localhost", None));
        assert_eq!(parse_address("LOCALHOST"), target(None, "LOCALHOST", None));
        assert_eq!(parse_address("prod:22"), target(None, "prod", Some(22)));
        assert_eq!(parse_address("::1"), target(None, "::1", None));
        assert_eq!(
            parse_address("admin@[2001:db8::1]:2222"),
            target(Some("admin"), "2001:db8::1", Some(2222))
        );
        // With a user the host form needs no dot.
        assert_eq!(
            parse_address("root@prod"),
            target(Some("root"), "prod", None)
        );
    }

    #[test]
    fn search_queries_are_not_addresses() {
        for query in [
            "",
            "   ",
            "web server",
            "prod db",
            "ssh",
            "ssh -v host.lan",
            "ssh host.lan uptime",
            "ssh host.lan -p",
            "host.lan:port",
            "host.lan:0",
            "host.lan:70000",
            "@host.lan",
            "user@",
            "user@:22",
            "a..b",
            "-bad.lan",
            "[::1",
            "[not-v6]:22",
            "us:er@host.lan",
            "ssh user@host.lan -p 22 extra",
            "ssh -é host.lan",
            "ssh -",
        ] {
            assert_eq!(parse_address(query), None, "{query:?}");
        }
    }

    #[test]
    fn titles() {
        assert_eq!(parse_address("root@db.lan").unwrap().title(), "root@db.lan");
        assert_eq!(parse_address("db.lan").unwrap().title(), "db.lan");
    }
}
