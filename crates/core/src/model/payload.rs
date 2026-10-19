//! The plaintext payload wrapper (docs/storage-format.md, sections 5.5 and 5.6).
//!
//! ```json
//! {"v": 1, "type": "host", "hlc": "0199f6a1b2c30001", "device_id": "0199...", "data": {}}
//! ```
//!
//! Serialized as compact UTF-8 JSON. A tombstone has no `data` and
//! `"deleted": true`.

use std::fmt;
use std::io;

use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::model::hlc::Hlc;
use crate::model::wipe::{WipeOnDrop, wipe_map, wipe_value};

/// A decoded payload. `data` holds decrypted record fields, so `Debug` prints
/// only the wrapper.
#[derive(Clone, PartialEq)]
pub struct Payload {
    pub v: u32,
    pub type_name: String,
    pub hlc: Hlc,
    pub device_id: Uuid,
    pub data: Option<Map<String, Value>>,
    pub deleted: bool,
}

/// Why a payload could not be decoded (the record is damaged).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PayloadError {
    #[error("payload is not valid JSON")]
    NotJson,
    #[error("payload is not a JSON object")]
    NotObject,
    #[error("payload member is missing or malformed: {0}")]
    BadMember(&'static str),
}

#[derive(Serialize)]
struct PayloadOut<'a> {
    v: u32,
    #[serde(rename = "type")]
    type_name: &'a str,
    hlc: String,
    device_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a Map<String, Value>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    deleted: bool,
}

impl Payload {
    /// A live payload.
    pub fn live(
        type_name: &str,
        v: u32,
        hlc: Hlc,
        device_id: Uuid,
        data: Map<String, Value>,
    ) -> Self {
        Self {
            v,
            type_name: type_name.to_owned(),
            hlc,
            device_id,
            data: Some(data),
            deleted: false,
        }
    }

    /// A tombstone payload: no `data`, `deleted: true`.
    pub fn tombstone(type_name: &str, v: u32, hlc: Hlc, device_id: Uuid) -> Self {
        Self {
            v,
            type_name: type_name.to_owned(),
            hlc,
            device_id,
            data: None,
            deleted: true,
        }
    }

    /// Compact UTF-8 JSON.
    pub fn to_json_bytes(&self) -> Zeroizing<Vec<u8>> {
        let out = PayloadOut {
            v: self.v,
            type_name: &self.type_name,
            hlc: self.hlc.to_text(),
            device_id: self.device_id,
            data: if self.deleted {
                None
            } else {
                self.data.as_ref()
            },
            deleted: self.deleted,
        };
        // Two passes: the length first, then one buffer of exactly that size.
        // A growing Vec would leave partial copies of the plaintext (secrets
        // included) in the freed smaller buffers. Serializing plain JSON
        // values cannot fail.
        let mut counter = ByteCounter(0);
        let _ = serde_json::to_writer(&mut counter, &out);
        let mut bytes = Zeroizing::new(Vec::with_capacity(counter.0));
        let _ = serde_json::to_writer(&mut *bytes, &out);
        bytes
    }

    /// Decodes and checks the wrapper members (check 6 of storage-format 5.4:
    /// valid UTF-8 JSON and an object).
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, PayloadError> {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| PayloadError::NotJson)?;
        // The decrypted members are wiped on every return path.
        let mut map = match value {
            Value::Object(map) => WipeOnDrop::new(map),
            mut other => {
                wipe_value(&mut other);
                return Err(PayloadError::NotObject);
            }
        };
        let v = map
            .get("v")
            .and_then(Value::as_u64)
            .filter(|v| *v >= 1)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(PayloadError::BadMember("v"))?;
        let type_name = map
            .get("type")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or(PayloadError::BadMember("type"))?
            .to_owned();
        let hlc = map
            .get("hlc")
            .and_then(Value::as_str)
            .and_then(Hlc::parse)
            .ok_or(PayloadError::BadMember("hlc"))?;
        let device_id = map
            .get("device_id")
            .and_then(Value::as_str)
            .and_then(|text| Uuid::parse_str(text).ok())
            .ok_or(PayloadError::BadMember("device_id"))?;
        let deleted = match map.get("deleted") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(true)) => true,
            Some(_) => return Err(PayloadError::BadMember("deleted")),
        };
        let data = match map.remove("data") {
            None | Some(Value::Null) => None,
            Some(Value::Object(data)) => Some(data),
            Some(mut other) => {
                wipe_value(&mut other);
                return Err(PayloadError::BadMember("data"));
            }
        };
        if !deleted && data.is_none() {
            // A live payload always carries `data`.
            return Err(PayloadError::BadMember("data"));
        }
        drop(map);
        Ok(Self {
            v,
            type_name,
            hlc,
            device_id,
            data,
            deleted,
        })
    }

    /// The `(hlc, device_id)` stamp used for last-writer-wins and singletons.
    pub fn stamp(&self) -> (Hlc, [u8; 16]) {
        (self.hlc, *self.device_id.as_bytes())
    }
}

