use std::collections::{BTreeMap, HashMap};

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use shellpp_core::import::csv::MAX_FILE_BYTES;
use shellpp_core::service::dto::{KeyImportInput, VaultKind, VaultStatus};
use shellpp_core::{Core, CoreError};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::strings;

/// Adapt the documented Termix host export to the existing CSV preview and
/// import. Passwords use the same encrypted host secret as CSV imports.
pub(super) struct TermixExport {
    pub bytes: Zeroizing<Vec<u8>>,
    pub keys: BTreeMap<usize, TermixKey>,
}

pub(super) struct TermixKey {
    private_key: SecretString,
    passphrase: Option<SecretString>,
}

pub(super) fn import_keys(
    core: &Core,
    keys: &BTreeMap<usize, TermixKey>,
) -> Result<BTreeMap<usize, Option<Uuid>>, CoreError> {
    let vault_id = core
        .vaults()?
        .into_iter()
        .find(|vault| vault.kind == VaultKind::Personal && vault.status == VaultStatus::Active)
        .ok_or(CoreError::Locked)?
        .vault_id;
    let mut imported = HashMap::new();
    Ok(keys
        .iter()
        .map(|(&row, key)| {
            let fingerprint_input = (
                key.private_key.expose_secret(),
                key.passphrase.as_ref().map(|secret| secret.expose_secret()),
            );
            let id = imported.entry(fingerprint_input).or_insert_with(|| {
                core.key_import(KeyImportInput {
                    vault_id,
                    label: None,
                    private_key: key.private_key.clone(),
                    passphrase: key.passphrase.clone(),
                    save_passphrase: true,
                })
                .ok()
                .map(|result| result.key.record_id)
            });
            (row, *id)
        })
        .collect())
}

pub(super) fn hosts_as_csv(bytes: &[u8]) -> Result<TermixExport, &'static str> {
    let export: Value = serde_json::from_slice(bytes).map_err(|_| strings::TERMIX_INVALID)?;
    let hosts = export
        .get("hosts")
        .and_then(Value::as_array)
        .ok_or(strings::TERMIX_INVALID)?;
    let mut csv =
        Zeroizing::new(b"label,address,port,username,group,tags,notes,password\n".to_vec());

    let mut keys = BTreeMap::new();
    let mut row = 1;
    for host in hosts {
        if !host.is_object() {
            return Err(strings::TERMIX_INVALID);
        }
        if host["connectionType"]
            .as_str()
            .is_some_and(|kind| kind != "ssh")
        {
            continue;
        }
        row += 1;
        if let Some(private_key) = secret(host, "key") {
            keys.insert(
                row,
                TermixKey {
                    private_key,
                    passphrase: secret(host, "keyPassword"),
                },
            );
        }
        let port = host["port"].as_u64().map(|port| port.to_string());
        let tags = host["tags"].as_array().map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(";")
        });
        let fields = [
            field(host, &["name"]),
            field(host, &["ip", "address", "host", "hostname"]),
            port.as_deref()
                .or_else(|| host["port"].as_str())
                .unwrap_or("22"),
            field(host, &["username", "user"]),
            field(host, &["folder", "group"]),
            tags.as_deref().unwrap_or_default(),
            field(host, &["notes"]),
            field(host, &["password"]),
        ];
        for (index, field) in fields.into_iter().enumerate() {
            if index != 0 {
                csv.push(b',');
            }
            csv.push(b'"');
            for byte in field.bytes() {
                csv.push(byte);
                if byte == b'"' {
                    csv.push(byte);
                }
            }
            csv.push(b'"');
        }
        csv.push(b'\n');
        if csv.len() > MAX_FILE_BYTES {
            return Err(strings::CSV_TOO_LARGE);
        }
    }
    Ok(TermixExport { bytes: csv, keys })
}

fn secret(host: &Value, name: &str) -> Option<SecretString> {
    host[name]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| SecretString::from(value.to_owned()))
}

fn field<'a>(host: &'a Value, names: &[&str]) -> &'a str {
    names
        .iter()
        .find_map(|name| host[*name].as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellpp_core::import::csv::records;

    #[test]
    fn converts_ssh_hosts_and_passwords() {
        let csv = hosts_as_csv(
            br#"{"hosts":[
            {"name":"A, \"B\"","hostname":"example.test","user":"ops",
             "folder":"Prod/Web","tags":["web","linux"],"notes":"One\nTwo",
             "password":" secret \"quoted\" ","key":"private"},
            {"connectionType":"rdp","ip":"rdp.example.test"}
        ]}"#,
        )
        .unwrap();
        let text = std::str::from_utf8(&csv.bytes).unwrap();
        assert!(csv.keys.contains_key(&2));
        let parsed = records(text, ',');
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(
            parsed.rows[1],
            [
                "A, \"B\"",
                "example.test",
                "22",
                "ops",
                "Prod/Web",
                "web;linux",
                "One\nTwo",
                " secret \"quoted\" "
            ]
        );
        assert!(!text.contains("private"));
    }
}
