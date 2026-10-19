//! Keeping members the app does not know (docs/data-model.md, section 1.2;
//! docs/storage-format.md, section 5.6): when a record is rewritten, unknown
//! members of the stored data and of the input are kept, the input winning,
//! and an input member set to `null` removes it.

use serde_json::{Map, Value};

/// Builds one level of a data object from the normalized known members plus
/// the unknown members of `stored` and `input`. `known` lists every member
/// the caller handles itself at this level (validated, managed or secret).
pub fn merge_level(
    known: &[&str],
    normalized: Map<String, Value>,
    input: Option<&Map<String, Value>>,
    stored: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(stored) = stored {
        for (key, value) in stored {
            if !known.contains(&key.as_str()) {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(input) = input {
        for (key, value) in input {
            if known.contains(&key.as_str()) {
                continue;
            }
            if value.is_null() {
                out.remove(key);
            } else {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    for (key, value) in normalized {
        out.insert(key, value);
    }
    out
}

/// Copies `keys` from `stored` into `out` (managed and secret members).
pub fn keep_from(out: &mut Map<String, Value>, stored: Option<&Map<String, Value>>, keys: &[&str]) {
    let Some(stored) = stored else {
        return;
    };
    for key in keys {
        if let Some(value) = stored.get(*key).filter(|value| !value.is_null()) {
            out.insert((*key).to_owned(), value.clone());
        }
    }
}

/// The object at `key`, if it is one.
pub fn object<'a>(
    map: Option<&'a Map<String, Value>>,
    key: &str,
) -> Option<&'a Map<String, Value>> {
    map.and_then(|map| map.get(key)).and_then(Value::as_object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn unknown_members_survive_and_input_wins() {
        let stored = map(json!({"name": "old", "x_future": 1, "x_gone": 2, "x_both": "stored"}));
        let input =
            map(json!({"name": "ignored", "x_gone": null, "x_both": "input", "x_new": true}));
        let normalized = map(json!({"name": "new"}));
        let out = merge_level(&["name"], normalized, Some(&input), Some(&stored));
        assert_eq!(
            Value::Object(out),
            json!({"name": "new", "x_future": 1, "x_both": "input", "x_new": true})
        );
    }

    #[test]
    fn keep_from_copies_present_members() {
        let stored = map(json!({"order_key": "V", "os": null}));
        let mut out = Map::new();
        keep_from(&mut out, Some(&stored), &["order_key", "os", "import_key"]);
        assert_eq!(Value::Object(out), json!({"order_key": "V"}));
    }
}
