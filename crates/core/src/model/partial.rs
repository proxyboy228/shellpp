//! Partial updates of a record (D220; docs/data-model.md, section 18.1): an
//! editor's save writes only the members it changed onto the stored record,
//! so a version that arrived meanwhile (a pull, another window) keeps every
//! member the edit did not touch.
//!
//! The members are compared as data objects: a member that is an object on
//! both sides is compared member by member, anything else (strings, numbers,
//! arrays such as tags or the jump chain) as one value. A `null` member is
//! the same as a missing one, as for validation ([`fields::present`]).
//!
//! [`fields::present`]: crate::model::fields::present

use serde_json::{Map, Value};

/// A member path, from the top of the data object down.
pub type Path = Vec<String>;

/// One changed member: its path and its new value (`None`: removed).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: Path,
    pub value: Option<Value>,
}

/// The members `edited` changes against `base`.
pub fn changes(base: &Map<String, Value>, edited: &Map<String, Value>) -> Vec<Change> {
    let mut out = Vec::new();
    diff_level(base, edited, &mut Vec::new(), &mut out);
    out
}

fn diff_level(
    base: &Map<String, Value>,
    edited: &Map<String, Value>,
    prefix: &mut Path,
    out: &mut Vec<Change>,
) {
    let mut keys: Vec<&String> = base.keys().collect();
    keys.extend(edited.keys().filter(|key| !base.contains_key(*key)));
    for key in keys {
        let old = present(base.get(key));
        let new = present(edited.get(key));
        prefix.push(key.clone());
        match (old, new) {
            (Some(Value::Object(old)), Some(Value::Object(new))) => {
                diff_level(old, new, prefix, out);
            }
            _ if old == new => {}
            _ => out.push(Change {
                path: prefix.clone(),
                value: new.cloned(),
            }),
        }
        prefix.pop();
    }
}

/// The member at `path`, `None` when it or an object on the way is missing.
pub fn value_at<'a>(data: &'a Map<String, Value>, path: &[String]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut level = data;
    for key in parents {
        level = present(level.get(key))?.as_object()?;
    }
    present(level.get(last))
}

/// Applies `changes` to `target`: a removed member is set to `null` (the
/// record builders then drop it, unknown members included), and a missing or
/// non-object level on the way becomes an object.
pub fn apply(target: &mut Map<String, Value>, changes: &[Change]) {
    for change in changes {
        let Some((last, parents)) = change.path.split_last() else {
            continue;
        };
        let mut level = &mut *target;
        for key in parents {
            let slot = level
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !slot.is_object() {
                *slot = Value::Object(Map::new());
            }
            level = slot.as_object_mut().expect("made an object above");
        }
        level.insert(last.clone(), change.value.clone().unwrap_or(Value::Null));
    }
}

/// The paths of `changes` that `current` also changed since `base`, to
/// another value than the edit's: the edit overwrites them, so the version
/// in `current` has to be kept as a conflict copy.
pub fn conflicts(
    base: &Map<String, Value>,
    current: &Map<String, Value>,
    changes: &[Change],
) -> Vec<Path> {
    changes
        .iter()
        .filter(|change| {
            let now = value_at(current, &change.path);
            now != value_at(base, &change.path) && now != change.value.as_ref()
        })
        .map(|change| change.path.clone())
        .collect()
}

/// `null` is the same as a missing member.
fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn path(text: &str) -> Path {
        text.split('.').map(str::to_owned).collect()
    }

    #[test]
    fn changes_go_down_objects_and_take_other_values_whole() {
        let base = map(json!({
            "label": "web",
            "address": "10.0.0.1",
            "tags": ["a", "b"],
            "ssh": {"port": 22, "username": "root", "proxy": {"type": "http"}},
            "notes": "x"
        }));
        let edited = map(json!({
            "label": "web",
            "address": "10.0.0.2",
            "tags": ["a"],
            "ssh": {"port": 22, "username": null, "proxy": {"type": "http", "port": 8080}},
            "color": "blue"
        }));
        let mut found = changes(&base, &edited);
        found.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            found,
            vec![
                Change {
                    path: path("address"),
                    value: Some(json!("10.0.0.2"))
                },
                Change {
                    path: path("color"),
                    value: Some(json!("blue"))
                },
                Change {
                    path: path("notes"),
                    value: None
                },
                Change {
                    path: path("ssh.proxy.port"),
                    value: Some(json!(8080))
                },
                Change {
                    path: path("ssh.username"),
                    value: None
                },
                Change {
                    path: path("tags"),
                    value: Some(json!(["a"]))
                },
            ]
        );
    }

    #[test]
    fn null_and_missing_are_the_same() {
        let base = map(json!({"label": null, "ssh": {"port": 22}}));
        let edited = map(json!({"ssh": {"port": 22, "username": null}}));
        assert!(changes(&base, &edited).is_empty());
    }

    #[test]
    fn an_object_replaced_by_a_value_is_one_change() {
        let base = map(json!({"ssh": {"proxy": {"type": "http"}}}));
        let edited = map(json!({"ssh": {"proxy": null}}));
        assert_eq!(
            changes(&base, &edited),
            vec![Change {
                path: path("ssh.proxy"),
                value: None
            }]
        );
    }

    #[test]
    fn apply_writes_only_the_changes() {
        let mut current = map(json!({
            "label": "pulled",
            "address": "10.0.0.1",
            "ssh": {"port": 2222, "username": "root"}
        }));
        let edit = vec![
            Change {
                path: path("address"),
                value: Some(json!("10.0.0.2")),
            },
            Change {
                path: path("ssh.username"),
                value: None,
            },
            Change {
                path: path("ssh.proxy.port"),
                value: Some(json!(8080)),
            },
        ];
        apply(&mut current, &edit);
        assert_eq!(
            Value::Object(current),
            json!({
                "label": "pulled",
                "address": "10.0.0.2",
                "ssh": {"port": 2222, "username": null, "proxy": {"port": 8080}}
            })
        );
    }

    #[test]
    fn apply_replaces_a_value_on_the_way_by_an_object() {
        let mut current = map(json!({"ssh": "broken"}));
        apply(
            &mut current,
            &[Change {
                path: path("ssh.port"),
                value: Some(json!(22)),
            }],
        );
        assert_eq!(Value::Object(current), json!({"ssh": {"port": 22}}));
    }

    #[test]
    fn conflicts_are_the_members_both_changed_differently() {
        let base = map(json!({"label": "a", "address": "h", "ssh": {"port": 22}}));
        let current = map(json!({"label": "b", "address": "h2", "ssh": {"port": 23}}));
        let edit = vec![
            // The other side changed it too, to another value.
            Change {
                path: path("label"),
                value: Some(json!("c")),
            },
            // The other side changed it to the same value.
            Change {
                path: path("address"),
                value: Some(json!("h2")),
            },
            // Only the edit changed it.
            Change {
                path: path("ssh.username"),
                value: Some(json!("root")),
            },
        ];
        assert_eq!(conflicts(&base, &current, &edit), vec![path("label")]);
    }

    #[test]
    fn a_removed_parent_on_the_other_side_conflicts() {
        let base = map(json!({"ssh": {"proxy": {"type": "http", "port": 1}}}));
        let current = map(json!({"ssh": {}}));
        let edit = vec![Change {
            path: path("ssh.proxy.port"),
            value: Some(json!(2)),
        }];
        assert_eq!(
            conflicts(&base, &current, &edit),
            vec![path("ssh.proxy.port")]
        );
    }
}
