//! Export of the SSH hosts as `shellpp-ssh-config` and of the keys as
//! OpenSSH files (docs/storage-format.md, sections 13.4 and 13.5; night plan,
//! stage 3 item 11): aliases, effective values after inheritance and the
//! identity, comments for what OpenSSH cannot express, key files encrypted
//! with the export passphrase, no silent overwrite; and the round trip: the
//! exported folder read back by the `~/.ssh/config` import.

mod common;

use std::collections::BTreeMap;

use common::{first_run, save};
use secrecy::SecretString;
use serde_json::json;
use shellpp_core::keys::generate::KeyKind;
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::dto::{
    ImportStatus, KeyGenerateInput, RecordType, SshConfigImportInput, SshConfigSource,
};
use shellpp_core::service::keychain::IdentitySaveInput;
use shellpp_core::service::ssh_export::{CONFIG_FILE, SshFilesExportInput};

#[test]
fn hosts_and_keys_are_written_as_openssh_files_and_read_back() {
    let u = first_run();
    let vault = u.vault_id;
    let deploy = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "Deploy key".into(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let ops_key = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "ops".into(),
            kind: KeyKind::EcdsaP256,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let identity = u
        .core
        .identity_save(IdentitySaveInput {
            vault_id: vault,
            record_id: None,
            label: "ops".into(),
            username: "ops".into(),
            key_id: Some(ops_key.record_id),
            password: PasswordChange::Keep,
        })
        .unwrap();
    let group = save(
        &u.core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "prod", "ssh": {"username": "deploy", "port": 2222, "keepalive_s": 30}}),
        &[],
    )
    .unwrap();
    let bastion = save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({"label": "Bastion", "address": "bastion.example.com"}),
        &[],
    )
    .unwrap();
    let secret = format!("pw-{}", common::random_hex());
    save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({
            "label": "Web 1",
            "address": "web1.example.com",
            "group_id": group.record_id().to_string(),
            "ssh": {
                "key_id": deploy.record_id.to_string(),
                "jump_chain": [bastion.record_id().to_string()],
                "agent_forwarding": true,
                "env": [{"name": "LANG", "value": "en_US.UTF-8"}, {"name": "GREETING", "value": "hello world"}]
            }
        }),
        &[("ssh.password", Some(secret.as_str()))],
    )
    .unwrap();
    save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({"label": "Web 1", "address": "web2.example.com"}),
        &[],
    )
    .unwrap();
    save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({"label": "db", "address": "192.0.2.44", "ssh": {"identity_id": identity.record_id.to_string()}}),
        &[],
    )
    .unwrap();

    let home = tempfile::tempdir().unwrap();
    let ssh_dir = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    let passphrase = format!("export-{}", common::random_hex());
    let input = SshFilesExportInput {
        dir: ssh_dir.clone(),
        keys: true,
        passphrase: Some(SecretString::from(passphrase.clone())),
        overwrite: false,
    };
    let summary = u.core.ssh_files_export(input.clone()).expect("export");
    assert_eq!(summary.hosts, 4);
    assert_eq!(summary.keys, 2);
    let config = std::fs::read_to_string(ssh_dir.join(CONFIG_FILE)).unwrap();
    assert!(config.starts_with("# Exported by Shell++ "), "{config}");
    for line in [
        "# Group: prod",
        "Host web-1\n",
        "  HostName web1.example.com",
        "  Port 2222",
        "  User deploy",
        "  IdentityFile ~/.ssh/shellpp_deploy-key",
        "  ProxyJump bastion",
        "  ForwardAgent yes",
        "  ServerAliveInterval 30",
        "  SetEnv LANG=en_US.UTF-8",
        "  SetEnv GREETING=\"hello world\"",
        "Host web-1-2\n",
        "Host db\n",
        "  User ops",
        "  IdentityFile ~/.ssh/shellpp_ops",
    ] {
        assert!(config.contains(line), "{line:?} missing in:\n{config}");
    }
    assert!(!config.contains(&secret), "passwords are never written");
    assert!(!config.contains('\r'), "LF line endings");
    let private = std::fs::read_to_string(ssh_dir.join("shellpp_deploy-key")).unwrap();
    let parsed = ssh_key::PrivateKey::from_openssh(&private).unwrap();
    assert!(
        parsed.is_encrypted(),
        "encrypted with the export passphrase"
    );
    let public = std::fs::read_to_string(ssh_dir.join("shellpp_deploy-key.pub")).unwrap();
    assert!(public.trim_end().ends_with("Deploy key"), "{public}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |name: &str| {
            std::fs::metadata(ssh_dir.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("shellpp_deploy-key"), 0o600);
        assert_eq!(mode("shellpp_deploy-key.pub"), 0o644);
    }
    // Only the user can read the private key, on every platform (Windows: a
    // protected DACL with the user alone, D42).
    assert!(shellpp_private_file::is_private(&ssh_dir.join("shellpp_deploy-key")).unwrap());

    // Existing files are not replaced without the user's yes.
    assert!(u.core.ssh_files_export(input.clone()).is_err());
    let again = u
        .core
        .ssh_files_export(SshFilesExportInput {
            overwrite: true,
            ..input
        })
        .expect("overwrite");
    assert_eq!(again.hosts, 4);

    // The exported folder reads back through the ~/.ssh/config import. It
    // is not the user's own config, so its ForwardAgent comes back off (D193).
    let target = first_run();
    let source = SshConfigSource {
        path: ssh_dir.join(CONFIG_FILE),
        home: home.path().to_path_buf(),
        own: false,
    };
    let preview = target.core.ssh_config_preview(source.clone()).unwrap();
    // Rule 4 orders by group path first: the host without a group is
    // "web-1", the one in "prod" is "web-1-2".
    let web = preview
        .hosts
        .iter()
        .find(|host| host.address == "web1.example.com")
        .unwrap();
    assert_eq!(web.alias, "web-1-2");
    assert_eq!(web.status, ImportStatus::New);
    assert_eq!(web.address, "web1.example.com");
    assert_eq!(web.port, Some(2222));
    assert_eq!(web.username.as_deref(), Some("deploy"));
    assert_eq!(web.jump, ["bastion"]);
    let passphrases: BTreeMap<String, SecretString> = preview
        .keys
        .iter()
        .map(|key| (key.path.clone(), SecretString::from(passphrase.clone())))
        .collect();
    let imported = target
        .core
        .ssh_config_import(SshConfigImportInput {
            source,
            aliases: None,
            passphrases,
            save_passphrases: false,
        })
        .unwrap();
    assert_eq!(imported.added, 4);
    assert_eq!(imported.keys_added, 2);
    assert_eq!(imported.hosts_agent_forwarding_off, 1);
    let fingerprints = |core: &shellpp_core::Core| {
        let mut prints: Vec<String> = core
            .keys_list()
            .unwrap()
            .into_iter()
            .filter_map(|key| key.fingerprint)
            .collect();
        prints.sort();
        prints
    };
    assert_eq!(fingerprints(&target.core), fingerprints(&u.core));
}

