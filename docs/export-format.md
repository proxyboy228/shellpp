# Export format

The native JSON export of Shell++ (SPEC.md, principle 4): what the app's
Export data writes and what Import data reads. The format ids, the
encryption and the default file names are defined in
[storage-format.md](storage-format.md), sections 13.1–13.3, and the shapes
of the records' `data` in [data-model.md](data-model.md); this document
gives the JSON Schemas of both files and the import rules (night decision
D34 in [decisions.md](decisions.md)).

## 1. Files

| File | `format` | Content |
|---|---|---|
| `shellpp-export-<UTC yyyymmddThhmmssZ>.json` | `shellpp.export.v1` | every record of the personal vault except the recovery key, in clear text (passwords, private keys and passphrases included), and the app settings |
| `shellpp-export-<UTC yyyymmddThhmmssZ>.encrypted.json` | `shellpp.export.encrypted.v1` | the plain export, encrypted with an export password |

Both are UTF-8 without a byte order mark, with LF line endings, indented
with 2 spaces. The app shows a warning before it writes the plain file,
because it holds every secret in clear text. The export password is at
least 8 characters (Unicode characters after NFC), like the master password
(D29), and is never stored.

## 2. Plain export: JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:shellpp:schema:export:v1",
  "title": "Shell++ export, version 1",
  "type": "object",
  "required": ["format", "exported_at", "app_version", "vaults"],
  "properties": {
    "format": {"const": "shellpp.export.v1"},
    "exported_at": {"type": "string", "description": "RFC 3339 UTC time of the export, milliseconds, Z"},
    "app_version": {"type": "string"},
    "vaults": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["vault_id", "kind", "records"],
        "properties": {
          "vault_id": {"type": "string", "format": "uuid"},
          "kind": {"enum": ["personal", "team"]},
          "name": {"type": "string"},
          "records": {
            "type": "array",
            "items": {
              "type": "object",
              "required": ["record_id", "type", "data"],
              "properties": {
                "record_id": {"type": "string", "format": "uuid", "description": "UUIDv7"},
                "revision": {"type": "integer", "minimum": 1},
                "v": {"type": "integer", "minimum": 1, "description": "payload version of the type"},
                "type": {"type": "string", "description": "record type name, docs/data-model.md, section 2.3"},
                "hlc": {"type": "string", "pattern": "^[0-9a-f]{16}$"},
                "device_id": {"type": "string", "format": "uuid"},
                "data": {"type": "object", "description": "the record's data as defined for its type in docs/data-model.md"}
              }
            }
          }
        }
      }
    },
    "settings": {
      "type": "object",
      "description": "app settings (docs/data-model.md, section 16.1) by key; optional",
      "additionalProperties": true
    }
  },
  "additionalProperties": true
}
```

- Records are sorted by `record_id`. Within `v1`, new optional members may
  appear at any level and readers ignore members they do not know
  (storage-format.md, section 13.2); `settings` is such a member, added in
  stage 3.
- Exported: every live record of the personal vault (hosts, groups, keys,
  identities, known hosts, command history, the connection log, the
  vault's metadata, device records), except the recovery key.

## 3. Encrypted export: JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:shellpp:schema:export-encrypted:v1",
  "title": "Shell++ encrypted export, version 1",
  "type": "object",
  "required": ["format", "kdf", "salt", "nonce", "ciphertext"],
  "properties": {
    "format": {"const": "shellpp.export.encrypted.v1"},
    "kdf": {
      "type": "object",
      "required": ["alg", "v", "m_kib", "t", "p"],
      "properties": {
        "alg": {"const": "argon2id"},
        "v": {"const": 1},
        "m_kib": {"type": "integer"},
        "t": {"type": "integer"},
        "p": {"type": "integer"}
      }
    },
    "salt": {"type": "string", "description": "base64url without padding, 16 bytes"},
    "nonce": {"type": "string", "description": "base64url without padding, 24 bytes"},
    "ciphertext": {"type": "string", "description": "base64url without padding: ciphertext ‖ 16-byte tag"}
  },
  "additionalProperties": false
}
```

The key, the AEAD and the 55-byte associated data are those of
storage-format.md, section 13.3; section 14.7 there opens the file with
Python and the `cryptography` and `argon2-cffi` packages. A reader refuses
KDF parameters outside the bounds of storage-format.md, section 6.5 before
it derives anything.

## 4. Import

Import data reads both files (the encrypted one asks for its password) and
merges them into the personal vault (D34):

- Taken: groups, hosts, keys, identities and known hosts, each checked by
  the same rules as a save (a record that breaks them is listed with its
  errors and left out; the others are imported). Skipped and counted:
  command history, the connection log, device and vault records, and types
  this version does not know.
- A record whose id is new here is added under that id, so references
  between the imported records stay as they were.
- A record whose id exists here with the same content (its place in a
  manual order aside), a key with the same SHA256 fingerprint or a known
  host with the same entry is "already exists" and skipped; references to
  it point to the local record.
- A record whose id exists here with other content is a conflict: both
  versions are kept. The imported one gets a new id and the suffix
  " (imported)" on its label or name; references inside the import point
  to it. Existing records are never overwritten (D21).
- Group parents that would form a cycle or a tree deeper than 32 levels are
  cleared (the group goes to the top level).
- The settings of the file are applied, each key on its own (an unknown or
  invalid value is left out), unless the user turns that off.
- A file over 64 MiB or with more than 500,000 records is refused.

The summary reads "added N, existing M, conflicts C, skipped S, errors K"
(UI 2.5).
