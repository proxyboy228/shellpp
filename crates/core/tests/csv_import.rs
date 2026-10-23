//! Import of hosts from CSV with a column mapping (SPEC "Import"; UI 2.5;
//! run 8 decision D141): detection of the character set, delimiter and
//! header; the preview's statuses against existing hosts (with inherited
//! port and username) and inside the file; group paths; tags, notes and a
//! password; a file without a header; a second import that adds nothing.

mod common;

use common::{as_group, as_host, first_run};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use shellpp_core::import::csv::Role;
use shellpp_core::service::dto::{
    CsvInput, HostView, ImportStatus, QueryScope, RecordType, RecordsQuery, SortMode,
};

fn input(bytes: Vec<u8>) -> CsvInput {
    CsvInput {
        bytes: zeroize::Zeroizing::new(bytes),
        ..CsvInput::default()
    }
}

fn hosts(u: &common::Unlocked) -> Vec<HostView> {
    u.core
        .records_query(RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Manual,
        })
        .unwrap()
        .into_iter()
        .map(as_host)
        .collect()
}

/// Windows-1251 bytes of `text` (Cyrillic and ASCII only).
fn cp1251(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c {
            'А'..='я' => (c as u32 - 'А' as u32 + 0xC0) as u8,
            c if c.is_ascii() => c as u8,
            other => panic!("{other}"),
        })
        .collect()
}