impl Drop for Payload {
    fn drop(&mut self) {
        if let Some(data) = self.data.as_mut() {
            wipe_map(data);
        }
    }
}

/// An `io::Write` that only counts bytes (first pass of [`Payload::to_json_bytes`]).
struct ByteCounter(usize);

impl io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Payload")
            .field("v", &self.v)
            .field("type", &self.type_name)
            .field("hlc", &self.hlc)
            .field("device_id", &self.device_id)
            .field("deleted", &self.deleted)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn device() -> Uuid {
        Uuid::parse_str("0199f6a1-b2c3-7d4e-8f00-112233445566").unwrap()
    }

    #[test]
    fn round_trip_is_compact() {
        let mut data = Map::new();
        data.insert("address".into(), json!("db1.example.com"));
        let payload = Payload::live("host", 1, Hlc::new(0x0199_f6a1_b2c3, 1), device(), data);
        let bytes = payload.to_json_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(
            text,
            r#"{"v":1,"type":"host","hlc":"0199f6a1b2c30001","device_id":"0199f6a1-b2c3-7d4e-8f00-112233445566","data":{"address":"db1.example.com"}}"#
        );
        assert_eq!(Payload::from_json_bytes(&bytes).unwrap(), payload);
    }

    #[test]
    fn serializes_into_one_buffer_of_the_exact_size() {
        // Larger than the first growth steps of a Vec: a growing buffer
        // would have been reallocated (and the old copies freed unwiped).
        let mut data = Map::new();
        data.insert("notes".into(), json!("n".repeat(5000)));
        data.insert("ssh".into(), json!({"password": "p".repeat(300)}));
        let payload = Payload::live("host", 1, Hlc::new(1, 0), device(), data);
        let bytes = payload.to_json_bytes();
        assert!(bytes.len() > 5300);
        assert_eq!(bytes.capacity(), bytes.len());
        assert_eq!(Payload::from_json_bytes(&bytes).unwrap(), payload);
    }

    #[test]
    fn tombstone_has_no_data() {
        let payload = Payload::tombstone("host", 1, Hlc::new(1, 0), device());
        let bytes = payload.to_json_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.ends_with(r#""deleted":true}"#));
        assert!(!text.contains("data"));
        let back = Payload::from_json_bytes(&bytes).unwrap();
        assert!(back.deleted);
        assert!(back.data.is_none());
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        assert_eq!(
            Payload::from_json_bytes(b"not json"),
            Err(PayloadError::NotJson)
        );
        assert_eq!(
            Payload::from_json_bytes(b"[1]"),
            Err(PayloadError::NotObject)
        );
        let base = json!({"v": 1, "type": "host", "hlc": "0000000000010000",
            "device_id": "0199f6a1-b2c3-7d4e-8f00-112233445566", "data": {}});
        assert!(Payload::from_json_bytes(base.to_string().as_bytes()).is_ok());
        for (member, value) in [
            ("v", json!(0)),
            ("v", json!("1")),
            ("type", json!("")),
            ("hlc", json!("XYZ")),
            ("device_id", json!("nope")),
            ("data", json!([1])),
            ("deleted", json!(false)),
        ] {
            let mut bad = base.clone();
            bad[member] = value;
            assert!(
                matches!(
                    Payload::from_json_bytes(bad.to_string().as_bytes()),
                    Err(PayloadError::BadMember(_))
                ),
                "{member}"
            );
        }
        let mut no_data = base.clone();
        no_data.as_object_mut().unwrap().remove("data");
        assert!(Payload::from_json_bytes(no_data.to_string().as_bytes()).is_err());
    }

    #[test]
    fn debug_hides_data() {
        let mut data = Map::new();
        data.insert("address".into(), json!("db1.example.com"));
        let payload = Payload::live("host", 1, Hlc::new(1, 0), device(), data);
        assert!(!format!("{payload:?}").contains("example.com"));
    }
}
