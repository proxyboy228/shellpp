//! Best-effort wiping of decrypted JSON values (docs/security.md, sections 8
//! and 9.5): every string inside the value is zeroized in place before the
//! memory is released. Object keys are member names, not data, and numbers
//! cannot be wiped through the `serde_json` API; neither holds secrets.

use std::fmt;
use std::ops::{Deref, DerefMut};

use serde_json::{Map, Value};
use zeroize::Zeroize;

/// Zeroizes every string inside `value` and leaves `Value::Null`.
pub fn wipe_value(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => {
            for item in items.iter_mut() {
                wipe_value(item);
            }
            items.clear();
        }
        Value::Object(map) => wipe_map(map),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    *value = Value::Null;
}

/// Zeroizes every string value inside `map` and empties it.
pub fn wipe_map(map: &mut Map<String, Value>) {
    for (_, value) in map.iter_mut() {
        wipe_value(value);
    }
    map.clear();
}

/// Owns a map of decrypted data and wipes it on every drop path (early
/// returns and errors included). [`WipeOnDrop::into_inner`] hands the map on
/// to a new owner that is responsible for wiping it.
#[derive(Default)]
pub struct WipeOnDrop(Map<String, Value>);

impl WipeOnDrop {
    pub fn new(map: Map<String, Value>) -> Self {
        Self(map)
    }

    /// Takes the map out; the guard then wipes nothing.
    pub fn into_inner(mut self) -> Map<String, Value> {
        std::mem::take(&mut self.0)
    }
}

impl Deref for WipeOnDrop {
    type Target = Map<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WipeOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for WipeOnDrop {
    fn drop(&mut self) {
        wipe_map(&mut self.0);
    }
}

impl fmt::Debug for WipeOnDrop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WipeOnDrop(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wipes_nested_values() {
        let mut value = json!({"a": "secret", "b": [{"c": "x"}], "d": 5});
        wipe_value(&mut value);
        assert_eq!(value, Value::Null);
        let Value::Object(mut map) = json!({"a": "secret"}) else {
            unreachable!()
        };
        wipe_map(&mut map);
        assert!(map.is_empty());
    }

    #[test]
    fn guard_wipes_unless_taken() {
        let Value::Object(map) = json!({"password": "secret"}) else {
            unreachable!()
        };
        let mut guard = WipeOnDrop::new(map);
        guard.insert("extra".into(), json!("x"));
        assert_eq!(guard.get("password"), Some(&json!("secret")));
        let map = guard.into_inner();
        assert_eq!(map.len(), 2);
        assert_eq!(format!("{:?}", WipeOnDrop::new(map)), "WipeOnDrop(..)");
    }
}