#[test]
fn key_files_never_share_a_name() {
    let u = first_run();
    // "work.pub" would write shellpp_work.pub, the public key of "work";
    // "work-cert" would write shellpp_work-cert.pub, its certificate name.
    let mut fingerprints = Vec::new();
    for label in ["work", "work.pub", "work-cert"] {
        let key = u
            .core
            .key_generate(KeyGenerateInput {
                vault_id: u.vault_id,
                label: label.into(),
                kind: KeyKind::Ed25519,
                passphrase: None,
                save_passphrase: false,
            })
            .unwrap()
            .key;
        fingerprints.push(key.fingerprint.unwrap());
    }
    let home = tempfile::tempdir().unwrap();
    let summary = u
        .core
        .ssh_files_export(SshFilesExportInput {
            dir: home.path().to_path_buf(),
            keys: true,
            passphrase: None,
            overwrite: false,
        })
        .expect("export");
    assert_eq!(summary.keys, 3);
    let names: std::collections::HashSet<_> = summary.files.iter().collect();
    assert_eq!(names.len(), summary.files.len(), "{:?}", summary.files);
    // Every private key file holds another key.
    let mut written = Vec::new();
    for path in &summary.files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("shellpp_") && !name.ends_with(".pub") {
            let text = std::fs::read_to_string(path).unwrap();
            let key = ssh_key::PrivateKey::from_openssh(text.as_bytes()).unwrap();
            written.push(key.fingerprint(ssh_key::HashAlg::Sha256).to_string());
        }
    }
    written.sort();
    fingerprints.sort();
    assert_eq!(written, fingerprints);
}
