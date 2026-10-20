//! Group inheritance (docs/data-model.md, section 8).
//!
//! Only `port`, `username`, `identity_id`, `key_id` and `jump_chain` inherit.
//! The lookup walks the `ssh` blocks of the record's group, its parent and so
//! on (at most 32 groups, ending at a missing group or a group treated as top
//! level by the cycle rule); the first level that sets a field supplies it.
//! A reference to a missing record counts as set, and `jump_chain: []` stops
//! the lookup.
//!
//! The identity term of the resolution (section 8.3, stage 3):
//! `username = first(L.username ?? idf(L).username)` and the same for
//! `key_id`, where `idf(L)` is the identity the level names
//! ([`resolve_with`]). A level naming a missing identity adds nothing.

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::service::dto::{Inherited, InheritedValues};

/// Maximum number of group levels in the lookup chain.
pub const MAX_LEVELS: usize = 32;

/// Resolves inherited values from group data objects, nearest group first,
/// without identities.
pub fn resolve<'a, I>(levels: I) -> InheritedValues
where
    I: IntoIterator<Item = (Uuid, &'a Map<String, Value>)>,
{
    resolve_with(levels, |_| None)
}

/// [`resolve`] with the identity term: at each level its own `username` and
/// `key_id`, else those of the identity it names (`identity` reads the
/// identity's data by id).
pub fn resolve_with<'a, I, F>(levels: I, identity: F) -> InheritedValues
where
    I: IntoIterator<Item = (Uuid, &'a Map<String, Value>)>,
    F: Fn(&Uuid) -> Option<&'a Map<String, Value>>,
{
    let mut out = InheritedValues::default();
    for (group_id, data) in levels.into_iter().take(MAX_LEVELS) {
        let Some(ssh) = data.get("ssh").and_then(Value::as_object) else {
            continue;
        };
        let idf = uuid_member(ssh, "identity_id").and_then(|id| identity(&id));
        if out.port.is_none()
            && let Some(port) = ssh
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port >= 1)
        {
            out.port = Some(Inherited {
                value: port,
                from_group_id: group_id,
            });
        }
        if out.username.is_none()
            && let Some(username) = ssh
                .get("username")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .or_else(|| idf.and_then(super::identity::username))
        {
            out.username = Some(Inherited {
                value: username.to_owned(),
                from_group_id: group_id,
            });
        }
        if out.identity_id.is_none()
            && let Some(id) = uuid_member(ssh, "identity_id")
        {
            out.identity_id = Some(Inherited {
                value: id,
                from_group_id: group_id,
            });
        }
        if out.key_id.is_none()
            && let Some(id) =
                uuid_member(ssh, "key_id").or_else(|| idf.and_then(super::identity::key_id))
        {
            out.key_id = Some(Inherited {
                value: id,
                from_group_id: group_id,
            });
        }
        if out.jump_chain.is_none()
            && let Some(items) = ssh.get("jump_chain").and_then(Value::as_array)
        {
            let chain: Vec<Uuid> = items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|raw| Uuid::parse_str(raw).ok())
                .collect();
            out.jump_chain = Some(Inherited {
                value: chain,
                from_group_id: group_id,
            });
        }
    }
    out
}

fn uuid_member(ssh: &Map<String, Value>, key: &str) -> Option<Uuid> {
    ssh.get(key)
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn nearest_group_wins_per_field() {
        let near = Uuid::from_u128(1);
        let far = Uuid::from_u128(2);
        let identity = Uuid::from_u128(9);
        let near_data =
            map(json!({"name": "web", "ssh": {"username": "deploy", "jump_chain": []}}));
        let far_data = map(
            json!({"name": "prod", "ssh": {"port": 2222, "username": "root",
            "identity_id": identity.to_string(), "jump_chain": [Uuid::from_u128(5).to_string()]}}),
        );
        let inherited = resolve([(near, &near_data), (far, &far_data)]);
        assert_eq!(
            inherited.port,
            Some(Inherited {
                value: 2222,
                from_group_id: far
            })
        );
        assert_eq!(
            inherited.username,
            Some(Inherited {
                value: "deploy".into(),
                from_group_id: near
            })
        );
        assert_eq!(
            inherited.identity_id,
            Some(Inherited {
                value: identity,
                from_group_id: far
            })
        );
        assert_eq!(inherited.key_id, None);
        // `[]` at a level stops the lookup.
        assert_eq!(
            inherited.jump_chain,
            Some(Inherited {
                value: vec![],
                from_group_id: near
            })
        );
    }

    #[test]
    fn chain_is_limited_to_32_levels() {
        let data: Vec<Map<String, Value>> = (0..40)
            .map(|i| {
                if i == 35 {
                    map(json!({"ssh": {"port": 22}}))
                } else {
                    map(json!({"ssh": {}}))
                }
            })
            .collect();
        let levels: Vec<(Uuid, &Map<String, Value>)> = data
            .iter()
            .enumerate()
            .map(|(i, d)| (Uuid::from_u128(i as u128), d))
            .collect();
        assert_eq!(resolve(levels).port, None);
    }

    #[test]
    fn empty_chain_inherits_nothing() {
        assert_eq!(resolve(std::iter::empty()), InheritedValues::default());
    }
}