#[test]
fn rows_are_mapped_previewed_and_imported_once() {
    let u = first_run();
    // An existing group whose hosts inherit port 2222 and user "ops", and
    // a host in it without its own port or user.
    let ops = u.group(json!({"name": "Ops", "ssh": {"port": 2222, "username": "ops"}}));
    u.host(json!({"address": "db.example.com", "group_id": ops.record_id.to_string()}));
    let password = common::random_hex();
    let text = format!(
        "Имя;Host;Port;User;Group;Tags;Notes;Password\r\n\
         Веб;web.example.com;22;deploy;Prod / Web;prod, web;первый;{password}\r\n\
         DB;DB.example.com;2222;ops;;;;\r\n\
         DB top;db.example.com;;;;;;\r\n\
         Again;web.example.com;;deploy;;;;\r\n\
         Bad port;x.example.com;99999;;;;;\r\n\
         No address;;22;;;;;\r\n\
         Bad address;two words;;;;;;\r\n\
         Deep;deep.example.com;;;Prod / Web / Blue;;;\r\n"
    );
    let bytes = cp1251(&text);

    let found = u.core.csv_inspect(&input(bytes.clone())).expect("inspect");
    assert_eq!(found.encoding, "windows-1251");
    assert_eq!(found.delimiter, ';');
    assert!(found.has_header);
    assert_eq!(found.columns[0], "Имя");
    assert_eq!(
        found.roles,
        [
            Role::Label,
            Role::Address,
            Role::Port,
            Role::Username,
            Role::Group,
            Role::Tags,
            Role::Notes,
            Role::Password
        ]
    );
    assert_eq!(found.rows, 8);
    assert_eq!(found.sample.len(), 5);
    assert_eq!(found.sample[0][0], "Веб");

    // The mapping as detected ("Имя" is the label).
    let mapped = CsvInput {
        roles: Some(found.roles.clone()),
        ..input(bytes)
    };
    let preview = u.core.csv_preview(&mapped).expect("preview");
    let statuses: Vec<(usize, ImportStatus)> = preview
        .rows
        .iter()
        .map(|row| (row.line, row.status))
        .collect();
    assert_eq!(
        statuses,
        [
            (2, ImportStatus::New),
            // DB.example.com:2222 as ops = the host in Ops (inherited).
            (3, ImportStatus::Exists),
            // Port 22 and no user at the top level: another host.
            (4, ImportStatus::New),
            // The same address, port and user as line 2.
            (5, ImportStatus::Exists),
            (6, ImportStatus::Error),
            (7, ImportStatus::Error),
            (8, ImportStatus::Error),
            (9, ImportStatus::New),
        ]
    );
    let reason = |line: usize| {
        preview
            .rows
            .iter()
            .find(|row| row.line == line)
            .unwrap()
            .reason
            .clone()
            .unwrap_or_default()
    };
    assert_eq!(reason(6), "the port is not valid");
    assert_eq!(reason(7), "the address is empty");
    assert_eq!(reason(8), "invalid address");
    assert_eq!(preview.groups_new, 3, "Prod, Prod / Web, Prod / Web / Blue");
    assert_eq!(preview.passwords, 1);
    assert_eq!(preview.rows[0].group.as_deref(), Some("Prod / Web"));

    let summary = u.core.csv_import(&mapped).expect("import");
    assert_eq!(summary.added, 3);
    assert_eq!(summary.skipped, 2);
    assert_eq!(summary.groups_added, 3);
    assert_eq!(
        summary
            .errors
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [6, 7, 8]
    );

    let all = hosts(&u);
    assert_eq!(all.len(), 4);
    let data = |label: &str| -> Value {
        all.iter()
            .find(|host| host.data["label"] == label)
            .unwrap_or_else(|| panic!("{label}"))
            .data
            .clone()
    };
    let web = data("Веб");
    assert_eq!(web["address"], "web.example.com");
    assert_eq!(web["ssh"]["port"], 22);
    assert_eq!(web["ssh"]["username"], "deploy");
    assert_eq!(web["tags"], json!(["prod", "web"]));
    assert_eq!(web["notes"], "первый");
    assert!(web.get("import_key").is_none());
    // The password is the host's secret.
    assert!(!web.to_string().contains(&password));
    let export = u.core.data_export(Default::default()).unwrap();
    assert!(export.text.expose_secret().contains(&password));
    // Groups: Prod > Web > Blue, each created once.
    let groups: Vec<_> = u
        .core
        .records_query(RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Group],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Manual,
        })
        .unwrap()
        .into_iter()
        .map(as_group)
        .collect();
    let group = |name: &str| {
        groups
            .iter()
            .find(|group| group.data["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    assert_eq!(groups.len(), 4);
    assert_eq!(
        group("Web").data["parent_id"],
        json!(group("Prod").record_id.to_string())
    );
    assert_eq!(
        group("Blue").data["parent_id"],
        json!(group("Web").record_id.to_string())
    );
    assert_eq!(web["group_id"], json!(group("Web").record_id.to_string()));
    assert_eq!(
        data("Deep")["group_id"],
        json!(group("Blue").record_id.to_string())
    );

    // A second import adds nothing: every row exists or fails.
    let again = u.core.csv_import(&mapped).expect("again");
    assert_eq!(again.added, 0);
    assert_eq!(again.groups_added, 0);
    assert_eq!(again.skipped, 5);
    assert_eq!(hosts(&u).len(), 4);
}

#[test]
fn a_file_without_a_header_takes_the_spec_order() {
    let u = first_run();
    let bytes = b"web,web.example.com,2200,deploy,Lab,\"a;b\"\nsolo,192.0.2.9\n".to_vec();
    let found = u.core.csv_inspect(&input(bytes.clone())).unwrap();
    assert_eq!(found.encoding, "utf-8");
    assert_eq!(found.delimiter, ',');
    assert!(!found.has_header);
    assert_eq!(found.columns[0], "Column 1");
    assert_eq!(found.roles[..6], Role::ORDER);
    assert_eq!(found.rows, 2);
    let summary = u.core.csv_import(&input(bytes)).unwrap();
    assert_eq!(summary.added, 2);
    assert_eq!(summary.groups_added, 1);
    let all = hosts(&u);
    let web = all.iter().find(|host| host.data["label"] == "web").unwrap();
    assert_eq!(web.data["ssh"]["port"], 2200);
    assert_eq!(web.data["tags"], json!(["a", "b"]));

    // A header row can be turned off or on by the user: here the first
    // row read as a header leaves one data row.
    let found = u
        .core
        .csv_inspect(&CsvInput {
            has_header: Some(true),
            ..input(b"web,web.example.com\nsolo,192.0.2.9\n".to_vec())
        })
        .unwrap();
    assert!(found.has_header);
    assert_eq!(found.rows, 1);
    assert_eq!(found.columns, ["web", "web.example.com"]);
}

#[test]
fn a_mapping_without_an_address_and_bad_settings_are_refused() {
    let u = first_run();
    let bytes = b"label,address\nweb,web.example.com\n".to_vec();
    let no_address = CsvInput {
        roles: Some(vec![Role::Label, Role::Ignore]),
        ..input(bytes.clone())
    };
    assert!(u.core.csv_preview(&no_address).is_err());
    assert!(u.core.csv_import(&no_address).is_err());
    let bad_encoding = CsvInput {
        encoding: Some("klingon".into()),
        ..input(bytes.clone())
    };
    assert!(u.core.csv_inspect(&bad_encoding).is_err());
    let bad_delimiter = CsvInput {
        delimiter: Some('"'),
        ..input(bytes)
    };
    assert!(u.core.csv_inspect(&bad_delimiter).is_err());
    let huge = input(vec![b'a'; 16 * 1024 * 1024 + 1]);
    assert!(u.core.csv_inspect(&huge).is_err());
    // A UTF-16 file with its byte order mark.
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "label,address\nweb,web.example.com\n".encode_utf16() {
        utf16.extend(unit.to_le_bytes());
    }
    let found = u.core.csv_inspect(&input(utf16)).unwrap();
    assert_eq!(found.encoding, "utf-16le");
    assert_eq!(found.sample[0], ["web", "web.example.com"]);
}

/// Review of D141: a bracketed IPv6 address is matched as it is stored; a
/// row under a partly new group path inherits from the deepest existing
/// group; an unclosed quote refuses the file.
#[test]
fn stored_forms_and_partly_new_group_paths_are_matched() {
    let u = first_run();
    // Prod sets the user "deploy"; 10.0.0.5 in Prod has none of its own.
    let prod = u.group(json!({"name": "Prod", "ssh": {"username": "deploy"}}));
    u.host(json!({"address": "10.0.0.5", "group_id": prod.record_id.to_string()}));
    let bytes = b"address,group\n[2001:db8::10],\n10.0.0.5,Prod / Web\n10.0.0.5,\n".to_vec();
    let preview = u.core.csv_preview(&input(bytes.clone())).unwrap();
    let statuses: Vec<ImportStatus> = preview.rows.iter().map(|row| row.status).collect();
    assert_eq!(
        statuses,
        [
            ImportStatus::New,
            // Under Prod / Web it inherits "deploy" from Prod: the same host.
            ImportStatus::Exists,
            // At the top level it has no user: another host.
            ImportStatus::New,
        ]
    );
    assert_eq!(preview.groups_new, 0);
    let summary = u.core.csv_import(&input(bytes.clone())).unwrap();
    assert_eq!(summary.added, 2);
    // The address is stored without brackets; a second import matches it.
    let again = u.core.csv_import(&input(bytes)).unwrap();
    assert_eq!(again.added, 0);
    assert_eq!(again.skipped, 3);
    assert!(
        hosts(&u)
            .iter()
            .any(|host| host.data["address"] == "2001:db8::10")
    );

    // A quote never closed: the rest of the file would be one field.
    let open = b"label,address,notes,password\nweb,web.example.com,\"see wiki,pw1\ndb,db.example.com,,pw2\n"
        .to_vec();
    assert!(u.core.csv_inspect(&input(open.clone())).is_err());
    assert!(u.core.csv_import(&input(open)).is_err());
    assert_eq!(hosts(&u).len(), 3);
}
